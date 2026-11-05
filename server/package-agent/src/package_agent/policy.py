from __future__ import annotations

import re
import shlex
from dataclasses import dataclass


@dataclass(frozen=True)
class Action:
    kind: str
    command: list[str]
    touches_project_files: bool = False


@dataclass(frozen=True)
class PolicyDecision:
    allowed: bool
    reason: str


ALLOWED_KINDS = {"inspect", "build", "env_repair", "artifact_collect"}
# Checkout cleanup and local Git operations are allowed only in the Runner-managed checkout.
BLOCKED_GIT_COMMANDS = {"commit", "push"}
GIT_OPTIONS_WITH_VALUES = {
    "-C",
    "-c",
    "--config-env",
    "--exec-path",
    "--git-dir",
    "--namespace",
    "--super-prefix",
    "--work-tree",
}
SHELL_COMMAND_FLAGS = {"-c", "-ic", "-lc"}
SHELL_LAUNCHERS = {"bash", "sh", "zsh"}
SHELL_SEPARATORS = set(";&|()\n")
PROTECTED_PATH_PATTERNS = (
    "lib/",
    "src/",
    "scripts/",
    "podfile",
    "pubspec.yaml",
    "fastfile",
    ".xcodeproj",
    ".xcworkspace",
    ".xcscheme",
    "project.pbxproj",
    "build.gradle",
    "build.gradle.kts",
    "settings.gradle",
    "settings.gradle.kts",
    "gradle.properties",
    "build.sh",
)
NATIVE_SOURCE_ROOTS = (
    "android/app/src/",
    "ios/runner/",
)
NATIVE_SOURCE_EXTENSIONS = {
    ".h",
    ".java",
    ".kt",
    ".m",
    ".mm",
    ".swift",
}
WRITE_OPERATIONS = {
    "cp",
    "dd",
    "echo",
    "install",
    "mv",
    "perl",
    "python",
    "python3",
    "ruby",
    "rm",
    "sed",
    "tee",
    "touch",
    "truncate",
}
WRITE_FLAG_PATTERNS = ("-i", "--in-place")
WRITE_REDIRECTION_RE = re.compile(r"(?:^|\s)(?:>|>>)\s*['\"]?([^'\"\s]+)")
PYTHON_OPEN_WRITE_RE = re.compile(r"open\(\s*['\"]([^'\"]+)['\"]\s*,\s*['\"][wa+]")
PATH_TOKEN_RE = re.compile(r"['\"]([^'\"]+)['\"]|([^\s]+)")


def evaluate_action(action: Action) -> PolicyDecision:
    if action.touches_project_files or action.kind == "project_modify":
        return PolicyDecision(False, "project_modification_blocked")
    if _contains_blocked_git_operation(action.command):
        return PolicyDecision(False, "blocked_git_operation")
    if _touches_protected_project_files(action.command):
        return PolicyDecision(False, "project_modification_blocked")
    if action.kind not in ALLOWED_KINDS:
        return PolicyDecision(False, "unknown_action_kind")
    return PolicyDecision(True, f"allowed_{action.kind}")


def _contains_blocked_git_operation(command: list[str]) -> bool:
    return any(_blocked_git_operation_in_command(view) for view in _iter_command_views(command))


def _touches_protected_project_files(command: list[str]) -> bool:
    protected_paths = _extract_write_targets(command)
    return any(_is_protected_path(path) for path in protected_paths)


def _extract_write_targets(command: list[str]) -> set[str]:
    targets: set[str] = set()

    for view in _iter_command_views(command):
        lowered = [token.lower() for token in view]
        command_text = " ".join(view)

        for match in WRITE_REDIRECTION_RE.finditer(command_text):
            targets.add(match.group(1))

        for match in PYTHON_OPEN_WRITE_RE.finditer(command_text):
            targets.add(match.group(1))

        if lowered and lowered[0] in WRITE_OPERATIONS:
            for token in view[1:]:
                if token.startswith("-"):
                    continue
                targets.update(_extract_path_like_values(token))

        if any(flag in lowered for flag in WRITE_FLAG_PATTERNS):
            for token in view:
                targets.update(_extract_path_like_values(token))

    return {target.lower() for target in targets}


def _iter_command_views(command: list[str]) -> list[list[str]]:
    views = [command]

    if not command:
        return views

    launcher = command[0].rsplit("/", 1)[-1].lower()
    if launcher not in SHELL_LAUNCHERS:
        return views

    for index, token in enumerate(command[:-1]):
        if token.lower() not in SHELL_COMMAND_FLAGS:
            continue
        shell_text = command[index + 1]
        try:
            nested_commands = _split_shell_commands(shell_text)
        except ValueError:
            continue
        for nested in nested_commands:
            views.extend(_iter_command_views(nested))
    return views


def _split_shell_commands(shell_text: str) -> list[list[str]]:
    lexer = shlex.shlex(shell_text, posix=True, punctuation_chars=";&|()\n")
    lexer.whitespace = " \t\r"
    lexer.whitespace_split = True
    lexer.commenters = ""
    commands: list[list[str]] = []
    current: list[str] = []
    for token in lexer:
        if token and all(character in SHELL_SEPARATORS for character in token):
            if current:
                commands.append(current)
                current = []
            continue
        current.append(token)
    if current:
        commands.append(current)
    return commands


def _blocked_git_operation_in_command(command: list[str]) -> bool:
    lowered = [token.lower() for token in command]
    if not lowered or lowered[0].rsplit("/", 1)[-1] != "git":
        return False

    subcommand_index = _git_subcommand_index(lowered)
    return subcommand_index is not None and lowered[subcommand_index] in BLOCKED_GIT_COMMANDS


def _git_subcommand_index(command: list[str]) -> int | None:
    index = 1
    while index < len(command):
        token = command[index]
        if token == "--":
            index += 1
            break
        if not token.startswith("-"):
            return index
        if any(
            token.startswith(f"{option}=")
            for option in GIT_OPTIONS_WITH_VALUES
            if option.startswith("--")
        ):
            index += 1
            continue
        if token in GIT_OPTIONS_WITH_VALUES:
            index += 2
            continue
        index += 1
    return index if index < len(command) else None


def _extract_path_like_values(token: str) -> set[str]:
    matches = set()
    for quoted, bare in PATH_TOKEN_RE.findall(token):
        candidate = quoted or bare
        if "/" in candidate or "." in candidate:
            matches.add(candidate)
    return matches


def _is_protected_path(path: str) -> bool:
    normalized = path.strip().lower().lstrip("./")
    basename = normalized.rsplit("/", 1)[-1]
    if _is_native_source_path(normalized):
        return True
    for pattern in PROTECTED_PATH_PATTERNS:
        if pattern.endswith("/"):
            if normalized == pattern[:-1] or normalized.startswith(pattern):
                return True
            continue
        if normalized == pattern or basename == pattern or normalized.endswith(f"/{pattern}"):
            return True
    return False


def _is_native_source_path(path: str) -> bool:
    if not any(path.startswith(root) for root in NATIVE_SOURCE_ROOTS):
        return False

    return any(path.endswith(extension) for extension in NATIVE_SOURCE_EXTENSIONS)

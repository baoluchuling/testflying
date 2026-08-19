from __future__ import annotations

import pytest

from package_agent.policy import Action, PolicyDecision, evaluate_action


def test_policy_allows_environment_repair_command() -> None:
    action = Action(
        kind="env_repair",
        command=["flutter", "pub", "get"],
        touches_project_files=False,
    )

    decision = evaluate_action(action)

    assert decision == PolicyDecision(allowed=True, reason="allowed_env_repair")


def test_policy_blocks_project_modification() -> None:
    action = Action(
        kind="project_modify",
        command=["python3", "-c", "open('pubspec.yaml','w')"],
        touches_project_files=True,
    )

    decision = evaluate_action(action)

    assert decision.allowed is False
    assert decision.reason == "project_modification_blocked"


def test_policy_blocks_git_push() -> None:
    action = Action(kind="inspect", command=["git", "push", "origin", "main"])

    decision = evaluate_action(action)

    assert decision == PolicyDecision(allowed=False, reason="blocked_git_operation")


def test_policy_blocks_shell_wrapped_git_commit() -> None:
    action = Action(kind="inspect", command=["bash", "-lc", "git commit -m test"])

    decision = evaluate_action(action)

    assert decision == PolicyDecision(allowed=False, reason="blocked_git_operation")


def test_policy_blocks_git_push_after_global_c_flag() -> None:
    action = Action(kind="inspect", command=["git", "-C", "repo", "push", "origin", "main"])

    decision = evaluate_action(action)

    assert decision == PolicyDecision(allowed=False, reason="blocked_git_operation")


def test_policy_blocks_git_commit_after_git_dir_option() -> None:
    action = Action(
        kind="inspect",
        command=["git", "--git-dir=.git", "commit", "-m", "test"],
    )

    decision = evaluate_action(action)

    assert decision == PolicyDecision(allowed=False, reason="blocked_git_operation")


@pytest.mark.parametrize(
    "command",
    [
        ["git", "pull", "--ff-only"],
        ["git", "fetch", "origin"],
        ["git", "tag", "local-checkpoint"],
        ["git", "reset", "--hard", "HEAD"],
        ["bash", "-lc", "git -C repo pull --ff-only origin main"],
    ],
)
def test_policy_allows_permitted_git_operations(command: list[str]) -> None:
    assert evaluate_action(Action(kind="inspect", command=command)).allowed is True


@pytest.mark.parametrize(
    "command",
    [
        ["git", "commit", "-m", "blocked"],
        ["git", "push", "origin", "main"],
        ["bash", "-lc", "git commit -m blocked"],
        ["zsh", "-lc", "git push origin main"],
        ["bash", "-lc", "git status --short && git push origin main"],
        ["zsh", "-lc", "echo ready; git commit -m blocked"],
        ["/usr/bin/git", "push", "origin", "main"],
        ["bash", "-lc", "git status --short\ngit push origin main"],
    ],
)
def test_policy_blocks_only_publishing_git_operations(command: list[str]) -> None:
    decision = evaluate_action(Action(kind="inspect", command=command))
    assert decision.reason == "blocked_git_operation"


def test_policy_blocks_protected_build_file_change() -> None:
    action = Action(
        kind="env_repair",
        command=["python3", "-c", "open('ios/Podfile','w').write('platform :ios')"],
    )

    decision = evaluate_action(action)

    assert decision == PolicyDecision(allowed=False, reason="project_modification_blocked")


def test_policy_blocks_build_action_that_writes_source_file() -> None:
    action = Action(
        kind="build",
        command=["python3", "-c", "open('lib/main.dart','w').write('// rewritten')"],
    )

    decision = evaluate_action(action)

    assert decision == PolicyDecision(allowed=False, reason="project_modification_blocked")


def test_policy_blocks_env_repair_that_writes_source_directory() -> None:
    action = Action(
        kind="env_repair",
        command=["cp", "/tmp/generated.dart", "src/generated/app.dart"],
    )

    decision = evaluate_action(action)

    assert decision == PolicyDecision(allowed=False, reason="project_modification_blocked")


def test_policy_blocks_build_script_rewrite() -> None:
    action = Action(
        kind="build",
        command=["bash", "-lc", "echo '#!/bin/sh' > scripts/build.sh"],
    )

    decision = evaluate_action(action)

    assert decision == PolicyDecision(allowed=False, reason="project_modification_blocked")


def test_policy_blocks_shell_wrapped_cp_to_protected_podfile() -> None:
    action = Action(
        kind="env_repair",
        command=["bash", "-lc", "cp /tmp/a ios/Podfile"],
    )

    decision = evaluate_action(action)

    assert decision == PolicyDecision(allowed=False, reason="project_modification_blocked")


def test_policy_blocks_shell_wrapped_rm_of_android_source() -> None:
    action = Action(
        kind="build",
        command=["bash", "-lc", "rm android/app/src/main/kotlin/MainActivity.kt"],
    )

    decision = evaluate_action(action)

    assert decision == PolicyDecision(allowed=False, reason="project_modification_blocked")


def test_policy_blocks_ios_runner_swift_source_edit() -> None:
    action = Action(
        kind="build",
        command=["python3", "-c", "open('ios/Runner/AppDelegate.swift','w').write('// rewrite')"],
    )

    decision = evaluate_action(action)

    assert decision == PolicyDecision(allowed=False, reason="project_modification_blocked")


def test_policy_blocks_android_app_kotlin_source_edit() -> None:
    action = Action(
        kind="env_repair",
        command=[
            "cp",
            "/tmp/MainActivity.kt",
            "android/app/src/main/kotlin/com/example/MainActivity.kt",
        ],
    )

    decision = evaluate_action(action)

    assert decision == PolicyDecision(allowed=False, reason="project_modification_blocked")


def test_policy_allows_git_status_for_inspection() -> None:
    action = Action(kind="inspect", command=["git", "status", "--short"])

    decision = evaluate_action(action)

    assert decision == PolicyDecision(allowed=True, reason="allowed_inspect")

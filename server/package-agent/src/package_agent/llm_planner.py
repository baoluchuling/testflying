from __future__ import annotations

import json
import shlex
import subprocess
import tempfile
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from package_agent.llm_discovery import LlmAdapter
from package_agent.models import BuildInput


@dataclass(frozen=True)
class PlanAction:
    kind: str
    command: list[str] = field(default_factory=list)
    package_paths: list[str] = field(default_factory=list)
    symbols_paths: list[str] = field(default_factory=list)
    log_paths: list[str] = field(default_factory=list)


@dataclass(frozen=True)
class LlmPlan:
    actions: list[PlanAction]


@dataclass(frozen=True)
class PlannerCommand:
    args: list[str]
    stdin: str | None
    output_path: Path | None = None


def request_llm_plan(
    adapter: LlmAdapter,
    build_input: BuildInput,
    *,
    timeout_seconds: int = 120,
) -> LlmPlan:
    prompt = _planner_prompt(build_input)
    if adapter.name == "codex_cli":
        with tempfile.NamedTemporaryFile() as output_file:
            command = planner_command(adapter, prompt, output_path=Path(output_file.name))
            return _run_planner_command(command, timeout_seconds=timeout_seconds)

    command = planner_command(adapter, prompt)
    return _run_planner_command(command, timeout_seconds=timeout_seconds)


def planner_command(
    adapter: LlmAdapter,
    prompt: str,
    *,
    output_path: Path | None = None,
) -> PlannerCommand:
    if adapter.name == "injected_llm_planner":
        return PlannerCommand(args=[adapter.executable], stdin=prompt)
    if adapter.name == "codex_cli":
        if output_path is None:
            raise ValueError("codex planner requires an output path")
        return PlannerCommand(
            args=[
                adapter.executable,
                "exec",
                "--output-last-message",
                str(output_path),
                "-",
            ],
            stdin=prompt,
            output_path=output_path,
        )
    if adapter.name == "claude_cli":
        return PlannerCommand(args=[adapter.executable, "-p", prompt], stdin=None)
    if adapter.name == "llm_runtime":
        # Local contract: `llm-runtime package-agent-plan-json` reads the JSON prompt from
        # stdin and returns only plan JSON on stdout.
        return PlannerCommand(
            args=[adapter.executable, "package-agent-plan-json"],
            stdin=prompt,
        )
    return PlannerCommand(args=[adapter.executable], stdin=prompt)


def _run_planner_command(command: PlannerCommand, *, timeout_seconds: int) -> LlmPlan:
    result = subprocess.run(
        command.args,
        input=command.stdin,
        capture_output=True,
        text=True,
        timeout=timeout_seconds,
        check=False,
    )
    if result.returncode != 0:
        raise ValueError(f"planner exited with code {result.returncode}")
    payload_text = _planner_output_text(command, result.stdout)
    try:
        payload = json.loads(payload_text)
    except json.JSONDecodeError as exc:
        raise ValueError("planner did not return valid JSON") from exc
    return parse_llm_plan(payload)


def _planner_output_text(command: PlannerCommand, stdout: str) -> str:
    if command.output_path is None:
        return stdout

    payload_text = command.output_path.read_text(encoding="utf-8")
    if not payload_text.strip():
        raise ValueError("planner did not return valid JSON")
    return payload_text


def parse_llm_plan(payload: object) -> LlmPlan:
    if not isinstance(payload, dict):
        raise ValueError("planner JSON must be an object")
    raw_actions = payload.get("actions")
    if not isinstance(raw_actions, list) or not raw_actions:
        raise ValueError("planner JSON must include non-empty actions")
    return LlmPlan(actions=[_parse_action(item) for item in raw_actions])


def _parse_action(payload: object) -> PlanAction:
    if not isinstance(payload, dict):
        raise ValueError("planner action must be an object")
    kind = payload.get("kind")
    if not isinstance(kind, str) or not kind.strip():
        raise ValueError("planner action kind must be a non-empty string")
    return PlanAction(
        kind=kind.strip(),
        command=_parse_command(payload.get("command")),
        package_paths=_string_list(payload.get("packagePaths")),
        symbols_paths=_string_list(payload.get("symbolsPaths")),
        log_paths=_string_list(payload.get("logPaths")),
    )


def _parse_command(value: object) -> list[str]:
    if value is None:
        return []
    if isinstance(value, list) and all(isinstance(item, str) and item for item in value):
        return list(value)
    if isinstance(value, str) and value.strip():
        return shlex.split(value)
    raise ValueError("planner action command must be a string or string array")


def _string_list(value: object) -> list[str]:
    if value is None:
        return []
    if not isinstance(value, list) or not all(isinstance(item, str) for item in value):
        raise ValueError("planner artifact paths must be string arrays")
    return list(value)


def _planner_prompt(build_input: BuildInput) -> str:
    payload: dict[str, Any] = {
        "task": "Return only JSON for a constrained TestFlying package-agent build plan.",
        "schema": {
            "actions": [
                {
                    "kind": "inspect|build|env_repair|artifact_collect",
                    "command": ["executable", "arg"],
                    "packagePaths": ["relative/glob/*.ipa"],
                    "symbolsPaths": ["relative/glob/*.zip"],
                    "logPaths": ["relative/glob/*.log"],
                }
            ]
        },
        "constraints": [
            "Do not modify protected project source, build scripts, git history, or credentials.",
            "Use commands that can pass package_agent.policy.evaluate_action.",
            "Collect package, symbols, and log artifacts with paths relative to projectDir.",
        ],
        "buildInput": {
            "projectDir": str(Path(build_input.project_dir)),
            "platform": build_input.platform,
            "environment": build_input.environment,
            "artifactType": build_input.artifact_type,
            "gitRef": build_input.git_ref,
            "repoSubpath": build_input.repo_subpath,
        },
    }
    return json.dumps(payload, ensure_ascii=False, indent=2)

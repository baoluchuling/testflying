from __future__ import annotations

from pathlib import Path
from types import SimpleNamespace

import pytest

from package_agent.llm_discovery import LlmAdapter
from package_agent.llm_planner import planner_command, request_llm_plan
from package_agent.models import BuildInput


def test_planner_command_uses_stdin_for_injected_planner() -> None:
    command = planner_command(LlmAdapter(name="injected_llm_planner", executable="/tmp/fake"), "prompt")

    assert command.args == ["/tmp/fake"]
    assert command.stdin == "prompt"


def test_planner_command_uses_codex_output_last_message(tmp_path: Path) -> None:
    output_path = tmp_path / "codex-plan.json"
    command = planner_command(
        LlmAdapter(name="codex_cli", executable="/usr/bin/codex"),
        "prompt",
        output_path=output_path,
    )

    assert command.args == [
        "/usr/bin/codex",
        "exec",
        "--output-last-message",
        str(output_path),
        "-",
    ]
    assert command.stdin == "prompt"
    assert command.output_path == output_path


def test_planner_command_rejects_codex_without_output_path() -> None:
    with pytest.raises(ValueError, match="requires an output path"):
        planner_command(LlmAdapter(name="codex_cli", executable="/usr/bin/codex"), "prompt")


def test_planner_command_uses_claude_prompt_flag() -> None:
    command = planner_command(LlmAdapter(name="claude_cli", executable="/usr/bin/claude"), "prompt")

    assert command.args == ["/usr/bin/claude", "-p", "prompt"]
    assert command.stdin is None


def test_planner_command_uses_llm_runtime_json_protocol() -> None:
    command = planner_command(LlmAdapter(name="llm_runtime", executable="/usr/bin/llm-runtime"), "prompt")

    assert command.args == ["/usr/bin/llm-runtime", "package-agent-plan-json"]
    assert command.stdin == "prompt"


def test_request_llm_plan_passes_adapter_specific_invocation(monkeypatch) -> None:
    captured: dict[str, object] = {}

    def fake_run(args, **kwargs):  # type: ignore[no-untyped-def]
        captured["args"] = args
        captured["input"] = kwargs.get("input")
        return SimpleNamespace(returncode=0, stdout='{"actions":[{"kind":"inspect"}]}')

    monkeypatch.setattr("package_agent.llm_planner.subprocess.run", fake_run)

    plan = request_llm_plan(
        LlmAdapter(name="claude_cli", executable="/usr/bin/claude"),
        BuildInput(
            project_dir="/tmp/project",
            platform="ios",
            environment="development",
            artifact_type="ipa",
        ),
    )

    assert len(plan.actions) == 1
    assert captured["args"][0:2] == ["/usr/bin/claude", "-p"]
    assert isinstance(captured["args"][2], str)
    assert '"projectDir": "/tmp/project"' in captured["args"][2]
    assert captured["input"] is None


def test_request_llm_plan_reads_codex_output_last_message_file(monkeypatch) -> None:
    captured: dict[str, object] = {}

    def fake_run(args, **kwargs):  # type: ignore[no-untyped-def]
        captured["args"] = args
        captured["input"] = kwargs.get("input")
        output_path = Path(args[3])
        output_path.write_text('{"actions":[{"kind":"inspect"}]}', encoding="utf-8")
        return SimpleNamespace(
            returncode=0,
            stdout='{"type":"thread.started"}\n{"type":"message.delta","delta":"ignored"}\n',
        )

    monkeypatch.setattr("package_agent.llm_planner.subprocess.run", fake_run)

    plan = request_llm_plan(
        LlmAdapter(name="codex_cli", executable="/usr/bin/codex"),
        BuildInput(
            project_dir="/tmp/project",
            platform="ios",
            environment="development",
            artifact_type="ipa",
        ),
    )

    assert len(plan.actions) == 1
    assert captured["args"] == [
        "/usr/bin/codex",
        "exec",
        "--output-last-message",
        captured["args"][3],
        "-",
    ]
    assert captured["input"] is not None
    assert '"projectDir": "/tmp/project"' in captured["input"]


def test_request_llm_plan_for_codex_does_not_parse_jsonl_stdout(monkeypatch) -> None:
    def fake_run(args, **kwargs):  # type: ignore[no-untyped-def]
        Path(args[3]).write_text("not json", encoding="utf-8")
        return SimpleNamespace(returncode=0, stdout='{"actions":[{"kind":"inspect"}]}\n')

    monkeypatch.setattr("package_agent.llm_planner.subprocess.run", fake_run)

    with pytest.raises(ValueError, match="planner did not return valid JSON"):
        request_llm_plan(
            LlmAdapter(name="codex_cli", executable="/usr/bin/codex"),
            BuildInput(
                project_dir="/tmp/project",
                platform="ios",
                environment="development",
                artifact_type="ipa",
            ),
        )

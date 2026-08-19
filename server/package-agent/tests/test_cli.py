from __future__ import annotations

import json
import sys
from pathlib import Path

from package_agent.cli import main


def test_cli_writes_report_and_returns_needs_human_for_missing_input(tmp_path: Path) -> None:
    output_dir = tmp_path / "output"

    exit_code = main(
        ["build", "--input", str(tmp_path / "missing.json"), "--output", str(output_dir)]
    )

    assert exit_code == 2
    report = json.loads((output_dir / "report.json").read_text(encoding="utf-8"))
    assert report["status"] == "needs_human"
    assert report["classification"] == "missing_input"


def test_cli_returns_failed_for_invalid_json(tmp_path: Path) -> None:
    input_path = tmp_path / "build-input.json"
    input_path.write_text("{not-json", encoding="utf-8")
    output_dir = tmp_path / "output"

    exit_code = main(["build", "--input", str(input_path), "--output", str(output_dir)])

    assert exit_code == 1
    report = json.loads((output_dir / "report.json").read_text(encoding="utf-8"))
    assert report["status"] == "failed"
    assert report["classification"] == "invalid_input_json"


def test_cli_returns_failed_for_non_object_json(tmp_path: Path) -> None:
    input_path = tmp_path / "build-input.json"
    input_path.write_text(json.dumps(["not", "an", "object"]), encoding="utf-8")
    output_dir = tmp_path / "output"

    exit_code = main(["build", "--input", str(input_path), "--output", str(output_dir)])

    assert exit_code == 1
    report = json.loads((output_dir / "report.json").read_text(encoding="utf-8"))
    assert report["status"] == "failed"
    assert report["classification"] == "invalid_input_shape"


def test_cli_returns_failed_for_missing_required_fields(tmp_path: Path) -> None:
    input_path = tmp_path / "build-input.json"
    input_path.write_text(
        json.dumps(
            {
                "projectDir": str(tmp_path / "project"),
                "platform": "ios",
                "environment": "prod",
            }
        ),
        encoding="utf-8",
    )
    output_dir = tmp_path / "output"

    exit_code = main(["build", "--input", str(input_path), "--output", str(output_dir)])

    assert exit_code == 1
    report = json.loads((output_dir / "report.json").read_text(encoding="utf-8"))
    assert report["status"] == "failed"
    assert report["classification"] == "invalid_input"
    assert "artifactType" in report["summary"]


def test_cli_returns_failed_for_invalid_artifact_lists(tmp_path: Path) -> None:
    input_path = tmp_path / "build-input.json"
    input_path.write_text(
        json.dumps(
            {
                "projectDir": str(tmp_path / "project"),
                "platform": "ios",
                "environment": "prod",
                "artifactType": "ipa",
                "packagePaths": "app.ipa",
            }
        ),
        encoding="utf-8",
    )
    output_dir = tmp_path / "output"

    exit_code = main(["build", "--input", str(input_path), "--output", str(output_dir)])

    assert exit_code == 1
    report = json.loads((output_dir / "report.json").read_text(encoding="utf-8"))
    assert report["status"] == "failed"
    assert report["classification"] == "invalid_input"
    assert "string lists" in report["summary"]


def test_cli_returns_failed_for_max_attempts_above_retry_cap(tmp_path: Path) -> None:
    input_path = tmp_path / "build-input.json"
    input_path.write_text(
        json.dumps(
            {
                "projectDir": str(tmp_path / "project"),
                "platform": "ios",
                "environment": "prod",
                "artifactType": "ipa",
                "maxAttempts": 6,
            }
        ),
        encoding="utf-8",
    )
    output_dir = tmp_path / "output"

    exit_code = main(["build", "--input", str(input_path), "--output", str(output_dir)])

    assert exit_code == 1
    report = json.loads((output_dir / "report.json").read_text(encoding="utf-8"))
    assert report["status"] == "failed"
    assert report["classification"] == "invalid_input"
    assert report["summary"] == "maxAttempts must be <= 5"


def test_cli_returns_failed_for_boolean_max_attempts(tmp_path: Path) -> None:
    input_path = tmp_path / "build-input.json"
    input_path.write_text(
        json.dumps(
            {
                "projectDir": str(tmp_path / "project"),
                "platform": "ios",
                "environment": "prod",
                "artifactType": "ipa",
                "maxAttempts": True,
            }
        ),
        encoding="utf-8",
    )
    output_dir = tmp_path / "output"

    exit_code = main(["build", "--input", str(input_path), "--output", str(output_dir)])

    assert exit_code == 1
    report = json.loads((output_dir / "report.json").read_text(encoding="utf-8"))
    assert report["status"] == "failed"
    assert report["classification"] == "invalid_input"
    assert report["summary"] == "maxAttempts must be a positive integer when provided"


def test_cli_returns_success_only_with_required_artifacts(tmp_path: Path) -> None:
    package_path = tmp_path / "app.ipa"
    symbols_path = tmp_path / "app.dsym.zip"
    log_path = tmp_path / "build.log"
    for path in (package_path, symbols_path, log_path):
        path.write_text("artifact", encoding="utf-8")
    input_path = tmp_path / "build-input.json"
    input_path.write_text(
        json.dumps(
            {
                "projectDir": str(tmp_path / "project"),
                "platform": "ios",
                "environment": "prod",
                "artifactType": "ipa",
                "packagePaths": [str(package_path)],
                "symbolsPaths": [str(symbols_path)],
                "logPaths": [str(log_path)],
            }
        ),
        encoding="utf-8",
    )

    output_dir = tmp_path / "output"
    exit_code = main(["build", "--input", str(input_path), "--output", str(output_dir)])

    assert exit_code == 0
    report = json.loads((output_dir / "report.json").read_text(encoding="utf-8"))
    assert report["status"] == "success"
    assert report["classification"] == "build_succeeded"


def test_cli_runs_project_config_command_and_collects_redacted_artifacts(tmp_path: Path) -> None:
    project_dir = tmp_path / "project"
    project_dir.mkdir()
    (project_dir / "testflying-package-agent.json").write_text(
        json.dumps(
            {
                "buildCommand": [
                    "python3",
                    "-c",
                    (
                        "from pathlib import Path; "
                        "Path('dist').mkdir(); Path('logs').mkdir(); "
                        "Path('dist/app.ipa').write_text('ipa'); "
                        "Path('dist/app.dSYM.zip').write_text('symbols'); "
                        "Path('logs/build.log').write_text('token=super-secret'); "
                        "print('token=super-secret')"
                    ),
                ],
                "packagePaths": ["dist/*.ipa"],
                "symbolsPaths": ["dist/*.dSYM.zip"],
                "logPaths": ["logs/*.log"],
                "version": "1.2.3",
                "buildNumber": "45",
            }
        ),
        encoding="utf-8",
    )
    input_path = tmp_path / "build-input.json"
    input_path.write_text(
        json.dumps(
            {
                "buildId": "build-agent-1",
                "projectDir": str(project_dir),
                "platform": "ios",
                "environment": "development",
                "artifactType": "ipa",
                "gitUrl": "file:///repo.git",
                "gitRef": "main",
                "repoSubpath": "",
                "commitSha": "abc123",
            }
        ),
        encoding="utf-8",
    )
    output_dir = tmp_path / "output"

    exit_code = main(["build", "--input", str(input_path), "--output", str(output_dir)])

    assert exit_code == 0
    report = json.loads((output_dir / "report.json").read_text(encoding="utf-8"))
    assert report["status"] == "success"
    assert report["version"] == "1.2.3"
    assert report["buildNumber"] == "45"
    assert report["commitSha"] == "abc123"
    assert Path(report["packagePaths"][0]).read_text(encoding="utf-8") == "ipa"
    assert Path(report["symbolsPaths"][0]).read_text(encoding="utf-8") == "symbols"
    assert len(report["logPaths"]) == 2
    assert "[REDACTED]" in (output_dir / "command.log").read_text(encoding="utf-8")
    assert "[REDACTED]" in Path(report["logPaths"][1]).read_text(encoding="utf-8")


def test_cli_returns_needs_human_when_config_override_is_missing(tmp_path: Path) -> None:
    project_dir = tmp_path / "project"
    project_dir.mkdir()
    input_path = _write_build_input(tmp_path, project_dir, artifact_type="apk")
    output_dir = tmp_path / "output"

    exit_code = main(
        [
            "build",
            "--input",
            str(input_path),
            "--output",
            str(output_dir),
            "--config",
            str(tmp_path / "missing-config.json"),
        ]
    )

    assert exit_code == 2
    report = json.loads((output_dir / "report.json").read_text(encoding="utf-8"))
    assert report["classification"] == "missing_build_config"


def test_cli_returns_failed_when_config_override_is_invalid_json(tmp_path: Path) -> None:
    project_dir = tmp_path / "project"
    project_dir.mkdir()
    input_path = _write_build_input(tmp_path, project_dir, artifact_type="apk")
    config_path = tmp_path / "acceptance.json"
    config_path.write_text("{invalid", encoding="utf-8")
    output_dir = tmp_path / "output"

    exit_code = main(
        [
            "build",
            "--input",
            str(input_path),
            "--output",
            str(output_dir),
            "--config",
            str(config_path),
        ]
    )

    assert exit_code == 1
    report = json.loads((output_dir / "report.json").read_text(encoding="utf-8"))
    assert report["classification"] == "invalid_build_config_json"


def test_cli_applies_policy_to_config_override(tmp_path: Path) -> None:
    project_dir = tmp_path / "project"
    project_dir.mkdir()
    input_path = _write_build_input(tmp_path, project_dir, artifact_type="apk")
    config_path = tmp_path / "blocked.json"
    config_path.write_text(json.dumps({"buildCommand": ["git", "commit", "-m", "blocked"]}))
    output_dir = tmp_path / "output"

    exit_code = main(
        [
            "build",
            "--input",
            str(input_path),
            "--output",
            str(output_dir),
            "--config",
            str(config_path),
        ]
    )

    assert exit_code == 2
    report = json.loads((output_dir / "report.json").read_text(encoding="utf-8"))
    assert report["classification"] == "blocked_git_operation"


def test_cli_uses_config_override_without_project_config(tmp_path: Path) -> None:
    project_dir = tmp_path / "project"
    project_dir.mkdir()
    input_path = _write_build_input(tmp_path, project_dir, artifact_type="apk")
    config_path = tmp_path / "acceptance.json"
    config_path.write_text(
        json.dumps(
            {
                "buildCommand": [
                    sys.executable,
                    "-c",
                    (
                        "from pathlib import Path; Path('dist').mkdir(); "
                        "Path('dist/app.apk').write_text('apk'); "
                        "Path('dist/mapping.txt').write_text('symbols')"
                    ),
                ],
                "packagePaths": ["dist/*.apk"],
                "symbolsPaths": ["dist/mapping.txt"],
            }
        ),
        encoding="utf-8",
    )
    output_dir = tmp_path / "output"

    exit_code = main(
        [
            "build",
            "--input",
            str(input_path),
            "--output",
            str(output_dir),
            "--config",
            str(config_path),
        ]
    )

    assert exit_code == 0
    assert not (project_dir / "testflying-package-agent.json").exists()
    report = json.loads((output_dir / "report.json").read_text(encoding="utf-8"))
    assert report["classification"] == "build_succeeded"
    assert Path(report["packagePaths"][0]).read_text(encoding="utf-8") == "apk"


def test_cli_returns_needs_human_without_config_or_artifact_paths(
    tmp_path: Path,
    monkeypatch,
) -> None:
    monkeypatch.delenv("TESTFLYING_PACKAGE_AGENT_LLM_PLANNER", raising=False)
    monkeypatch.setenv("PATH", "")
    project_dir = tmp_path / "project"
    project_dir.mkdir()
    input_path = tmp_path / "build-input.json"
    input_path.write_text(
        json.dumps(
            {
                "projectDir": str(project_dir),
                "platform": "ios",
                "environment": "development",
                "artifactType": "ipa",
            }
        ),
        encoding="utf-8",
    )
    output_dir = tmp_path / "output"

    exit_code = main(["build", "--input", str(input_path), "--output", str(output_dir)])

    assert exit_code == 2
    report = json.loads((output_dir / "report.json").read_text(encoding="utf-8"))
    assert report["status"] == "needs_human"
    assert report["classification"] == "llm_unavailable"
    assert "testflying-package-agent.json" in report["humanAction"]


def test_cli_uses_fake_llm_planner_without_project_config(tmp_path: Path, monkeypatch) -> None:
    project_dir = tmp_path / "project"
    project_dir.mkdir()
    planner = _write_fake_planner(
        tmp_path,
        {
            "actions": [
                {
                    "kind": "build",
                    "command": [
                        sys.executable,
                        "-c",
                        (
                            "from pathlib import Path; "
                            "Path('dist').mkdir(); Path('logs').mkdir(); "
                            "Path('dist/app.ipa').write_text('ipa'); "
                            "Path('dist/app.dSYM.zip').write_text('symbols'); "
                            "Path('logs/build.log').write_text('password=planner-secret')"
                        ),
                    ],
                    "packagePaths": ["dist/*.ipa"],
                    "symbolsPaths": ["dist/*.dSYM.zip"],
                    "logPaths": ["logs/*.log"],
                }
            ]
        },
    )
    monkeypatch.setenv("TESTFLYING_PACKAGE_AGENT_LLM_PLANNER", str(planner))
    input_path = tmp_path / "build-input.json"
    input_path.write_text(
        json.dumps(
            {
                "projectDir": str(project_dir),
                "platform": "ios",
                "environment": "development",
                "artifactType": "ipa",
                "maxAttempts": 1,
            }
        ),
        encoding="utf-8",
    )
    output_dir = tmp_path / "output"

    exit_code = main(["build", "--input", str(input_path), "--output", str(output_dir)])

    assert exit_code == 0
    report = json.loads((output_dir / "report.json").read_text(encoding="utf-8"))
    assert report["status"] == "success"
    assert report["adapter"] == "injected_llm_planner"
    assert Path(report["packagePaths"][0]).read_text(encoding="utf-8") == "ipa"
    assert Path(report["symbolsPaths"][0]).read_text(encoding="utf-8") == "symbols"
    assert "[REDACTED]" in Path(report["logPaths"][0]).read_text(encoding="utf-8")


def test_cli_returns_needs_human_for_policy_blocked_llm_plan(
    tmp_path: Path,
    monkeypatch,
) -> None:
    project_dir = tmp_path / "project"
    (project_dir / "src").mkdir(parents=True)
    planner = _write_fake_planner(
        tmp_path,
        {
            "actions": [
                {
                    "kind": "build",
                    "command": [
                        sys.executable,
                        "-c",
                        "open('src/main.swift', 'w').write('modified')",
                    ],
                }
            ]
        },
    )
    monkeypatch.setenv("TESTFLYING_PACKAGE_AGENT_LLM_PLANNER", str(planner))
    input_path = tmp_path / "build-input.json"
    input_path.write_text(
        json.dumps(
            {
                "projectDir": str(project_dir),
                "platform": "ios",
                "environment": "development",
                "artifactType": "ipa",
            }
        ),
        encoding="utf-8",
    )
    output_dir = tmp_path / "output"

    exit_code = main(["build", "--input", str(input_path), "--output", str(output_dir)])

    assert exit_code == 2
    report = json.loads((output_dir / "report.json").read_text(encoding="utf-8"))
    assert report["status"] == "needs_human"
    assert report["classification"] == "project_modification_blocked"
    assert not (project_dir / "src/main.swift").exists()


def _write_fake_planner(tmp_path: Path, payload: dict[str, object]) -> Path:
    planner = tmp_path / "fake-planner.py"
    planner.write_text(
        "\n".join(
            [
                "#!/usr/bin/env python3",
                "import json",
                "import sys",
                "sys.stdin.read()",
                f"print(json.dumps({payload!r}))",
            ]
        ),
        encoding="utf-8",
    )
    planner.chmod(0o755)
    return planner


def _write_build_input(tmp_path: Path, project_dir: Path, *, artifact_type: str) -> Path:
    input_path = tmp_path / "build-input.json"
    input_path.write_text(
        json.dumps(
            {
                "projectDir": str(project_dir),
                "platform": "android" if artifact_type in {"apk", "aab"} else "ios",
                "environment": "development",
                "artifactType": artifact_type,
            }
        ),
        encoding="utf-8",
    )
    return input_path

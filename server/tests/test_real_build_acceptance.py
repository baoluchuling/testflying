from __future__ import annotations

import json
import subprocess
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[1]
SCRIPT = REPO_ROOT / "scripts/verify_real_build.sh"


def test_acceptance_succeeds_for_clean_immutable_build(tmp_path: Path) -> None:
    project = _git_project(tmp_path)

    result, acceptance = _run_acceptance(tmp_path, project, {})

    assert result.returncode == 0, result.stderr
    assert acceptance["status"] == "success"
    assert acceptance["git"]["violations"] == []


def test_acceptance_preserves_preexisting_dirty_state(tmp_path: Path) -> None:
    project = _git_project(tmp_path)
    (project / "README.md").write_text("preexisting dirty\n", encoding="utf-8")
    before = _git_status(project)

    result, acceptance = _run_acceptance(tmp_path, project, {})

    assert result.returncode == 0, result.stderr
    assert _git_status(project) == before
    assert acceptance["git"]["violations"] == []


def test_acceptance_rejects_change_to_preexisting_dirty_file(tmp_path: Path) -> None:
    project = _git_project(tmp_path)
    (project / "README.md").write_text("preexisting dirty\n", encoding="utf-8")

    result, acceptance = _run_acceptance(
        tmp_path,
        project,
        {"behavior": "modify_tracked"},
    )

    assert result.returncode != 0
    assert acceptance["status"] == "failed"
    assert "preexisting_path_changed" in acceptance["git"]["violations"]


def test_acceptance_rejects_new_tracked_file_change(tmp_path: Path) -> None:
    project = _git_project(tmp_path)

    result, acceptance = _run_acceptance(
        tmp_path,
        project,
        {"behavior": "modify_tracked"},
    )

    assert result.returncode != 0
    assert acceptance["classification"] == "immutable_project_violation"
    assert "README.md" in acceptance["git"]["unexpectedPaths"]


@pytest.mark.parametrize("path", ["lib/generated.dart", "testflying-package-agent.json"])
def test_acceptance_rejects_new_untracked_source_or_config(tmp_path: Path, path: str) -> None:
    project = _git_project(tmp_path)

    result, acceptance = _run_acceptance(
        tmp_path,
        project,
        {"behavior": "create_untracked", "untrackedPath": path},
    )

    assert result.returncode != 0
    assert path in acceptance["git"]["unexpectedPaths"]


def test_acceptance_allows_generated_build_output(tmp_path: Path) -> None:
    project = _git_project(tmp_path)

    result, acceptance = _run_acceptance(
        tmp_path,
        project,
        {"behavior": "create_build_output"},
    )

    assert result.returncode == 0, result.stderr
    assert acceptance["status"] == "success"
    assert (project / "build/generated/output.txt").is_file()


def test_acceptance_writes_failure_when_report_is_missing(tmp_path: Path) -> None:
    project = _git_project(tmp_path)

    result, acceptance = _run_acceptance(tmp_path, project, {"behavior": "missing_report"})

    assert result.returncode != 0
    assert acceptance["classification"] == "missing_agent_report"


def test_acceptance_rejects_non_success_report(tmp_path: Path) -> None:
    project = _git_project(tmp_path)

    result, acceptance = _run_acceptance(tmp_path, project, {"reportStatus": "needs_human"})

    assert result.returncode != 0
    assert acceptance["classification"] == "agent_report_not_success"


@pytest.mark.parametrize("missing", ["packagePaths", "symbolsPaths", "logPaths"])
def test_acceptance_requires_all_artifact_categories(tmp_path: Path, missing: str) -> None:
    project = _git_project(tmp_path)

    result, acceptance = _run_acceptance(tmp_path, project, {"missingArtifact": missing})

    assert result.returncode != 0
    assert acceptance["classification"] == "missing_required_artifacts"


def _run_acceptance(
    tmp_path: Path,
    project: Path,
    config: dict[str, object],
) -> tuple[subprocess.CompletedProcess[str], dict[str, object]]:
    package_agent = _fake_package_agent(tmp_path)
    config_path = tmp_path / "acceptance-config.json"
    config_path.write_text(json.dumps(config), encoding="utf-8")
    output_dir = tmp_path / "acceptance-output"
    result = subprocess.run(
        [
            str(SCRIPT),
            str(project),
            str(package_agent),
            str(config_path),
            "android",
            "development",
            "apk",
            str(output_dir),
        ],
        text=True,
        capture_output=True,
        check=False,
    )
    acceptance_path = output_dir / "acceptance.json"
    assert acceptance_path.is_file(), result.stderr
    return result, json.loads(acceptance_path.read_text(encoding="utf-8"))


def _git_project(tmp_path: Path) -> Path:
    project = tmp_path / "project"
    (project / "lib").mkdir(parents=True)
    (project / "README.md").write_text("clean\n", encoding="utf-8")
    (project / ".gitignore").write_text(
        "build/\n.dart_tool/\n.gradle/\nPods/\nDerivedData/\n",
        encoding="utf-8",
    )
    subprocess.run(["git", "init", "-q", str(project)], check=True)
    subprocess.run(["git", "-C", str(project), "add", "."], check=True)
    subprocess.run(
        [
            "git",
            "-C",
            str(project),
            "-c",
            "user.name=TestFlying Test",
            "-c",
            "user.email=testflying@example.test",
            "commit",
            "-qm",
            "initial",
        ],
        check=True,
    )
    return project


def _git_status(project: Path) -> bytes:
    return subprocess.run(
        ["git", "-C", str(project), "status", "--porcelain=v1", "-z", "--untracked-files=all"],
        check=True,
        capture_output=True,
    ).stdout


def _fake_package_agent(tmp_path: Path) -> Path:
    script = tmp_path / "fake-package-agent"
    script.write_text(
        """#!/usr/bin/env python3
import argparse
import json
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument("command")
parser.add_argument("--input", required=True)
parser.add_argument("--output", required=True)
parser.add_argument("--config", required=True)
args = parser.parse_args()
build_input = json.loads(Path(args.input).read_text())
config = json.loads(Path(args.config).read_text())
project = Path(build_input["projectDir"])
output = Path(args.output)
output.mkdir(parents=True, exist_ok=True)

behavior = config.get("behavior")
if behavior == "modify_tracked":
    (project / "README.md").write_text("modified by agent\\n")
elif behavior == "create_untracked":
    target = project / config["untrackedPath"]
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text("created by agent\\n")
elif behavior == "create_build_output":
    target = project / "build/generated/output.txt"
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text("generated\\n")

if behavior == "missing_report":
    raise SystemExit(0)

artifacts = output / "artifacts"
artifacts.mkdir(exist_ok=True)
paths = {
    "packagePaths": artifacts / "app.apk",
    "symbolsPaths": artifacts / "symbols.zip",
    "logPaths": artifacts / "command.log",
}
for path in paths.values():
    path.write_text("artifact\\n")
missing = config.get("missingArtifact")
report = {
    "status": config.get("reportStatus", "success"),
    "classification": "fake_agent_result",
    "summary": "fake package-agent completed",
}
for key, path in paths.items():
    report[key] = [] if key == missing else [str(path)]
(output / "report.json").write_text(json.dumps(report))
raise SystemExit(0 if report["status"] == "success" else 2)
""",
        encoding="utf-8",
    )
    script.chmod(0o755)
    return script

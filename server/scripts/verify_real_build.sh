#!/usr/bin/env bash
set -euo pipefail

exec python3 - "$@" <<'PY'
from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path
from typing import Any


ALLOWED_GENERATED_ROOTS = {".dart_tool", ".gradle", "Pods", "DerivedData", "build"}


def main() -> int:
    if len(sys.argv) != 8:
        print(
            "usage: verify_real_build.sh PROJECT_DIR PACKAGE_AGENT_BIN CONFIG_JSON "
            "PLATFORM ENVIRONMENT ARTIFACT_TYPE OUTPUT_DIR",
            file=sys.stderr,
        )
        return 2

    project_dir = Path(sys.argv[1]).resolve()
    package_agent = Path(sys.argv[2]).resolve()
    config_path = Path(sys.argv[3]).resolve()
    platform = sys.argv[4]
    environment = sys.argv[5]
    artifact_type = sys.argv[6]
    output_dir = Path(sys.argv[7]).resolve()
    agent_output = output_dir / "agent"
    input_path = output_dir / "build-input.json"
    stdout_path = output_dir / "agent.stdout.log"
    stderr_path = output_dir / "agent.stderr.log"
    acceptance_path = output_dir / "acceptance.json"

    output_dir.mkdir(parents=True, exist_ok=True)
    agent_output.mkdir(parents=True, exist_ok=True)

    acceptance: dict[str, Any] = {
        "status": "failed",
        "classification": "acceptance_failed",
        "summary": "Real build acceptance did not complete.",
        "platform": platform,
        "environment": environment,
        "artifactType": artifact_type,
        "projectDir": str(project_dir),
        "agentExitCode": None,
        "agentReport": None,
        "artifacts": {
            "report": str(agent_output / "report.json"),
            "packagePaths": [],
            "symbolsPaths": [],
            "logPaths": [],
        },
        "git": {
            "headBefore": None,
            "headAfter": None,
            "beforeStatus": [],
            "afterStatus": [],
            "violations": [],
            "unexpectedPaths": [],
        },
        "logs": {
            "stdout": str(stdout_path),
            "stderr": str(stderr_path),
        },
    }

    try:
        before_head = git_head(project_dir)
        before_status = git_snapshot(project_dir)
        acceptance["git"]["headBefore"] = before_head
        acceptance["git"]["beforeStatus"] = serialize_snapshot(before_status)

        input_path.write_text(
            json.dumps(
                {
                    "buildId": "real-build-acceptance",
                    "projectDir": str(project_dir),
                    "platform": platform,
                    "environment": environment,
                    "artifactType": artifact_type,
                    "gitUrl": "",
                    "gitRef": "",
                    "repoSubpath": "",
                    "commitSha": before_head,
                    "maxAttempts": 5,
                },
                ensure_ascii=False,
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )

        completed = subprocess.run(
            [
                str(package_agent),
                "build",
                "--input",
                str(input_path),
                "--output",
                str(agent_output),
                "--config",
                str(config_path),
            ],
            cwd=project_dir,
            text=True,
            capture_output=True,
            check=False,
        )
        stdout_path.write_text(completed.stdout, encoding="utf-8")
        stderr_path.write_text(completed.stderr, encoding="utf-8")
        acceptance["agentExitCode"] = completed.returncode

        report_path = agent_output / "report.json"
        if report_path.is_file():
            try:
                report = json.loads(report_path.read_text(encoding="utf-8"))
            except (OSError, json.JSONDecodeError) as error:
                acceptance["classification"] = "invalid_agent_report"
                acceptance["summary"] = f"Could not parse report.json: {type(error).__name__}."
            else:
                acceptance["agentReport"] = report
                for key in ("packagePaths", "symbolsPaths", "logPaths"):
                    values = report.get(key) if isinstance(report, dict) else None
                    if isinstance(values, list):
                        acceptance["artifacts"][key] = [str(value) for value in values]
                if not isinstance(report, dict) or report.get("status") != "success":
                    acceptance["classification"] = "agent_report_not_success"
                    acceptance["summary"] = "package-agent did not report success."
                elif completed.returncode != 0:
                    acceptance["classification"] = "agent_process_failed"
                    acceptance["summary"] = "package-agent exited unsuccessfully."
                elif not required_artifacts_exist(report):
                    acceptance["classification"] = "missing_required_artifacts"
                    acceptance["summary"] = (
                        "package-agent success report is missing package, symbols, or log artifacts."
                    )
                else:
                    acceptance["classification"] = "build_succeeded"
                    acceptance["summary"] = "package-agent produced all required artifacts."
        else:
            acceptance["classification"] = "missing_agent_report"
            acceptance["summary"] = "package-agent did not produce report.json."
    except (OSError, subprocess.SubprocessError, ValueError) as error:
        acceptance["classification"] = "acceptance_execution_failed"
        acceptance["summary"] = f"Acceptance execution failed: {type(error).__name__}."
        stdout_path.write_text("", encoding="utf-8") if not stdout_path.exists() else None
        stderr_path.write_text(str(error), encoding="utf-8") if not stderr_path.exists() else None
    finally:
        finalize_git_check(project_dir, acceptance)
        if acceptance["git"]["violations"]:
            acceptance["status"] = "failed"
            acceptance["classification"] = "immutable_project_violation"
            acceptance["summary"] = "The build changed protected project or Git state."
        elif acceptance["classification"] == "build_succeeded":
            acceptance["status"] = "success"
        else:
            acceptance["status"] = "failed"
        acceptance_path.write_text(
            json.dumps(acceptance, ensure_ascii=False, indent=2) + "\n",
            encoding="utf-8",
        )

    return 0 if acceptance["status"] == "success" else 1


def git_head(project_dir: Path) -> str:
    return subprocess.check_output(
        ["git", "-C", str(project_dir), "rev-parse", "HEAD"],
        text=True,
    ).strip()


def git_snapshot(project_dir: Path) -> dict[str, dict[str, str]]:
    raw = subprocess.check_output(
        [
            "git",
            "-C",
            str(project_dir),
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
        ]
    )
    fields = raw.decode("utf-8", errors="surrogateescape").split("\0")
    snapshot: dict[str, dict[str, str]] = {}
    index = 0
    while index < len(fields):
        field = fields[index]
        index += 1
        if not field:
            continue
        code = field[:2]
        path = field[3:] if len(field) >= 3 else ""
        if code[0] in {"R", "C"} and index < len(fields):
            index += 1
        if not path:
            continue
        absolute = project_dir / path
        snapshot[path] = {"code": code, "digest": file_digest(absolute)}
    return snapshot


def file_digest(path: Path) -> str:
    if not path.exists() or not path.is_file():
        return "<missing>"
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def finalize_git_check(project_dir: Path, acceptance: dict[str, Any]) -> None:
    git = acceptance["git"]
    try:
        after_head = git_head(project_dir)
        after_status = git_snapshot(project_dir)
    except (OSError, subprocess.SubprocessError) as error:
        git["violations"].append("git_state_unreadable")
        git["afterStatus"] = []
        git["headAfter"] = None
        acceptance["summary"] = f"Could not read final Git state: {type(error).__name__}."
        return

    before_status = {
        entry["path"]: {"code": entry["code"], "digest": entry["digest"]}
        for entry in parse_serialized_status(git["beforeStatus"])
    }
    git["headAfter"] = after_head
    git["afterStatus"] = serialize_snapshot(after_status)

    if git["headBefore"] != after_head:
        git["violations"].append("head_changed")

    for path, entry in after_status.items():
        if path not in before_status:
            if not allowed_project_path(project_dir, path, acceptance):
                git["unexpectedPaths"].append(path)
            continue
        previous = before_status[path]
        if previous["code"] != entry["code"] or previous["digest"] != entry["digest"]:
            git["violations"].append("preexisting_path_changed")

    for path in before_status:
        if path not in after_status:
            git["violations"].append("preexisting_path_removed")

    if git["unexpectedPaths"]:
        git["violations"].append("unexpected_project_path")
    git["violations"] = sorted(set(git["violations"]))
    git["unexpectedPaths"] = sorted(set(git["unexpectedPaths"]))


def parse_serialized_status(entries: list[str]) -> list[dict[str, str]]:
    result: list[dict[str, str]] = []
    for entry in entries:
        if not isinstance(entry, str) or "\t" not in entry:
            continue
        code, path, digest = entry.split("\t", 2)
        result.append({"code": code, "path": path, "digest": digest})
    return result


def serialize_snapshot(snapshot: dict[str, dict[str, str]]) -> list[str]:
    return [
        f"{entry['code']}\t{path}\t{entry['digest']}"
        for path, entry in sorted(snapshot.items())
    ]


def required_artifacts_exist(report: dict[str, Any]) -> bool:
    for key in ("packagePaths", "symbolsPaths", "logPaths"):
        values = report.get(key)
        if not isinstance(values, list) or not values:
            return False
        if not any(Path(str(value)).is_file() for value in values):
            return False
    return True


def allowed_project_path(
    project_dir: Path,
    relative_path: str,
    acceptance: dict[str, Any],
) -> bool:
    path = Path(relative_path)
    parts = path.parts
    if parts and parts[0] in ALLOWED_GENERATED_ROOTS:
        return True
    try:
        candidate = (project_dir / path).resolve()
        output_dir = Path(acceptance["logs"]["stdout"]).parent.resolve()
        candidate.relative_to(output_dir)
        return True
    except ValueError:
        return False


if __name__ == "__main__":
    raise SystemExit(main())
PY

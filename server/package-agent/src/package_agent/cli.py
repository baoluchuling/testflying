from __future__ import annotations

import argparse
import glob
import json
import shlex
import shutil
import subprocess
from pathlib import Path
from typing import Any

from package_agent.llm_discovery import discover_llm_adapter
from package_agent.llm_planner import LlmPlan, PlanAction, request_llm_plan
from package_agent.models import AgentReport, BuildInput, classify_build
from package_agent.policy import Action, evaluate_action
from package_agent.redaction import redact_text

CONFIG_FILE = "testflying-package-agent.json"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="package-agent")
    subparsers = parser.add_subparsers(dest="command", required=True)

    build_parser = subparsers.add_parser("build")
    build_parser.add_argument("--input", required=True)
    build_parser.add_argument("--output", required=True)
    build_parser.add_argument("--config")

    args = parser.parse_args(argv)
    if args.command != "build":
        parser.error(f"unsupported command: {args.command}")

    output_dir = Path(args.output)
    output_dir.mkdir(parents=True, exist_ok=True)

    report = _build_report(
        Path(args.input),
        output_dir=output_dir,
        config_override=Path(args.config).resolve() if args.config else None,
    )
    _write_report(output_dir=output_dir, report=report)
    return report.exit_code()


def _build_report(
    input_path: Path,
    *,
    output_dir: Path,
    config_override: Path | None = None,
) -> AgentReport:
    if not input_path.exists():
        return AgentReport(
            status="needs_human",
            classification="missing_input",
            summary="Build input JSON does not exist.",
        )

    try:
        payload = json.loads(input_path.read_text(encoding="utf-8"))
    except json.JSONDecodeError:
        return AgentReport(
            status="failed",
            classification="invalid_input_json",
            summary="Build input is not valid JSON.",
        )

    if not isinstance(payload, dict):
        return AgentReport(
            status="failed",
            classification="invalid_input_shape",
            summary="Build input JSON must be an object.",
        )

    try:
        build_input = BuildInput.from_dict(payload)
    except ValueError as exc:
        return AgentReport(
            status="failed",
            classification="invalid_input",
            summary=str(exc),
        )

    if config_override is not None:
        if not config_override.is_file():
            return AgentReport(
                status="needs_human",
                classification="missing_build_config",
                summary="The requested external build config is not a readable regular file.",
                commit_sha=build_input.commit_sha,
                max_attempts=build_input.max_attempts,
            )
        return _build_from_project_config(
            build_input,
            config_path=config_override,
            output_dir=output_dir,
        )

    config_path = Path(build_input.project_dir) / CONFIG_FILE
    if config_path.exists():
        return _build_from_project_config(
            build_input,
            config_path=config_path,
            output_dir=output_dir,
        )

    adapter = discover_llm_adapter()
    if not any((build_input.package_paths, build_input.symbols_paths, build_input.log_paths)):
        return _build_from_llm_plan(build_input, output_dir=output_dir, adapter=adapter)

    return classify_build(build_input=build_input, adapter_name=adapter.name if adapter else None)


def _write_report(output_dir: Path, report: AgentReport) -> None:
    report_path = output_dir / "report.json"
    report_payload = json.dumps(report.to_dict(), ensure_ascii=False, indent=2)
    report_path.write_text(report_payload, encoding="utf-8")


def _build_from_project_config(
    build_input: BuildInput,
    *,
    config_path: Path,
    output_dir: Path,
) -> AgentReport:
    try:
        config = json.loads(config_path.read_text(encoding="utf-8"))
    except OSError:
        return AgentReport(
            status="needs_human",
            classification="unreadable_build_config",
            summary="The build config is not readable.",
            commit_sha=build_input.commit_sha,
            max_attempts=build_input.max_attempts,
        )
    except json.JSONDecodeError:
        return AgentReport(
            status="failed",
            classification="invalid_build_config_json",
            summary=f"{CONFIG_FILE} is not valid JSON.",
            commit_sha=build_input.commit_sha,
            max_attempts=build_input.max_attempts,
        )
    if not isinstance(config, dict):
        return AgentReport(
            status="failed",
            classification="invalid_build_config_shape",
            summary=f"{CONFIG_FILE} must be a JSON object.",
            commit_sha=build_input.commit_sha,
            max_attempts=build_input.max_attempts,
        )

    command = _config_command(config.get("buildCommand"))
    if not command:
        return AgentReport(
            status="needs_human",
            classification="missing_build_command",
            summary=f"{CONFIG_FILE} must define buildCommand.",
            human_action="Add a non-empty buildCommand array or string.",
            commit_sha=build_input.commit_sha,
            max_attempts=build_input.max_attempts,
        )

    decision = evaluate_action(Action(kind="build", command=command))
    if not decision.allowed:
        return AgentReport(
            status="needs_human",
            classification=decision.reason,
            summary=f"Configured build command is blocked by policy: {decision.reason}.",
            human_action=(
                "Replace the buildCommand with a policy-allowed build command that does "
                "not modify protected project files."
            ),
            commit_sha=build_input.commit_sha,
            max_attempts=build_input.max_attempts,
        )

    output_dir.mkdir(parents=True, exist_ok=True)
    log_path = output_dir / "command.log"
    run_result = subprocess.run(
        command,
        cwd=Path(build_input.project_dir),
        capture_output=True,
        text=True,
        timeout=int(config.get("timeoutSeconds") or 1800),
        check=False,
    )
    log_text = redact_text(
        f"$ {shlex.join(command)}\n\n[stdout]\n{run_result.stdout}\n\n[stderr]\n{run_result.stderr}"
    )
    log_path.write_text(log_text, encoding="utf-8")

    collected = _collect_config_artifacts(
        project_dir=Path(build_input.project_dir),
        output_dir=output_dir,
        config=config,
    )
    collected["log"].insert(0, str(log_path))

    if run_result.returncode != 0:
        return AgentReport(
            status="failed",
            classification="build_command_failed",
            summary=redact_text(
                f"Configured build command exited with code {run_result.returncode}."
            ),
            human_action="Inspect command.log, fix the project build, and rerun.",
            package_paths=collected["package"],
            symbols_paths=collected["symbols"],
            log_paths=collected["log"],
            commit_sha=build_input.commit_sha,
            max_attempts=build_input.max_attempts,
        )

    report = classify_build(
        build_input=BuildInput(
            project_dir=str(output_dir),
            platform=build_input.platform,
            environment=build_input.environment,
            artifact_type=build_input.artifact_type,
            build_id=build_input.build_id,
            git_url=build_input.git_url,
            git_ref=build_input.git_ref,
            repo_subpath=build_input.repo_subpath,
            commit_sha=build_input.commit_sha,
            max_attempts=build_input.max_attempts,
            package_paths=collected["package"],
            symbols_paths=collected["symbols"],
            log_paths=collected["log"],
        ),
        adapter_name="project_config",
    )
    return AgentReport(
        status=report.status,
        classification=report.classification,
        summary=redact_text(str(config.get("summary") or report.summary)),
        human_action=report.human_action,
        package_paths=report.package_paths,
        symbols_paths=report.symbols_paths,
        log_paths=report.log_paths,
        version=str(config.get("version") or "") or None,
        build_number=str(config.get("buildNumber") or "") or None,
        commit_sha=build_input.commit_sha,
        adapter=report.adapter,
        max_attempts=report.max_attempts,
    )


def _build_from_llm_plan(
    build_input: BuildInput,
    *,
    output_dir: Path,
    adapter: object,
) -> AgentReport:
    if adapter is None:
        return AgentReport(
            status="needs_human",
            classification="llm_unavailable",
            summary="No supported LLM adapter was discovered in automatic order.",
            human_action=(
                f"Add {CONFIG_FILE} with safe buildCommand and artifact globs, "
                "or configure Codex CLI, Claude CLI, or llm-runtime."
            ),
            commit_sha=build_input.commit_sha,
            max_attempts=build_input.max_attempts,
        )

    try:
        plan = request_llm_plan(adapter, build_input)
    except (OSError, subprocess.SubprocessError, ValueError) as exc:
        return AgentReport(
            status="needs_human",
            classification="invalid_llm_plan",
            summary=redact_text(f"LLM planner could not produce a valid plan: {exc}."),
            human_action=f"Add {CONFIG_FILE} or adjust the LLM planner to return valid JSON.",
            commit_sha=build_input.commit_sha,
            adapter=getattr(adapter, "name", None),
            max_attempts=build_input.max_attempts,
        )

    blocked = _first_blocked_plan_action(plan)
    if blocked is not None:
        action, reason = blocked
        return AgentReport(
            status="needs_human",
            classification=reason,
            summary=f"LLM plan action '{action.kind}' is blocked by policy: {reason}.",
            human_action="Revise the build plan so every action stays within package-agent policy.",
            commit_sha=build_input.commit_sha,
            adapter=getattr(adapter, "name", None),
            max_attempts=build_input.max_attempts,
        )

    project_dir = Path(build_input.project_dir)
    collected = {"package": [], "symbols": [], "log": []}
    last_failure = ""
    for attempt in range(1, build_input.max_attempts + 1):
        attempt_log = output_dir / f"llm-plan-attempt-{attempt}.log"
        action_failed = False
        log_parts: list[str] = []
        for action in plan.actions:
            if not action.command:
                _collect_plan_artifacts(project_dir, output_dir, action, collected)
                continue
            result = subprocess.run(
                action.command,
                cwd=project_dir,
                capture_output=True,
                text=True,
                timeout=1800,
                check=False,
            )
            log_parts.append(
                "\n".join(
                    [
                        f"$ {shlex.join(action.command)}",
                        f"[exit] {result.returncode}",
                        "[stdout]",
                        result.stdout,
                        "[stderr]",
                        result.stderr,
                    ]
                )
            )
            _collect_plan_artifacts(project_dir, output_dir, action, collected)
            if result.returncode != 0:
                last_failure = (
                    f"LLM plan action '{action.kind}' exited with code {result.returncode}."
                )
                action_failed = True
                break
        attempt_log.write_text(redact_text("\n\n".join(log_parts)), encoding="utf-8")
        collected["log"].append(str(attempt_log))

        report = classify_build(
            build_input=BuildInput(
                project_dir=str(output_dir),
                platform=build_input.platform,
                environment=build_input.environment,
                artifact_type=build_input.artifact_type,
                build_id=build_input.build_id,
                git_url=build_input.git_url,
                git_ref=build_input.git_ref,
                repo_subpath=build_input.repo_subpath,
                commit_sha=build_input.commit_sha,
                max_attempts=build_input.max_attempts,
                package_paths=collected["package"],
                symbols_paths=collected["symbols"],
                log_paths=collected["log"],
            ),
            adapter_name=getattr(adapter, "name", None),
        )
        if report.status == "success":
            return report
        if action_failed:
            continue

    summary = (
        last_failure
        or "LLM plan finished without required package, symbols, and log artifacts."
    )
    return AgentReport(
        status="needs_human",
        classification="missing_artifacts",
        summary=redact_text(summary),
        human_action=f"Add {CONFIG_FILE} or update the planner artifact globs and rerun.",
        package_paths=collected["package"],
        symbols_paths=collected["symbols"],
        log_paths=collected["log"],
        commit_sha=build_input.commit_sha,
        adapter=getattr(adapter, "name", None),
        max_attempts=build_input.max_attempts,
    )


def _first_blocked_plan_action(plan: LlmPlan) -> tuple[PlanAction, str] | None:
    for action in plan.actions:
        decision = evaluate_action(Action(kind=action.kind, command=action.command))
        if not decision.allowed:
            return action, decision.reason
    return None


def _collect_plan_artifacts(
    project_dir: Path,
    output_dir: Path,
    action: PlanAction,
    collected: dict[str, list[str]],
) -> None:
    collected["package"].extend(
        _copy_artifact_matches(project_dir, output_dir, "package", action.package_paths)
    )
    collected["symbols"].extend(
        _copy_artifact_matches(project_dir, output_dir, "symbols", action.symbols_paths)
    )
    collected["log"].extend(
        _copy_artifact_matches(project_dir, output_dir, "log", action.log_paths)
    )


def _config_command(value: object) -> list[str]:
    if isinstance(value, list) and all(isinstance(item, str) and item for item in value):
        return list(value)
    if isinstance(value, str) and value.strip():
        return shlex.split(value)
    return []


def _collect_config_artifacts(
    *,
    project_dir: Path,
    output_dir: Path,
    config: dict[str, Any],
) -> dict[str, list[str]]:
    return {
        "package": _copy_artifact_matches(
            project_dir,
            output_dir,
            "package",
            config.get("packagePaths"),
        ),
        "symbols": _copy_artifact_matches(
            project_dir,
            output_dir,
            "symbols",
            config.get("symbolsPaths"),
        ),
        "log": _copy_artifact_matches(project_dir, output_dir, "log", config.get("logPaths")),
    }


def _copy_artifact_matches(
    project_dir: Path,
    output_dir: Path,
    artifact_type: str,
    raw_patterns: object,
) -> list[str]:
    if not isinstance(raw_patterns, list) or not all(
        isinstance(item, str) for item in raw_patterns
    ):
        return []

    project_root = project_dir.resolve()
    artifact_dir = output_dir / "artifacts" / artifact_type
    artifact_dir.mkdir(parents=True, exist_ok=True)
    copied: list[str] = []
    for pattern in raw_patterns:
        for match in sorted(glob.glob(str(project_dir / pattern), recursive=True)):
            source = Path(match).resolve()
            if not source.is_file() or not _is_within(project_root, source):
                continue
            destination = artifact_dir / source.name
            if destination.exists():
                destination = artifact_dir / f"{source.stem}-{len(copied)}{source.suffix}"
            if artifact_type == "log":
                _copy_redacted_log(source, destination)
            else:
                shutil.copy2(source, destination)
            copied.append(str(destination))
    return copied


def _is_within(root: Path, candidate: Path) -> bool:
    try:
        candidate.relative_to(root)
    except ValueError:
        return False
    return True


def _copy_redacted_log(source: Path, destination: Path) -> None:
    try:
        text = source.read_text(encoding="utf-8")
    except UnicodeDecodeError:
        shutil.copy2(source, destination)
        return
    destination.write_text(redact_text(text), encoding="utf-8")


if __name__ == "__main__":
    raise SystemExit(main())

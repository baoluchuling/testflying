from __future__ import annotations

from dataclasses import asdict, dataclass, field
from pathlib import Path

MAX_BUILD_ATTEMPTS = 5


@dataclass(frozen=True)
class BuildInput:
    project_dir: str
    platform: str
    environment: str
    artifact_type: str
    build_id: str | None = None
    git_url: str = ""
    git_ref: str = ""
    repo_subpath: str = ""
    commit_sha: str | None = None
    max_attempts: int = MAX_BUILD_ATTEMPTS
    package_paths: list[str] = field(default_factory=list)
    symbols_paths: list[str] = field(default_factory=list)
    log_paths: list[str] = field(default_factory=list)

    @classmethod
    def from_dict(cls, payload: dict[str, object]) -> BuildInput:
        required_keys = ("projectDir", "platform", "environment", "artifactType")
        missing = [key for key in required_keys if not isinstance(payload.get(key), str)]
        if missing:
            raise ValueError(f"missing required string fields: {', '.join(missing)}")

        max_attempts = payload.get("maxAttempts", MAX_BUILD_ATTEMPTS)
        if isinstance(max_attempts, bool) or not isinstance(max_attempts, int) or max_attempts < 1:
            raise ValueError("maxAttempts must be a positive integer when provided")
        if max_attempts > MAX_BUILD_ATTEMPTS:
            raise ValueError(f"maxAttempts must be <= {MAX_BUILD_ATTEMPTS}")

        return cls(
            build_id=str(payload["buildId"]) if isinstance(payload.get("buildId"), str) else None,
            project_dir=str(payload["projectDir"]),
            platform=str(payload["platform"]),
            environment=str(payload["environment"]),
            artifact_type=str(payload["artifactType"]),
            git_url=str(payload.get("gitUrl") or ""),
            git_ref=str(payload.get("gitRef") or ""),
            repo_subpath=str(payload.get("repoSubpath") or ""),
            commit_sha=(
                str(payload["commitSha"]) if isinstance(payload.get("commitSha"), str) else None
            ),
            max_attempts=max_attempts,
            package_paths=_string_list(payload.get("packagePaths")),
            symbols_paths=_string_list(payload.get("symbolsPaths")),
            log_paths=_string_list(payload.get("logPaths")),
        )


@dataclass(frozen=True)
class AgentReport:
    status: str
    classification: str
    summary: str
    human_action: str = ""
    package_paths: list[str] = field(default_factory=list)
    symbols_paths: list[str] = field(default_factory=list)
    log_paths: list[str] = field(default_factory=list)
    version: str | None = None
    build_number: str | None = None
    commit_sha: str | None = None
    adapter: str | None = None
    max_attempts: int = MAX_BUILD_ATTEMPTS

    def to_dict(self) -> dict[str, object]:
        payload = asdict(self)
        return {
            "status": payload["status"],
            "classification": payload["classification"],
            "summary": payload["summary"],
            "humanAction": payload["human_action"],
            "packagePaths": payload["package_paths"],
            "symbolsPaths": payload["symbols_paths"],
            "logPaths": payload["log_paths"],
            "version": payload["version"],
            "buildNumber": payload["build_number"],
            "commitSha": payload["commit_sha"],
            "adapter": payload["adapter"],
            "maxAttempts": payload["max_attempts"],
        }

    def exit_code(self) -> int:
        if self.status == "success":
            return 0
        if self.status == "failed":
            return 1
        return 2

    def artifacts_complete(self) -> bool:
        return all((self.package_paths, self.symbols_paths, self.log_paths))


def classify_build(build_input: BuildInput, adapter_name: str | None) -> AgentReport:
    package_paths = _existing_paths(build_input.package_paths)
    symbols_paths = _existing_paths(build_input.symbols_paths)
    log_paths = _existing_paths(build_input.log_paths)

    if package_paths and symbols_paths and log_paths:
        return AgentReport(
            status="success",
            classification="build_succeeded",
            summary="Required package, symbols, and logs are present.",
            human_action="",
            package_paths=package_paths,
            symbols_paths=symbols_paths,
            log_paths=log_paths,
            commit_sha=build_input.commit_sha,
            adapter=adapter_name,
            max_attempts=build_input.max_attempts,
        )

    if adapter_name is None:
        return AgentReport(
            status="needs_human",
            classification="llm_unavailable",
            summary="No supported LLM adapter was discovered in automatic order.",
            human_action=(
                "Add testflying-package-agent.json with a safe buildCommand and artifact "
                "globs, or configure a supported LLM adapter."
            ),
            package_paths=package_paths,
            symbols_paths=symbols_paths,
            log_paths=log_paths,
            commit_sha=build_input.commit_sha,
            adapter=None,
            max_attempts=build_input.max_attempts,
        )

    return AgentReport(
        status="needs_human",
        classification="missing_artifacts",
        summary="Automatic success requires package, symbols, and logs.",
        human_action=(
            "Update testflying-package-agent.json artifact globs or inspect the build output."
        ),
        package_paths=package_paths,
        symbols_paths=symbols_paths,
        log_paths=log_paths,
        commit_sha=build_input.commit_sha,
        adapter=adapter_name,
        max_attempts=build_input.max_attempts,
    )


def _string_list(value: object) -> list[str]:
    if value is None:
        return []
    if not isinstance(value, list) or not all(isinstance(item, str) for item in value):
        raise ValueError("artifact path fields must be string lists when provided")
    return list(value)


def _existing_paths(paths: list[str]) -> list[str]:
    return [path for path in paths if Path(path).exists()]

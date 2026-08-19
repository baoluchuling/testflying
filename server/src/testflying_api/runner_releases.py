from __future__ import annotations

import json
import re
from dataclasses import dataclass
from pathlib import Path

from testflying_api.errors import ApiError

SEMANTIC_VERSION_RE = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+$")
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
ALLOWED_PLATFORMS = {"darwin"}
ALLOWED_ARCHITECTURES = {"arm64", "amd64"}


@dataclass(frozen=True)
class RunnerReleaseManifest:
    version: str
    runner_version: str
    package_agent_version: str
    platform: str
    arch: str
    bundle_file: str
    bundle_path: Path
    sha256: str

    @classmethod
    def load(cls, root: Path, platform: str, arch: str) -> RunnerReleaseManifest:
        normalized_platform = platform.strip().lower()
        normalized_arch = arch.strip().lower()
        if normalized_platform not in ALLOWED_PLATFORMS:
            raise ApiError(
                "unsupported_runner_platform",
                "Runner platform 不受支持",
                status_code=422,
            )
        if normalized_arch not in ALLOWED_ARCHITECTURES:
            raise ApiError("unsupported_runner_arch", "Runner arch 不受支持", status_code=422)

        release_root = root.resolve()
        manifest_path = (
            release_root / normalized_platform / normalized_arch / "release.json"
        ).resolve()
        _require_within(release_root, manifest_path)
        if not manifest_path.is_file():
            raise ApiError("runner_release_not_found", "Runner release 不存在", status_code=404)
        try:
            payload = json.loads(manifest_path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            raise ApiError(
                "invalid_runner_release_manifest",
                "Runner release manifest 无效",
                status_code=500,
            ) from error
        if not isinstance(payload, dict):
            raise ApiError(
                "invalid_runner_release_manifest",
                "Runner release manifest 必须是对象",
                status_code=500,
            )

        version = _semantic_version(payload, "version")
        runner_version = _semantic_version(payload, "runnerVersion")
        package_agent_version = _semantic_version(payload, "packageAgentVersion")
        manifest_platform = _required_string(payload, "platform").lower()
        manifest_arch = _required_string(payload, "arch").lower()
        if manifest_platform != normalized_platform or manifest_arch != normalized_arch:
            raise ApiError(
                "runner_release_scope_mismatch",
                "Runner release manifest 与请求平台不匹配",
                status_code=500,
            )

        bundle_file = _required_string(payload, "bundleFile")
        if (
            Path(bundle_file).name != bundle_file
            or "\\" in bundle_file
            or not bundle_file.endswith(".zip")
        ):
            raise ApiError(
                "invalid_runner_release_bundle",
                "Runner release bundle 文件名无效",
                status_code=500,
            )
        bundle_path = (manifest_path.parent / bundle_file).resolve()
        _require_within(release_root, bundle_path)
        if not bundle_path.is_file():
            raise ApiError(
                "runner_release_bundle_not_found",
                "Runner bundle 不存在",
                status_code=404,
            )

        digest = _required_string(payload, "sha256")
        if not SHA256_RE.fullmatch(digest):
            raise ApiError(
                "invalid_runner_release_sha256",
                "Runner release SHA-256 无效",
                status_code=500,
            )
        return cls(
            version=version,
            runner_version=runner_version,
            package_agent_version=package_agent_version,
            platform=manifest_platform,
            arch=manifest_arch,
            bundle_file=bundle_file,
            bundle_path=bundle_path,
            sha256=digest,
        )


def runner_release_status(
    root: Path,
    *,
    platform: str,
    arch: str,
    runner_version: str,
    package_agent_version: str,
) -> tuple[str, str, str]:
    if not platform.strip() or not arch.strip():
        return "", "unknown", "未检测到发布版本"
    try:
        manifest = RunnerReleaseManifest.load(root, platform, arch)
    except ApiError:
        return "", "unknown", "未检测到发布版本"
    if (
        runner_version.strip() == manifest.runner_version
        and package_agent_version.strip() == manifest.package_agent_version
    ):
        return manifest.version, "current", "已是最新版本"
    return manifest.version, "outdated", f"可更新至 {manifest.version}"


def _semantic_version(payload: dict[str, object], key: str) -> str:
    value = _required_string(payload, key)
    if not SEMANTIC_VERSION_RE.fullmatch(value):
        raise ApiError(
            "invalid_runner_release_version",
            f"Runner release {key} 无效",
            status_code=500,
        )
    return value


def _required_string(payload: dict[str, object], key: str) -> str:
    value = payload.get(key)
    if not isinstance(value, str) or not value.strip():
        raise ApiError(
            "invalid_runner_release_manifest",
            f"Runner release {key} 必须是非空字符串",
            status_code=500,
        )
    return value.strip()


def _require_within(root: Path, candidate: Path) -> None:
    try:
        candidate.relative_to(root)
    except ValueError as error:
        raise ApiError(
            "runner_release_path_escape",
            "Runner release 路径越界",
            status_code=500,
        ) from error

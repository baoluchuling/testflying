from __future__ import annotations

from enum import StrEnum


class Platform(StrEnum):
    IOS = "ios"
    ANDROID = "android"


class BuildEnvironment(StrEnum):
    DEVELOPMENT = "development"
    PRODUCTION = "production"


class BuildSource(StrEnum):
    UPLOAD = "upload"
    AGENT = "agent"


class BuildLifecycleStatus(StrEnum):
    QUEUED = "queued"
    ASSIGNED = "assigned"
    PREPARING = "preparing"
    BUILDING = "building"
    DIAGNOSING = "diagnosing"
    REPAIRING = "repairing"
    RETRYING = "retrying"
    UPLOADING_ARTIFACTS = "uploading_artifacts"
    SUCCEEDED = "succeeded"
    FAILED = "failed"
    NEEDS_HUMAN = "needs_human"
    CANCELLED = "cancelled"


class ArtifactType(StrEnum):
    PACKAGE = "package"
    SYMBOLS = "symbols"
    REPORT = "report"
    LOG = "log"
    MANIFEST = "manifest"


class NotificationType(StrEnum):
    BUILD = "build"
    ACCOUNT = "account"
    DEVICE = "device"


def channel_for_environment(environment: str) -> str:
    return "prod" if environment == BuildEnvironment.PRODUCTION else "dev"


def normalize_platform(value: str) -> Platform:
    normalized = value.strip().lower()
    if normalized == Platform.ANDROID:
        return Platform.ANDROID
    if normalized == Platform.IOS:
        return Platform.IOS
    raise ValueError(f"unsupported platform: {value}")


def normalize_environment(value: str) -> BuildEnvironment:
    normalized = value.strip().lower()
    if normalized == BuildEnvironment.PRODUCTION:
        return BuildEnvironment.PRODUCTION
    if normalized == BuildEnvironment.DEVELOPMENT:
        return BuildEnvironment.DEVELOPMENT
    raise ValueError(f"unsupported environment: {value}")

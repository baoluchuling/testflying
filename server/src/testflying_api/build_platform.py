from __future__ import annotations

import hmac
import re
import secrets
from datetime import UTC, datetime, timedelta
from hashlib import sha256
from uuid import uuid4

from sqlalchemy import select
from sqlalchemy.orm import Session, joinedload, selectinload

from testflying_api.admin.view_models import (
    environment_label,
    format_datetime,
    format_size,
    platform_label,
)
from testflying_api.admin_api.errors import AdminApiError
from testflying_api.admin_api.schemas import (
    AppDetailState,
    BuildAppItem,
    BuildAppsState,
    BuildAppSummary,
    BuildArtifactItem,
    BuildEventItem,
    BuildItem,
    BuildSettingItem,
)
from testflying_api.build_notifications import enqueue_terminal_build_notifications
from testflying_api.domain import (
    ArtifactType,
    BuildLifecycleStatus,
    channel_for_environment,
    normalize_environment,
)
from testflying_api.errors import ApiError
from testflying_api.redaction import redact_json, redact_text
from testflying_api.schema import App, AppBuildSetting, Artifact, Build, BuildEvent, BuildRunner
from testflying_api.storage import ArtifactStorage

_CREDENTIAL_REF_MAX_LENGTH = 120
_CREDENTIAL_REF_ID_MAX_LENGTH = 40
_PEM_MARKER_RE = re.compile(r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----", re.IGNORECASE)
_JWT_RE = re.compile(r"^[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+$")
_HEX_SECRET_RE = re.compile(r"^[A-Fa-f0-9]{32,}$")
_TOKEN_PREFIXES = ("ghp_", "gho_", "github_pat_", "glpat-", "sk-", "xoxb-", "xoxp-")
_SECRET_WORD_RE = re.compile(r"(secret|token|apikey|api_key|password|passwd|private[_-]?key)", re.I)
_CREDENTIAL_REF_ID_RE = re.compile(
    r"^(?:git|ios|android|apple|google|keystore|certificate|provisioning|signing|runner|mac)"
    r"-[a-z0-9]+(?:-[a-z0-9]+)*$"
)
_RUNNER_REQUIRED_ARTIFACTS = {
    ArtifactType.PACKAGE.value,
    ArtifactType.SYMBOLS.value,
    ArtifactType.REPORT.value,
    ArtifactType.LOG.value,
}
_RUNNER_ALLOWED_ARTIFACTS = _RUNNER_REQUIRED_ARTIFACTS
_RUNNER_TOKEN_DIGEST_PREFIX = "hmac-sha256:"
_RUNNER_ASSIGNMENT_LEASE_SECONDS = 15 * 60
_RUNNER_ID_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$")
RUNNER_BUILD_PLATFORMS = ("ios", "android")
_FAILURE_CLASSIFICATION_ALLOWED_RE = re.compile(r"[^a-z0-9_:-]+")
_FAILURE_CLASSIFICATION_SECRET_RE = re.compile(
    r"(?i)(-----begin [a-z0-9 ]*private key-----|"
    r"\b(password|token|secret|api[_-]?key|private[_-]?key)\b\s*[:=])"
)
_FAILURE_CLASSIFICATION_MAX_LENGTH = 80
_FAILURE_CLASSIFICATION_FALLBACK = "runner_reported_failure"
_TERMINAL_BUILD_LIFECYCLE_STATUSES = {
    BuildLifecycleStatus.SUCCEEDED.value,
    BuildLifecycleStatus.FAILED.value,
    BuildLifecycleStatus.NEEDS_HUMAN.value,
    BuildLifecycleStatus.CANCELLED.value,
}
_RUNNER_EVENT_LIFECYCLE_STATUSES = {
    status.value
    for status in BuildLifecycleStatus
    if status.value not in _TERMINAL_BUILD_LIFECYCLE_STATUSES
}


def app_or_404(session: Session, app_id: str) -> App:
    app = session.get(App, app_id)
    if app is None:
        raise ApiError("app_not_found", "应用不存在", status_code=404)
    return app


def list_app_builds(session: Session, app_id: str) -> list[Build]:
    return list(
        session.scalars(
            select(Build)
            .where(Build.app_id == app_id)
            .options(joinedload(Build.app), selectinload(Build.artifacts))
            .order_by(Build.uploaded_at.desc())
        )
    )


def get_build_setting(session: Session, app_id: str) -> AppBuildSetting | None:
    return session.scalar(
        select(AppBuildSetting).where(AppBuildSetting.app_id == app_id)
    )


def app_detail_state(session: Session, app_id: str) -> AppDetailState:
    app = app_or_404(session, app_id)
    return AppDetailState(
        app=_build_app_summary(app),
        builds=[build_item(build) for build in list_app_builds(session, app.id)],
        build_setting=build_setting_item(get_build_setting(session, app.id)),
    )


def build_apps_state(session: Session) -> BuildAppsState:
    apps = list(
        session.scalars(
            select(App)
            .join(App.build_setting)
            .options(
                selectinload(App.build_setting),
                selectinload(App.builds).selectinload(Build.artifacts),
                selectinload(App.builds).selectinload(Build.events),
            )
            .distinct()
        ).unique()
    )
    apps.sort(key=lambda app: (app.name.casefold(), app.id))
    unconfigured_apps = list(
        session.scalars(select(App).where(~App.build_setting.has()))
    )
    unconfigured_apps.sort(key=lambda app: (app.name.casefold(), app.id))
    runners = list(session.scalars(select(BuildRunner)))
    items: list[BuildAppItem] = []
    for app in apps:
        setting = app.build_setting
        if setting is None:
            continue
        matches = [
            runner
            for runner in runners
            if _runner_matches_setting(runner, setting=setting)
        ]
        setting_item = build_setting_item(setting)
        if setting_item is None:
            continue
        latest_build = max(app.builds, key=lambda build: build.uploaded_at, default=None)
        items.append(
            BuildAppItem(
                app=_build_app_summary(app),
                setting=setting_item,
                matching_runner_count=len(matches),
                has_online_runner=bool(matches),
                latest_build=build_item(latest_build) if latest_build is not None else None,
            )
        )
    return BuildAppsState(
        apps=items,
        available_apps=[_build_app_summary(app) for app in unconfigured_apps],
        total=len(items),
    )


def _runner_matches_setting(
    runner: BuildRunner,
    *,
    setting: AppBuildSetting,
) -> bool:
    if runner.status not in {"online", "busy"}:
        return False
    required_labels = {
        str(item).strip() for item in setting.runner_labels_json or [] if str(item).strip()
    }
    runner_labels = {
        str(item).strip() for item in runner.labels_json or [] if str(item).strip()
    }
    return required_labels.issubset(runner_labels)


def normalize_runner_capabilities(capabilities: dict[str, object]) -> dict[str, object]:
    normalized = dict(capabilities)
    normalized["platforms"] = list(RUNNER_BUILD_PLATFORMS)
    return normalized


def _parse_environment(value: str) -> str:
    try:
        return normalize_environment(value).value
    except ValueError as error:
        raise ApiError(
            "invalid_environment",
            "environment 必须是 development 或 production",
            status_code=422,
        ) from error


def save_build_setting(
    session: Session,
    *,
    app_id: str,
    git_url: str,
    runner_labels: list[str],
    credential_refs: dict[str, str],
    artifact_type: str,
    optional_defaults: dict[str, object],
) -> AppBuildSetting:
    app_or_404(session, app_id)
    normalized_git_url = _require_non_blank(git_url, field="git_url", label="git_url")
    normalized_artifact_type = _require_non_blank(
        artifact_type,
        field="artifact_type",
        label="artifact_type",
    )
    normalized_credential_refs = _normalize_credential_refs(credential_refs)
    existing = get_build_setting(session, app_id)
    setting = existing or AppBuildSetting(
        id=f"build-setting-{app_id}-{uuid4().hex[:8]}",
        app_id=app_id,
    )
    setting.git_url = normalized_git_url
    setting.runner_labels_json = [label.strip() for label in runner_labels if label.strip()]
    setting.credential_refs_json = normalized_credential_refs
    setting.artifact_type = normalized_artifact_type
    setting.optional_defaults_json = dict(optional_defaults)
    setting.updated_at = datetime.now(UTC)
    session.add(setting)
    session.commit()
    return setting


def create_agent_build(
    session: Session,
    *,
    app_id: str,
    environment: str,
    git_ref: str,
) -> Build:
    app = app_or_404(session, app_id)
    normalized_environment = _parse_environment(environment)
    normalized_git_ref = _require_non_blank(git_ref, field="git_ref", label="git_ref")
    setting = get_build_setting(session, app.id)
    if setting is None:
        raise ApiError(
            "build_setting_not_configured",
            "请先配置应用的源码构建设置",
            status_code=409,
        )
    build = Build(
        id=f"build-agent-{uuid4().hex[:12]}",
        app_id=app.id,
        version=None,
        build_number=None,
        channel=channel_for_environment(normalized_environment),
        environment=normalized_environment,
        requested_environment=normalized_environment,
        platform=app.platform,
        source="agent",
        lifecycle_status="queued",
        git_url=setting.git_url,
        git_ref=normalized_git_ref,
        runner_labels_json={
            "required": list(setting.runner_labels_json or []),
            "credentialRefs": dict(setting.credential_refs_json or {}),
            "artifactType": setting.artifact_type,
            "optionalDefaults": dict(setting.optional_defaults_json or {}),
            "environment": normalized_environment,
            "gitRef": normalized_git_ref,
        },
        attempt_count=0,
        note="",
        status="pending",
        uploaded_at=datetime.now(UTC),
    )
    session.add(build)
    session.commit()
    return session.scalars(
        select(Build)
        .where(Build.id == build.id)
        .options(joinedload(Build.app), selectinload(Build.artifacts))
    ).one()


def provision_runner(
    session: Session,
    *,
    runner_id: str,
    name: str,
    labels: list[str],
    version: str,
    package_agent_version: str,
    capabilities: dict[str, object],
    token_pepper: str,
) -> tuple[BuildRunner, str]:
    normalized_runner_id = _normalize_runner_id(runner_id)
    token = secrets.token_urlsafe(32)
    runner = session.get(BuildRunner, normalized_runner_id) or BuildRunner(id=normalized_runner_id)
    runner.name = _require_non_blank(name, field="name", label="name")
    runner.token_hash = hash_runner_token(token, token_pepper=token_pepper)
    runner.labels_json = [label.strip() for label in labels if label.strip()]
    runner.capabilities_json = normalize_runner_capabilities(capabilities)
    runner.status = "busy" if runner.current_build_id else "offline"
    runner.version = version.strip()
    runner.package_agent_version = package_agent_version.strip()
    session.add(runner)
    session.commit()
    return runner, token


def register_runner(
    session: Session,
    *,
    runner_id: str,
    name: str,
    token: str,
    labels: list[str],
    version: str,
    package_agent_version: str,
    capabilities: dict[str, object],
    token_pepper: str,
) -> BuildRunner:
    runner = session.get(BuildRunner, runner_id)
    normalized_token = _require_non_blank(token, field="token", label="token")
    if runner is None:
        raise ApiError("unknown_runner", "Runner 未预配", status_code=401)
    if not verify_runner_token(normalized_token, runner.token_hash, token_pepper=token_pepper):
        raise ApiError("invalid_runner_token", "Runner token 不正确", status_code=401)
    runner.name = _require_non_blank(name, field="name", label="name")
    runner.labels_json = [label.strip() for label in labels if label.strip()]
    merged_capabilities = dict(runner.capabilities_json or {})
    merged_capabilities.update(capabilities)
    runner.capabilities_json = normalize_runner_capabilities(merged_capabilities)
    runner.status = "busy" if runner.current_build_id else "online"
    runner.version = version.strip()
    runner.package_agent_version = package_agent_version.strip()
    now = datetime.now(UTC)
    runner.last_seen_at = now
    _refresh_runner_assignment_lease(session, runner=runner, now=now)
    session.add(runner)
    session.commit()
    return runner


def heartbeat_runner(
    session: Session,
    *,
    runner_id: str,
    name: str,
    token: str,
    labels: list[str],
    version: str,
    package_agent_version: str,
    capabilities: dict[str, object],
    token_pepper: str,
) -> BuildRunner:
    return register_runner(
        session,
        runner_id=runner_id,
        name=name,
        token=token,
        labels=labels,
        version=version,
        package_agent_version=package_agent_version,
        capabilities=capabilities,
        token_pepper=token_pepper,
    )


def poll_runner_build(
    session: Session,
    *,
    runner_id: str,
    token: str,
    token_pepper: str,
    dingtalk_enabled: bool = False,
    public_base_url: str = "",
) -> Build | None:
    runner = _runner_or_401(session, runner_id=runner_id, token=token, token_pepper=token_pepper)
    now = datetime.now(UTC)
    runner.last_seen_at = now
    _recover_expired_assignments(
        session,
        now=now,
        dingtalk_enabled=dingtalk_enabled,
        public_base_url=public_base_url,
    )
    _terminalize_retry_cap_builds(
        session,
        dingtalk_enabled=dingtalk_enabled,
        public_base_url=public_base_url,
    )
    session.flush()
    if runner.current_build_id:
        build = session.get(Build, runner.current_build_id)
        if build is not None and _is_active_assignment(build, runner=runner, now=now):
            _refresh_build_assignment_lease(build, now=now)
            runner.status = "busy"
            session.add_all([build, runner])
            session.commit()
            return _build_with_relations(session, build.id)
        runner.current_build_id = None

    queued_builds = session.scalars(
        select(Build)
        .where(
            Build.source == "agent",
            Build.lifecycle_status == BuildLifecycleStatus.QUEUED.value,
            Build.attempt_count < 5,
        )
        .order_by(Build.uploaded_at.asc())
    )
    runner_labels = set(runner.labels_json or [])
    for build in queued_builds:
        required_labels = set((build.runner_labels_json or {}).get("required", []))
        if required_labels and not required_labels.issubset(runner_labels):
            continue
        build.lifecycle_status = BuildLifecycleStatus.ASSIGNED.value
        build.runner_id = runner.id
        build.attempt_count += 1
        build.started_at = build.started_at or now
        _refresh_build_assignment_lease(build, now=now)
        runner.current_build_id = build.id
        runner.status = "busy"
        session.add_all([build, runner])
        session.commit()
        return _build_with_relations(session, build.id)

    runner.status = "online"
    session.add(runner)
    session.commit()
    return None


def append_build_event(
    session: Session,
    *,
    build_id: str,
    runner_id: str,
    token: str,
    token_pepper: str,
    event_type: str,
    message: str,
    lifecycle_status: str | None = None,
    payload: dict[str, object] | None = None,
) -> BuildEvent:
    build, runner = _runner_build_pair(
        session,
        build_id=build_id,
        runner_id=runner_id,
        token=token,
        token_pepper=token_pepper,
    )
    normalized_type = _require_non_blank(event_type, field="type", label="type")
    normalized_message = redact_text(_require_non_blank(message, field="message", label="message"))
    normalized_lifecycle_status = _normalize_runner_event_lifecycle_status(lifecycle_status)
    now = datetime.now(UTC)
    event = BuildEvent(
        id=f"build-event-{uuid4().hex[:12]}",
        build_id=build.id,
        runner_id=runner.id,
        type=normalized_type,
        message=normalized_message,
        payload_json=redact_json(dict(payload or {})),
    )
    if normalized_lifecycle_status is not None:
        build.lifecycle_status = normalized_lifecycle_status
    build.started_at = build.started_at or now
    _refresh_build_assignment_lease(build, now=now)
    runner.last_seen_at = now
    session.add_all([event, build, runner])
    session.commit()
    return event


def upload_build_artifact(
    session: Session,
    *,
    storage: ArtifactStorage,
    build_id: str,
    runner_id: str,
    token: str,
    token_pepper: str,
    artifact_type: str,
    file_name: str,
    content: bytes,
    content_type: str,
) -> Artifact:
    build, runner = _runner_build_pair(
        session,
        build_id=build_id,
        runner_id=runner_id,
        token=token,
        token_pepper=token_pepper,
    )
    normalized_type = _require_non_blank(
        artifact_type,
        field="artifact_type",
        label="artifact_type",
    ).lower()
    if normalized_type not in _RUNNER_ALLOWED_ARTIFACTS:
        raise ApiError(
            "invalid_artifact_type",
            "Runner artifact_type 不受支持",
            status_code=422,
            extra={"field": "artifact_type"},
        )
    stored = storage.save(
        build.id,
        file_name=file_name,
        content=content,
        content_type=content_type or "application/octet-stream",
    )
    artifact = Artifact(
        id=f"artifact-{build.id}-{normalized_type}-{uuid4().hex[:8]}",
        build_id=build.id,
        artifact_type=normalized_type,
        file_name=file_name,
        content_type=content_type or "application/octet-stream",
        storage_backend=storage.backend,
        storage_key=stored.storage_key,
        download_url=stored.download_url,
        manifest_url=None,
        install_url=(stored.download_url if normalized_type == ArtifactType.PACKAGE.value else ""),
        size_bytes=len(content),
        metadata_json={"source": "runner", "runnerId": runner.id},
    )
    now = datetime.now(UTC)
    build.lifecycle_status = BuildLifecycleStatus.UPLOADING_ARTIFACTS.value
    _refresh_build_assignment_lease(build, now=now)
    runner.last_seen_at = now
    event = BuildEvent(
        id=f"build-event-{uuid4().hex[:12]}",
        build_id=build.id,
        runner_id=runner.id,
        type="artifact_uploaded",
        message=f"{normalized_type} uploaded",
        payload_json={"artifactType": normalized_type, "fileName": file_name},
    )
    session.add_all([artifact, build, runner, event])
    session.commit()
    return artifact


def complete_runner_build(
    session: Session,
    *,
    build_id: str,
    runner_id: str,
    token: str,
    token_pepper: str,
    status: str,
    version: str | None = None,
    build_number: str | None = None,
    commit_sha: str | None = None,
    note: str | None = None,
    failure_classification: str | None = None,
    failure_summary: str | None = None,
    human_action: str | None = None,
    dingtalk_enabled: bool = False,
    public_base_url: str = "",
) -> Build:
    build, runner = _runner_build_pair(
        session,
        build_id=build_id,
        runner_id=runner_id,
        token=token,
        token_pepper=token_pepper,
    )
    normalized_status = _normalize_complete_status(status)
    artifacts = session.scalars(select(Artifact).where(Artifact.build_id == build.id)).all()
    if normalized_status == BuildLifecycleStatus.SUCCEEDED.value:
        existing_types = {artifact.artifact_type for artifact in artifacts}
        missing = sorted(_RUNNER_REQUIRED_ARTIFACTS - existing_types)
        if missing:
            raise ApiError(
                "missing_required_artifacts",
                f"自动构建缺少必需制品: {', '.join(missing)}",
                status_code=422,
            )
        build.status = "available"
    elif normalized_status == BuildLifecycleStatus.NEEDS_HUMAN.value:
        build.status = "pending"
    else:
        build.status = "failed"
    if version is not None and version.strip():
        build.version = version.strip()
    if build_number is not None and build_number.strip():
        build.build_number = build_number.strip()
    if commit_sha is not None and commit_sha.strip():
        build.commit_sha = commit_sha.strip()
    if note is not None and note.strip():
        build.note = redact_text(note.strip())
    build.lifecycle_status = normalized_status
    build.failure_classification = sanitize_failure_classification(failure_classification)
    build.failure_summary = redact_text(failure_summary.strip()) if failure_summary else None
    build.human_action = redact_text(human_action.strip()) if human_action else None
    build.started_at = _ensure_utc_datetime(build.started_at) or datetime.now(UTC)
    build.finished_at = datetime.now(UTC)
    build.duration_seconds = int((build.finished_at - build.started_at).total_seconds())
    build.assignment_lease_expires_at = None
    runner.current_build_id = None
    runner.status = "online"
    runner.last_seen_at = datetime.now(UTC)
    event = BuildEvent(
        id=f"build-event-{uuid4().hex[:12]}",
        build_id=build.id,
        runner_id=runner.id,
        type="complete",
        message=f"build {normalized_status}",
        payload_json={
            "status": normalized_status,
            "version": build.version or "",
            "buildNumber": build.build_number or "",
            "commitSha": build.commit_sha or "",
        },
    )
    enqueue_terminal_build_notifications(
        session,
        build,
        dingtalk_enabled=dingtalk_enabled,
        public_base_url=public_base_url,
    )
    session.add_all([build, runner, event])
    session.commit()
    return _build_with_relations(session, build.id)


def build_item(build: Build) -> BuildItem:
    artifact = build.package_artifact()
    artifacts = sorted(build.artifacts, key=lambda item: (item.artifact_type, item.file_name))
    app = build.app
    if app is None:
        raise AdminApiError("build_app_not_found", "构建关联的应用不存在", status_code=500)
    return BuildItem(
        id=build.id,
        app=_build_app_summary(app),
        version=build.version or "",
        build_number=build.build_number or "",
        source=build.source,
        lifecycle_status=build.lifecycle_status,
        git_ref=build.git_ref or "",
        platform=build.platform,
        platform_label=platform_label(build.platform),
        environment=build.environment,
        environment_label=environment_label(build.environment),
        status=build.status,
        note=build.note or "",
        min_os_version=build.min_os_version or "",
        uploaded_at=_iso_datetime(build.uploaded_at),
        uploaded_at_label=format_datetime(build.uploaded_at),
        expires_at=_optional_iso_datetime(build.expires_at),
        expires_at_label=format_datetime(build.expires_at),
        artifact=(
            _artifact_item(artifact)
            if artifact
            else None
        ),
        artifacts=[_artifact_item(item) for item in artifacts],
        failure_classification=build.failure_classification or "",
        failure_summary=build.failure_summary or "",
        human_action=build.human_action or "",
        recent_events=[_event_item(event) for event in _recent_events_for_build(build)],
    )


def build_setting_item(setting: AppBuildSetting | None) -> BuildSettingItem | None:
    if setting is None:
        return None
    return BuildSettingItem(
        git_url=setting.git_url,
        runner_labels=list(setting.runner_labels_json or []),
        credential_refs=dict(setting.credential_refs_json or {}),
        artifact_type=setting.artifact_type,
        optional_defaults=dict(setting.optional_defaults_json or {}),
        updated_at_label=format_datetime(setting.updated_at),
    )


def _build_app_summary(app: App) -> BuildAppSummary:
    return BuildAppSummary(
        id=app.id,
        name=app.name,
        bundle_identifier=app.bundle_identifier,
        platform=app.platform,
        icon_color=app.icon_color,
        icon_text=app.name[:2].upper(),
    )


def _iso_datetime(value: datetime) -> str:
    if value.tzinfo is None:
        value = value.replace(tzinfo=UTC)
    return value.astimezone(UTC).isoformat().replace("+00:00", "Z")


def _optional_iso_datetime(value: datetime | None) -> str | None:
    if value is None:
        return None
    return _iso_datetime(value)


def _require_non_blank(value: str, *, field: str, label: str) -> str:
    normalized = value.strip()
    if normalized:
        return normalized
    raise ApiError(
        "invalid_build_input",
        f"{label} 不能为空",
        status_code=422,
        extra={"field": field},
    )


def _normalize_runner_id(value: str) -> str:
    normalized = _require_non_blank(value, field="runner_id", label="runner_id")
    if not _RUNNER_ID_RE.fullmatch(normalized):
        raise ApiError(
            "invalid_runner_id",
            "runner_id 仅支持字母、数字、点、下划线和短横线，且长度不能超过 64",
            status_code=422,
        )
    return normalized


def hash_runner_token(token: str, *, token_pepper: str) -> str:
    digest = hmac.new(
        token_pepper.encode("utf-8"),
        token.encode("utf-8"),
        sha256,
    ).hexdigest()
    return f"{_RUNNER_TOKEN_DIGEST_PREFIX}{digest}"


def verify_runner_token(token: str, stored_hash: str, *, token_pepper: str) -> bool:
    expected = hash_runner_token(token, token_pepper=token_pepper)
    return hmac.compare_digest(stored_hash, expected)


def authenticate_runner(
    session: Session,
    *,
    runner_id: str,
    token: str,
    token_pepper: str,
) -> BuildRunner:
    return _runner_or_401(
        session,
        runner_id=runner_id,
        token=token,
        token_pepper=token_pepper,
    )


def _normalize_credential_refs(credential_refs: dict[str, str]) -> dict[str, str]:
    normalized: dict[str, str] = {}
    for key, raw_value in credential_refs.items():
        normalized[key] = _validate_credential_ref(key=key, value=raw_value)
    return normalized


def _validate_credential_ref(*, key: str, value: str) -> str:
    normalized = value.strip()
    if not normalized:
        raise _invalid_credential_ref(key, "credential ref 不能为空")
    if "\n" in value or "\r" in value:
        raise _invalid_credential_ref(key, "credential ref 不能包含换行")
    if len(normalized) > _CREDENTIAL_REF_MAX_LENGTH:
        raise _invalid_credential_ref(key, "credential ref 过长")
    lowered = normalized.lower()
    if _PEM_MARKER_RE.search(value):
        raise _invalid_credential_ref(key, "credential ref 不能是私钥内容")
    if normalized.startswith(_TOKEN_PREFIXES) or _JWT_RE.fullmatch(normalized):
        raise _invalid_credential_ref(key, "credential ref 不能是 token 或密钥")
    if _HEX_SECRET_RE.fullmatch(normalized) and any(char.isalpha() for char in normalized):
        raise _invalid_credential_ref(key, "credential ref 不能是 token 或密钥")
    if _SECRET_WORD_RE.search(lowered) and len(normalized) >= 16:
        raise _invalid_credential_ref(key, "credential ref 不能是 secret 文本")
    if len(normalized) > _CREDENTIAL_REF_ID_MAX_LENGTH:
        raise _invalid_credential_ref(key, "credential ref 过长")
    if not _CREDENTIAL_REF_ID_RE.fullmatch(normalized):
        raise _invalid_credential_ref(
            key,
            "credential ref 必须是受支持前缀的小写 kebab-case 标识",
        )
    return normalized


def _invalid_credential_ref(key: str, message: str) -> ApiError:
    return ApiError(
        "invalid_credential_ref",
        f"{key}: {message}",
        status_code=422,
        extra={"field": "credential_refs", "key": key},
    )


def _build_with_relations(session: Session, build_id: str) -> Build:
    return session.scalars(
        select(Build)
        .where(Build.id == build_id)
        .options(joinedload(Build.app), selectinload(Build.artifacts))
    ).one()


def _recover_expired_assignments(
    session: Session,
    *,
    now: datetime,
    dingtalk_enabled: bool,
    public_base_url: str,
) -> None:
    expired_builds = session.scalars(
        select(Build).where(
            Build.source == "agent",
            Build.lifecycle_status.not_in(_TERMINAL_BUILD_LIFECYCLE_STATUSES),
            Build.lifecycle_status != BuildLifecycleStatus.QUEUED.value,
            (
                (Build.assignment_lease_expires_at.is_(None))
                | (Build.assignment_lease_expires_at <= now)
            ),
        )
    ).all()
    for build in expired_builds:
        owning_runner = (
            session.get(BuildRunner, build.runner_id)
            if build.runner_id
            else None
        )
        if owning_runner is not None and owning_runner.current_build_id == build.id:
            owning_runner.current_build_id = None
            owning_runner.status = "online"
            session.add(owning_runner)
        build.runner_id = None
        build.assignment_lease_expires_at = None
        if build.attempt_count >= 5:
            _mark_retry_cap_needs_human(
                session,
                build,
                dingtalk_enabled=dingtalk_enabled,
                public_base_url=public_base_url,
            )
        else:
            build.lifecycle_status = BuildLifecycleStatus.QUEUED.value
            build.status = "pending"
        session.add(build)


def _terminalize_retry_cap_builds(
    session: Session,
    *,
    dingtalk_enabled: bool,
    public_base_url: str,
) -> None:
    capped_builds = session.scalars(
        select(Build).where(
            Build.source == "agent",
            Build.lifecycle_status == BuildLifecycleStatus.QUEUED.value,
            Build.attempt_count >= 5,
        )
    ).all()
    for build in capped_builds:
        _mark_retry_cap_needs_human(
            session,
            build,
            dingtalk_enabled=dingtalk_enabled,
            public_base_url=public_base_url,
        )
        session.add(build)


def _mark_retry_cap_needs_human(
    session: Session,
    build: Build,
    *,
    dingtalk_enabled: bool,
    public_base_url: str,
) -> None:
    build.lifecycle_status = BuildLifecycleStatus.NEEDS_HUMAN.value
    build.status = "pending"
    build.failure_classification = "retry_cap_reached"
    build.failure_summary = "Automatic build assignment reached the retry cap."
    build.human_action = "Inspect runner availability and build logs before manually retrying."
    build.finished_at = build.finished_at or datetime.now(UTC)
    enqueue_terminal_build_notifications(
        session,
        build,
        dingtalk_enabled=dingtalk_enabled,
        public_base_url=public_base_url,
    )


def sanitize_failure_classification(value: str | None) -> str | None:
    if value is None:
        return None
    normalized = value.strip().lower()
    if not normalized:
        return None
    if _FAILURE_CLASSIFICATION_SECRET_RE.search(normalized) or any(
        char.isspace() or char == "=" for char in normalized
    ):
        return _FAILURE_CLASSIFICATION_FALLBACK
    normalized = _FAILURE_CLASSIFICATION_ALLOWED_RE.sub("_", normalized)
    normalized = normalized.strip("_:-")
    if not normalized:
        return _FAILURE_CLASSIFICATION_FALLBACK
    return normalized[:_FAILURE_CLASSIFICATION_MAX_LENGTH]


def _is_active_assignment(build: Build, *, runner: BuildRunner, now: datetime) -> bool:
    if build.runner_id != runner.id:
        return False
    if build.lifecycle_status in _TERMINAL_BUILD_LIFECYCLE_STATUSES:
        return False
    lease_expires_at = _ensure_utc_datetime(build.assignment_lease_expires_at)
    return lease_expires_at is not None and lease_expires_at > now


def _refresh_runner_assignment_lease(
    session: Session,
    *,
    runner: BuildRunner,
    now: datetime,
) -> None:
    if not runner.current_build_id:
        return
    build = session.get(Build, runner.current_build_id)
    if build is None or build.runner_id != runner.id:
        return
    if build.lifecycle_status in _TERMINAL_BUILD_LIFECYCLE_STATUSES:
        return
    _refresh_build_assignment_lease(build, now=now)
    session.add(build)


def _refresh_build_assignment_lease(build: Build, *, now: datetime) -> None:
    build.assignment_lease_expires_at = now + timedelta(seconds=_RUNNER_ASSIGNMENT_LEASE_SECONDS)


def _artifact_item(artifact: Artifact) -> BuildArtifactItem:
    return BuildArtifactItem(
        artifact_type=artifact.artifact_type,
        artifact_type_label=_artifact_type_label(artifact.artifact_type),
        file_name=artifact.file_name,
        size_label=format_size(artifact.size_bytes),
        install_url=artifact.install_url,
        download_url=artifact.download_url,
        manifest_url=artifact.manifest_url,
    )


def _artifact_type_label(artifact_type: str) -> str:
    labels = {
        "package": "Package",
        "symbols": "Symbols",
        "report": "Report",
        "log": "Log",
    }
    return labels.get(artifact_type, artifact_type)


def _recent_events_for_build(build: Build) -> list[BuildEvent]:
    events = sorted(build.events, key=lambda event: event.created_at, reverse=True)
    return events[:5]


def _event_item(event: BuildEvent) -> BuildEventItem:
    return BuildEventItem(
        type=event.type,
        message=event.message,
        created_at_label=format_datetime(event.created_at),
    )


def _normalize_runner_event_lifecycle_status(value: str | None) -> str | None:
    if value is None:
        return None
    normalized = _require_non_blank(
        value,
        field="lifecycle_status",
        label="lifecycle_status",
    ).lower()
    if normalized not in _RUNNER_EVENT_LIFECYCLE_STATUSES:
        raise ApiError(
            "invalid_runner_event_lifecycle_status",
            "Runner event lifecycle_status 只支持非终态构建状态",
            status_code=422,
            extra={"field": "lifecycle_status"},
        )
    return normalized


def _normalize_complete_status(value: str) -> str:
    normalized = _require_non_blank(value, field="status", label="status").lower()
    if normalized not in _TERMINAL_BUILD_LIFECYCLE_STATUSES:
        raise ApiError(
            "invalid_runner_complete_status",
            "status 必须是受支持的终态构建状态",
            status_code=422,
            extra={"field": "status"},
        )
    return normalized


def _runner_or_401(
    session: Session,
    *,
    runner_id: str,
    token: str,
    token_pepper: str,
) -> BuildRunner:
    runner = session.get(BuildRunner, runner_id)
    if runner is None:
        raise ApiError("unknown_runner", "Runner 未预配", status_code=401)
    if not verify_runner_token(token, runner.token_hash, token_pepper=token_pepper):
        raise ApiError("invalid_runner_token", "Runner token 不正确", status_code=401)
    return runner


def _runner_build_pair(
    session: Session,
    *,
    build_id: str,
    runner_id: str,
    token: str,
    token_pepper: str,
) -> tuple[Build, BuildRunner]:
    runner = _runner_or_401(
        session,
        runner_id=runner_id,
        token=token,
        token_pepper=token_pepper,
    )
    build = session.get(Build, build_id)
    if build is None:
        raise ApiError("build_not_found", "构建不存在", status_code=404)
    if build.runner_id != runner.id or runner.current_build_id != build.id:
        raise ApiError("runner_build_mismatch", "Runner 未持有该构建", status_code=409)
    return build, runner


def _ensure_utc_datetime(value: datetime | None) -> datetime | None:
    if value is None:
        return None
    if value.tzinfo is None:
        return value.replace(tzinfo=UTC)
    return value.astimezone(UTC)

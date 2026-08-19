from __future__ import annotations

import os
import re
from datetime import UTC, datetime, timedelta
from importlib import resources
from pathlib import Path
from typing import Annotated, Any
from uuid import uuid4

from fastapi import APIRouter, Depends, File, Form, Query, Request, UploadFile
from fastapi.responses import FileResponse
from sqlalchemy import func, select
from sqlalchemy.orm import Session

from testflying_api import build_platform, runner_releases
from testflying_api.admin.security import require_admin
from testflying_api.admin.services import (
    bind_app_to_account,
    parse_admin_datetime,
    save_developer_account,
    unbind_app_from_account,
    update_bound_app_store_settings,
)
from testflying_api.admin.view_models import (
    account_detail_context,
    account_status_label,
    dashboard_context,
    format_datetime,
    list_accounts,
    list_apps,
    list_builds,
    list_devices,
    list_notifications,
    marketing_page_context,
    platform_label,
    store_marketing_context,
    store_metadata_context,
    upload_context,
)
from testflying_api.admin_api.errors import AdminApiError
from testflying_api.admin_api.schemas import (
    AccountAppBindRequest,
    AccountAppItem,
    AccountAppSettingsRequest,
    AccountDetailActionResponse,
    AdminBootstrapResponse,
    AdminHealthState,
    AdminNavItem,
    AdminUploadResponse,
    AgentBuildCreateRequest,
    ApiDocEndpointItem,
    ApiDocParamItem,
    ApiDocsState,
    AppBuildActionResponse,
    AppDetailState,
    AppLogsState,
    BuildAppsState,
    BuildItem,
    BuildRunnerItem,
    BuildRunnersState,
    BuildSettingSaveRequest,
    BuildsState,
    ConnectorActionResponse,
    ConnectorSaveRequest,
    ConnectorState,
    DashboardState,
    DashboardStatItem,
    DeveloperAccountDetailState,
    DeveloperAccountForm,
    DeveloperAccountSaveResponse,
    DeveloperAccountsState,
    DeveloperAccountsStats,
    DeveloperAccountSummary,
    DeviceItem,
    DevicesState,
    DingTalkConfigState,
    GeneralSettingsSaveRequest,
    GeneralSettingsState,
    LlmConfigState,
    LlmFeatureBindingItem,
    LlmFeatureBindingSaveRequest,
    LlmFeatureBindingSaveResponse,
    LlmPresetItem,
    LlmProfileItem,
    LlmProfileSaveRequest,
    LlmProfileSaveResponse,
    LlmProtocolItem,
    MarketingPageActionResponse,
    MarketingPageCreateRequest,
    MarketingPageDetailState,
    MarketingPageLocaleContent,
    MarketingPageLocaleInput,
    MarketingPageSaveRequest,
    MarketingPageSyncRequest,
    NotificationItem,
    NotificationSettingsSaveRequest,
    NotificationSettingsState,
    NotificationsState,
    NotificationTypeCount,
    ReviewAnalysisIssue,
    ReviewAnalysisRunItem,
    ReviewAppItem,
    ReviewFetchRunItem,
    ReviewItem,
    ReviewScopeRequest,
    ReviewStats,
    RunnerBuildPayload,
    RunnerCompleteRequest,
    RunnerEventRequest,
    RunnerHeartbeatRequest,
    RunnerPollRequest,
    RunnerPollResponse,
    RunnerProvisionRequest,
    RunnerProvisionResponse,
    RunnerUpdateCheckRequest,
    RunnerUpdateCheckResponse,
    RuntimeEnvironmentItem,
    SettingsActionResponse,
    SettingsState,
    StoreAppBuildItem,
    StoreAppItem,
    StoreAppsAccountSummary,
    StoreAppsState,
    StoreAppsStats,
    StoreImageDeleteRequest,
    StoreLocaleContent,
    StoreLocaleContentInput,
    StoreMarketingPageSummary,
    StoreReviewAnalysisResponse,
    StoreReviewFetchResponse,
    StoreReviewsState,
    StoreTranslationRequest,
    StoreTranslationResponse,
    StoreWorkspaceActionResponse,
    StoreWorkspaceSaveRequest,
    StoreWorkspaceState,
    StoreWorkspaceSyncRequest,
    SyncRunSummary,
    UnassignedAppItem,
    UploadAccountOption,
    UploadResult,
    UploadState,
)
from testflying_api.app_logs import LEVELS, build_app_log_connect_context
from testflying_api.config import Settings
from testflying_api.database import get_db_session
from testflying_api.dingtalk import send_dingtalk_markdown
from testflying_api.errors import ApiError
from testflying_api.llm_config import (
    LLM_FEATURES,
    LLM_PRESETS,
    LLM_PROTOCOLS,
    auth_header_label,
    list_llm_bindings,
    list_llm_profiles,
    mask_api_key,
    protocol_label,
    save_feature_binding,
    save_llm_profile,
)
from testflying_api.schema import (
    App,
    Build,
    BuildRunner,
    DeveloperAccount,
    Device,
    LlmFeatureBinding,
    LlmProfile,
    Notification,
    StoreAppMetadataDraft,
    StoreImageSuiteLocale,
    StoreMarketingPageLocale,
    StoreReview,
    StoreReviewAnalysisRun,
    StoreReviewFetchRun,
    WebhookDelivery,
)
from testflying_api.store_image_requirements import validate_store_image
from testflying_api.store_reviews import (
    analyze_store_reviews,
    fetch_store_reviews_incremental,
    store_reviews_context,
)
from testflying_api.store_sync import (
    CURRENT_METADATA_VERSION,
    DEFAULT_CONTENT_SET_ID,
    DEFAULT_CONTENT_SET_NAME,
    DEFAULT_LOCALE,
    UPDATE_APP_METADATA,
    UPDATE_MARKETING_PAGE,
    account_connector,
    check_connector_health,
    create_marketing_page,
    delete_marketing_page,
    duplicate_marketing_page,
    get_or_refresh_preflight,
    marketing_page_for_scope,
    save_connector,
    save_current_app_metadata_draft,
    save_marketing_page,
    save_release_note_draft,
    scoped_app,
    sync_current_app_metadata,
    sync_marketing_page,
    sync_release_notes,
)
from testflying_api.system_settings import (
    database_setting_keys,
    effective_business_settings,
    save_general_settings,
    save_notification_settings,
)
from testflying_api.translation import translate_store_metadata_text
from testflying_api.upload_service import create_package_upload

router = APIRouter(prefix="/admin/api", tags=["admin-api"])
AdminDep = Annotated[None, Depends(require_admin)]
SessionDep = Annotated[Session, Depends(get_db_session)]
STORE_IMAGE_SLOT_KEYS = {"feature_graphic_url", "phone_screenshots", "tablet_screenshots"}
PUBLIC_API_DOC_PATH = "docs/store-management-public-api.md"
CONNECTOR_AUTO_CHECK_TTL = timedelta(minutes=5)

RUNTIME_ENVIRONMENT_DESCRIPTORS = (
    ("TESTFLYING_DATABASE_URL", "数据库连接", "服务", "database_url", True),
    ("TESTFLYING_PUBLIC_BASE_URL", "公网服务地址", "服务", "public_base_url", False),
    ("TESTFLYING_STORAGE_BACKEND", "存储后端", "存储", "storage_backend", False),
    ("TESTFLYING_STORAGE_ROOT", "本地存储目录", "存储", "storage_root", False),
    ("TESTFLYING_S3_ENDPOINT_URL", "S3 服务地址", "存储", "s3_endpoint_url", False),
    ("TESTFLYING_S3_PUBLIC_BASE_URL", "S3 公网地址", "存储", "s3_public_base_url", False),
    ("TESTFLYING_S3_BUCKET", "S3 Bucket", "存储", "s3_bucket", False),
    ("TESTFLYING_S3_ACCESS_KEY_ID", "S3 Access Key", "安全", "s3_access_key_id", True),
    (
        "TESTFLYING_S3_SECRET_ACCESS_KEY",
        "S3 Secret Key",
        "安全",
        "s3_secret_access_key",
        True,
    ),
    ("TESTFLYING_STATIC_TOKEN", "静态访问 Token", "安全", "static_token", True),
    ("TESTFLYING_CORS_ALLOWED_ORIGINS", "CORS 来源", "安全", "cors_allowed_origins", False),
    ("TESTFLYING_ADMIN_USERNAME", "后台管理员", "安全", "admin_username", False),
    ("TESTFLYING_RUNNER_RELEASE_ROOT", "Runner 发布目录", "Runner", "runner_release_root", False),
)


@router.get("/bootstrap", response_model=AdminBootstrapResponse, response_model_by_alias=True)
def admin_bootstrap(_: AdminDep) -> AdminBootstrapResponse:
    return AdminBootstrapResponse(
        app_name="testflying",
        nav_items=[
            AdminNavItem(key="dashboard", label="总览", path="/admin"),
            AdminNavItem(key="uploads", label="上传", path="/admin/uploads"),
            AdminNavItem(key="apps", label="商店管理", path="/admin/apps"),
            AdminNavItem(key="store-reviews", label="商店评论", path="/admin/store-reviews"),
            AdminNavItem(key="api-docs", label="接口文档", path="/admin/api-docs"),
            AdminNavItem(key="builds", label="构建", path="/admin/builds/apps"),
            AdminNavItem(key="devices", label="设备", path="/admin/devices"),
            AdminNavItem(key="app-logs", label="App 日志", path="/admin/app-logs"),
            AdminNavItem(key="notifications", label="通知", path="/admin/notifications"),
            AdminNavItem(key="settings", label="设置", path="/admin/settings/general"),
        ],
        health=AdminHealthState(state="idle", label="未检查"),
    )


@router.get(
    "/dashboard",
    response_model=DashboardState,
    response_model_by_alias=True,
)
def dashboard_state(
    session: SessionDep,
    _: AdminDep,
) -> DashboardState:
    context = dashboard_context(session)
    return DashboardState(
        stats=[
            DashboardStatItem(label=stat.label, value=stat.value, tone=stat.tone)
            for stat in context["stats"]
        ],
        recent_builds=[_build_item(build) for build in context["recent_builds"]],
        recent_notifications=[
            _notification_item(notification) for notification in context["recent_notifications"]
        ],
    )


@router.get(
    "/builds",
    response_model=BuildsState,
    response_model_by_alias=True,
)
def builds_state(
    session: SessionDep,
    _: AdminDep,
) -> BuildsState:
    builds = list_builds(session)
    return BuildsState(builds=[_build_item(build) for build in builds], total=len(builds))


@router.get(
    "/builds/apps",
    response_model=BuildAppsState,
    response_model_by_alias=True,
)
def build_apps_state(
    session: SessionDep,
    _: AdminDep,
) -> BuildAppsState:
    return build_platform.build_apps_state(session)


@router.get(
    "/build-runners",
    response_model=BuildRunnersState,
    response_model_by_alias=True,
)
def build_runners_state(
    request: Request,
    session: SessionDep,
    _: AdminDep,
) -> BuildRunnersState:
    runners = list(session.scalars(select(BuildRunner).order_by(BuildRunner.name.asc())))
    return BuildRunnersState(
        runners=[
            _build_runner_item(
                runner,
                release_root=request.app.state.settings.runner_release_root,
            )
            for runner in runners
        ],
        total=len(runners),
    )


@router.get(
    "/apps/{app_id}",
    response_model=AppDetailState,
    response_model_by_alias=True,
)
def app_detail_state(
    session: SessionDep,
    app_id: str,
    _: AdminDep,
) -> AppDetailState:
    try:
        return build_platform.app_detail_state(session, app_id)
    except ApiError as error:
        raise _admin_api_error(error) from error


@router.put(
    "/apps/{app_id}/build-setting",
    response_model=AppBuildActionResponse,
    response_model_by_alias=True,
)
def save_app_build_setting(
    session: SessionDep,
    app_id: str,
    payload: BuildSettingSaveRequest,
    _: AdminDep,
) -> AppBuildActionResponse:
    try:
        build_platform.save_build_setting(
            session,
            app_id=app_id,
            git_url=payload.git_url,
            runner_labels=payload.runner_labels,
            credential_refs=payload.credential_refs,
            artifact_type=payload.artifact_type,
            optional_defaults=payload.optional_defaults,
        )
        return AppBuildActionResponse(
            message="构建配置已保存",
            build=None,
            state=build_platform.app_detail_state(session, app_id),
        )
    except ApiError as error:
        raise _admin_api_error(error) from error


@router.post(
    "/apps/{app_id}/builds",
    response_model=AppBuildActionResponse,
    response_model_by_alias=True,
)
def create_app_agent_build(
    session: SessionDep,
    app_id: str,
    payload: AgentBuildCreateRequest,
    _: AdminDep,
) -> AppBuildActionResponse:
    try:
        build = build_platform.create_agent_build(
            session,
            app_id=app_id,
            environment=payload.environment,
            git_ref=payload.git_ref,
        )
        return AppBuildActionResponse(
            message="构建任务已创建",
            build=_build_item(build),
            state=build_platform.app_detail_state(session, app_id),
        )
    except ApiError as error:
        raise _admin_api_error(error) from error


def _runner_token(request: Request) -> str:
    authorization = request.headers.get("Authorization", "")
    prefix = "Bearer "
    if not authorization.startswith(prefix):
        raise AdminApiError("missing_runner_token", "缺少 Runner token", status_code=401)
    token = authorization[len(prefix) :].strip()
    if not token:
        raise AdminApiError("missing_runner_token", "缺少 Runner token", status_code=401)
    return token


@router.post(
    "/build-runners/provision",
    response_model=RunnerProvisionResponse,
    response_model_by_alias=True,
)
def runner_provision(
    payload: RunnerProvisionRequest,
    request: Request,
    session: SessionDep,
    _: AdminDep,
) -> RunnerProvisionResponse:
    try:
        runner, token = build_platform.provision_runner(
            session,
            runner_id=payload.runner_id,
            name=payload.name,
            labels=payload.labels,
            version=payload.version,
            package_agent_version=payload.package_agent_version,
            capabilities=payload.capabilities,
            token_pepper=request.app.state.settings.static_token,
        )
        return RunnerProvisionResponse(
            runner=_build_runner_item(
                runner,
                release_root=request.app.state.settings.runner_release_root,
            ),
            token=token,
        )
    except ApiError as error:
        raise _admin_api_error(error) from error


@router.post("/build-runners/register", response_model=dict, response_model_by_alias=True)
def runner_register(
    payload: RunnerHeartbeatRequest,
    request: Request,
    session: SessionDep,
) -> dict[str, bool]:
    try:
        build_platform.register_runner(
            session,
            runner_id=payload.runner_id,
            name=payload.name,
            token=_runner_token(request),
            labels=payload.labels,
            version=payload.version,
            package_agent_version=payload.package_agent_version,
            capabilities=payload.capabilities,
            token_pepper=request.app.state.settings.static_token,
        )
        return {"ok": True}
    except ApiError as error:
        raise _admin_api_error(error) from error


@router.post("/build-runners/heartbeat", response_model=dict, response_model_by_alias=True)
def runner_heartbeat(
    payload: RunnerHeartbeatRequest,
    request: Request,
    session: SessionDep,
) -> dict[str, bool]:
    try:
        build_platform.heartbeat_runner(
            session,
            runner_id=payload.runner_id,
            name=payload.name,
            token=_runner_token(request),
            labels=payload.labels,
            version=payload.version,
            package_agent_version=payload.package_agent_version,
            capabilities=payload.capabilities,
            token_pepper=request.app.state.settings.static_token,
        )
        return {"ok": True}
    except ApiError as error:
        raise _admin_api_error(error) from error


@router.post("/build-runners/poll", response_model=RunnerPollResponse, response_model_by_alias=True)
def runner_poll(
    payload: RunnerPollRequest,
    request: Request,
    session: SessionDep,
) -> RunnerPollResponse:
    try:
        effective = effective_business_settings(session, request.app.state.settings)
        build = build_platform.poll_runner_build(
            session,
            runner_id=payload.runner_id,
            token=_runner_token(request),
            token_pepper=request.app.state.settings.static_token,
            dingtalk_enabled=effective.dingtalk_configured,
            public_base_url=request.app.state.settings.public_base_url,
        )
        return RunnerPollResponse(build=_runner_build_payload(build) if build else None)
    except ApiError as error:
        raise _admin_api_error(error) from error


@router.post(
    "/build-runners/{runner_id}/updates/check",
    response_model=RunnerUpdateCheckResponse,
    response_model_by_alias=True,
)
def runner_update_check(
    runner_id: str,
    payload: RunnerUpdateCheckRequest,
    request: Request,
    session: SessionDep,
) -> RunnerUpdateCheckResponse:
    try:
        build_platform.authenticate_runner(
            session,
            runner_id=runner_id,
            token=_runner_token(request),
            token_pepper=request.app.state.settings.static_token,
        )
        manifest = runner_releases.RunnerReleaseManifest.load(
            request.app.state.settings.runner_release_root,
            payload.platform,
            payload.arch,
        )
        if (
            payload.runner_version == manifest.runner_version
            and payload.package_agent_version == manifest.package_agent_version
        ):
            return RunnerUpdateCheckResponse(update_available=False)
        bundle_url = (
            f"/admin/api/build-runners/{runner_id}/updates/"
            f"{manifest.platform}/{manifest.arch}/{manifest.version}/bundle"
        )
        return RunnerUpdateCheckResponse(
            update_available=True,
            version=manifest.version,
            runner_version=manifest.runner_version,
            package_agent_version=manifest.package_agent_version,
            bundle_url=bundle_url,
            sha256=manifest.sha256,
        )
    except ApiError as error:
        raise _admin_api_error(error) from error


@router.get(
    "/build-runners/{runner_id}/updates/{platform}/{arch}/{version}/bundle",
    response_class=FileResponse,
)
def runner_update_bundle(
    runner_id: str,
    platform: str,
    arch: str,
    version: str,
    request: Request,
    session: SessionDep,
) -> FileResponse:
    try:
        build_platform.authenticate_runner(
            session,
            runner_id=runner_id,
            token=_runner_token(request),
            token_pepper=request.app.state.settings.static_token,
        )
        manifest = runner_releases.RunnerReleaseManifest.load(
            request.app.state.settings.runner_release_root,
            platform,
            arch,
        )
        if manifest.version != version:
            raise ApiError(
                "runner_release_not_found",
                "Runner release 不存在",
                status_code=404,
            )
        return FileResponse(
            manifest.bundle_path,
            media_type="application/zip",
            filename=manifest.bundle_file,
        )
    except ApiError as error:
        raise _admin_api_error(error) from error


@router.post(
    "/build-runners/builds/{build_id}/events",
    response_model=dict,
    response_model_by_alias=True,
)
def runner_build_event(
    build_id: str,
    payload: RunnerEventRequest,
    request: Request,
    session: SessionDep,
) -> dict[str, bool]:
    try:
        build_platform.append_build_event(
            session,
            build_id=build_id,
            runner_id=payload.runner_id,
            token=_runner_token(request),
            token_pepper=request.app.state.settings.static_token,
            event_type=payload.type,
            message=payload.message,
            lifecycle_status=payload.lifecycle_status,
            payload=payload.payload,
        )
        return {"ok": True}
    except ApiError as error:
        raise _admin_api_error(error) from error


@router.post(
    "/build-runners/builds/{build_id}/artifacts",
    response_model=dict,
    response_model_by_alias=True,
)
async def runner_build_artifact(
    build_id: str,
    request: Request,
    session: SessionDep,
    runner_id: Annotated[str, Form(alias="runnerId")],
    artifact_type: Annotated[str, Form(alias="artifactType")],
    file: Annotated[UploadFile, File()],
) -> dict[str, bool]:
    try:
        build_platform.upload_build_artifact(
            session,
            storage=request.app.state.artifact_storage,
            build_id=build_id,
            runner_id=runner_id,
            token=_runner_token(request),
            token_pepper=request.app.state.settings.static_token,
            artifact_type=artifact_type,
            file_name=file.filename or "",
            content=await file.read(),
            content_type=file.content_type or "application/octet-stream",
        )
        return {"ok": True}
    except ApiError as error:
        raise _admin_api_error(error) from error


@router.post(
    "/build-runners/builds/{build_id}/complete",
    response_model=dict,
    response_model_by_alias=True,
)
def runner_build_complete(
    build_id: str,
    payload: RunnerCompleteRequest,
    request: Request,
    session: SessionDep,
) -> dict[str, bool]:
    try:
        effective = effective_business_settings(session, request.app.state.settings)
        build_platform.complete_runner_build(
            session,
            build_id=build_id,
            runner_id=payload.runner_id,
            token=_runner_token(request),
            token_pepper=request.app.state.settings.static_token,
            status=payload.status,
            version=payload.version,
            build_number=payload.build_number,
            commit_sha=payload.commit_sha,
            note=payload.note,
            failure_classification=payload.failure_classification,
            failure_summary=payload.failure_summary,
            human_action=payload.human_action,
            dingtalk_enabled=effective.dingtalk_configured,
            public_base_url=request.app.state.settings.public_base_url,
        )
        return {"ok": True}
    except ApiError as error:
        raise _admin_api_error(error) from error


@router.get(
    "/devices",
    response_model=DevicesState,
    response_model_by_alias=True,
)
def devices_state(
    session: SessionDep,
    _: AdminDep,
) -> DevicesState:
    devices = list_devices(session)
    return DevicesState(devices=[_device_item(device) for device in devices], total=len(devices))


@router.get(
    "/notifications",
    response_model=NotificationsState,
    response_model_by_alias=True,
)
def notifications_state(
    request: Request,
    session: SessionDep,
    _: AdminDep,
    type: Annotated[str, Query()] = "all",
) -> NotificationsState:
    effective = effective_business_settings(session, request.app.state.settings)
    notifications = list_notifications(session)
    normalized_type = type if type in _notification_types(notifications) else "all"
    filtered = [
        notification
        for notification in notifications
        if normalized_type == "all" or notification.type == normalized_type
    ]
    return NotificationsState(
        notifications=[_notification_item(notification) for notification in filtered],
        type_counts=_notification_type_counts(notifications),
        active_type=normalized_type,
        total=len(filtered),
        dingtalk=DingTalkConfigState(
            configured=effective.dingtalk_configured,
            webhook_configured=bool(effective.dingtalk_webhook_url),
            secret_configured=bool(effective.dingtalk_secret),
            triggers=["failed", "needs_human"],
            pending_delivery_count=_webhook_delivery_count(session, status="pending"),
            dead_delivery_count=_webhook_delivery_count(session, status="dead"),
        ),
    )


@router.get(
    "/settings",
    response_model=SettingsState,
    response_model_by_alias=True,
)
def settings_state(
    request: Request,
    session: SessionDep,
    _: AdminDep,
) -> SettingsState:
    return _settings_state(session, request.app.state.settings)


@router.put(
    "/settings/general",
    response_model=SettingsActionResponse,
    response_model_by_alias=True,
)
def update_general_settings(
    payload: GeneralSettingsSaveRequest,
    request: Request,
    session: SessionDep,
    _: AdminDep,
) -> SettingsActionResponse:
    try:
        save_general_settings(
            session,
            connector_base_url_template=payload.connector_base_url_template,
            actor=request.app.state.settings.admin_username,
        )
    except ValueError as error:
        session.rollback()
        raise AdminApiError(
            "invalid_connector_template",
            "Connector 地址模板无效，只支持 {account_id} 占位符",
            status_code=422,
        ) from error
    return SettingsActionResponse(
        message="通用配置已保存",
        state=_settings_state(session, request.app.state.settings),
    )


@router.put(
    "/settings/notifications",
    response_model=SettingsActionResponse,
    response_model_by_alias=True,
)
def update_notification_settings(
    payload: NotificationSettingsSaveRequest,
    request: Request,
    session: SessionDep,
    _: AdminDep,
) -> SettingsActionResponse:
    try:
        save_notification_settings(
            session,
            enabled=payload.enabled,
            webhook_url=payload.webhook_url,
            secret=payload.secret,
            timeout_seconds=payload.timeout_seconds,
            dispatch_interval_seconds=payload.dispatch_interval_seconds,
            actor=request.app.state.settings.admin_username,
        )
    except ValueError as error:
        session.rollback()
        raise AdminApiError(
            "invalid_settings",
            "通知配置格式无效",
            status_code=422,
        ) from error
    return SettingsActionResponse(
        message="通知配置已保存",
        state=_settings_state(session, request.app.state.settings),
    )


@router.post(
    "/settings/notifications/check",
    response_model=SettingsActionResponse,
    response_model_by_alias=True,
)
def check_notification_settings(
    request: Request,
    session: SessionDep,
    _: AdminDep,
) -> SettingsActionResponse:
    effective = effective_business_settings(session, request.app.state.settings)
    if not effective.dingtalk_webhook_url or not effective.dingtalk_secret:
        raise AdminApiError(
            "notification_not_configured",
            "请先配置钉钉 Webhook 和加签密钥",
            status_code=422,
        )
    try:
        send_dingtalk_markdown(
            webhook_url=effective.dingtalk_webhook_url,
            secret=effective.dingtalk_secret,
            title="TestFlying 配置检查",
            markdown="### TestFlying 配置检查\n\n钉钉通知连接正常。",
            timeout_seconds=effective.dingtalk_timeout_seconds,
        )
    except Exception as error:  # noqa: BLE001 - provider details must stay redacted
        raise AdminApiError(
            "notification_check_failed",
            "钉钉配置检查失败，请确认凭据和网络连接",
            status_code=502,
        ) from error
    return SettingsActionResponse(
        message="钉钉配置检查消息已发送",
        state=_settings_state(session, request.app.state.settings),
    )


@router.get(
    "/api-docs",
    response_model=ApiDocsState,
    response_model_by_alias=True,
)
def api_docs_state(_: AdminDep) -> ApiDocsState:
    markdown = _public_api_markdown()
    return ApiDocsState(
        endpoints=_parse_public_api_endpoints(markdown),
        download_url="/admin/api-docs/store-management.md",
    )


@router.get(
    "/llm-config",
    response_model=LlmConfigState,
    response_model_by_alias=True,
)
def llm_config_state(
    session: SessionDep,
    _: AdminDep,
) -> LlmConfigState:
    return _llm_config_state(session)


@router.post(
    "/llm-config/profiles",
    response_model=LlmProfileSaveResponse,
    response_model_by_alias=True,
)
def create_llm_profile(
    payload: LlmProfileSaveRequest,
    session: SessionDep,
    _: AdminDep,
) -> LlmProfileSaveResponse:
    try:
        profile = save_llm_profile(
            session,
            profile_id=None,
            name=payload.name,
            protocol=payload.protocol,
            base_url=payload.base_url,
            model=payload.model,
            api_key=payload.api_key,
            auth_header=payload.auth_header,
        )
        session.commit()
    except ApiError as error:
        session.rollback()
        raise _admin_api_error(error) from error
    return LlmProfileSaveResponse(
        message="LLM 模型已保存",
        profile=_llm_profile_item(profile),
        state=_llm_config_state(session),
    )


@router.patch(
    "/llm-config/profiles/{profile_id}",
    response_model=LlmProfileSaveResponse,
    response_model_by_alias=True,
)
def update_llm_profile(
    profile_id: str,
    payload: LlmProfileSaveRequest,
    session: SessionDep,
    _: AdminDep,
) -> LlmProfileSaveResponse:
    try:
        profile = save_llm_profile(
            session,
            profile_id=profile_id,
            name=payload.name,
            protocol=payload.protocol,
            base_url=payload.base_url,
            model=payload.model,
            api_key=payload.api_key,
            auth_header=payload.auth_header,
        )
        session.commit()
    except ApiError as error:
        session.rollback()
        raise _admin_api_error(error) from error
    return LlmProfileSaveResponse(
        message="LLM 模型已更新",
        profile=_llm_profile_item(profile),
        state=_llm_config_state(session),
    )


@router.put(
    "/llm-config/bindings/{feature_key}",
    response_model=LlmFeatureBindingSaveResponse,
    response_model_by_alias=True,
)
def update_llm_feature_binding(
    feature_key: str,
    payload: LlmFeatureBindingSaveRequest,
    session: SessionDep,
    _: AdminDep,
) -> LlmFeatureBindingSaveResponse:
    try:
        binding = save_feature_binding(
            session,
            feature_key=feature_key,
            primary_profile_id=payload.primary_profile_id,
            fallback_profile_id=payload.fallback_profile_id,
        )
        session.commit()
    except ApiError as error:
        session.rollback()
        raise _admin_api_error(error) from error
    return LlmFeatureBindingSaveResponse(
        message="功能绑定已保存",
        binding=_llm_feature_binding_item(binding, list_llm_profiles(session)),
        state=_llm_config_state(session),
    )


@router.get(
    "/store-apps",
    response_model=StoreAppsState,
    response_model_by_alias=True,
)
def store_apps_state(
    session: SessionDep,
    _: AdminDep,
    filter: Annotated[str, Query()] = "all",
    appId: Annotated[str, Query()] = "",
) -> StoreAppsState:
    return _store_apps_state(session, active_filter=filter, app_id=appId)


@router.get(
    "/developer-accounts",
    response_model=DeveloperAccountsState,
    response_model_by_alias=True,
)
def developer_accounts_state(
    session: SessionDep,
    _: AdminDep,
) -> DeveloperAccountsState:
    return _developer_accounts_state(session)


@router.post(
    "/developer-accounts",
    response_model=DeveloperAccountSaveResponse,
    response_model_by_alias=True,
)
def create_developer_account(
    payload: DeveloperAccountForm,
    session: SessionDep,
    _: AdminDep,
) -> DeveloperAccountSaveResponse:
    try:
        if not payload.account_id:
            raise ApiError("invalid_account", "账号 ID 不能为空", status_code=422)
        account = save_developer_account(
            session,
            account_id=payload.account_id,
            team_name=payload.team_name,
            expires_at=parse_admin_datetime(payload.expires_at),
            status=payload.status,
            renewal_action_label=payload.renewal_action_label,
        )
        session.commit()
    except ApiError as error:
        session.rollback()
        raise _admin_api_error(error) from error
    return DeveloperAccountSaveResponse(
        message="开发者账号已保存",
        account=_developer_account_summary(
            {
                "account": account,
                "remaining_days": _remaining_days_from_account(session, account.id),
                "apps": [app.name for app in account.apps],
                "connector": account_connector(session, account.id),
                "latest_sync": None,
            }
        ),
        state=_developer_accounts_state(session),
    )


@router.get(
    "/developer-accounts/{account_id}",
    response_model=DeveloperAccountDetailState,
    response_model_by_alias=True,
)
def developer_account_detail_state(
    account_id: str,
    session: SessionDep,
    _: AdminDep,
) -> DeveloperAccountDetailState:
    _auto_check_connector(session, account_id)
    return _developer_account_detail_state(session, account_id)


@router.get(
    "/developer-accounts/{account_id}/apps/{app_id}/workspace",
    response_model=StoreWorkspaceState,
    response_model_by_alias=True,
)
@router.get(
    "/store-workspace/{account_id}/{app_id}",
    response_model=StoreWorkspaceState,
    response_model_by_alias=True,
)
def developer_account_app_workspace_state(
    account_id: str,
    app_id: str,
    session: SessionDep,
    _: AdminDep,
    section: Annotated[str, Query()] = "store",
    locale: Annotated[str, Query()] = "",
) -> StoreWorkspaceState:
    return _store_workspace_state(
        session,
        account_id=account_id,
        app_id=app_id,
        section=section,
        locale=locale,
    )


@router.post(
    "/store-translation",
    response_model=StoreTranslationResponse,
    response_model_by_alias=True,
)
def translate_store_workspace_text(
    payload: StoreTranslationRequest,
    request: Request,
    session: SessionDep,
    _: AdminDep,
) -> StoreTranslationResponse:
    try:
        translations = translate_store_metadata_text(
            request.app.state.settings,
            session=session,
            source_locale=payload.source_locale,
            target_locales=payload.target_locales,
            field=payload.field,
            text=payload.text,
        )
    except ApiError as error:
        raise _admin_api_error(error) from error
    return StoreTranslationResponse(translations=translations)


@router.put(
    "/developer-accounts/{account_id}/apps/{app_id}/workspace/metadata",
    response_model=StoreWorkspaceActionResponse,
    response_model_by_alias=True,
)
@router.put(
    "/store-workspace/{account_id}/{app_id}/metadata",
    response_model=StoreWorkspaceActionResponse,
    response_model_by_alias=True,
)
def save_store_workspace_metadata(
    account_id: str,
    app_id: str,
    payload: StoreWorkspaceSaveRequest,
    session: SessionDep,
    _: AdminDep,
) -> StoreWorkspaceActionResponse:
    try:
        app = _scoped_app_or_error(session, account_id, app_id)
        rows = _workspace_rows_from_payload(payload.locales, fallback_locale=payload.locale)
        _preserve_readonly_keywords_for_rows(session, account_id=account_id, app=app, rows=rows)
        _save_metadata_rows(session, account_id=account_id, app_id=app_id, rows=rows)
        if payload.version:
            _save_release_note_rows(
                session,
                account_id=account_id,
                app_id=app_id,
                version=payload.version,
                rows=rows,
            )
        session.commit()
    except ApiError as error:
        session.rollback()
        raise _admin_api_error(error) from error
    return _store_workspace_action_response(
        session,
        account_id=account_id,
        app_id=app_id,
        section="store",
        locale=payload.locale,
        message=f"商店草稿已保存 {len(rows)} 个语言",
    )


@router.put(
    "/developer-accounts/{account_id}/apps/{app_id}/workspace/release-notes",
    response_model=StoreWorkspaceActionResponse,
    response_model_by_alias=True,
)
@router.put(
    "/store-workspace/{account_id}/{app_id}/release-notes",
    response_model=StoreWorkspaceActionResponse,
    response_model_by_alias=True,
)
def save_store_workspace_release_notes(
    account_id: str,
    app_id: str,
    payload: StoreWorkspaceSaveRequest,
    session: SessionDep,
    _: AdminDep,
) -> StoreWorkspaceActionResponse:
    try:
        if not payload.version:
            raise ApiError("missing_version", "保存版本说明前需要目标商店版本", status_code=422)
        _scoped_app_or_error(session, account_id, app_id)
        rows = _workspace_rows_from_payload(payload.locales, fallback_locale=payload.locale)
        _save_release_note_rows(
            session,
            account_id=account_id,
            app_id=app_id,
            version=payload.version,
            rows=rows,
        )
        session.commit()
    except ApiError as error:
        session.rollback()
        raise _admin_api_error(error) from error
    return _store_workspace_action_response(
        session,
        account_id=account_id,
        app_id=app_id,
        section="release-notes",
        locale=payload.locale,
        message=f"版本说明草稿已保存 {len(rows)} 个语言",
    )


@router.post(
    "/developer-accounts/{account_id}/apps/{app_id}/workspace/metadata/preflight",
    response_model=StoreWorkspaceActionResponse,
    response_model_by_alias=True,
)
@router.post(
    "/store-workspace/{account_id}/{app_id}/metadata/preflight",
    response_model=StoreWorkspaceActionResponse,
    response_model_by_alias=True,
)
def check_store_workspace_preflight(
    account_id: str,
    app_id: str,
    payload: StoreWorkspaceSyncRequest,
    session: SessionDep,
    _: AdminDep,
) -> StoreWorkspaceActionResponse:
    try:
        if not payload.version:
            raise ApiError("missing_version", "同步前需要目标商店版本", status_code=422)
        _scoped_app_or_error(session, account_id, app_id)
        locale = _source_locale_from_payload(payload.locales, payload.locale)
        get_or_refresh_preflight(
            session,
            account_id=account_id,
            app_id=app_id,
            version=payload.version,
            locale=locale,
            operation=UPDATE_APP_METADATA,
            force_refresh=True,
        )
        session.commit()
    except ApiError as error:
        session.rollback()
        raise _admin_api_error(error) from error
    return _store_workspace_action_response(
        session,
        account_id=account_id,
        app_id=app_id,
        section="store",
        locale=locale,
        message="同步前检查已完成",
    )


@router.post(
    "/developer-accounts/{account_id}/apps/{app_id}/workspace/metadata/sync",
    response_model=StoreWorkspaceActionResponse,
    response_model_by_alias=True,
)
@router.post(
    "/store-workspace/{account_id}/{app_id}/metadata/sync",
    response_model=StoreWorkspaceActionResponse,
    response_model_by_alias=True,
)
def sync_store_workspace_metadata(
    account_id: str,
    app_id: str,
    payload: StoreWorkspaceSyncRequest,
    session: SessionDep,
    _: AdminDep,
) -> StoreWorkspaceActionResponse:
    try:
        if not payload.version:
            raise ApiError("missing_version", "同步到商店前需要目标商店版本", status_code=422)
        sync_scopes = set(payload.sync_scopes)
        if not sync_scopes:
            raise ApiError("missing_sync_scope", "请至少勾选一个要同步的内容", status_code=422)
        app = _scoped_app_or_error(session, account_id, app_id)
        rows = _workspace_rows_from_payload(payload.locales, fallback_locale=payload.locale)
        _preserve_readonly_keywords_for_rows(session, account_id=account_id, app=app, rows=rows)
        sync_runs = []
        include_store_images = "store_images" in sync_scopes
        for row in rows:
            if sync_scopes & {"metadata", "description", "promotional_text", "store_images"}:
                selected_scopes = sorted(
                    sync_scopes & {"metadata", "description", "promotional_text", "store_images"}
                )
                sync_runs.append(
                    sync_current_app_metadata(
                        session,
                        account_id=account_id,
                        app_id=app_id,
                        version=payload.version,
                        locale=row["locale"],
                        keywords=row["keywords"],
                        promotional_text=row["promotional_text"],
                        description=row["description"],
                        actor="admin",
                        store_images=row["store_images"],
                        include_store_images_in_payload=include_store_images,
                        sync_scopes=selected_scopes,
                    )
                )
            if "release_notes" in sync_scopes:
                sync_runs.append(
                    sync_release_notes(
                        session,
                        account_id=account_id,
                        app_id=app_id,
                        version=payload.version,
                        locale=row["locale"],
                        release_notes=row["release_notes"],
                        actor="admin",
                    )
                )
        session.commit()
    except ApiError as error:
        session.rollback()
        raise _admin_api_error(error) from error
    return _store_workspace_action_response(
        session,
        account_id=account_id,
        app_id=app_id,
        section="store",
        locale=payload.locale,
        message=f"已创建 {len(sync_runs)} 个同步任务",
        sync_runs=sync_runs,
    )


@router.delete(
    "/developer-accounts/{account_id}/apps/{app_id}/workspace/metadata/store-images",
    response_model=StoreWorkspaceActionResponse,
    response_model_by_alias=True,
)
@router.delete(
    "/store-workspace/{account_id}/{app_id}/metadata/store-images",
    response_model=StoreWorkspaceActionResponse,
    response_model_by_alias=True,
)
def delete_store_workspace_image(
    account_id: str,
    app_id: str,
    payload: StoreImageDeleteRequest,
    request: Request,
    session: SessionDep,
    _: AdminDep,
) -> StoreWorkspaceActionResponse:
    try:
        app = _scoped_app_or_error(session, account_id, app_id)
        _delete_store_image_from_current_draft(
            session,
            storage=request.app.state.artifact_storage,
            account_id=account_id,
            app=app,
            locale=payload.locale,
            slot_key=payload.slot_key,
            storage_key=payload.storage_key,
        )
        session.commit()
    except ApiError as error:
        session.rollback()
        raise _admin_api_error(error) from error
    return _store_workspace_action_response(
        session,
        account_id=account_id,
        app_id=app_id,
        section="store",
        locale=payload.locale,
        message="已删除中心后台的商店图草稿",
    )


@router.post(
    "/developer-accounts/{account_id}/apps/{app_id}/workspace/metadata/store-images",
    response_model=StoreWorkspaceActionResponse,
    response_model_by_alias=True,
)
@router.post(
    "/store-workspace/{account_id}/{app_id}/metadata/store-images",
    response_model=StoreWorkspaceActionResponse,
    response_model_by_alias=True,
)
async def upload_store_workspace_images(
    account_id: str,
    app_id: str,
    request: Request,
    session: SessionDep,
    _: AdminDep,
) -> StoreWorkspaceActionResponse:
    try:
        app = _scoped_app_or_error(session, account_id, app_id)
        uploaded_assets = await _store_image_assets_from_form(
            await request.form(),
            storage=request.app.state.artifact_storage,
            account_id=account_id,
            app_id=app.id,
            platform=app.platform,
            version=CURRENT_METADATA_VERSION,
            content_set_id=DEFAULT_CONTENT_SET_ID,
        )
        uploaded_count = _uploaded_asset_count(uploaded_assets)
        if uploaded_count == 0:
            raise ApiError("missing_store_image", "请选择要上传的商店图", status_code=422)
        _append_metadata_store_image_assets(
            session,
            account_id=account_id,
            app=app,
            uploaded_assets=uploaded_assets,
        )
        session.commit()
    except ApiError as error:
        session.rollback()
        raise _admin_api_error(error) from error
    return _store_workspace_action_response(
        session,
        account_id=account_id,
        app_id=app_id,
        section="store",
        locale=_first_uploaded_locale(uploaded_assets),
        message=f"已上传 {uploaded_count} 张中心后台商店图草稿",
    )


@router.get(
    "/developer-accounts/{account_id}/apps/{app_id}/workspace/marketing-pages/{page_id}",
    response_model=MarketingPageDetailState,
    response_model_by_alias=True,
)
@router.get(
    "/store-workspace/{account_id}/{app_id}/marketing-pages/{page_id}",
    response_model=MarketingPageDetailState,
    response_model_by_alias=True,
)
def store_marketing_page_detail_state(
    account_id: str,
    app_id: str,
    page_id: str,
    session: SessionDep,
    _: AdminDep,
    locale: Annotated[str, Query()] = "",
) -> MarketingPageDetailState:
    return _marketing_page_detail_state(
        session,
        account_id=account_id,
        app_id=app_id,
        page_id=page_id,
        locale=locale,
    )


@router.post(
    "/developer-accounts/{account_id}/apps/{app_id}/workspace/marketing-pages",
    response_model=MarketingPageActionResponse,
    response_model_by_alias=True,
)
@router.post(
    "/store-workspace/{account_id}/{app_id}/marketing-pages",
    response_model=MarketingPageActionResponse,
    response_model_by_alias=True,
)
def create_store_marketing_page_state(
    account_id: str,
    app_id: str,
    payload: MarketingPageCreateRequest,
    session: SessionDep,
    _: AdminDep,
) -> MarketingPageActionResponse:
    try:
        page = create_marketing_page(
            session,
            account_id=account_id,
            app_id=app_id,
            page_id=payload.page_id,
            page_name=payload.page_name,
            page_type=payload.page_type,
            deep_link_url=payload.deep_link_url,
            locale_rows=_marketing_rows_from_payload(payload.locales, payload.locale),
        )
        session.commit()
    except ApiError as error:
        session.rollback()
        raise _admin_api_error(error) from error
    return MarketingPageActionResponse(
        message="营销页面已创建",
        state=_marketing_page_detail_state(
            session,
            account_id=account_id,
            app_id=app_id,
            page_id=page.page_id,
            locale=payload.locale,
        ),
        workspace=_store_workspace_state(
            session,
            account_id=account_id,
            app_id=app_id,
            section="marketing",
            locale=payload.locale,
        ),
    )


@router.put(
    "/developer-accounts/{account_id}/apps/{app_id}/workspace/marketing-pages/{page_id}",
    response_model=MarketingPageActionResponse,
    response_model_by_alias=True,
)
@router.put(
    "/store-workspace/{account_id}/{app_id}/marketing-pages/{page_id}",
    response_model=MarketingPageActionResponse,
    response_model_by_alias=True,
)
def save_store_marketing_page_state(
    account_id: str,
    app_id: str,
    page_id: str,
    payload: MarketingPageSaveRequest,
    session: SessionDep,
    _: AdminDep,
) -> MarketingPageActionResponse:
    try:
        _save_marketing_page_from_payload(
            session,
            account_id=account_id,
            app_id=app_id,
            page_id=page_id,
            payload=payload,
        )
        session.commit()
    except ApiError as error:
        session.rollback()
        raise _admin_api_error(error) from error
    return _marketing_page_action_response(
        session,
        account_id=account_id,
        app_id=app_id,
        page_id=page_id,
        locale=payload.locale,
        message="营销页面草稿已保存",
    )


@router.post(
    "/developer-accounts/{account_id}/apps/{app_id}/workspace/marketing-pages/{page_id}/copy",
    response_model=MarketingPageActionResponse,
    response_model_by_alias=True,
)
@router.post(
    "/store-workspace/{account_id}/{app_id}/marketing-pages/{page_id}/copy",
    response_model=MarketingPageActionResponse,
    response_model_by_alias=True,
)
def copy_store_marketing_page_state(
    account_id: str,
    app_id: str,
    page_id: str,
    session: SessionDep,
    _: AdminDep,
) -> MarketingPageActionResponse:
    try:
        page = duplicate_marketing_page(
            session,
            account_id=account_id,
            app_id=app_id,
            page_id=page_id,
        )
        session.commit()
    except ApiError as error:
        session.rollback()
        raise _admin_api_error(error) from error
    return _marketing_page_action_response(
        session,
        account_id=account_id,
        app_id=app_id,
        page_id=page.page_id,
        locale=DEFAULT_LOCALE,
        message="已复制营销页面",
    )


@router.delete(
    "/developer-accounts/{account_id}/apps/{app_id}/workspace/marketing-pages/{page_id}",
    response_model=MarketingPageActionResponse,
    response_model_by_alias=True,
)
@router.delete(
    "/store-workspace/{account_id}/{app_id}/marketing-pages/{page_id}",
    response_model=MarketingPageActionResponse,
    response_model_by_alias=True,
)
def delete_store_marketing_page_state(
    account_id: str,
    app_id: str,
    page_id: str,
    session: SessionDep,
    _: AdminDep,
) -> MarketingPageActionResponse:
    try:
        delete_marketing_page(
            session,
            account_id=account_id,
            app_id=app_id,
            page_id=page_id,
        )
        session.commit()
    except ApiError as error:
        session.rollback()
        raise _admin_api_error(error) from error
    return MarketingPageActionResponse(
        message="已删除中心后台的营销页面",
        state=None,
        workspace=_store_workspace_state(
            session,
            account_id=account_id,
            app_id=app_id,
            section="marketing",
            locale=DEFAULT_LOCALE,
        ),
    )


@router.post(
    "/developer-accounts/{account_id}/apps/{app_id}/workspace/marketing-pages/{page_id}/preflight",
    response_model=MarketingPageActionResponse,
    response_model_by_alias=True,
)
@router.post(
    "/store-workspace/{account_id}/{app_id}/marketing-pages/{page_id}/preflight",
    response_model=MarketingPageActionResponse,
    response_model_by_alias=True,
)
def check_store_marketing_page_preflight_state(
    account_id: str,
    app_id: str,
    page_id: str,
    payload: MarketingPageSyncRequest,
    session: SessionDep,
    _: AdminDep,
) -> MarketingPageActionResponse:
    try:
        page = _marketing_page_or_error(
            session,
            account_id=account_id,
            app_id=app_id,
            page_id=page_id,
        )
        locale = _source_locale_from_marketing_payload(payload.locales, payload.locale)
        get_or_refresh_preflight(
            session,
            account_id=account_id,
            app_id=app_id,
            version=page.page_id,
            locale=locale,
            operation=UPDATE_MARKETING_PAGE,
            force_refresh=True,
        )
        session.commit()
    except ApiError as error:
        session.rollback()
        raise _admin_api_error(error) from error
    return _marketing_page_action_response(
        session,
        account_id=account_id,
        app_id=app_id,
        page_id=page_id,
        locale=locale,
        message="已实时查询营销页面同步状态",
    )


@router.post(
    "/developer-accounts/{account_id}/apps/{app_id}/workspace/marketing-pages/{page_id}/sync",
    response_model=MarketingPageActionResponse,
    response_model_by_alias=True,
)
@router.post(
    "/store-workspace/{account_id}/{app_id}/marketing-pages/{page_id}/sync",
    response_model=MarketingPageActionResponse,
    response_model_by_alias=True,
)
def sync_store_marketing_page_state(
    account_id: str,
    app_id: str,
    page_id: str,
    payload: MarketingPageSyncRequest,
    session: SessionDep,
    _: AdminDep,
) -> MarketingPageActionResponse:
    try:
        _save_marketing_page_from_payload(
            session,
            account_id=account_id,
            app_id=app_id,
            page_id=page_id,
            payload=payload,
        )
        rows = _marketing_rows_from_payload(payload.locales, payload.locale)
        sync_runs = [
            sync_marketing_page(
                session,
                account_id=account_id,
                app_id=app_id,
                page_id=page_id,
                locale=str(row["locale"]),
                sync_scopes=payload.sync_scopes,
                actor="admin",
            )
            for row in rows
        ]
        session.commit()
    except ApiError as error:
        session.rollback()
        raise _admin_api_error(error) from error
    return _marketing_page_action_response(
        session,
        account_id=account_id,
        app_id=app_id,
        page_id=page_id,
        locale=payload.locale,
        message=f"营销页面已同步 {len(sync_runs)} 个语言",
        sync_runs=sync_runs,
    )


@router.delete(
    "/developer-accounts/{account_id}/apps/{app_id}/workspace/marketing-pages/{page_id}/store-images",
    response_model=MarketingPageActionResponse,
    response_model_by_alias=True,
)
@router.delete(
    "/store-workspace/{account_id}/{app_id}/marketing-pages/{page_id}/store-images",
    response_model=MarketingPageActionResponse,
    response_model_by_alias=True,
)
def delete_store_marketing_page_image_state(
    account_id: str,
    app_id: str,
    page_id: str,
    payload: StoreImageDeleteRequest,
    request: Request,
    session: SessionDep,
    _: AdminDep,
) -> MarketingPageActionResponse:
    try:
        _delete_marketing_page_image_from_draft(
            session,
            storage=request.app.state.artifact_storage,
            account_id=account_id,
            app_id=app_id,
            page_id=page_id,
            locale=payload.locale,
            slot_key=payload.slot_key,
            storage_key=payload.storage_key,
        )
        session.commit()
    except ApiError as error:
        session.rollback()
        raise _admin_api_error(error) from error
    return _marketing_page_action_response(
        session,
        account_id=account_id,
        app_id=app_id,
        page_id=page_id,
        locale=payload.locale,
        message="已删除中心后台的营销页面截图",
    )


@router.post(
    "/developer-accounts/{account_id}/apps/{app_id}/workspace/marketing-pages/{page_id}/store-images",
    response_model=MarketingPageActionResponse,
    response_model_by_alias=True,
)
@router.post(
    "/store-workspace/{account_id}/{app_id}/marketing-pages/{page_id}/store-images",
    response_model=MarketingPageActionResponse,
    response_model_by_alias=True,
)
async def upload_store_marketing_page_images_state(
    account_id: str,
    app_id: str,
    page_id: str,
    request: Request,
    session: SessionDep,
    _: AdminDep,
) -> MarketingPageActionResponse:
    try:
        app = _scoped_app_or_error(session, account_id, app_id)
        page = _marketing_page_or_error(
            session,
            account_id=account_id,
            app_id=app.id,
            page_id=page_id,
        )
        uploaded_assets = await _store_image_assets_from_form(
            await request.form(),
            storage=request.app.state.artifact_storage,
            account_id=account_id,
            app_id=app.id,
            platform=app.platform,
            version=CURRENT_METADATA_VERSION,
            content_set_id=page_id,
        )
        uploaded_count = _uploaded_asset_count(uploaded_assets)
        if uploaded_count == 0:
            raise ApiError("missing_store_image", "请选择要上传的营销页面截图", status_code=422)
        _append_marketing_page_store_image_assets(
            session,
            marketing_page_id=str(page.id),
            uploaded_assets=uploaded_assets,
        )
        session.commit()
    except ApiError as error:
        session.rollback()
        raise _admin_api_error(error) from error
    return _marketing_page_action_response(
        session,
        account_id=account_id,
        app_id=app_id,
        page_id=page_id,
        locale=_first_uploaded_locale(uploaded_assets),
        message=f"已上传 {uploaded_count} 张营销页面截图草稿",
    )


@router.patch(
    "/developer-accounts/{account_id}",
    response_model=DeveloperAccountSaveResponse,
    response_model_by_alias=True,
)
def update_developer_account(
    account_id: str,
    payload: DeveloperAccountForm,
    session: SessionDep,
    _: AdminDep,
) -> DeveloperAccountSaveResponse:
    if session.get(DeveloperAccount, account_id) is None:
        raise AdminApiError("account_not_found", "开发者账号不存在", status_code=404)
    try:
        account = save_developer_account(
            session,
            account_id=account_id,
            team_name=payload.team_name,
            expires_at=parse_admin_datetime(payload.expires_at),
            status=payload.status,
            renewal_action_label=payload.renewal_action_label,
        )
        session.commit()
    except ApiError as error:
        session.rollback()
        raise _admin_api_error(error) from error
    return DeveloperAccountSaveResponse(
        message="开发者账号已更新",
        account=_account_summary_for_id(session, account.id),
        state=_developer_accounts_state(session),
    )


@router.post(
    "/developer-accounts/{account_id}/connector",
    response_model=ConnectorActionResponse,
    response_model_by_alias=True,
)
def save_account_connector(
    account_id: str,
    payload: ConnectorSaveRequest,
    request: Request,
    session: SessionDep,
    _: AdminDep,
) -> ConnectorActionResponse:
    try:
        effective = effective_business_settings(session, request.app.state.settings)
        save_connector(
            session,
            account_id=account_id,
            name=payload.name,
            base_url=payload.base_url,
            auth_token=payload.auth_token,
            base_url_template=effective.connector_base_url_template,
        )
        session.commit()
    except ApiError as error:
        session.rollback()
        raise _admin_api_error(error) from error
    state = _developer_account_detail_state(session, account_id)
    return ConnectorActionResponse(message="Connector 已保存", result=state.connector, state=state)


@router.post(
    "/developer-accounts/{account_id}/connector/check",
    response_model=ConnectorActionResponse,
    response_model_by_alias=True,
)
def check_account_connector(
    account_id: str,
    session: SessionDep,
    _: AdminDep,
) -> ConnectorActionResponse:
    try:
        result = check_connector_health(session, account_id=account_id)
        session.commit()
    except ApiError as error:
        session.rollback()
        raise _admin_api_error(error) from error
    state = _developer_account_detail_state(session, account_id)
    return ConnectorActionResponse(message=result.message, result=state.connector, state=state)


@router.post(
    "/developer-accounts/{account_id}/apps",
    response_model=AccountDetailActionResponse,
    response_model_by_alias=True,
)
def bind_account_app(
    account_id: str,
    payload: AccountAppBindRequest,
    session: SessionDep,
    _: AdminDep,
) -> AccountDetailActionResponse:
    try:
        bind_app_to_account(
            session,
            account_id=account_id,
            app_id=payload.app_id,
            store_app_id=payload.store_app_id,
            store_package_name=payload.store_package_name,
        )
        session.commit()
    except ApiError as error:
        session.rollback()
        raise _admin_api_error(error) from error
    return AccountDetailActionResponse(
        message="App 已绑定到账号",
        state=_developer_account_detail_state(session, account_id),
    )


@router.patch(
    "/developer-accounts/{account_id}/apps/{app_id}/settings",
    response_model=AccountDetailActionResponse,
    response_model_by_alias=True,
)
def update_account_app_settings(
    account_id: str,
    app_id: str,
    payload: AccountAppSettingsRequest,
    session: SessionDep,
    _: AdminDep,
) -> AccountDetailActionResponse:
    try:
        update_bound_app_store_settings(
            session,
            account_id=account_id,
            app_id=app_id,
            store_app_id=payload.store_app_id,
            store_package_name=payload.store_package_name,
        )
        session.commit()
    except ApiError as error:
        session.rollback()
        raise _admin_api_error(error) from error
    return AccountDetailActionResponse(
        message="商店标识已保存",
        state=_developer_account_detail_state(session, account_id),
    )


@router.delete(
    "/developer-accounts/{account_id}/apps/{app_id}",
    response_model=AccountDetailActionResponse,
    response_model_by_alias=True,
)
def unbind_account_app(
    account_id: str,
    app_id: str,
    session: SessionDep,
    _: AdminDep,
) -> AccountDetailActionResponse:
    try:
        unbind_app_from_account(session, account_id=account_id, app_id=app_id)
        session.commit()
    except ApiError as error:
        session.rollback()
        raise _admin_api_error(error) from error
    return AccountDetailActionResponse(
        message="App 已解绑",
        state=_developer_account_detail_state(session, account_id),
    )


@router.get(
    "/uploads",
    response_model=UploadState,
    response_model_by_alias=True,
)
def upload_state(
    session: SessionDep,
    _: AdminDep,
) -> UploadState:
    return _upload_state(session)


@router.get(
    "/app-logs",
    response_model=AppLogsState,
    response_model_by_alias=True,
)
def app_logs_state(
    request: Request,
    _: AdminDep,
    cursor: Annotated[int, Query()] = 0,
    limit: Annotated[int, Query()] = 500,
) -> AppLogsState:
    return _app_logs_state(request, cursor=cursor, limit=limit)


@router.get(
    "/app-logs/events",
    response_model=AppLogsState,
    response_model_by_alias=True,
)
def app_logs_events(
    request: Request,
    _: AdminDep,
    cursor: Annotated[int, Query()] = 0,
    limit: Annotated[int, Query()] = 500,
) -> AppLogsState:
    return _app_logs_state(request, cursor=cursor, limit=limit)


@router.post(
    "/uploads",
    response_model=AdminUploadResponse,
    response_model_by_alias=True,
)
async def upload_package(
    request: Request,
    session: SessionDep,
    _: AdminDep,
    file: Annotated[UploadFile, File()],
    platform: Annotated[str, Form()],
    environment: Annotated[str, Form()],
    changelog: Annotated[str, Form()] = "",
    app_name: Annotated[str | None, Form(alias="appName")] = None,
    developer_account_id: Annotated[str | None, Form(alias="developerAccountId")] = None,
    store_app_id: Annotated[str | None, Form(alias="storeAppId")] = None,
    store_package_name: Annotated[str | None, Form(alias="storePackageName")] = None,
) -> AdminUploadResponse:
    try:
        upload = create_package_upload(
            session=session,
            storage=request.app.state.artifact_storage,
            content=await file.read(),
            file_name=file.filename or "",
            content_type=file.content_type or "",
            platform=platform,
            environment=environment,
            changelog=changelog,
            app_name=app_name,
            developer_account_id=developer_account_id,
            store_app_id=store_app_id,
            store_package_name=store_package_name,
        )
    except ApiError as error:
        session.rollback()
        raise _admin_api_error(error) from error

    return AdminUploadResponse(
        message="上传成功，包信息已自动解析",
        result=_upload_result(session, upload.app.id),
        state=_upload_state(session),
    )


def _upload_state(session: Session) -> UploadState:
    context = upload_context(session)
    accounts = context.get("accounts", [])
    return UploadState(
        accounts=[
            UploadAccountOption(
                id=account.id,
                team_name=account.team_name,
                status=account.status,
                platform=None,
            )
            for account in accounts
            if isinstance(account, DeveloperAccount)
        ]
    )


def _upload_result(session: Session, app_id: str) -> UploadResult:
    app = session.get(App, app_id)
    if app is None:
        raise AdminApiError("upload_app_not_found", "上传结果中的 App 不存在", status_code=500)
    latest_build = max(app.builds, key=lambda build: build.uploaded_at, default=None)
    if latest_build is None:
        raise AdminApiError("upload_build_not_found", "上传结果中的构建不存在", status_code=500)
    install_info = latest_build.package_artifact()
    return UploadResult(
        app_id=app.id,
        app_name=app.name,
        bundle_identifier=app.bundle_identifier,
        platform=latest_build.platform,
        environment=latest_build.environment,
        version=latest_build.version or "",
        build_number=latest_build.build_number or "",
        developer_account=(
            app.developer_account.team_name if app.developer_account else "未绑定账号"
        ),
        store_identifier=_store_identifier(app) or "未填写",
        install_url=install_info.install_url if install_info else "",
        manifest_url=install_info.manifest_url if install_info else None,
        download_url=install_info.download_url if install_info else None,
    )


def _app_logs_state(request: Request, *, cursor: int = 0, limit: int = 500) -> AppLogsState:
    snapshot = request.app.state.app_log_hub.snapshot(cursor=cursor, limit=limit)
    return AppLogsState(
        connect=build_app_log_connect_context(request),
        cursor=snapshot.cursor,
        devices=snapshot.devices,
        logs=snapshot.logs,
        errors=snapshot.errors,
        levels=list(LEVELS),
    )


def _build_item(build: Build) -> BuildItem:
    return build_platform.build_item(build)


def _device_item(device: Device) -> DeviceItem:
    return DeviceItem(
        id=device.id,
        name=device.name,
        owner=device.owner,
        platform=device.platform,
        platform_label=platform_label(device.platform),
        status=device.status,
        status_color=device.status_color,
        detail=device.detail,
        udid=device.udid,
        os_version=device.os_version,
        certificate_status=device.certificate_status,
        registered_at=_iso_datetime(device.registered_at),
        registered_at_label=format_datetime(device.registered_at),
    )


def _notification_item(notification: Notification) -> NotificationItem:
    return NotificationItem(
        id=notification.id,
        type=notification.type,
        section=notification.section,
        icon_key=notification.icon_key,
        title=notification.title,
        subtitle=notification.subtitle,
        tag=notification.tag,
        tag_color=notification.tag_color,
        created_at=_iso_datetime(notification.created_at),
        created_at_label=format_datetime(notification.created_at),
    )


def _notification_types(notifications: list[Notification]) -> set[str]:
    return {notification.type for notification in notifications} | {"all"}


def _notification_type_counts(notifications: list[Notification]) -> list[NotificationTypeCount]:
    labels = {"all": "全部", "build": "构建", "account": "账号", "device": "设备"}
    counts = {"all": len(notifications)}
    for notification in notifications:
        counts[notification.type] = counts.get(notification.type, 0) + 1
    return [
        NotificationTypeCount(
            type=type_key,
            label=labels.get(type_key, type_key),
            count=counts[type_key],
        )
        for type_key in ["all", *sorted(key for key in counts if key != "all")]
    ]


def _webhook_delivery_count(session: Session, *, status: str) -> int:
    return int(
        session.scalar(
            select(func.count(WebhookDelivery.id)).where(
                WebhookDelivery.channel == "dingtalk",
                WebhookDelivery.status == status,
            )
        )
        or 0
    )


def _public_api_markdown() -> str:
    return (
        resources.files("testflying_api").joinpath(PUBLIC_API_DOC_PATH).read_text(encoding="utf-8")
    )


def _parse_public_api_endpoints(markdown: str) -> list[ApiDocEndpointItem]:
    headings = list(re.finditer(r"^##\s+\d+\.\s+(.+)$", markdown, flags=re.MULTILINE))
    endpoints: list[ApiDocEndpointItem] = []
    for index, heading in enumerate(headings, start=1):
        section_end = headings[index].start() if index < len(headings) else len(markdown)
        section = markdown[heading.start() : section_end]
        request_line = _first_markdown_code_block(section, "http").splitlines()[0].strip()
        method, path = (request_line.split(" ", 1) + [""])[:2]
        endpoints.append(
            ApiDocEndpointItem(
                anchor=f"endpoint-{index}",
                title=heading.group(1).strip(),
                method=method,
                path=path.strip(),
                summary=_public_api_section_summary(section),
                params=_parse_public_api_params(section),
                curl=_first_markdown_code_block(section, "bash").strip(),
                response=_first_markdown_code_block(section, "json").strip(),
            )
        )
    return endpoints


def _public_api_section_summary(section: str) -> str:
    without_code = re.sub(r"```.*?```", "", section, flags=re.DOTALL)
    for line in without_code.splitlines()[1:]:
        value = line.strip()
        if not value or value == "参数：" or value.startswith("|"):
            continue
        return value
    return ""


def _parse_public_api_params(section: str) -> list[ApiDocParamItem]:
    match = re.search(r"参数：\n\n((?:\|.*\|\n?)+)", section)
    if not match:
        return []
    params: list[ApiDocParamItem] = []
    for line in match.group(1).splitlines():
        cells = [_strip_markdown_cell(cell) for cell in line.strip().strip("|").split("|")]
        if len(cells) < 4 or cells[0] in {"参数", "---"} or set(cells[0]) == {"-"}:
            continue
        params.append(
            ApiDocParamItem(
                name=cells[0],
                location=cells[1],
                required=cells[2],
                description=cells[3],
            )
        )
    return params


def _first_markdown_code_block(section: str, language: str) -> str:
    match = re.search(rf"```{re.escape(language)}\n(.*?)\n```", section, flags=re.DOTALL)
    return match.group(1).strip() if match else ""


def _strip_markdown_cell(value: str) -> str:
    return value.strip().strip("`").replace("**", "")


@router.get(
    "/store-reviews",
    response_model=StoreReviewsState,
    response_model_by_alias=True,
)
def store_reviews_state(
    session: SessionDep,
    _: AdminDep,
    accountId: Annotated[str, Query()] = "",
    appId: Annotated[str, Query()] = "",
    rating: Annotated[int | None, Query()] = None,
) -> StoreReviewsState:
    return _store_reviews_state(session, account_id=accountId, app_id=appId, rating=rating)


@router.post(
    "/store-reviews/fetch",
    response_model=StoreReviewFetchResponse,
    response_model_by_alias=True,
)
def fetch_store_reviews(
    scope: ReviewScopeRequest,
    session: SessionDep,
    _: AdminDep,
) -> StoreReviewFetchResponse:
    try:
        result = fetch_store_reviews_incremental(
            session,
            account_id=scope.account_id,
            app_id=scope.app_id,
        )
        session.commit()
    except ApiError as error:
        session.rollback()
        raise _admin_api_error(error) from error
    return StoreReviewFetchResponse(
        message="最新评论已拉取",
        result=_fetch_run_item(result.run),
        state=_store_reviews_state(
            session,
            account_id=scope.account_id,
            app_id=scope.app_id,
            rating=None,
        ),
    )


@router.post(
    "/store-reviews/analyze",
    response_model=StoreReviewAnalysisResponse,
    response_model_by_alias=True,
)
def analyze_reviews(
    request: Request,
    scope: ReviewScopeRequest,
    session: SessionDep,
    _: AdminDep,
) -> StoreReviewAnalysisResponse:
    try:
        run = analyze_store_reviews(
            session,
            request.app.state.settings,
            account_id=scope.account_id,
            app_id=scope.app_id,
        )
        session.commit()
        return StoreReviewAnalysisResponse(
            message="评论分析已完成",
            result=_analysis_run_item(run),
            state=_store_reviews_state(
                session,
                account_id=scope.account_id,
                app_id=scope.app_id,
                rating=None,
            ),
        )
    except ApiError as error:
        session.commit()
        return StoreReviewAnalysisResponse(
            message="评论分析失败",
            result=None,
            state=_store_reviews_state(
                session,
                account_id=scope.account_id,
                app_id=scope.app_id,
                rating=None,
            ),
            error={
                "code": error.code,
                "message": error.message,
            },
        )


def _llm_config_state(session: Session) -> LlmConfigState:
    profiles = list_llm_profiles(session)
    bindings = list_llm_bindings(session)
    return LlmConfigState(
        protocols=[
            LlmProtocolItem(
                key=str(protocol["key"]),
                label=str(protocol["label"]),
                default_base_url=str(protocol["defaultBaseUrl"]),
                default_model=str(protocol["defaultModel"]),
                default_auth_header=str(protocol["defaultAuthHeader"]),
            )
            for protocol in LLM_PROTOCOLS
        ],
        presets=[
            LlmPresetItem(
                key=str(preset["key"]),
                label=str(preset["label"]),
                protocol=str(preset["protocol"]),
                base_url=str(preset["baseUrl"]),
                model=str(preset["model"]),
                auth_header=str(preset["authHeader"]),
            )
            for preset in LLM_PRESETS
        ],
        profiles=[_llm_profile_item(profile) for profile in profiles],
        feature_bindings=[
            _llm_feature_binding_item(bindings.get(str(feature["key"])), profiles, feature=feature)
            for feature in LLM_FEATURES
        ],
    )


def _llm_profile_item(profile: LlmProfile) -> LlmProfileItem:
    status_label = "已配置" if profile.api_key else "缺少 Key"
    if profile.status == "unchecked" and profile.api_key:
        status_label = "未检查"
    return LlmProfileItem(
        id=profile.id,
        name=profile.name,
        protocol=profile.protocol,
        protocol_label=protocol_label(profile.protocol),
        base_url=profile.base_url,
        model=profile.model,
        auth_header=profile.auth_header,
        auth_header_label=auth_header_label(profile.auth_header),
        api_key_set=bool(profile.api_key),
        api_key_preview=mask_api_key(profile.api_key),
        status=profile.status,
        status_label=status_label,
        updated_at_label=format_datetime(profile.updated_at),
    )


def _llm_feature_binding_item(
    binding: LlmFeatureBinding | None,
    profiles: list[LlmProfile],
    *,
    feature: dict[str, object] | None = None,
) -> LlmFeatureBindingItem:
    if feature is None and binding is not None:
        feature = next(
            (item for item in LLM_FEATURES if str(item["key"]) == binding.feature_key),
            None,
        )
    feature_key = str(feature["key"] if feature else binding.feature_key if binding else "")
    feature_label = str(feature["label"] if feature else feature_key)
    description = str(feature["description"] if feature else "")
    profile_by_id = {profile.id: profile for profile in profiles}
    primary_id = binding.primary_profile_id if binding else None
    primary_profile = profile_by_id.get(primary_id or "")
    if primary_profile is None:
        status = "unbound"
        status_label = "未绑定"
        effective_label = "未选择模型"
    elif primary_profile.api_key:
        status = "ready"
        status_label = "已启用"
        effective_label = primary_profile.name
    else:
        status = "needs_key"
        status_label = "缺少 Key"
        effective_label = primary_profile.name
    return LlmFeatureBindingItem(
        feature_key=feature_key,
        feature_label=feature_label,
        description=description,
        primary_profile_id=primary_id,
        fallback_profile_id=binding.fallback_profile_id if binding else None,
        effective_profile_label=effective_label,
        status=status,
        status_label=status_label,
    )


def _store_apps_state(
    session: Session,
    *,
    active_filter: str,
    app_id: str,
) -> StoreAppsState:
    all_apps = list_apps(session)
    normalized_filter = (
        active_filter if active_filter in {"all", "ios", "android", "needs", "ok"} else "all"
    )
    app_items = [_store_app_item(app, selected_app_id=app_id) for app in all_apps]
    filtered_apps = [
        item for item in app_items if _store_app_matches_filter(item, normalized_filter)
    ]
    selected_app = next((item for item in filtered_apps if item.id == app_id), None)
    if selected_app is None:
        selected_app = filtered_apps[0] if filtered_apps else None
    if selected_app is not None:
        filtered_apps = [
            item.model_copy(update={"selected": item.id == selected_app.id})
            for item in filtered_apps
        ]
        selected_app = next((item for item in filtered_apps if item.selected), selected_app)
    return StoreAppsState(
        apps=filtered_apps,
        selected_app=selected_app,
        filter=normalized_filter,
        stats=StoreAppsStats(
            total=len(app_items),
            ios=sum(1 for item in app_items if item.platform == "ios"),
            android=sum(1 for item in app_items if item.platform == "android"),
            ready=sum(1 for item in app_items if item.status == "ready"),
            needs=sum(1 for item in app_items if item.status != "ready"),
        ),
        account_summary=_store_apps_account_summary(session, app_items),
    )


def _store_app_item(app: App, *, selected_app_id: str) -> StoreAppItem:
    latest_build = max(app.builds, key=lambda build: build.uploaded_at, default=None)
    account = app.developer_account
    store_identifier = _store_identifier(app)
    status = _store_app_status(app, store_identifier)
    return StoreAppItem(
        id=app.id,
        name=app.name,
        bundle_identifier=app.bundle_identifier,
        platform=app.platform,
        developer_account_id=account.id if account else None,
        developer_account_name=account.team_name if account else "",
        icon_color=app.icon_color,
        icon_text=app.name[:2].upper(),
        store_identifier=store_identifier,
        status=status,
        status_label=_store_app_status_label(status),
        latest_build=(
            StoreAppBuildItem(
                version=latest_build.version or "",
                build_number=latest_build.build_number or "",
                environment=latest_build.environment,
                uploaded_at=_iso_datetime(latest_build.uploaded_at),
            )
            if latest_build
            else None
        ),
        selected=app.id == selected_app_id,
        store_management_path=(
            f"/admin/accounts/{account.id}/apps/{app.id}/store" if account else ""
        ),
        reviews_path=(
            f"/admin/store-reviews?accountId={account.id}&appId={app.id}" if account else ""
        ),
    )


def _store_app_matches_filter(app: StoreAppItem, active_filter: str) -> bool:
    if active_filter == "all":
        return True
    if active_filter in {"ios", "android"}:
        return app.platform == active_filter
    if active_filter == "ok":
        return app.status == "ready"
    if active_filter == "needs":
        return app.status != "ready"
    return True


def _store_apps_account_summary(
    session: Session,
    app_items: list[StoreAppItem],
) -> StoreAppsAccountSummary:
    accounts = list_accounts(session)
    return StoreAppsAccountSummary(
        total_accounts=len(accounts),
        bound_apps=sum(1 for item in app_items if item.developer_account_id),
        connector_ok=sum(
            1 for item in accounts if getattr(item.get("connector"), "status", "") == "ok"
        ),
        connector_needs=sum(
            1 for item in accounts if getattr(item.get("connector"), "status", "") != "ok"
        ),
        renewal_reminders=sum(
            1 for item in accounts if getattr(item.get("account"), "status", "") != "ok"
        ),
    )


def _store_identifier(app: App) -> str:
    if app.platform == "ios":
        return app.store_app_id or ""
    if app.platform == "android":
        return app.store_package_name or app.bundle_identifier
    return app.store_app_id or app.store_package_name or ""


def _store_app_status(app: App, store_identifier: str) -> str:
    if app.developer_account is None:
        return "needs_account"
    if not store_identifier:
        return "needs_identifier"
    return "ready"


def _store_app_status_label(status: str) -> str:
    if status == "ready":
        return "可同步"
    if status == "needs_identifier":
        return "待填写商店标识"
    return "未绑定账号"


def _developer_accounts_state(session: Session) -> DeveloperAccountsState:
    accounts = [_developer_account_summary(row) for row in list_accounts(session)]
    return DeveloperAccountsState(
        accounts=accounts,
        stats=DeveloperAccountsStats(
            total=len(accounts),
            ok=sum(
                1
                for account in accounts
                if account.status == "ok" and account.connector_status == "ok"
            ),
            needs=sum(
                1
                for account in accounts
                if account.status != "ok" or account.connector_status != "ok"
            ),
            bound_apps=sum(len(account.app_names) for account in accounts),
            connector_needs=sum(1 for account in accounts if account.connector_status != "ok"),
        ),
    )


def _developer_account_summary(row: dict[str, object]) -> DeveloperAccountSummary:
    account = row.get("account")
    if not isinstance(account, DeveloperAccount):
        raise AdminApiError("invalid_account_row", "开发者账号数据格式异常", status_code=500)
    apps = row.get("apps")
    connector = row.get("connector")
    latest_sync = row.get("latest_sync")
    return DeveloperAccountSummary(
        id=account.id,
        team_name=account.team_name,
        status=account.status,
        status_label=account_status_label(account.status),
        expires_at=_iso_datetime(account.expires_at),
        expires_at_label=format_datetime(account.expires_at),
        remaining_days=int(row.get("remaining_days") or 0),
        app_names=[str(app_name) for app_name in apps] if isinstance(apps, list) else [],
        connector_name=str(getattr(connector, "name", "") or "未配置"),
        connector_status=str(getattr(connector, "status", "") or "missing"),
        connector_status_label=_connector_status_label(
            str(getattr(connector, "status", "") or "missing")
        ),
        latest_sync_status=str(getattr(latest_sync, "status", "") or "无"),
        latest_sync_at_label=format_datetime(getattr(latest_sync, "started_at", None)),
        detail_path=f"/admin/accounts/{account.id}",
    )


def _account_summary_for_id(session: Session, account_id: str) -> DeveloperAccountSummary:
    for row in list_accounts(session):
        account = row.get("account")
        if isinstance(account, DeveloperAccount) and account.id == account_id:
            return _developer_account_summary(row)
    raise AdminApiError("account_not_found", "开发者账号不存在", status_code=404)


def _auto_check_connector(session: Session, account_id: str) -> None:
    connector = account_connector(session, account_id)
    if connector and _recently_checked(getattr(connector, "last_checked_at", None)):
        return
    try:
        check_connector_health(session, account_id=account_id)
        session.commit()
    except ApiError:
        session.rollback()


def _recently_checked(value: datetime | None) -> bool:
    if value is None:
        return False
    checked_at = value if value.tzinfo else value.replace(tzinfo=UTC)
    return datetime.now(UTC) - checked_at.astimezone(UTC) < CONNECTOR_AUTO_CHECK_TTL


def _developer_account_detail_state(
    session: Session,
    account_id: str,
) -> DeveloperAccountDetailState:
    context = account_detail_context(session, account_id)
    account = context.get("account")
    if not isinstance(account, DeveloperAccount):
        raise AdminApiError("account_not_found", "开发者账号不存在", status_code=404)

    connector = context.get("connector")
    sync_runs = [
        run
        for run in context.get("sync_runs", [])
        if hasattr(run, "id") and hasattr(run, "started_at")
    ]
    account_summary = _developer_account_summary(
        {
            "account": account,
            "remaining_days": context.get("remaining_days") or 0,
            "apps": [
                item["app"].name
                for item in context.get("apps", [])
                if isinstance(item, dict) and isinstance(item.get("app"), App)
            ],
            "connector": connector,
            "latest_sync": sync_runs[0] if sync_runs else None,
        }
    )
    return DeveloperAccountDetailState(
        account=account_summary,
        connector=_connector_state(connector),
        account_store_platform=str(context.get("account_store_platform") or "mixed"),
        apps=[
            _account_app_item(item, account_id=account.id)
            for item in context.get("apps", [])
            if isinstance(item, dict)
        ],
        unassigned_apps=[
            _unassigned_app_item(app)
            for app in context.get("unassigned_apps", [])
            if isinstance(app, App)
        ],
        sync_runs=[_sync_run_summary(run) for run in sync_runs[:8]],
    )


def _account_app_item(item: dict[str, object], *, account_id: str) -> AccountAppItem:
    app = item.get("app")
    latest_build = item.get("latest_build")
    if not isinstance(app, App):
        raise AdminApiError("invalid_account_app", "账号 App 数据格式异常", status_code=500)
    return AccountAppItem(
        id=app.id,
        name=app.name,
        bundle_identifier=app.bundle_identifier,
        platform=app.platform,
        platform_label=platform_label(app.platform),
        icon_color=app.icon_color,
        icon_text=app.name[:2].upper(),
        store_app_id=app.store_app_id or "",
        store_package_name=app.store_package_name or "",
        latest_version_label=_latest_build_label(latest_build),
        store_path=f"/admin/accounts/{account_id}/apps/{app.id}/store",
        marketing_path=f"/admin/accounts/{account_id}/apps/{app.id}/marketing",
        release_notes_path=f"/admin/accounts/{account_id}/apps/{app.id}/release-notes",
        connection_path=f"/admin/accounts/{account_id}/apps/{app.id}/connection",
    )


def _unassigned_app_item(app: App) -> UnassignedAppItem:
    return UnassignedAppItem(
        id=app.id,
        name=app.name,
        bundle_identifier=app.bundle_identifier,
        platform=app.platform,
        platform_label=platform_label(app.platform),
    )


def _connector_state(connector: object | None) -> ConnectorState | None:
    if connector is None:
        return None
    status = str(getattr(connector, "status", "") or "unknown")
    return ConnectorState(
        name=str(getattr(connector, "name", "") or ""),
        base_url=str(getattr(connector, "base_url", "") or ""),
        auth_token=str(getattr(connector, "auth_token", "") or ""),
        status=status,
        status_label=_connector_status_label(status),
        checked_at_label=format_datetime(getattr(connector, "last_checked_at", None)),
    )


def _sync_run_summary(run: object) -> SyncRunSummary:
    operation = str(getattr(run, "operation", "") or "")
    status = str(getattr(run, "status", "") or "")
    error_summary = str(getattr(run, "error_summary", "") or "")
    version = str(getattr(run, "version", "") or "")
    locale = str(getattr(run, "locale", "") or "")
    summary = error_summary or " / ".join(item for item in [operation, version, locale] if item)
    return SyncRunSummary(
        id=str(getattr(run, "id", "") or ""),
        operation=operation,
        status=status,
        started_at_label=format_datetime(getattr(run, "started_at", None)),
        summary=summary or "已记录同步任务",
    )


def _connector_status_label(status: str) -> str:
    return {
        "ok": "正常",
        "missing": "未配置",
        "unknown": "未检查",
        "error": "异常",
    }.get(status, status or "未配置")


def _remaining_days_from_account(session: Session, account_id: str) -> int:
    for row in list_accounts(session):
        account = row.get("account")
        if isinstance(account, DeveloperAccount) and account.id == account_id:
            return int(row.get("remaining_days") or 0)
    return 0


def _latest_build_label(latest_build: object | None) -> str:
    if latest_build is None:
        return "暂无构建"
    version = str(getattr(latest_build, "version", "") or "")
    build_number = str(getattr(latest_build, "build_number", "") or "")
    if version and build_number:
        return f"{version} ({build_number})"
    return version or build_number or "暂无构建"


def _store_workspace_state(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    section: str,
    locale: str,
) -> StoreWorkspaceState:
    normalized_section = (
        section if section in {"store", "marketing", "release-notes", "connection"} else "store"
    )
    context = (
        store_marketing_context(
            session,
            account_id=account_id,
            app_id=app_id,
            locale=locale or "en-US",
        )
        if normalized_section == "marketing"
        else store_metadata_context(
            session,
            account_id=account_id,
            app_id=app_id,
            locale=locale or "en-US",
        )
    )
    account = context.get("account")
    app = context.get("app")
    latest_build = context.get("latest_build")
    if not isinstance(account, DeveloperAccount):
        raise AdminApiError("account_not_found", "开发者账号不存在", status_code=404)
    if not isinstance(app, App):
        raise AdminApiError("app_not_found", "当前开发者账号下没有这个 App", status_code=404)

    preflight = context.get("preflight")
    return StoreWorkspaceState(
        account=_account_summary_for_id(session, account.id),
        app=_account_app_item({"app": app, "latest_build": latest_build}, account_id=account.id),
        section=normalized_section,
        version=str(context.get("version") or _latest_build_label(latest_build)),
        locale=str(context.get("locale") or ""),
        source_locale=str(context.get("source_locale") or ""),
        supported_locales=[str(item) for item in context.get("supported_locales", [])],
        localized_metadata=[
            _store_locale_content(item)
            for item in context.get("localized_metadata", [])
            if isinstance(item, dict)
        ],
        connector=_connector_state(context.get("connector")),
        preflight_status=_preflight_status(preflight),
        preflight_label=_preflight_label(preflight),
        sync_runs=[
            _sync_run_summary(run)
            for run in context.get("sync_runs", [])
            if hasattr(run, "id") and hasattr(run, "started_at")
        ],
        marketing_pages=[
            _store_marketing_page_summary(
                item,
                account_id=account.id,
                app_id=app.id,
            )
            for item in context.get("marketing_pages", [])
        ],
    )


def _store_locale_content(item: dict[str, object]) -> StoreLocaleContent:
    store_images = item.get("store_images")
    return StoreLocaleContent(
        locale=str(item.get("locale") or ""),
        is_source=bool(item.get("is_source")),
        keywords=str(item.get("keywords") or ""),
        promotional_text=str(item.get("promotional_text") or ""),
        description=str(item.get("description") or ""),
        release_notes=str(item.get("release_notes") or ""),
        store_images=store_images if isinstance(store_images, dict) else {},
    )


def _workspace_rows_from_payload(
    locales: list[StoreLocaleContentInput],
    *,
    fallback_locale: str,
) -> list[dict[str, object]]:
    rows: list[dict[str, object]] = []
    for item in locales:
        locale = item.locale.strip()
        if not locale:
            continue
        rows.append(
            {
                "locale": locale,
                "keywords": "",
                "promotional_text": item.promotional_text.strip(),
                "description": item.description.strip(),
                "release_notes": item.release_notes.strip(),
                "store_images": _store_images_from_input(item.store_images),
            }
        )
    if rows:
        return rows
    locale = fallback_locale.strip() or DEFAULT_LOCALE
    return [
        {
            "locale": locale,
            "keywords": "",
            "promotional_text": "",
            "description": "",
            "release_notes": "",
            "store_images": {},
        }
    ]


def _source_locale_from_payload(
    locales: list[StoreLocaleContentInput],
    fallback_locale: str,
) -> str:
    if fallback_locale.strip():
        return fallback_locale.strip()
    if locales:
        return locales[0].locale.strip() or DEFAULT_LOCALE
    return DEFAULT_LOCALE


def _store_images_from_input(value: object) -> dict[str, object]:
    if not isinstance(value, dict):
        return {}
    images: dict[str, object] = {}
    for slot_key, raw_value in value.items():
        normalized_slot = str(slot_key or "").strip()
        if normalized_slot not in STORE_IMAGE_SLOT_KEYS:
            continue
        if not isinstance(raw_value, dict):
            images[normalized_slot] = raw_value
            continue
        if _truthy(raw_value.get("inherited")):
            images[normalized_slot] = {"urls": [], "assets": []}
            continue
        urls = _string_list(raw_value.get("urls"))
        if not urls:
            urls = _string_list(raw_value.get("value"))
        images[normalized_slot] = {
            "urls": urls,
            "assets": _own_store_image_assets(raw_value.get("assets")),
        }
    return images


def _own_store_image_assets(value: object) -> list[dict[str, object]]:
    return [item for item in _asset_list(value) if not _truthy(item.get("inherited"))]


def _truthy(value: object) -> bool:
    if isinstance(value, bool):
        return value
    return str(value or "").strip().lower() in {"1", "true", "yes", "on"}


async def _store_image_assets_from_form(
    form: object,
    *,
    storage: object,
    account_id: str,
    app_id: str,
    platform: str,
    version: str,
    content_set_id: str,
) -> dict[str, dict[str, list[dict[str, object]]]]:
    assets_by_locale: dict[str, dict[str, list[dict[str, object]]]] = {}
    if not hasattr(form, "multi_items"):
        return assets_by_locale

    for field_name, value in form.multi_items():
        if not isinstance(field_name, str) or not field_name.startswith("storeImageFiles__"):
            continue
        parts = field_name.split("__", 2)
        if len(parts) != 3:
            continue
        slot_key, locale = parts[1].strip(), parts[2].strip()
        if slot_key not in STORE_IMAGE_SLOT_KEYS or not locale:
            continue
        filename = str(getattr(value, "filename", "") or "").strip()
        if not filename or not hasattr(value, "read"):
            continue
        content = await value.read()
        if not content:
            continue
        content_type = str(getattr(value, "content_type", "") or "application/octet-stream")
        validation = validate_store_image(
            platform=platform,
            slot_key=slot_key,
            filename=filename,
            content_type=content_type,
            content=content,
        )
        if not validation.valid:
            raise ApiError(
                "store_image_invalid",
                f"{locale} {Path(filename).name}: {validation.message}",
                status_code=422,
            )
        stored = storage.save(
            _store_image_storage_folder(
                account_id=account_id,
                app_id=app_id,
                version=version,
                content_set_id=content_set_id,
                locale=locale,
                slot_key=slot_key,
            ),
            filename,
            content,
            content_type=content_type,
        )
        assets_by_locale.setdefault(locale, {}).setdefault(slot_key, []).append(
            {
                "fileName": Path(filename).name,
                "contentType": content_type,
                "sizeBytes": len(content),
                "storageKey": stored.storage_key,
                "downloadUrl": stored.download_url,
                "width": validation.image.width if validation.image else None,
                "height": validation.image.height if validation.image else None,
                "format": validation.image.format if validation.image else None,
                "validationMessage": validation.message,
                "matchedLabel": validation.matched_label,
            }
        )
    return assets_by_locale


def _append_metadata_store_image_assets(
    session: Session,
    *,
    account_id: str,
    app: App,
    uploaded_assets: dict[str, dict[str, list[dict[str, object]]]],
) -> None:
    for locale, slots in uploaded_assets.items():
        draft = session.scalar(
            select(StoreAppMetadataDraft).where(
                StoreAppMetadataDraft.developer_account_id == account_id,
                StoreAppMetadataDraft.app_id == app.id,
                StoreAppMetadataDraft.platform == app.platform,
                StoreAppMetadataDraft.version == CURRENT_METADATA_VERSION,
                StoreAppMetadataDraft.locale == locale,
                StoreAppMetadataDraft.content_set_id == DEFAULT_CONTENT_SET_ID,
            )
        )
        if draft is None:
            draft = StoreAppMetadataDraft(
                id=f"metadata-{uuid4().hex[:12]}",
                developer_account_id=account_id,
                app_id=app.id,
                platform=app.platform,
                version=CURRENT_METADATA_VERSION,
                locale=locale,
                content_set_id=DEFAULT_CONTENT_SET_ID,
                content_set_name=DEFAULT_CONTENT_SET_NAME,
                store_images_json={},
            )
            session.add(draft)
        draft.store_images_json = _merge_uploaded_store_image_assets(
            draft.store_images_json,
            slots,
        )
        draft.updated_at = datetime.now(UTC)


def _append_marketing_page_store_image_assets(
    session: Session,
    *,
    marketing_page_id: str,
    uploaded_assets: dict[str, dict[str, list[dict[str, object]]]],
) -> None:
    for locale, slots in uploaded_assets.items():
        row = session.scalar(
            select(StoreMarketingPageLocale).where(
                StoreMarketingPageLocale.marketing_page_id == marketing_page_id,
                StoreMarketingPageLocale.locale == locale,
            )
        )
        if row is None:
            row = StoreMarketingPageLocale(
                id=f"marketing-locale-{uuid4().hex[:12]}",
                marketing_page_id=marketing_page_id,
                locale=locale,
                promotional_text="",
                store_images_json={},
            )
            session.add(row)
        row.store_images_json = _merge_uploaded_store_image_assets(row.store_images_json, slots)
        row.updated_at = datetime.now(UTC)


def _merge_uploaded_store_image_assets(
    current_images: object,
    uploaded_slots: dict[str, list[dict[str, object]]],
) -> dict[str, object]:
    images = dict(current_images) if isinstance(current_images, dict) else {}
    for slot_key, assets in uploaded_slots.items():
        if slot_key not in STORE_IMAGE_SLOT_KEYS:
            continue
        slot = dict(images.get(slot_key)) if isinstance(images.get(slot_key), dict) else {}
        slot["assets"] = _dedupe_store_image_assets(
            [*_asset_list(slot.get("assets")), *assets],
        )
        slot["urls"] = _string_list(slot.get("urls"))
        images[slot_key] = slot
    return images


def _dedupe_store_image_assets(
    assets: list[dict[str, object]],
) -> list[dict[str, object]]:
    deduped: list[dict[str, object]] = []
    seen: set[str] = set()
    for asset in assets:
        storage_key = str(asset.get("storageKey") or "").strip()
        marker = storage_key or f"{asset.get('fileName')}:{asset.get('downloadUrl')}"
        if marker in seen:
            continue
        seen.add(marker)
        deduped.append(asset)
    return deduped


def _string_list(value: object) -> list[str]:
    if isinstance(value, list | tuple):
        raw_values = [str(item or "") for item in value]
    else:
        raw_values = str(value or "").splitlines()
    return [item.strip() for item in raw_values if item.strip()]


def _asset_list(value: object) -> list[dict[str, object]]:
    if not isinstance(value, list | tuple):
        return []
    return [item for item in value if isinstance(item, dict)]


def _uploaded_asset_count(
    uploaded_assets: dict[str, dict[str, list[dict[str, object]]]],
) -> int:
    return sum(len(assets) for slots in uploaded_assets.values() for assets in slots.values())


def _first_uploaded_locale(uploaded_assets: dict[str, object]) -> str:
    return next(iter(uploaded_assets.keys()), DEFAULT_LOCALE)


def _store_image_storage_folder(
    *,
    account_id: str,
    app_id: str,
    version: str,
    content_set_id: str,
    locale: str,
    slot_key: str,
) -> str:
    return "/".join(
        [
            "store-assets",
            _safe_storage_part(account_id),
            _safe_storage_part(app_id),
            _safe_storage_part(content_set_id),
            _safe_storage_part(version),
            _safe_storage_part(locale),
            _safe_storage_part(slot_key),
        ]
    )


def _safe_storage_part(value: str) -> str:
    normalized = re.sub(r"[^A-Za-z0-9._-]+", "-", value.strip())
    return normalized.strip("-") or "default"


def _scoped_app_or_error(session: Session, account_id: str, app_id: str) -> App:
    app = scoped_app(session, account_id, app_id)
    if app is None:
        raise ApiError("app_not_found", "当前开发者账号下没有这个 App", status_code=404)
    return app


def _preserve_readonly_keywords_for_rows(
    session: Session,
    *,
    account_id: str,
    app: App,
    rows: list[dict[str, object]],
) -> None:
    keywords_by_locale = {
        draft.locale: draft.keywords
        for draft in session.scalars(
            select(StoreAppMetadataDraft).where(
                StoreAppMetadataDraft.developer_account_id == account_id,
                StoreAppMetadataDraft.app_id == app.id,
                StoreAppMetadataDraft.platform == app.platform,
                StoreAppMetadataDraft.version == CURRENT_METADATA_VERSION,
                StoreAppMetadataDraft.content_set_id == DEFAULT_CONTENT_SET_ID,
            )
        )
    }
    for row in rows:
        locale = str(row.get("locale") or "").strip()
        row["keywords"] = keywords_by_locale.get(locale, "")


def _save_metadata_rows(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    rows: list[dict[str, object]],
) -> None:
    for row in rows:
        save_current_app_metadata_draft(
            session,
            account_id=account_id,
            app_id=app_id,
            locale=str(row["locale"]),
            keywords=str(row["keywords"]),
            promotional_text=str(row["promotional_text"]),
            description=str(row["description"]),
            store_images=_store_images_from_input(row.get("store_images")),
        )


def _save_release_note_rows(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    version: str,
    rows: list[dict[str, object]],
) -> None:
    for row in rows:
        save_release_note_draft(
            session,
            account_id=account_id,
            app_id=app_id,
            version=version,
            locale=str(row["locale"]),
            release_notes=str(row.get("release_notes") or ""),
        )


def _store_workspace_action_response(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    section: str,
    locale: str,
    message: str,
    sync_runs: list[object] | None = None,
) -> StoreWorkspaceActionResponse:
    return StoreWorkspaceActionResponse(
        message=message,
        state=_store_workspace_state(
            session,
            account_id=account_id,
            app_id=app_id,
            section=section,
            locale=locale,
        ),
        sync_runs=[_sync_run_summary(run) for run in sync_runs or []],
    )


def _marketing_page_detail_state(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    page_id: str,
    locale: str,
) -> MarketingPageDetailState:
    context = marketing_page_context(
        session,
        account_id=account_id,
        app_id=app_id,
        page_id=page_id,
        locale=locale or DEFAULT_LOCALE,
    )
    account = context.get("account")
    app = context.get("app")
    page = context.get("page")
    if not isinstance(account, DeveloperAccount):
        raise AdminApiError("account_not_found", "开发者账号不存在", status_code=404)
    if not isinstance(app, App):
        raise AdminApiError("app_not_found", "当前开发者账号下没有这个 App", status_code=404)
    if page is None or not hasattr(page, "page_id"):
        raise AdminApiError("marketing_page_not_found", "营销页面不存在", status_code=404)

    return MarketingPageDetailState(
        account=_account_summary_for_id(session, account.id),
        app=_account_app_item(
            {
                "app": app,
                "latest_build": context.get("latest_build"),
            },
            account_id=account.id,
        ),
        page=_store_marketing_page_summary(
            {
                "page": page,
                "type_label": _marketing_page_type_label(str(getattr(page, "page_type", ""))),
                "status_label": _marketing_page_status_label(str(getattr(page, "status", ""))),
                "apple_page_id_label": str(getattr(page, "apple_page_id", "") or "未同步后回填"),
                "language_count": len(context.get("localized_marketing_page", [])),
                "filled_text_count": sum(
                    1
                    for item in context.get("localized_marketing_page", [])
                    if isinstance(item, dict) and str(item.get("promotional_text") or "").strip()
                ),
                "asset_count": sum(
                    _store_image_asset_count(item.get("store_images"))
                    for item in context.get("localized_marketing_page", [])
                    if isinstance(item, dict)
                ),
            },
            account_id=account.id,
            app_id=app.id,
        ),
        locale=str(context.get("locale") or ""),
        source_locale=str(context.get("source_locale") or ""),
        supported_locales=[str(item) for item in context.get("supported_locales", [])],
        localized_page=[
            _marketing_locale_content(item)
            for item in context.get("localized_marketing_page", [])
            if isinstance(item, dict)
        ],
        connector=_connector_state(context.get("connector")),
        preflight_status=_preflight_status(context.get("preflight")),
        preflight_label=_preflight_label(context.get("preflight")),
        sync_runs=[
            _sync_run_summary(run)
            for run in context.get("sync_runs", [])
            if hasattr(run, "id") and hasattr(run, "started_at")
        ],
    )


def _marketing_locale_content(item: dict[str, object]) -> MarketingPageLocaleContent:
    store_images = item.get("store_images")
    return MarketingPageLocaleContent(
        locale=str(item.get("locale") or ""),
        is_source=bool(item.get("is_source")),
        promotional_text=str(item.get("promotional_text") or ""),
        store_images=store_images if isinstance(store_images, dict) else {},
    )


def _marketing_rows_from_payload(
    locales: list[MarketingPageLocaleInput],
    fallback_locale: str,
) -> list[dict[str, object]]:
    rows: list[dict[str, object]] = []
    for item in locales:
        locale = item.locale.strip()
        if not locale:
            continue
        rows.append(
            {
                "locale": locale,
                "promotional_text": item.promotional_text.strip(),
                "store_images": _store_images_from_input(item.store_images),
            }
        )
    if rows:
        return rows
    return [
        {
            "locale": fallback_locale.strip() or DEFAULT_LOCALE,
            "promotional_text": "",
            "store_images": {},
        }
    ]


def _source_locale_from_marketing_payload(
    locales: list[MarketingPageLocaleInput],
    fallback_locale: str,
) -> str:
    if fallback_locale.strip():
        return fallback_locale.strip()
    if locales:
        return locales[0].locale.strip() or DEFAULT_LOCALE
    return DEFAULT_LOCALE


def _save_marketing_page_from_payload(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    page_id: str,
    payload: MarketingPageSaveRequest,
) -> None:
    page = _marketing_page_or_error(session, account_id=account_id, app_id=app_id, page_id=page_id)
    save_marketing_page(
        session,
        account_id=account_id,
        app_id=app_id,
        page_id=page_id,
        page_name=payload.page_name or str(getattr(page, "page_name", "")),
        page_type=payload.page_type or str(getattr(page, "page_type", "")),
        keywords=str(getattr(page, "keywords", "") or ""),
        apple_page_id=str(getattr(page, "apple_page_id", "") or ""),
        deep_link_url=payload.deep_link_url or str(getattr(page, "deep_link_url", "") or ""),
        locale_rows=_marketing_rows_from_payload(payload.locales, payload.locale),
    )


def _marketing_page_action_response(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    page_id: str,
    locale: str,
    message: str,
    sync_runs: list[object] | None = None,
) -> MarketingPageActionResponse:
    return MarketingPageActionResponse(
        message=message,
        state=_marketing_page_detail_state(
            session,
            account_id=account_id,
            app_id=app_id,
            page_id=page_id,
            locale=locale,
        ),
        workspace=_store_workspace_state(
            session,
            account_id=account_id,
            app_id=app_id,
            section="marketing",
            locale=locale,
        ),
        sync_runs=[_sync_run_summary(run) for run in sync_runs or []],
    )


def _marketing_page_or_error(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    page_id: str,
) -> object:
    page = marketing_page_for_scope(
        session,
        account_id=account_id,
        app_id=app_id,
        page_id=page_id,
    )
    if page is None:
        raise ApiError("marketing_page_not_found", "营销页面不存在", status_code=404)
    return page


def _delete_marketing_page_image_from_draft(
    session: Session,
    *,
    storage: object,
    account_id: str,
    app_id: str,
    page_id: str,
    locale: str,
    slot_key: str,
    storage_key: str,
) -> None:
    page = _marketing_page_or_error(session, account_id=account_id, app_id=app_id, page_id=page_id)
    normalized_slot = slot_key.strip()
    normalized_storage_key = storage_key.strip()
    normalized_locale = locale.strip() or DEFAULT_LOCALE
    if normalized_slot not in {"feature_graphic_url", "phone_screenshots", "tablet_screenshots"}:
        raise ApiError("invalid_store_image_slot", "商店图类型不合法", status_code=422)
    if not normalized_storage_key:
        raise ApiError("invalid_store_image", "缺少要删除的商店图", status_code=422)
    locale_row = session.scalar(
        select(StoreMarketingPageLocale).where(
            StoreMarketingPageLocale.marketing_page_id == page.id,
            StoreMarketingPageLocale.locale == normalized_locale,
        )
    )
    if locale_row is None:
        raise ApiError("store_image_not_found", "这个语言下还没有营销页面截图", status_code=404)
    updated_images, removed = _remove_store_image_asset_from_json(
        locale_row.store_images_json,
        slot_key=normalized_slot,
        storage_key=normalized_storage_key,
    )
    if not removed:
        raise ApiError(
            "store_image_not_found",
            "这张营销页面截图已经不存在或已被删除",
            status_code=404,
        )
    locale_row.store_images_json = updated_images
    locale_row.updated_at = datetime.now(UTC)
    session.flush()
    if hasattr(storage, "delete"):
        try:
            if not _store_image_storage_key_is_referenced(session, normalized_storage_key):
                storage.delete(normalized_storage_key)
        except Exception:
            pass


def _store_image_asset_count(store_images: object) -> int:
    if not isinstance(store_images, dict):
        return 0
    total = 0
    for value in store_images.values():
        if isinstance(value, dict):
            assets = value.get("assets")
            if isinstance(assets, list):
                total += len(assets)
        elif isinstance(value, list):
            total += len(value)
    return total


def _marketing_page_type_label(page_type: str) -> str:
    return "产品页面优化" if page_type == "product_page_optimization" else "自定义产品页面"


def _marketing_page_status_label(status: str) -> str:
    return {
        "draft": "未同步",
        "synced": "已同步",
        "archived": "已归档",
    }.get(status, status or "未同步")


def _delete_store_image_from_current_draft(
    session: Session,
    *,
    storage: object,
    account_id: str,
    app: App,
    locale: str,
    slot_key: str,
    storage_key: str,
) -> None:
    normalized_slot = slot_key.strip()
    normalized_storage_key = storage_key.strip()
    normalized_locale = locale.strip() or DEFAULT_LOCALE
    if normalized_slot not in {"feature_graphic_url", "phone_screenshots", "tablet_screenshots"}:
        raise ApiError("invalid_store_image_slot", "商店图类型不合法", status_code=422)
    if not normalized_storage_key:
        raise ApiError("invalid_store_image", "缺少要删除的商店图", status_code=422)

    draft = session.scalar(
        select(StoreAppMetadataDraft).where(
            StoreAppMetadataDraft.developer_account_id == account_id,
            StoreAppMetadataDraft.app_id == app.id,
            StoreAppMetadataDraft.platform == app.platform,
            StoreAppMetadataDraft.version == CURRENT_METADATA_VERSION,
            StoreAppMetadataDraft.locale == normalized_locale,
            StoreAppMetadataDraft.content_set_id == DEFAULT_CONTENT_SET_ID,
        )
    )
    if draft is None:
        raise ApiError("store_image_not_found", "这个语言下还没有商店图", status_code=404)
    updated_images, removed = _remove_store_image_asset_from_json(
        draft.store_images_json,
        slot_key=normalized_slot,
        storage_key=normalized_storage_key,
    )
    if not removed:
        raise ApiError("store_image_not_found", "这张商店图已经不存在或已被删除", status_code=404)
    draft.store_images_json = updated_images
    draft.updated_at = datetime.now(UTC)
    session.flush()
    if hasattr(storage, "delete"):
        try:
            if not _store_image_storage_key_is_referenced(session, normalized_storage_key):
                storage.delete(normalized_storage_key)
        except Exception:
            pass


def _remove_store_image_asset_from_json(
    store_images: object,
    *,
    slot_key: str,
    storage_key: str,
) -> tuple[dict[str, object], bool]:
    if not isinstance(store_images, dict):
        return {}, False
    images = dict(store_images)
    slot = images.get(slot_key)
    if not isinstance(slot, dict):
        return images, False
    assets = slot.get("assets")
    if not isinstance(assets, list | tuple):
        return images, False
    kept_assets: list[dict[str, object]] = []
    removed = False
    for item in assets:
        if isinstance(item, dict) and str(item.get("storageKey") or "") == storage_key:
            removed = True
            continue
        if isinstance(item, dict):
            kept_assets.append(item)
    images[slot_key] = {**slot, "assets": kept_assets}
    return images, removed


def _store_image_storage_key_is_referenced(session: Session, storage_key: str) -> bool:
    for model in (StoreAppMetadataDraft, StoreImageSuiteLocale, StoreMarketingPageLocale):
        store_images_rows = session.scalars(select(model.store_images_json))
        for store_images in store_images_rows:
            if _store_images_reference_storage_key(store_images, storage_key):
                return True
    return False


def _store_images_reference_storage_key(store_images: object, storage_key: str) -> bool:
    if not isinstance(store_images, dict):
        return False
    for slot in store_images.values():
        if not isinstance(slot, dict):
            continue
        assets = slot.get("assets")
        if not isinstance(assets, list | tuple):
            continue
        for item in assets:
            if isinstance(item, dict) and str(item.get("storageKey") or "") == storage_key:
                return True
    return False


def _store_marketing_page_summary(
    item: object,
    *,
    account_id: str,
    app_id: str,
) -> StoreMarketingPageSummary:
    page = item.get("page") if isinstance(item, dict) else item
    if page is None or not hasattr(page, "page_id"):
        raise AdminApiError("invalid_marketing_page", "营销页面数据格式异常", status_code=500)
    page_id = str(getattr(page, "page_id", "") or "")
    return StoreMarketingPageSummary(
        id=str(getattr(page, "id", "") or page_id),
        page_id=page_id,
        page_name=str(getattr(page, "page_name", "") or "未命名营销页面"),
        page_type=str(getattr(page, "page_type", "") or ""),
        type_label=(
            str(item.get("type_label"))
            if isinstance(item, dict) and item.get("type_label")
            else "营销页面"
        ),
        status=str(getattr(page, "status", "") or "draft"),
        status_label=(
            str(item.get("status_label"))
            if isinstance(item, dict) and item.get("status_label")
            else str(getattr(page, "status", "") or "draft")
        ),
        apple_page_id_label=(
            str(item.get("apple_page_id_label"))
            if isinstance(item, dict) and item.get("apple_page_id_label")
            else str(getattr(page, "apple_page_id", "") or "未同步后回填")
        ),
        deep_link_url=str(getattr(page, "deep_link_url", "") or ""),
        language_count=int(item.get("language_count") or 0) if isinstance(item, dict) else 0,
        filled_text_count=(
            int(item.get("filled_text_count") or 0) if isinstance(item, dict) else 0
        ),
        asset_count=int(item.get("asset_count") or 0) if isinstance(item, dict) else 0,
        detail_path=(f"/admin/accounts/{account_id}/apps/{app_id}/marketing-pages/{page_id}"),
    )


def _preflight_status(preflight: object | None) -> str:
    if preflight is None:
        return "unchecked"
    return "ok" if bool(getattr(preflight, "can_sync", False)) else "needs"


def _preflight_label(preflight: object | None) -> str:
    if preflight is None:
        return "等待检查"
    return str(getattr(preflight, "message", "") or "已检查")


def _settings_state(session: Session, environment: Settings) -> SettingsState:
    effective = effective_business_settings(session, environment)
    database_keys = database_setting_keys(session)
    general_source = (
        "database"
        if "connector_base_url_template" in database_keys
        else "environment"
        if environment.connector_base_url_template
        else "default"
    )
    notification_keys = {
        "dingtalk_enabled",
        "dingtalk_webhook_url",
        "dingtalk_secret",
        "dingtalk_timeout_seconds",
        "dingtalk_dispatch_interval_seconds",
    }
    notification_source = (
        "database"
        if database_keys.intersection(notification_keys)
        else "environment"
        if environment.dingtalk_configured
        else "default"
    )
    return SettingsState(
        general=GeneralSettingsState(
            connector_base_url_template=effective.connector_base_url_template or "",
            source=general_source,
        ),
        notifications=NotificationSettingsState(
            enabled=effective.dingtalk_enabled,
            configured=effective.dingtalk_configured,
            webhook_configured=bool(effective.dingtalk_webhook_url),
            secret_configured=bool(effective.dingtalk_secret),
            timeout_seconds=effective.dingtalk_timeout_seconds,
            dispatch_interval_seconds=effective.dingtalk_dispatch_interval_seconds,
            pending_delivery_count=_webhook_delivery_count(session, status="pending"),
            dead_delivery_count=_webhook_delivery_count(session, status="dead"),
            source=notification_source,
        ),
        runtime=_runtime_environment_items(environment),
    )


def _runtime_environment_items(environment: Settings) -> list[RuntimeEnvironmentItem]:
    items: list[RuntimeEnvironmentItem] = []
    for key, label, group, attribute, sensitive in RUNTIME_ENVIRONMENT_DESCRIPTORS:
        value = getattr(environment, attribute)
        configured = _runtime_value_configured(value)
        items.append(
            RuntimeEnvironmentItem(
                key=key,
                label=label,
                group=group,
                source="environment" if os.getenv(key) is not None else "default",
                value_label=(
                    ("已配置" if configured else "未配置")
                    if sensitive
                    else _runtime_value_label(value)
                ),
                configured=configured,
                sensitive=sensitive,
                restart_required=True,
            )
        )
    return items


def _runtime_value_configured(value: object) -> bool:
    if value is None:
        return False
    if isinstance(value, str):
        return bool(value.strip())
    if isinstance(value, tuple | list):
        return bool(value)
    return True


def _runtime_value_label(value: object) -> str:
    if value is None:
        return "未配置"
    if isinstance(value, tuple | list):
        return ", ".join(str(item) for item in value) or "未配置"
    text = str(value).strip()
    return text or "未配置"


def _store_reviews_state(
    session: Session,
    *,
    account_id: str = "",
    app_id: str = "",
    rating: int | None = None,
) -> StoreReviewsState:
    context = store_reviews_context(session, account_id=account_id, app_id=app_id, rating=rating)
    selected_account = context["selected_review_account"]
    selected_app = context["selected_review_app"]
    return StoreReviewsState(
        apps=[
            _review_app_item(
                item,
                selected_account=selected_account,
                selected_app=selected_app,
            )
            for item in context["review_apps"]
        ],
        selected_account_id=(
            selected_account.id if isinstance(selected_account, DeveloperAccount) else None
        ),
        selected_app_id=selected_app.id if isinstance(selected_app, App) else None,
        rating=rating,
        stats=ReviewStats(**context["review_stats"]),
        reviews=[_review_item(review) for review in context["reviews"]],
        latest_fetch=_fetch_run_item(context["latest_review_fetch"]),
        latest_analysis=_analysis_run_item(context["latest_review_analysis"]),
        analysis_issues=[_analysis_issue_item(issue) for issue in context["analysis_issues"]],
        analysis_boundaries=list(context["analysis_boundaries"]),
    )


def _review_app_item(
    item: dict[str, object],
    *,
    selected_account: object,
    selected_app: object,
) -> ReviewAppItem:
    app = item["app"]
    account = item["account"]
    if not isinstance(app, App) or not isinstance(account, DeveloperAccount):
        raise AdminApiError("invalid_review_app", "评论应用数据格式异常", status_code=500)
    return ReviewAppItem(
        account_id=account.id,
        app_id=app.id,
        app_name=app.name,
        bundle_identifier=app.bundle_identifier,
        platform=app.platform,
        account_name=account.team_name,
        icon_color=app.icon_color,
        review_count=int(item.get("review_count") or 0),
        selected=(
            isinstance(selected_account, DeveloperAccount)
            and isinstance(selected_app, App)
            and selected_account.id == account.id
            and selected_app.id == app.id
        ),
    )


def _review_item(review: StoreReview) -> ReviewItem:
    return ReviewItem(
        id=review.id,
        store_review_id=review.store_review_id,
        rating=review.rating,
        title=review.title or "",
        body=review.body or "",
        author_name=review.author_name or "",
        locale=review.locale or "",
        territory=review.territory or "",
        app_version=review.app_version or "",
        created_at=_iso_datetime(review.created_at),
    )


def _fetch_run_item(run: StoreReviewFetchRun | None) -> ReviewFetchRunItem | None:
    if run is None:
        return None
    return ReviewFetchRunItem(
        id=run.id,
        status=run.status,
        page_count=run.page_count,
        fetched_count=run.fetched_count,
        inserted_count=run.inserted_count,
        duplicate_count=run.duplicate_count,
        stopped_reason=run.stopped_reason,
        finished_at=_optional_iso_datetime(run.finished_at),
        error_summary=run.error_summary or "",
    )


def _analysis_run_item(run: StoreReviewAnalysisRun | None) -> ReviewAnalysisRunItem | None:
    if run is None:
        return None
    return ReviewAnalysisRunItem(
        id=run.id,
        status=run.status,
        review_count=run.review_count,
        low_rating_count=run.low_rating_count,
        issue_count=run.issue_count,
        summary=run.summary or "",
        finished_at=_optional_iso_datetime(run.finished_at),
        error_summary=run.error_summary or "",
    )


def _analysis_issue_item(issue: dict[str, object]) -> ReviewAnalysisIssue:
    representative = issue.get("representativeReviewIds") or issue.get("representative_review_ids")
    return ReviewAnalysisIssue(
        title=str(issue.get("title") or "未命名关注点"),
        severity=str(issue.get("severity") or "medium"),
        count=_optional_int(issue.get("count")),
        focus=str(issue.get("focus") or "需要人工确认影响范围。"),
        evidence=_string_list(issue.get("evidence")),
        suggestion=str(issue.get("suggestion") or "请结合代表评论和版本信息进一步确认。"),
        representative_review_ids=_string_list(representative),
    )


def _iso_datetime(value: datetime) -> str:
    return value.isoformat()


def _optional_iso_datetime(value: datetime | None) -> str | None:
    return value.isoformat() if value is not None else None


def _optional_int(value: Any) -> int | None:
    try:
        return int(value)
    except (TypeError, ValueError):
        return None


def _string_list(value: Any) -> list[str]:
    if isinstance(value, list):
        return [str(item).strip() for item in value if str(item).strip()]
    text = str(value or "").strip()
    return [text] if text else []


def _runner_build_payload(build: Build) -> RunnerBuildPayload:
    runner_data = dict(build.runner_labels_json or {})
    credential_refs = runner_data.get("credentialRefs")
    return RunnerBuildPayload(
        id=build.id,
        app_id=build.app_id,
        platform=build.platform,
        environment=build.environment,
        git_url=build.git_url or "",
        git_ref=build.git_ref or "",
        artifact_type=str(runner_data.get("artifactType") or ""),
        credential_refs=credential_refs if isinstance(credential_refs, dict) else {},
    )


def _build_runner_item(runner: BuildRunner, *, release_root: Path) -> BuildRunnerItem:
    capabilities = build_platform.normalize_runner_capabilities(
        dict(runner.capabilities_json or {})
    )
    latest_version, update_status, update_status_label = runner_releases.runner_release_status(
        release_root,
        platform=str(capabilities.get("hostPlatform") or "darwin"),
        arch=str(capabilities.get("arch") or ""),
        runner_version=runner.version,
        package_agent_version=runner.package_agent_version,
    )
    return BuildRunnerItem(
        id=runner.id,
        name=runner.name,
        status=runner.status,
        labels=[str(item).strip() for item in runner.labels_json or [] if str(item).strip()],
        version=runner.version,
        package_agent_version=runner.package_agent_version,
        last_seen_at_label=format_datetime(runner.last_seen_at),
        current_build_id=runner.current_build_id,
        capabilities=capabilities,
        latest_version=latest_version,
        update_status=update_status,
        update_status_label=update_status_label,
    )


def _admin_api_error(error: ApiError) -> AdminApiError:
    return AdminApiError(
        error.code,
        error.message,
        status_code=error.status_code,
        detail={"retryable": error.retryable, **error.extra},
    )

from __future__ import annotations

import hashlib
import json
from dataclasses import dataclass
from datetime import UTC, datetime, timedelta
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.parse import urlencode
from urllib.request import Request, urlopen
from uuid import uuid4

from sqlalchemy import select
from sqlalchemy.orm import Session

from testflying_api.active_connector import ActiveConnectorTimeoutError, active_connector_hub
from testflying_api.errors import ApiError
from testflying_api.schema import (
    App,
    AuditLog,
    Build,
    DeveloperAccount,
    DeveloperAccountApp,
    StoreAppMetadataDraft,
    StoreConnector,
    StoreMarketingPage,
    StoreMarketingPageLocale,
    StorePreflightCheck,
    StoreReleaseNoteDraft,
    StoreSyncRun,
)

PREFLIGHT_TTL = timedelta(minutes=5)
PREFLIGHT_FORCE_REFRESH_COOLDOWN = timedelta(minutes=1)
UPDATE_RELEASE_NOTES = "update_release_notes"
UPDATE_APP_METADATA = "update_app_metadata"
UPDATE_MARKETING_PAGE = "update_marketing_page"
DEFAULT_LOCALE = "zh-Hans"
DEFAULT_CONTENT_SET_ID = "default"
DEFAULT_CONTENT_SET_NAME = "默认上架内容"
CURRENT_METADATA_VERSION = "__current__"
APP_STORE_DESCRIPTION_MAX_LENGTH = 4000
APP_STORE_DESCRIPTION_MIN_LENGTH = 10
APP_STORE_KEYWORDS_MAX_LENGTH = 100
APP_STORE_PROMOTIONAL_TEXT_MAX_LENGTH = 170
GOOGLE_PLAY_FULL_DESCRIPTION_MAX_LENGTH = 4000
GOOGLE_PLAY_FULL_DESCRIPTION_MIN_LENGTH = 10
GOOGLE_PLAY_SHORT_DESCRIPTION_MAX_LENGTH = 80
APP_METADATA_SYNC_SCOPE = "metadata"
APP_METADATA_DESCRIPTION_SCOPE = "description"
APP_METADATA_PROMOTIONAL_TEXT_SCOPE = "promotional_text"
APP_METADATA_SHORT_DESCRIPTION_SCOPE = "short_description"
APP_METADATA_KEYWORDS_SCOPE = "keywords"
APP_METADATA_STORE_IMAGES_SCOPE = "store_images"
APP_METADATA_TEXT_SYNC_SCOPES = {
    APP_METADATA_SYNC_SCOPE,
    APP_METADATA_DESCRIPTION_SCOPE,
    APP_METADATA_PROMOTIONAL_TEXT_SCOPE,
    APP_METADATA_SHORT_DESCRIPTION_SCOPE,
    APP_METADATA_KEYWORDS_SCOPE,
}
APP_METADATA_SYNC_SCOPES = [
    APP_METADATA_SYNC_SCOPE,
    APP_METADATA_DESCRIPTION_SCOPE,
    APP_METADATA_PROMOTIONAL_TEXT_SCOPE,
    APP_METADATA_SHORT_DESCRIPTION_SCOPE,
    APP_METADATA_KEYWORDS_SCOPE,
    APP_METADATA_STORE_IMAGES_SCOPE,
]
MARKETING_SYNC_TEXT_SCOPE = "marketing_text"
MARKETING_SYNC_IMAGE_SCOPE = "store_images"


@dataclass(frozen=True)
class PreflightState:
    can_sync: bool
    reason_code: str | None
    message: str
    store_state: dict[str, object]
    checked_at: datetime
    expires_at: datetime
    cached: bool
    throttled: bool = False

    @property
    def remaining_seconds(self) -> int:
        return max(int((_as_utc(self.expires_at) - datetime.now(UTC)).total_seconds()), 0)

    @property
    def source_label(self) -> str:
        if self.throttled:
            return "1 分钟节流"
        return "缓存" if self.cached else "实时检查"


@dataclass(frozen=True)
class ConnectorHealthResult:
    connector: StoreConnector
    ok: bool
    message: str


class StoreConnectorClient:
    def health(self, connector: StoreConnector) -> dict[str, object]:
        if connector.base_url.startswith("mock://"):
            return {
                "status": "ok",
                "developerAccountId": connector.developer_account_id,
            }
        if _is_active_connector(connector):
            return _active_request_json(connector, "GET", "/health", include_auth=False, timeout=3)
        return _get_json(connector, "/health", include_auth=False, timeout=3)

    def supported_locales(
        self,
        connector: StoreConnector,
        *,
        account_id: str,
        app: App,
        version: str,
        store_app_id: str | None = None,
        package_name: str | None = None,
    ) -> list[str]:
        if connector.base_url.startswith("mock://"):
            return _mock_supported_locales(app.platform)
        target = _app_payload(app, store_app_id=store_app_id, package_name=package_name)
        query = urlencode(
            {
                "developerAccountId": account_id,
                "platform": app.platform,
                "version": version,
                "storeAppId": target.get("storeAppId") or "",
                "packageName": target.get("packageName") or "",
            }
        )
        path = f"/v1/apps/{app.id}/supported-locales?{query}"
        response = (
            _active_request_json(connector, "GET", path)
            if _is_active_connector(connector)
            else _get_json(connector, path)
        )
        return _normalize_connector_locales(response.get("locales"))

    def store_listings(
        self,
        connector: StoreConnector,
        *,
        account_id: str,
        app: App,
        version: str = "",
        store_app_id: str | None = None,
        package_name: str | None = None,
    ) -> dict[str, object]:
        if connector.base_url.startswith("mock://"):
            return _mock_store_listings(app)
        target = _app_payload(app, store_app_id=store_app_id, package_name=package_name)
        query = urlencode(
            {
                "developerAccountId": account_id,
                "platform": app.platform,
                "version": version,
                "storeAppId": target.get("storeAppId") or "",
                "packageName": target.get("packageName") or "",
            }
        )
        path = f"/v1/apps/{app.id}/store-listings?{query}"
        return (
            _active_request_json(connector, "GET", path)
            if _is_active_connector(connector)
            else _get_json(connector, path)
        )

    def store_images(
        self,
        connector: StoreConnector,
        *,
        account_id: str,
        app: App,
        version: str = "",
        store_app_id: str | None = None,
        package_name: str | None = None,
    ) -> dict[str, object]:
        if connector.base_url.startswith("mock://"):
            return _mock_store_images(app)
        target = _app_payload(app, store_app_id=store_app_id, package_name=package_name)
        query = urlencode(
            {
                "developerAccountId": account_id,
                "platform": app.platform,
                "version": version,
                "storeAppId": target.get("storeAppId") or "",
                "packageName": target.get("packageName") or "",
            }
        )
        path = f"/v1/apps/{app.id}/store-images?{query}"
        return (
            _active_request_json(connector, "GET", path)
            if _is_active_connector(connector)
            else _get_json(connector, path)
        )

    def store_releases(
        self,
        connector: StoreConnector,
        *,
        account_id: str,
        app: App,
        version: str = "",
        store_app_id: str | None = None,
        package_name: str | None = None,
    ) -> dict[str, object]:
        if connector.base_url.startswith("mock://"):
            return _mock_store_releases(app)
        target = _app_payload(app, store_app_id=store_app_id, package_name=package_name)
        query = urlencode(
            {
                "developerAccountId": account_id,
                "platform": app.platform,
                "version": version,
                "storeAppId": target.get("storeAppId") or "",
                "packageName": target.get("packageName") or "",
            }
        )
        path = f"/v1/apps/{app.id}/store-releases?{query}"
        return (
            _active_request_json(connector, "GET", path)
            if _is_active_connector(connector)
            else _get_json(connector, path)
        )

    def store_reviews(
        self,
        connector: StoreConnector,
        *,
        account_id: str,
        app: App,
        store_app_id: str | None = None,
        package_name: str | None = None,
        store_query: dict[str, str] | None = None,
    ) -> dict[str, object]:
        if connector.base_url.startswith("mock://"):
            return _mock_store_reviews(app)
        target = _app_payload(app, store_app_id=store_app_id, package_name=package_name)
        query_values = {
            "developerAccountId": account_id,
            "platform": app.platform,
            "storeAppId": target.get("storeAppId") or "",
            "packageName": target.get("packageName") or "",
            **dict(store_query or {}),
        }
        query = urlencode(query_values)
        path = f"/v1/apps/{app.id}/store-reviews?{query}"
        return (
            _active_request_json(connector, "GET", path)
            if _is_active_connector(connector)
            else _get_json(connector, path)
        )

    def product_page_optimizations(
        self,
        connector: StoreConnector,
        *,
        account_id: str,
        app: App,
        store_app_id: str | None = None,
    ) -> dict[str, object]:
        if connector.base_url.startswith("mock://"):
            return _mock_product_page_optimizations(app)
        target = _app_payload(app, store_app_id=store_app_id)
        query = urlencode(
            {
                "developerAccountId": account_id,
                "platform": app.platform,
                "storeAppId": target.get("storeAppId") or "",
            }
        )
        path = f"/v1/apps/{app.id}/product-page-optimizations?{query}"
        return (
            _active_request_json(connector, "GET", path)
            if _is_active_connector(connector)
            else _get_json(connector, path)
        )

    def create_product_page_optimization(
        self,
        connector: StoreConnector,
        *,
        account_id: str,
        app: App,
        name: str,
        traffic_proportion: int,
        locales: list[str],
        treatments: list[dict[str, object]],
        store_app_id: str | None = None,
    ) -> dict[str, object]:
        payload: dict[str, object] = {
            "developerAccountId": account_id,
            "platform": app.platform,
            "name": name,
            "trafficProportion": traffic_proportion,
            "locales": locales,
            "treatments": treatments,
            "app": _app_payload(app, store_app_id=store_app_id),
        }
        if connector.base_url.startswith("mock://"):
            return _mock_create_product_page_optimization(app, payload)
        path = f"/v1/apps/{app.id}/product-page-optimizations"
        return (
            _active_request_json(connector, "POST", path, payload)
            if _is_active_connector(connector)
            else _post_json(connector, path, payload)
        )

    def preflight(
        self,
        connector: StoreConnector,
        payload: dict[str, object],
    ) -> dict[str, object]:
        if connector.base_url.startswith("mock://"):
            return _mock_preflight(payload)
        if _is_active_connector(connector):
            return _active_request_json(connector, "POST", "/v1/preflight", payload)
        return _post_json(connector, "/v1/preflight", payload)

    def sync_release_notes(
        self,
        connector: StoreConnector,
        payload: dict[str, object],
    ) -> dict[str, object]:
        return self.sync_store_operation(connector, payload)

    def sync_store_operation(
        self,
        connector: StoreConnector,
        payload: dict[str, object],
    ) -> dict[str, object]:
        if connector.base_url.startswith("mock://"):
            operation = str(payload.get("operation") or UPDATE_RELEASE_NOTES)
            return {
                "status": "succeeded",
                "message": f"{_operation_label(operation)}已同步。",
            }
        if _is_active_connector(connector):
            return _active_request_json(connector, "POST", "/v1/sync-runs", payload)
        return _post_json(connector, "/v1/sync-runs", payload)


def account_apps(session: Session, account_id: str) -> list[App]:
    direct_apps = list(
        session.scalars(
            select(App)
            .where(App.developer_account_id == account_id)
            .order_by(App.added_at.desc(), App.name.asc())
        )
    )
    legacy_app_ids = list(
        session.scalars(
            select(DeveloperAccountApp.app_id).where(
                DeveloperAccountApp.developer_account_id == account_id
            )
        )
    )
    legacy_apps = (
        list(
            session.scalars(
                select(App)
                .where(App.id.in_(legacy_app_ids), App.developer_account_id.is_(None))
                .order_by(App.added_at.desc(), App.name.asc())
            )
        )
        if legacy_app_ids
        else []
    )
    apps_by_id = {app.id: app for app in [*direct_apps, *legacy_apps]}
    return sorted(apps_by_id.values(), key=lambda app: (app.added_at, app.name), reverse=True)


def account_connector(session: Session, account_id: str) -> StoreConnector | None:
    return session.scalar(
        select(StoreConnector)
        .where(StoreConnector.developer_account_id == account_id)
        .order_by(StoreConnector.created_at.asc())
        .limit(1)
    )


def save_connector(
    session: Session,
    *,
    account_id: str,
    name: str,
    base_url: str,
    auth_token: str,
    base_url_template: str | None = None,
) -> StoreConnector:
    account_or_404(session, account_id)
    normalized_name = name.strip() or "Store Connector"
    normalized_base_url = resolve_connector_base_url(
        account_id=account_id,
        base_url=base_url,
        base_url_template=base_url_template,
    )
    normalized_token = auth_token.strip()
    if not normalized_base_url:
        raise ApiError("invalid_connector", "connector 地址不能为空", status_code=422)
    connector = account_connector(session, account_id)
    if connector is None:
        if not normalized_token:
            raise ApiError(
                "invalid_connector",
                "首次配置 connector 时必须填写调用 token",
                status_code=422,
            )
        connector = StoreConnector(
            id=f"connector-{uuid4().hex[:12]}",
            developer_account_id=account_id,
            name=normalized_name,
            base_url=normalized_base_url,
            auth_token=normalized_token,
            status="unknown",
        )
        session.add(connector)
    else:
        connector.name = normalized_name
        connector.base_url = normalized_base_url
        if normalized_token:
            connector.auth_token = normalized_token
        connector.status = connector.status or "unknown"
    session.flush()
    return connector


def check_connector_health(
    session: Session,
    *,
    account_id: str,
    client: StoreConnectorClient | None = None,
) -> ConnectorHealthResult:
    account_or_404(session, account_id)
    connector = account_connector(session, account_id)
    if connector is None:
        raise ApiError("connector_missing", "当前开发者账号还没有配置 connector", status_code=422)
    connector_client = client or StoreConnectorClient()
    checked_at = datetime.now(UTC)
    try:
        response = connector_client.health(connector)
        status = str(response.get("status") or "")
        response_account_id = str(response.get("developerAccountId") or account_id)
        if status != "ok":
            raise ConnectorCallError(f"connector 健康状态异常: {status or 'unknown'}")
        if response_account_id != account_id:
            raise ConnectorCallError("connector 返回的开发者账号和当前账号不一致")
    except ConnectorCallError as error:
        connector.status = "error"
        connector.last_checked_at = checked_at
        session.flush()
        return ConnectorHealthResult(connector=connector, ok=False, message=error.message)
    connector.status = "ok"
    connector.last_checked_at = checked_at
    session.flush()
    return ConnectorHealthResult(connector=connector, ok=True, message="Connector 连接正常")


def resolve_connector_base_url(
    *,
    account_id: str,
    base_url: str = "",
    base_url_template: str | None = None,
) -> str:
    normalized_base_url = base_url.strip().rstrip("/")
    if normalized_base_url:
        return normalized_base_url
    if base_url_template is None or not base_url_template.strip():
        return ""
    try:
        return base_url_template.strip().format(account_id=account_id).rstrip("/")
    except (IndexError, KeyError, ValueError) as exc:
        raise ApiError(
            "invalid_connector_template",
            "connector 地址模板无效，只支持 {account_id} 占位符",
            status_code=422,
        ) from exc


def latest_build_for_app(session: Session, app_id: str) -> Build | None:
    return session.scalar(
        select(Build).where(Build.app_id == app_id).order_by(Build.uploaded_at.desc()).limit(1)
    )


def draft_for_scope(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    platform: str,
    version: str,
    locale: str,
) -> StoreReleaseNoteDraft | None:
    return session.scalar(
        select(StoreReleaseNoteDraft).where(
            StoreReleaseNoteDraft.developer_account_id == account_id,
            StoreReleaseNoteDraft.app_id == app_id,
            StoreReleaseNoteDraft.platform == platform,
            StoreReleaseNoteDraft.version == version,
            StoreReleaseNoteDraft.locale == locale,
        )
    )


def metadata_draft_for_scope(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    platform: str,
    version: str,
    locale: str,
    content_set_id: str = DEFAULT_CONTENT_SET_ID,
) -> StoreAppMetadataDraft | None:
    return session.scalar(
        select(StoreAppMetadataDraft).where(
            StoreAppMetadataDraft.developer_account_id == account_id,
            StoreAppMetadataDraft.app_id == app_id,
            StoreAppMetadataDraft.platform == platform,
            StoreAppMetadataDraft.version == version,
            StoreAppMetadataDraft.locale == locale,
            StoreAppMetadataDraft.content_set_id == _normalize_content_set_id(content_set_id),
        )
    )


def metadata_drafts_for_scope(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    platform: str,
    version: str,
    content_set_id: str = DEFAULT_CONTENT_SET_ID,
) -> dict[str, StoreAppMetadataDraft]:
    drafts = session.scalars(
        select(StoreAppMetadataDraft).where(
            StoreAppMetadataDraft.developer_account_id == account_id,
            StoreAppMetadataDraft.app_id == app_id,
            StoreAppMetadataDraft.platform == platform,
            StoreAppMetadataDraft.version == version,
            StoreAppMetadataDraft.content_set_id == _normalize_content_set_id(content_set_id),
        )
    )
    return {draft.locale: draft for draft in drafts}


def current_metadata_drafts_for_app(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    platform: str,
) -> dict[str, StoreAppMetadataDraft]:
    return metadata_drafts_for_scope(
        session,
        account_id=account_id,
        app_id=app_id,
        platform=platform,
        version=CURRENT_METADATA_VERSION,
        content_set_id=DEFAULT_CONTENT_SET_ID,
    )


def metadata_content_sets_for_scope(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    platform: str,
    version: str,
) -> list[dict[str, str]]:
    rows = session.execute(
        select(
            StoreAppMetadataDraft.content_set_id,
            StoreAppMetadataDraft.content_set_name,
        )
        .where(
            StoreAppMetadataDraft.developer_account_id == account_id,
            StoreAppMetadataDraft.app_id == app_id,
            StoreAppMetadataDraft.platform == platform,
            StoreAppMetadataDraft.version == version,
        )
        .order_by(StoreAppMetadataDraft.content_set_name.asc())
    )
    sets = [
        {"id": DEFAULT_CONTENT_SET_ID, "name": DEFAULT_CONTENT_SET_NAME},
    ]
    seen = {DEFAULT_CONTENT_SET_ID}
    for content_set_id, content_set_name in rows:
        normalized_id = _normalize_content_set_id(content_set_id)
        if normalized_id in seen:
            continue
        sets.append(
            {
                "id": normalized_id,
                "name": _normalize_content_set_name(content_set_name, normalized_id),
            }
        )
        seen.add(normalized_id)
    return sets


def save_release_note_draft(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    version: str,
    locale: str,
    release_notes: str,
) -> StoreReleaseNoteDraft:
    app = scoped_app(session, account_id, app_id)
    if app is None:
        raise ApiError("app_not_found", "当前开发者账号下没有这个 App", status_code=404)
    normalized_notes = release_notes.strip()
    draft = draft_for_scope(
        session,
        account_id=account_id,
        app_id=app.id,
        platform=app.platform,
        version=version,
        locale=locale,
    )
    if draft is None:
        draft = StoreReleaseNoteDraft(
            id=f"draft-{uuid4().hex[:12]}",
            developer_account_id=account_id,
            app_id=app.id,
            platform=app.platform,
            version=version,
            locale=locale,
            release_notes=normalized_notes,
        )
        session.add(draft)
    else:
        draft.release_notes = normalized_notes
        draft.updated_at = datetime.now(UTC)
    session.flush()
    return draft


def supported_locales_for_app(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    version: str,
    fallback_locale: str = DEFAULT_LOCALE,
    fallback_locales: list[str] | None = None,
    client: StoreConnectorClient | None = None,
) -> list[str]:
    app = scoped_app(session, account_id, app_id)
    connector = account_connector(session, account_id)
    fallback = _normalize_locales(
        fallback_locales if fallback_locales is not None else [fallback_locale]
    )
    if app is None or connector is None or not version:
        return fallback
    connector_client = client or StoreConnectorClient()
    try:
        locales = connector_client.supported_locales(
            connector,
            account_id=account_id,
            app=app,
            version=version,
        )
    except ConnectorCallError:
        return fallback
    return _normalize_locales(locales)


def save_app_metadata_draft(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    version: str,
    locale: str,
    content_set_id: str = DEFAULT_CONTENT_SET_ID,
    content_set_name: str = DEFAULT_CONTENT_SET_NAME,
    keywords: str,
    promotional_text: str,
    description: str,
    store_images: dict[str, object] | None = None,
) -> StoreAppMetadataDraft:
    app = scoped_app(session, account_id, app_id)
    if app is None:
        raise ApiError("app_not_found", "当前开发者账号下没有这个 App", status_code=404)
    normalized_description = description.strip()
    if not normalized_description:
        raise ApiError("invalid_metadata", "应用描述不能为空", status_code=422)
    normalized_content_set_id = _normalize_content_set_id(content_set_id)
    normalized_content_set_name = _normalize_content_set_name(
        content_set_name,
        normalized_content_set_id,
    )

    draft = metadata_draft_for_scope(
        session,
        account_id=account_id,
        app_id=app.id,
        platform=app.platform,
        version=version,
        locale=locale,
        content_set_id=normalized_content_set_id,
    )
    values = {
        "content_set_id": normalized_content_set_id,
        "content_set_name": normalized_content_set_name,
        "title": app.name,
        "subtitle": "",
        "keywords": keywords.strip(),
        "promotional_text": promotional_text.strip(),
        "description": normalized_description,
        "privacy_policy_url": "",
        "support_url": "",
        "marketing_url": "",
        "store_images_json": _normalize_store_images(store_images),
    }
    if draft is None:
        draft = StoreAppMetadataDraft(
            id=f"metadata-{uuid4().hex[:12]}",
            developer_account_id=account_id,
            app_id=app.id,
            platform=app.platform,
            version=version,
            locale=locale,
            **values,
        )
        session.add(draft)
    else:
        for key, value in values.items():
            setattr(draft, key, value)
        draft.updated_at = datetime.now(UTC)
    session.flush()
    return draft


def save_current_app_metadata_draft(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    locale: str,
    keywords: str,
    promotional_text: str,
    description: str,
    store_images: dict[str, object] | None = None,
) -> StoreAppMetadataDraft:
    return save_app_metadata_draft(
        session,
        account_id=account_id,
        app_id=app_id,
        version=CURRENT_METADATA_VERSION,
        locale=locale,
        content_set_id=DEFAULT_CONTENT_SET_ID,
        content_set_name=DEFAULT_CONTENT_SET_NAME,
        keywords=keywords,
        promotional_text=promotional_text,
        description=description,
        store_images=store_images,
    )


def marketing_page_for_scope(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    page_id: str,
) -> StoreMarketingPage | None:
    app = scoped_app(session, account_id, app_id)
    if app is None:
        return None
    return session.scalar(
        select(StoreMarketingPage).where(
            StoreMarketingPage.developer_account_id == account_id,
            StoreMarketingPage.app_id == app.id,
            StoreMarketingPage.platform == app.platform,
            StoreMarketingPage.page_id == page_id,
        )
    )


def marketing_page_locales(
    session: Session,
    marketing_page_id: str,
) -> dict[str, StoreMarketingPageLocale]:
    rows = session.scalars(
        select(StoreMarketingPageLocale).where(
            StoreMarketingPageLocale.marketing_page_id == marketing_page_id
        )
    )
    return {row.locale: row for row in rows}


def create_marketing_page(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    page_id: str,
    page_name: str,
    page_type: str,
    locale_rows: list[dict[str, object]],
    deep_link_url: str = "",
) -> StoreMarketingPage:
    app = scoped_app(session, account_id, app_id)
    if app is None:
        raise ApiError("app_not_found", "当前开发者账号下没有这个 App", status_code=404)
    if app.platform != "ios":
        raise ApiError(
            "unsupported_marketing_page",
            "营销页面当前仅支持 App Store Connect",
            status_code=422,
        )
    now = datetime.now(UTC)
    page = StoreMarketingPage(
        id=f"marketing-page-{uuid4().hex[:12]}",
        developer_account_id=account_id,
        app_id=app.id,
        platform=app.platform,
        page_id=page_id.strip() or f"page-{uuid4().hex[:8]}",
        page_name=page_name.strip() or "新的自定义产品页面",
        page_type=_normalize_marketing_page_type(page_type),
        status="draft",
        store_images_json={},
        created_at=now,
        updated_at=now,
    )
    session.add(page)
    session.flush()
    return save_marketing_page(
        session,
        account_id=account_id,
        app_id=app.id,
        page_id=page.page_id,
        page_name=page.page_name,
        page_type=page.page_type,
        keywords="",
        apple_page_id="",
        deep_link_url=deep_link_url,
        locale_rows=locale_rows,
    )


def save_marketing_page(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    page_id: str,
    page_name: str,
    page_type: str,
    locale_rows: list[dict[str, object]],
    keywords: str = "",
    apple_page_id: str = "",
    deep_link_url: str = "",
) -> StoreMarketingPage:
    app = scoped_app(session, account_id, app_id)
    if app is None:
        raise ApiError("app_not_found", "当前开发者账号下没有这个 App", status_code=404)
    if app.platform != "ios":
        raise ApiError(
            "unsupported_marketing_page",
            "营销页面当前仅支持 App Store Connect",
            status_code=422,
        )
    page = marketing_page_for_scope(
        session,
        account_id=account_id,
        app_id=app.id,
        page_id=page_id,
    )
    if page is None:
        raise ApiError("marketing_page_not_found", "营销页面不存在", status_code=404)

    normalized_page_name = page_name.strip() or page.page_name
    normalized_keywords = keywords.strip()
    _validate_text_length(
        locale=DEFAULT_LOCALE,
        label="关键词",
        value=normalized_keywords,
        max_length=APP_STORE_KEYWORDS_MAX_LENGTH,
    )
    page.page_name = normalized_page_name[:160]
    page.page_type = _normalize_marketing_page_type(page_type)
    page.keywords = normalized_keywords
    page.apple_page_id = apple_page_id.strip()
    page.deep_link_url = deep_link_url.strip()
    page.status = "draft"
    page.updated_at = datetime.now(UTC)

    existing_locales = marketing_page_locales(session, page.id)
    for row in locale_rows:
        locale = str(row.get("locale") or "").strip()
        if not locale:
            continue
        promotional_text = str(row.get("promotional_text") or "").strip()
        _validate_text_length(
            locale=locale,
            label="Promotional Text（宣传文本）",
            value=promotional_text,
            max_length=APP_STORE_PROMOTIONAL_TEXT_MAX_LENGTH,
        )
        store_images = _normalize_store_images(
            row.get("store_images") if isinstance(row.get("store_images"), dict) else {}
        )
        locale_row = existing_locales.get(locale)
        if locale_row is None:
            locale_row = StoreMarketingPageLocale(
                id=f"marketing-locale-{uuid4().hex[:12]}",
                marketing_page_id=page.id,
                locale=locale,
                promotional_text=promotional_text,
                store_images_json=store_images,
            )
            session.add(locale_row)
        else:
            locale_row.promotional_text = promotional_text
            locale_row.store_images_json = store_images
            locale_row.updated_at = datetime.now(UTC)

    session.flush()
    return page


def duplicate_marketing_page(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    page_id: str,
) -> StoreMarketingPage:
    source = marketing_page_for_scope(
        session,
        account_id=account_id,
        app_id=app_id,
        page_id=page_id,
    )
    if source is None:
        raise ApiError("marketing_page_not_found", "营销页面不存在", status_code=404)
    now = datetime.now(UTC)
    copied = StoreMarketingPage(
        id=f"marketing-page-{uuid4().hex[:12]}",
        developer_account_id=source.developer_account_id,
        app_id=source.app_id,
        platform=source.platform,
        page_id=f"page-{uuid4().hex[:8]}",
        page_name=f"{source.page_name} 副本"[:160],
        page_type=source.page_type,
        status="draft",
        apple_page_id="",
        deep_link_url=source.deep_link_url,
        keywords=source.keywords,
        store_images_json=dict(source.store_images_json or {}),
        created_at=now,
        updated_at=now,
    )
    session.add(copied)
    session.flush()
    for locale_row in marketing_page_locales(session, source.id).values():
        session.add(
            StoreMarketingPageLocale(
                id=f"marketing-locale-{uuid4().hex[:12]}",
                marketing_page_id=copied.id,
                locale=locale_row.locale,
                promotional_text=locale_row.promotional_text,
                store_images_json=dict(locale_row.store_images_json or {}),
                updated_at=now,
            )
        )
    session.flush()
    return copied


def delete_marketing_page(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    page_id: str,
) -> None:
    page = marketing_page_for_scope(
        session,
        account_id=account_id,
        app_id=app_id,
        page_id=page_id,
    )
    if page is None:
        raise ApiError("marketing_page_not_found", "营销页面不存在", status_code=404)
    session.delete(page)
    session.flush()


def get_or_refresh_preflight(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    version: str,
    locale: str,
    operation: str = UPDATE_RELEASE_NOTES,
    force_refresh: bool = False,
    store_app_id: str | None = None,
    package_name: str | None = None,
    client: StoreConnectorClient | None = None,
) -> PreflightState:
    app = scoped_app(session, account_id, app_id)
    if app is None:
        return _ephemeral_preflight(
            can_sync=False,
            reason_code="app_not_found",
            message="当前开发者账号下没有这个 App。",
        )

    connector = account_connector(session, account_id)
    if connector is None:
        return _ephemeral_preflight(
            can_sync=False,
            reason_code="connector_missing",
            message="当前开发者账号还没有配置 connector。",
        )

    payload = _preflight_payload(
        account_id=account_id,
        app=app,
        connector=connector,
        operation=operation,
        version=version,
        locale=locale,
        store_app_id=store_app_id,
        package_name=package_name,
    )
    request_hash = _request_hash(payload)
    now = datetime.now(UTC)
    if force_refresh:
        throttled = session.scalar(
            select(StorePreflightCheck)
            .where(
                StorePreflightCheck.request_hash == request_hash,
                StorePreflightCheck.checked_at > now - PREFLIGHT_FORCE_REFRESH_COOLDOWN,
            )
            .order_by(StorePreflightCheck.checked_at.desc())
            .limit(1)
        )
        if throttled is not None and throttled.store_state_json.get("manualRefresh") is True:
            return _state_from_check(throttled, cached=True, throttled=True)

    cached = session.scalar(
        select(StorePreflightCheck)
        .where(
            StorePreflightCheck.request_hash == request_hash,
            StorePreflightCheck.expires_at > now,
        )
        .order_by(StorePreflightCheck.checked_at.desc())
        .limit(1)
    )
    if cached is not None and not force_refresh:
        return _state_from_check(cached, cached=True)

    connector_client = client or StoreConnectorClient()
    try:
        response = connector_client.preflight(connector, payload)
    except ConnectorCallError as error:
        response = {
            "canSync": False,
            "reasonCode": "connector_error",
            "message": error.message,
            "storeState": {"reachable": False},
        }

    check = StorePreflightCheck(
        id=f"preflight-{uuid4().hex[:12]}",
        developer_account_id=account_id,
        app_id=app.id,
        connector_id=connector.id,
        platform=app.platform,
        operation=operation,
        version=version,
        locale=locale,
        request_hash=request_hash,
        can_sync=bool(response.get("canSync")),
        reason_code=_optional_str(response.get("reasonCode")),
        message=str(response.get("message") or "已完成同步条件检查。"),
        store_state_json={
            **dict(response.get("storeState") or {}),
            **({"manualRefresh": True} if force_refresh else {}),
        },
        checked_at=now,
        expires_at=now + PREFLIGHT_TTL,
    )
    session.add(check)
    session.flush()
    return _state_from_check(check, cached=False)


def cached_preflight_for_app(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    version: str,
    locale: str,
    operation: str = UPDATE_RELEASE_NOTES,
) -> PreflightState | None:
    app = scoped_app(session, account_id, app_id)
    connector = account_connector(session, account_id)
    if app is None or connector is None:
        return None
    payload = _preflight_payload(
        account_id=account_id,
        app=app,
        connector=connector,
        operation=operation,
        version=version,
        locale=locale,
    )
    now = datetime.now(UTC)
    cached = session.scalar(
        select(StorePreflightCheck)
        .where(
            StorePreflightCheck.request_hash == _request_hash(payload),
            StorePreflightCheck.expires_at > now,
        )
        .order_by(StorePreflightCheck.checked_at.desc())
        .limit(1)
    )
    return _state_from_check(cached, cached=True) if cached is not None else None


def sync_release_notes(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    version: str,
    locale: str,
    release_notes: str,
    actor: str,
    client: StoreConnectorClient | None = None,
) -> StoreSyncRun:
    app = scoped_app(session, account_id, app_id)
    if app is None:
        raise ApiError("app_not_found", "当前开发者账号下没有这个 App", status_code=404)
    connector = account_connector(session, account_id)
    if connector is None:
        raise ApiError("connector_missing", "当前开发者账号还没有配置 connector", status_code=422)
    draft = save_release_note_draft(
        session,
        account_id=account_id,
        app_id=app.id,
        version=version,
        locale=locale,
        release_notes=release_notes,
    )
    preflight = get_or_refresh_preflight(
        session,
        account_id=account_id,
        app_id=app.id,
        version=version,
        locale=locale,
        client=client,
    )
    if not preflight.can_sync:
        raise ApiError("preflight_failed", preflight.message, status_code=422)

    run = StoreSyncRun(
        id=f"sync-{uuid4().hex[:12]}",
        developer_account_id=account_id,
        app_id=app.id,
        connector_id=connector.id,
        draft_id=draft.id,
        platform=app.platform,
        operation=UPDATE_RELEASE_NOTES,
        version=version,
        locale=locale,
        status="running",
    )
    session.add(run)
    session.flush()

    payload = _sync_payload(
        run=run,
        account_id=account_id,
        app=app,
        operation=UPDATE_RELEASE_NOTES,
        version=version,
        locale=locale,
        release_notes=draft.release_notes,
        sync_scopes=["release_notes"],
    )
    run.sync_scopes_json = {"scopes": ["release_notes"]}
    run.payload_snapshot_json = {
        "version": version,
        "locale": locale,
        "releaseNotes": draft.release_notes,
    }
    connector_client = client or StoreConnectorClient()
    try:
        response = connector_client.sync_release_notes(connector, payload)
    except ConnectorCallError as error:
        run.status = "failed"
        run.error_code = "connector_error"
        run.error_summary = error.message
    else:
        status = str(response.get("status") or "succeeded")
        run.status = "succeeded" if status in {"ok", "success", "succeeded"} else status
        run.error_code = _optional_str(response.get("errorCode"))
        run.error_summary = _optional_str(response.get("errorSummary") or response.get("message"))
        if run.status == "succeeded":
            run.error_code = None
            run.error_summary = None
    run.finished_at = datetime.now(UTC)
    session.add(
        AuditLog(
            id=f"audit-{uuid4().hex[:12]}",
            developer_account_id=account_id,
            actor=actor,
            action="store.release_notes.sync",
            target_type="store_sync_run",
            target_id=run.id,
        )
    )
    session.flush()
    return run


def sync_app_metadata(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    version: str,
    locale: str,
    content_set_id: str = DEFAULT_CONTENT_SET_ID,
    content_set_name: str = DEFAULT_CONTENT_SET_NAME,
    keywords: str,
    promotional_text: str,
    description: str,
    actor: str,
    store_images: dict[str, object] | None = None,
    include_store_images_in_payload: bool = True,
    sync_scopes: list[str] | None = None,
    draft_version: str | None = None,
    client: StoreConnectorClient | None = None,
) -> StoreSyncRun:
    app = scoped_app(session, account_id, app_id)
    if app is None:
        raise ApiError("app_not_found", "当前开发者账号下没有这个 App", status_code=404)
    connector = account_connector(session, account_id)
    if connector is None:
        raise ApiError("connector_missing", "当前开发者账号还没有配置 connector", status_code=422)
    normalized_scopes = sync_scopes or (
        ["metadata", "store_images"] if include_store_images_in_payload else ["metadata"]
    )
    include_text_metadata_in_payload = _has_app_metadata_text_scope(normalized_scopes)
    if include_text_metadata_in_payload:
        validate_app_metadata_for_sync(
            platform=app.platform,
            locale=locale,
            keywords=keywords,
            promotional_text=promotional_text,
            description=description,
            sync_scopes=normalized_scopes,
        )
    draft = save_app_metadata_draft(
        session,
        account_id=account_id,
        app_id=app.id,
        version=draft_version or version,
        locale=locale,
        content_set_id=content_set_id,
        content_set_name=content_set_name,
        keywords=keywords,
        promotional_text=promotional_text,
        description=description,
        store_images=store_images,
    )
    preflight = get_or_refresh_preflight(
        session,
        account_id=account_id,
        app_id=app.id,
        version=version,
        locale=locale,
        operation=UPDATE_APP_METADATA,
        client=client,
    )
    if not preflight.can_sync:
        raise ApiError("preflight_failed", preflight.message, status_code=422)

    run = StoreSyncRun(
        id=f"sync-{uuid4().hex[:12]}",
        developer_account_id=account_id,
        app_id=app.id,
        connector_id=connector.id,
        metadata_draft_id=draft.id,
        platform=app.platform,
        operation=UPDATE_APP_METADATA,
        version=version,
        locale=locale,
        status="running",
    )
    session.add(run)
    session.flush()

    payload = _sync_payload(
        run=run,
        account_id=account_id,
        app=app,
        operation=UPDATE_APP_METADATA,
        version=version,
        locale=locale,
        metadata=_metadata_payload(
            draft,
            include_text_metadata=include_text_metadata_in_payload,
            include_store_images=include_store_images_in_payload,
        ),
        sync_scopes=normalized_scopes,
    )
    run.sync_scopes_json = {"scopes": normalized_scopes}
    run.payload_snapshot_json = {
        "version": version,
        "locale": locale,
        "metadata": payload.get("metadata", {}),
    }
    connector_client = client or StoreConnectorClient()
    try:
        response = connector_client.sync_store_operation(connector, payload)
    except ConnectorCallError as error:
        run.status = "failed"
        run.error_code = "connector_error"
        run.error_summary = error.message
    else:
        _apply_sync_response(run, response)
    run.finished_at = datetime.now(UTC)
    session.add(
        AuditLog(
            id=f"audit-{uuid4().hex[:12]}",
            developer_account_id=account_id,
            actor=actor,
            action="store.app_metadata.sync",
            target_type="store_sync_run",
            target_id=run.id,
        )
    )
    session.flush()
    return run


def sync_current_app_metadata(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    version: str,
    locale: str,
    keywords: str,
    promotional_text: str,
    description: str,
    actor: str,
    store_images: dict[str, object] | None = None,
    include_store_images_in_payload: bool = True,
    sync_scopes: list[str] | None = None,
    client: StoreConnectorClient | None = None,
) -> StoreSyncRun:
    return sync_app_metadata(
        session,
        account_id=account_id,
        app_id=app_id,
        version=version,
        draft_version=CURRENT_METADATA_VERSION,
        locale=locale,
        content_set_id=DEFAULT_CONTENT_SET_ID,
        content_set_name=DEFAULT_CONTENT_SET_NAME,
        keywords=keywords,
        promotional_text=promotional_text,
        description=description,
        actor=actor,
        store_images=store_images,
        include_store_images_in_payload=include_store_images_in_payload,
        sync_scopes=sync_scopes,
        client=client,
    )


def sync_existing_release_notes(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    version: str,
    locale: str,
    actor: str,
    store_track: str = "",
    store_version_code: str = "",
    store_app_id: str | None = None,
    package_name: str | None = None,
    client: StoreConnectorClient | None = None,
) -> StoreSyncRun:
    app = scoped_app(session, account_id, app_id)
    if app is None:
        raise ApiError("app_not_found", "当前开发者账号下没有这个 App", status_code=404)
    connector = account_connector(session, account_id)
    if connector is None:
        raise ApiError("connector_missing", "当前开发者账号还没有配置 connector", status_code=422)
    draft = draft_for_scope(
        session,
        account_id=account_id,
        app_id=app.id,
        platform=app.platform,
        version=version,
        locale=locale,
    )
    if draft is None or not draft.release_notes.strip():
        raise ApiError(
            "release_notes_draft_missing",
            f"{locale} 的 {version} 版本说明草稿不存在",
            status_code=422,
        )
    preflight = get_or_refresh_preflight(
        session,
        account_id=account_id,
        app_id=app.id,
        version=version,
        locale=locale,
        store_app_id=store_app_id,
        package_name=package_name,
        client=client,
    )
    if not preflight.can_sync:
        raise ApiError("preflight_failed", preflight.message, status_code=422)

    run = StoreSyncRun(
        id=f"sync-{uuid4().hex[:12]}",
        developer_account_id=account_id,
        app_id=app.id,
        connector_id=connector.id,
        draft_id=draft.id,
        platform=app.platform,
        operation=UPDATE_RELEASE_NOTES,
        version=version,
        locale=locale,
        status="running",
    )
    session.add(run)
    session.flush()

    payload = _sync_payload(
        run=run,
        account_id=account_id,
        app=app,
        operation=UPDATE_RELEASE_NOTES,
        version=version,
        locale=locale,
        release_notes=draft.release_notes,
        store_release=_store_release_payload(
            track=store_track,
            version_code=store_version_code,
        ),
        store_app_id=store_app_id,
        package_name=package_name,
        sync_scopes=["release_notes"],
    )
    run.sync_scopes_json = {"scopes": ["release_notes"]}
    run.payload_snapshot_json = {
        "version": version,
        "locale": locale,
        "app": payload["app"],
        "releaseNotes": draft.release_notes,
        "storeRelease": _store_release_payload(
            track=store_track,
            version_code=store_version_code,
        ),
    }
    connector_client = client or StoreConnectorClient()
    try:
        response = connector_client.sync_release_notes(connector, payload)
    except ConnectorCallError as error:
        run.status = "failed"
        run.error_code = "connector_error"
        run.error_summary = error.message
    else:
        status = str(response.get("status") or "succeeded")
        run.status = "succeeded" if status in {"ok", "success", "succeeded"} else status
        run.error_code = _optional_str(response.get("errorCode"))
        run.error_summary = _optional_str(response.get("errorSummary") or response.get("message"))
        store_state = response.get("storeState")
        if isinstance(store_state, dict):
            snapshot = dict(run.payload_snapshot_json or {})
            snapshot["storeState"] = store_state
            run.payload_snapshot_json = snapshot
        if run.status == "succeeded":
            run.error_code = None
            run.error_summary = None
    run.finished_at = datetime.now(UTC)
    session.add(
        AuditLog(
            id=f"audit-{uuid4().hex[:12]}",
            developer_account_id=account_id,
            actor=actor,
            action="store.release_notes.sync",
            target_type="store_sync_run",
            target_id=run.id,
        )
    )
    session.flush()
    return run


def sync_existing_current_app_metadata(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    version: str,
    locale: str,
    actor: str,
    sync_scopes: list[str],
    store_app_id: str | None = None,
    package_name: str | None = None,
    client: StoreConnectorClient | None = None,
) -> StoreSyncRun:
    app = scoped_app(session, account_id, app_id)
    if app is None:
        raise ApiError("app_not_found", "当前开发者账号下没有这个 App", status_code=404)
    connector = account_connector(session, account_id)
    if connector is None:
        raise ApiError("connector_missing", "当前开发者账号还没有配置 connector", status_code=422)
    normalized_scopes = _normalize_app_metadata_sync_scopes(sync_scopes)
    draft = metadata_draft_for_scope(
        session,
        account_id=account_id,
        app_id=app.id,
        platform=app.platform,
        version=CURRENT_METADATA_VERSION,
        locale=locale,
        content_set_id=DEFAULT_CONTENT_SET_ID,
    )
    if draft is None:
        raise ApiError(
            "metadata_draft_missing",
            f"{locale} 的默认商店页草稿不存在",
            status_code=422,
        )
    include_text_metadata_in_payload = _has_app_metadata_text_scope(normalized_scopes)
    include_store_images_in_payload = APP_METADATA_STORE_IMAGES_SCOPE in normalized_scopes
    if include_text_metadata_in_payload:
        validate_app_metadata_for_sync(
            platform=app.platform,
            locale=locale,
            keywords=draft.keywords,
            promotional_text=draft.promotional_text,
            description=draft.description,
            sync_scopes=normalized_scopes,
        )
    preflight = get_or_refresh_preflight(
        session,
        account_id=account_id,
        app_id=app.id,
        version=version,
        locale=locale,
        operation=UPDATE_APP_METADATA,
        store_app_id=store_app_id,
        package_name=package_name,
        client=client,
    )
    if not preflight.can_sync:
        raise ApiError("preflight_failed", preflight.message, status_code=422)

    run = StoreSyncRun(
        id=f"sync-{uuid4().hex[:12]}",
        developer_account_id=account_id,
        app_id=app.id,
        connector_id=connector.id,
        metadata_draft_id=draft.id,
        platform=app.platform,
        operation=UPDATE_APP_METADATA,
        version=version,
        locale=locale,
        status="running",
    )
    session.add(run)
    session.flush()

    payload = _sync_payload(
        run=run,
        account_id=account_id,
        app=app,
        operation=UPDATE_APP_METADATA,
        version=version,
        locale=locale,
        metadata=_metadata_payload(
            draft,
            include_text_metadata=include_text_metadata_in_payload,
            include_store_images=include_store_images_in_payload,
        ),
        store_app_id=store_app_id,
        package_name=package_name,
        sync_scopes=normalized_scopes,
    )
    run.sync_scopes_json = {"scopes": normalized_scopes}
    run.payload_snapshot_json = {
        "version": version,
        "locale": locale,
        "app": payload["app"],
        "metadata": payload.get("metadata", {}),
    }
    connector_client = client or StoreConnectorClient()
    try:
        response = connector_client.sync_store_operation(connector, payload)
    except ConnectorCallError as error:
        run.status = "failed"
        run.error_code = "connector_error"
        run.error_summary = error.message
    else:
        _apply_sync_response(run, response)
    run.finished_at = datetime.now(UTC)
    session.add(
        AuditLog(
            id=f"audit-{uuid4().hex[:12]}",
            developer_account_id=account_id,
            actor=actor,
            action="store.app_metadata.sync",
            target_type="store_sync_run",
            target_id=run.id,
        )
    )
    session.flush()
    return run


def sync_marketing_page(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    page_id: str,
    locale: str,
    sync_scopes: list[str],
    actor: str,
    store_app_id: str | None = None,
    client: StoreConnectorClient | None = None,
) -> StoreSyncRun:
    app = scoped_app(session, account_id, app_id)
    if app is None:
        raise ApiError("app_not_found", "当前开发者账号下没有这个 App", status_code=404)
    if app.platform != "ios":
        raise ApiError(
            "unsupported_marketing_page",
            "营销页面当前仅支持 App Store Connect",
            status_code=422,
        )
    connector = account_connector(session, account_id)
    if connector is None:
        raise ApiError("connector_missing", "当前开发者账号还没有配置 connector", status_code=422)
    page = marketing_page_for_scope(
        session,
        account_id=account_id,
        app_id=app.id,
        page_id=page_id,
    )
    if page is None:
        raise ApiError("marketing_page_not_found", "营销页面不存在", status_code=404)
    locale_row = marketing_page_locales(session, page.id).get(locale)
    if locale_row is None:
        raise ApiError(
            "marketing_page_locale_missing",
            f"{locale} 还没有营销页面内容",
            status_code=422,
        )
    normalized_scopes = _normalize_marketing_sync_scopes(sync_scopes)
    if MARKETING_SYNC_TEXT_SCOPE in normalized_scopes:
        _validate_text_length(
            locale=locale,
            label="Promotional Text（宣传文本）",
            value=locale_row.promotional_text.strip(),
            max_length=APP_STORE_PROMOTIONAL_TEXT_MAX_LENGTH,
        )

    preflight = get_or_refresh_preflight(
        session,
        account_id=account_id,
        app_id=app.id,
        version=page.page_id,
        locale=locale,
        operation=UPDATE_MARKETING_PAGE,
        store_app_id=store_app_id,
        client=client,
    )
    if not preflight.can_sync:
        raise ApiError("preflight_failed", preflight.message, status_code=422)

    run = StoreSyncRun(
        id=f"sync-{uuid4().hex[:12]}",
        developer_account_id=account_id,
        app_id=app.id,
        connector_id=connector.id,
        platform=app.platform,
        operation=UPDATE_MARKETING_PAGE,
        version=page.page_id,
        locale=locale,
        status="running",
    )
    session.add(run)
    session.flush()

    marketing_payload = _marketing_page_payload(
        page,
        locale_row=locale_row,
        sync_scopes=normalized_scopes,
    )
    payload = _sync_payload(
        run=run,
        account_id=account_id,
        app=app,
        operation=UPDATE_MARKETING_PAGE,
        version=page.page_id,
        locale=locale,
        marketing_page=marketing_payload,
        store_app_id=store_app_id,
        sync_scopes=normalized_scopes,
    )
    run.sync_scopes_json = {"scopes": normalized_scopes}
    run.payload_snapshot_json = {
        "pageId": page.page_id,
        "locale": locale,
        "app": payload["app"],
        "marketingPage": marketing_payload,
    }
    connector_client = client or StoreConnectorClient()
    try:
        response = connector_client.sync_store_operation(connector, payload)
    except ConnectorCallError as error:
        run.status = "failed"
        run.error_code = "connector_error"
        run.error_summary = error.message
    else:
        _apply_sync_response(run, response)
        if run.status == "succeeded":
            store_state = response.get("storeState")
            if isinstance(store_state, dict):
                apple_page_id = _optional_str(store_state.get("applePageId"))
                if apple_page_id:
                    page.apple_page_id = apple_page_id
            page.status = "synced"
            page.updated_at = datetime.now(UTC)
    run.finished_at = datetime.now(UTC)
    session.add(
        AuditLog(
            id=f"audit-{uuid4().hex[:12]}",
            developer_account_id=account_id,
            actor=actor,
            action="store.marketing_page.sync",
            target_type="store_sync_run",
            target_id=run.id,
        )
    )
    session.flush()
    return run


def validate_app_metadata_for_sync(
    *,
    platform: str,
    locale: str,
    keywords: str,
    promotional_text: str,
    description: str,
    sync_scopes: list[str] | None = None,
) -> None:
    normalized_description = description.strip()
    normalized_locale = locale.strip() or DEFAULT_LOCALE
    scopes = (
        _normalize_app_metadata_sync_scopes(sync_scopes)
        if sync_scopes
        else [APP_METADATA_SYNC_SCOPE]
    )
    sync_all_metadata = APP_METADATA_SYNC_SCOPE in scopes
    sync_description = sync_all_metadata or APP_METADATA_DESCRIPTION_SCOPE in scopes
    sync_promotional_text = sync_all_metadata or APP_METADATA_PROMOTIONAL_TEXT_SCOPE in scopes
    sync_short_description = sync_all_metadata or APP_METADATA_SHORT_DESCRIPTION_SCOPE in scopes
    sync_keywords = sync_all_metadata or APP_METADATA_KEYWORDS_SCOPE in scopes
    if platform == "ios":
        if sync_description:
            _validate_text_length(
                locale=normalized_locale,
                label="Description（描述）",
                value=normalized_description,
                min_length=APP_STORE_DESCRIPTION_MIN_LENGTH,
                max_length=APP_STORE_DESCRIPTION_MAX_LENGTH,
                required=True,
            )
        if sync_promotional_text:
            _validate_text_length(
                locale=normalized_locale,
                label="Promotional Text（宣传文本）",
                value=promotional_text.strip(),
                max_length=APP_STORE_PROMOTIONAL_TEXT_MAX_LENGTH,
            )
        if sync_keywords:
            _validate_text_length(
                locale=normalized_locale,
                label="Keywords（关键词）",
                value=keywords.strip(),
                max_length=APP_STORE_KEYWORDS_MAX_LENGTH,
            )
        return

    if platform == "android":
        if sync_description:
            _validate_text_length(
                locale=normalized_locale,
                label="Full description（完整描述）",
                value=normalized_description,
                min_length=GOOGLE_PLAY_FULL_DESCRIPTION_MIN_LENGTH,
                max_length=GOOGLE_PLAY_FULL_DESCRIPTION_MAX_LENGTH,
                required=True,
            )
        if sync_short_description or sync_promotional_text:
            _validate_text_length(
                locale=normalized_locale,
                label="Short description（简短说明）",
                value=promotional_text.strip(),
                max_length=GOOGLE_PLAY_SHORT_DESCRIPTION_MAX_LENGTH,
                required=APP_METADATA_SHORT_DESCRIPTION_SCOPE in scopes
                or APP_METADATA_PROMOTIONAL_TEXT_SCOPE in scopes,
            )
        return

    if sync_description:
        _validate_text_length(
            locale=normalized_locale,
            label="Description（描述）",
            value=normalized_description,
            min_length=APP_STORE_DESCRIPTION_MIN_LENGTH,
            max_length=APP_STORE_DESCRIPTION_MAX_LENGTH,
            required=True,
        )


def _validate_text_length(
    *,
    locale: str,
    label: str,
    value: str,
    max_length: int,
    min_length: int | None = None,
    required: bool = False,
) -> None:
    length = len(value)
    if required and length == 0:
        raise ApiError(
            "invalid_store_metadata",
            f"{locale} 的 {label} 不能为空。",
            status_code=422,
        )
    if length == 0:
        return
    if min_length is not None and length < min_length:
        raise ApiError(
            "invalid_store_metadata",
            f"{locale} 的 {label} 太短，至少需要 {min_length} 个字符，当前 {length} 个字符。",
            status_code=422,
        )
    if length > max_length:
        raise ApiError(
            "invalid_store_metadata",
            f"{locale} 的 {label} 不能超过 {max_length} 个字符，当前 {length} 个字符。",
            status_code=422,
        )


def recent_sync_runs(
    session: Session,
    *,
    account_id: str,
    app_id: str | None = None,
    limit: int = 8,
) -> list[StoreSyncRun]:
    statement = (
        select(StoreSyncRun)
        .where(StoreSyncRun.developer_account_id == account_id)
        .order_by(StoreSyncRun.started_at.desc())
        .limit(limit)
    )
    if app_id is not None:
        statement = statement.where(StoreSyncRun.app_id == app_id)
    return list(session.scalars(statement))


def scoped_app(session: Session, account_id: str, app_id: str) -> App | None:
    app = session.get(App, app_id)
    if app is None:
        return None
    if app.developer_account_id == account_id:
        return app
    legacy_link = session.scalar(
        select(DeveloperAccountApp).where(
            DeveloperAccountApp.developer_account_id == account_id,
            DeveloperAccountApp.app_id == app_id,
        )
    )
    return app if legacy_link is not None and app.developer_account_id is None else None


def account_or_404(session: Session, account_id: str) -> DeveloperAccount:
    account = session.get(DeveloperAccount, account_id)
    if account is None:
        raise ApiError("account_not_found", "开发者账号不存在", status_code=404)
    return account


class ConnectorCallError(RuntimeError):
    def __init__(self, message: str) -> None:
        super().__init__(message)
        self.message = message


def _is_active_connector(connector: StoreConnector) -> bool:
    return connector.base_url.startswith("active://")


def _preflight_payload(
    *,
    account_id: str,
    app: App,
    connector: StoreConnector,
    operation: str,
    version: str,
    locale: str,
    store_app_id: str | None = None,
    package_name: str | None = None,
) -> dict[str, object]:
    return {
        "developerAccountId": account_id,
        "connectorId": connector.id,
        "operation": operation,
        "platform": app.platform,
        "version": version,
        "locale": locale,
        "app": _app_payload(app, store_app_id=store_app_id, package_name=package_name),
    }


def _sync_payload(
    *,
    run: StoreSyncRun,
    account_id: str,
    app: App,
    operation: str,
    version: str,
    locale: str,
    release_notes: str | None = None,
    store_release: dict[str, object] | None = None,
    metadata: dict[str, object] | None = None,
    marketing_page: dict[str, object] | None = None,
    store_app_id: str | None = None,
    package_name: str | None = None,
    sync_scopes: list[str] | None = None,
) -> dict[str, object]:
    payload: dict[str, object] = {
        "runId": run.id,
        "developerAccountId": account_id,
        "operation": operation,
        "platform": app.platform,
        "version": version,
        "locale": locale,
        "app": _app_payload(app, store_app_id=store_app_id, package_name=package_name),
    }
    if release_notes is not None:
        payload["releaseNotes"] = release_notes
    if store_release is not None:
        payload["storeRelease"] = store_release
    if metadata is not None:
        payload["metadata"] = metadata
    if marketing_page is not None:
        payload["marketingPage"] = marketing_page
    if sync_scopes is not None:
        payload["syncScopes"] = sync_scopes
    return payload


def _store_release_payload(*, track: str = "", version_code: str = "") -> dict[str, object] | None:
    body: dict[str, object] = {}
    normalized_track = track.strip()
    normalized_version_code = version_code.strip()
    if normalized_track:
        body["track"] = normalized_track
    if normalized_version_code:
        body["versionCode"] = normalized_version_code
    return body or None


def _app_payload(
    app: App,
    *,
    store_app_id: str | None = None,
    package_name: str | None = None,
) -> dict[str, object]:
    normalized_store_app_id = _optional_str(store_app_id)
    normalized_package_name = _optional_str(package_name)
    return {
        "appId": app.id,
        "bundleIdentifier": app.bundle_identifier,
        "storeAppId": (
            normalized_store_app_id
            if normalized_store_app_id is not None
            else app.store_app_id
        ),
        "packageName": normalized_package_name
        if normalized_package_name is not None
        else app.store_package_name or app.bundle_identifier,
    }


def _connector_app_payload(app: App) -> dict[str, object]:
    return _app_payload(app)


def _metadata_payload(
    draft: StoreAppMetadataDraft,
    *,
    include_text_metadata: bool = True,
    include_store_images: bool = True,
) -> dict[str, object]:
    payload: dict[str, object] = {
        "contentSet": {
            "id": draft.content_set_id,
            "name": draft.content_set_name,
        },
    }
    if include_text_metadata:
        text_payload = {
            "promotionalText": draft.promotional_text,
            "shortDescription": draft.promotional_text,
            "description": draft.description,
        }
        if draft.keywords.strip():
            text_payload["keywords"] = draft.keywords
        payload.update(text_payload)
    if include_store_images:
        payload["storeImages"] = draft.store_images_json
    return payload


def _marketing_page_payload(
    page: StoreMarketingPage,
    *,
    locale_row: StoreMarketingPageLocale,
    sync_scopes: list[str],
) -> dict[str, object]:
    payload: dict[str, object] = {
        "pageId": page.page_id,
        "pageName": page.page_name,
        "pageType": page.page_type,
        "applePageId": page.apple_page_id,
        "deepLinkUrl": page.deep_link_url,
        "locale": locale_row.locale,
    }
    if MARKETING_SYNC_TEXT_SCOPE in sync_scopes:
        payload.update(
            {
                "promotionalText": locale_row.promotional_text,
            }
        )
    if MARKETING_SYNC_IMAGE_SCOPE in sync_scopes:
        payload["storeImages"] = _normalize_store_images(locale_row.store_images_json)
    return payload


def _normalize_marketing_page_type(value: str) -> str:
    normalized = str(value or "").strip()
    if normalized in {"custom_product_page", "product_page_optimization"}:
        return normalized
    return "custom_product_page"


def _normalize_app_metadata_sync_scopes(values: list[str] | None) -> list[str]:
    allowed = APP_METADATA_SYNC_SCOPES
    selected = [value for value in values or [] if value in allowed]
    deduped: list[str] = []
    for value in selected:
        if value not in deduped:
            deduped.append(value)
    if not deduped:
        raise ApiError(
            "invalid_sync_scopes",
            "同步范围至少需要包含一个商店页同步项",
            status_code=422,
        )
    return deduped


def _has_app_metadata_text_scope(scopes: list[str]) -> bool:
    return any(scope in APP_METADATA_TEXT_SYNC_SCOPES for scope in scopes)


def _normalize_marketing_sync_scopes(values: list[str] | None) -> list[str]:
    allowed = [MARKETING_SYNC_TEXT_SCOPE, MARKETING_SYNC_IMAGE_SCOPE]
    selected = [value for value in values or [] if value in allowed]
    deduped: list[str] = []
    for value in selected:
        if value not in deduped:
            deduped.append(value)
    if not deduped:
        raise ApiError("missing_sync_scope", "请至少勾选一个要同步的营销页面内容", status_code=422)
    return deduped


def _normalize_store_images(raw_images: dict[str, object] | None) -> dict[str, object]:
    return {
        "feature_graphic_url": _normalize_store_image_slot(raw_images, "feature_graphic_url"),
        "phone_screenshots": _normalize_store_image_slot(raw_images, "phone_screenshots"),
        "tablet_screenshots": _normalize_store_image_slot(raw_images, "tablet_screenshots"),
    }


def _normalize_store_image_slot(
    raw_images: dict[str, object] | None,
    key: str,
) -> dict[str, object]:
    raw_value = (raw_images or {}).get(key)
    if isinstance(raw_value, dict):
        urls = raw_value.get("urls")
        assets = raw_value.get("assets")
        return {
            "urls": _string_list(urls),
            "assets": _asset_list(assets),
        }
    return {
        "urls": _string_list(raw_value),
        "assets": [],
    }


def _string_list(value: object) -> list[str]:
    if isinstance(value, list | tuple):
        raw_values = [str(item or "") for item in value]
    else:
        raw_values = str(value or "").splitlines()
    return [item.strip() for item in raw_values if item.strip()]


def _asset_list(value: object) -> list[dict[str, object]]:
    if not isinstance(value, list | tuple):
        return []
    assets: list[dict[str, object]] = []
    for item in value:
        if not isinstance(item, dict):
            continue
        download_url = str(item.get("downloadUrl") or item.get("download_url") or "").strip()
        if not download_url:
            continue
        assets.append(
            {
                "fileName": str(item.get("fileName") or item.get("file_name") or "").strip(),
                "contentType": str(
                    item.get("contentType") or item.get("content_type") or ""
                ).strip(),
                "sizeBytes": int(item.get("sizeBytes") or item.get("size_bytes") or 0),
                "storageKey": str(item.get("storageKey") or item.get("storage_key") or "").strip(),
                "downloadUrl": download_url,
                "width": int(item.get("width") or 0) or None,
                "height": int(item.get("height") or 0) or None,
                "format": str(item.get("format") or "").strip(),
                "validationMessage": str(
                    item.get("validationMessage") or item.get("validation_message") or ""
                ).strip(),
                "matchedLabel": str(
                    item.get("matchedLabel") or item.get("matched_label") or ""
                ).strip(),
            }
        )
    return assets


def _normalize_content_set_id(content_set_id: str | None) -> str:
    value = str(content_set_id or "").strip()
    return value or DEFAULT_CONTENT_SET_ID


def _normalize_content_set_name(content_set_name: str | None, content_set_id: str) -> str:
    value = str(content_set_name or "").strip()
    if value:
        return value
    return DEFAULT_CONTENT_SET_NAME if content_set_id == DEFAULT_CONTENT_SET_ID else content_set_id


def _post_json(
    connector: StoreConnector,
    path: str,
    payload: dict[str, object],
) -> dict[str, object]:
    url = connector.base_url.rstrip("/") + path
    request = Request(
        url,
        data=json.dumps(payload).encode("utf-8"),
        headers={
            "Authorization": f"Bearer {connector.auth_token}",
            "Content-Type": "application/json",
            "Accept": "application/json",
        },
        method="POST",
    )
    try:
        with urlopen(request, timeout=20) as response:  # noqa: S310 - connector URL is admin configured.
            return json.loads(response.read().decode("utf-8"))
    except HTTPError as error:
        body = error.read().decode("utf-8", errors="replace")
        raise ConnectorCallError(f"connector 返回 HTTP {error.code}: {body[:180]}") from error
    except (URLError, TimeoutError, OSError, json.JSONDecodeError) as error:
        raise ConnectorCallError(f"connector 调用失败: {error}") from error


def _active_request_json(
    connector: StoreConnector,
    method: str,
    path: str,
    payload: dict[str, object] | None = None,
    *,
    include_auth: bool = True,
    timeout: int = 20,
) -> dict[str, object]:
    headers = {"Accept": "application/json"}
    if payload is not None:
        headers["Content-Type"] = "application/json"
    if include_auth:
        headers["Authorization"] = f"Bearer {connector.auth_token}"
    try:
        result = active_connector_hub.dispatch(
            account_id=connector.developer_account_id,
            method=method,
            path=path,
            headers=headers,
            body=payload,
            timeout_seconds=timeout,
        )
    except ActiveConnectorTimeoutError as error:
        raise ConnectorCallError(error.args[0]) from error

    if result.status_code < 200 or result.status_code >= 300:
        raise ConnectorCallError(f"connector 返回 HTTP {result.status_code}: {result.body[:180]}")
    try:
        return json.loads(result.body or "{}")
    except json.JSONDecodeError as error:
        raise ConnectorCallError(f"connector 返回内容不是 JSON: {error}") from error


def _get_json(
    connector: StoreConnector,
    path: str,
    *,
    include_auth: bool = True,
    timeout: int = 20,
) -> dict[str, object]:
    url = connector.base_url.rstrip("/") + path
    headers = {"Accept": "application/json"}
    if include_auth:
        headers["Authorization"] = f"Bearer {connector.auth_token}"
    request = Request(
        url,
        headers=headers,
        method="GET",
    )
    try:
        with urlopen(request, timeout=timeout) as response:  # noqa: S310 - connector URL is admin configured.
            return json.loads(response.read().decode("utf-8"))
    except HTTPError as error:
        body = error.read().decode("utf-8", errors="replace")
        raise ConnectorCallError(f"connector 返回 HTTP {error.code}: {body[:180]}") from error
    except (URLError, TimeoutError, OSError, json.JSONDecodeError) as error:
        raise ConnectorCallError(f"connector 调用失败: {error}") from error


def _mock_preflight(payload: dict[str, object]) -> dict[str, object]:
    version = str(payload.get("version") or "")
    operation = str(payload.get("operation") or UPDATE_RELEASE_NOTES)
    operation_label = _operation_label(operation)
    if operation == UPDATE_MARKETING_PAGE:
        return {
            "canSync": True,
            "reasonCode": None,
            "message": "营销页面可同步；同步前会按勾选项提交文案或商店图。",
            "storeState": {
                "versionExists": True,
                "editable": True,
                "currentStatus": "marketing_page_editable",
            },
        }
    if not version or "missing" in version.lower():
        return {
            "canSync": False,
            "reasonCode": "store_version_missing",
            "message": f"商店中还没有创建 {version or '目标'} 版本，暂不能同步{operation_label}。",
            "storeState": {
                "versionExists": False,
                "editable": False,
                "currentStatus": "missing",
            },
        }
    return {
        "canSync": True,
        "reasonCode": None,
        "message": f"商店中已存在 {version} 版本，当前状态允许修改{operation_label}。",
        "storeState": {
            "versionExists": True,
            "editable": True,
            "currentStatus": "prepare_for_submission",
        },
    }


def _mock_supported_locales(platform: str) -> list[str]:
    return ["zh-Hans", "en-US", "ja", "ko"] if platform == "ios" else ["zh-Hans", "en-US"]


def _mock_store_listings(app: App) -> dict[str, object]:
    locales = _mock_supported_locales(app.platform)
    listings: list[dict[str, object]] = []
    for locale in locales:
        listing: dict[str, object] = {
            "locale": locale,
            "description": "Mock store description.",
            "fullDescription": "Mock store description.",
            "keywords": "mock,test",
            "promotionalText": "Mock promotional text.",
            "releaseNotes": "Mock release notes.",
        }
        if app.platform == "android":
            listing.update(
                {
                    "title": app.name,
                    "shortDescription": "Mock short description.",
                    "videoUrl": "https://example.test/video",
                }
            )
        listings.append(listing)
    return {"listings": listings}


def _mock_store_images(app: App) -> dict[str, object]:
    locales = _mock_supported_locales(app.platform)
    return {
        "locales": [
            {
                "locale": locale,
                "images": {
                    "phone_screenshots": [
                        {
                            "id": f"mock-phone-{locale}",
                            "fileName": "phone-1.png",
                            "url": f"https://cdn.example.test/{locale}/phone-1.png",
                            "width": 1290,
                            "height": 2796,
                        }
                    ],
                    "tablet_screenshots": [],
                },
            }
            for locale in locales
        ]
    }


def _mock_store_releases(app: App) -> dict[str, object]:
    if app.platform != "android":
        return {"releases": []}
    return {
        "releases": [
            {
                "track": "production",
                "name": "3.1.0",
                "status": "completed",
                "versionCodes": ["310"],
                "releaseNotes": [
                    {
                        "language": "en-US",
                        "text": "Mock production release notes.",
                    }
                ],
            },
            {
                "track": "internal",
                "name": "3.2.0",
                "status": "draft",
                "versionCodes": ["320"],
                "releaseNotes": [],
            },
        ]
    }


def _mock_store_reviews(app: App) -> dict[str, object]:
    return {
        "reviews": [
            {
                "id": f"review-{app.id}-1",
                "platform": app.platform,
                "rating": 5,
                "title": "Mock review",
                "body": "The app works well for internal testing.",
                "authorName": "mock-user",
                "reviewerNickname": "mock-user",
                "locale": "en-US",
                "territory": "US",
                "appVersion": "1.0.0",
                "createdAt": "2026-06-24T10:00:00Z",
                "updatedAt": "2026-06-24T10:00:00Z",
            },
            {
                "id": f"review-{app.id}-2",
                "platform": app.platform,
                "rating": 3,
                "title": "Mock review old",
                "body": "Needs more checks.",
                "authorName": "mock-user-2",
                "reviewerNickname": "mock-user-2",
                "locale": "zh-Hans",
                "territory": "CN",
                "appVersion": "1.0.0",
                "createdAt": "2026-06-23T10:00:00Z",
                "updatedAt": "2026-06-23T10:00:00Z",
            },
        ],
        "nextPageToken": "",
    }


def _mock_product_page_optimizations(app: App) -> dict[str, object]:
    if app.platform != "ios":
        return {"experiments": []}
    return {
        "experiments": [
            {
                "id": f"ppo-{app.id}",
                "name": "Mock Product Page Optimization",
                "platform": "IOS",
                "state": "PREPARE_FOR_SUBMISSION",
                "trafficProportion": 50,
                "reviewRequired": False,
                "treatments": [
                    {
                        "id": "treatment-mock-a",
                        "name": "Variant A",
                        "locales": ["en-US", "zh-Hant"],
                    }
                ],
            }
        ]
    }


def _mock_create_product_page_optimization(
    app: App,
    payload: dict[str, object],
) -> dict[str, object]:
    if app.platform != "ios":
        raise ConnectorCallError("产品页面优化当前仅支持 App Store Connect")
    name = str(payload.get("name") or "").strip()
    if not name:
        raise ConnectorCallError("产品页面优化名称不能为空")
    traffic_proportion = int(payload.get("trafficProportion") or 0)
    if traffic_proportion <= 0 or traffic_proportion > 100:
        raise ConnectorCallError("trafficProportion 必须在 1 到 100 之间")
    locales = _normalize_connector_locales(payload.get("locales"))
    raw_treatments = payload.get("treatments")
    treatment_inputs = raw_treatments if isinstance(raw_treatments, list) else []
    if not treatment_inputs:
        treatment_inputs = [{"name": "Variant A"}]
    treatments: list[dict[str, object]] = []
    for index, raw_treatment in enumerate(treatment_inputs):
        treatment = raw_treatment if isinstance(raw_treatment, dict) else {}
        treatment_locales = _normalize_connector_locales(treatment.get("locales")) or locales
        treatments.append(
            {
                "id": f"treatment-{app.id}-{index + 1}",
                "name": str(treatment.get("name") or f"Variant {index + 1}").strip(),
                "appIconName": str(treatment.get("appIconName") or "").strip(),
                "locales": treatment_locales,
            }
        )
    return {
        "experiment": {
            "id": f"ppo-created-{app.id}",
            "name": name,
            "platform": "IOS",
            "state": "PREPARE_FOR_SUBMISSION",
            "trafficProportion": traffic_proportion,
            "reviewRequired": True,
            "treatments": treatments,
        }
    }


def _normalize_locales(raw_locales: object) -> list[str]:
    if not isinstance(raw_locales, list | tuple):
        return [DEFAULT_LOCALE]
    locales: list[str] = []
    seen: set[str] = set()
    for raw_locale in raw_locales:
        locale = str(raw_locale or "").strip()
        if locale and locale not in seen:
            locales.append(locale)
            seen.add(locale)
    return locales or [DEFAULT_LOCALE]


def _normalize_connector_locales(raw_locales: object) -> list[str]:
    if not isinstance(raw_locales, list | tuple):
        return []
    locales: list[str] = []
    seen: set[str] = set()
    for raw_locale in raw_locales:
        locale = str(raw_locale or "").strip()
        if locale and locale not in seen:
            locales.append(locale)
            seen.add(locale)
    return locales


def _apply_sync_response(run: StoreSyncRun, response: dict[str, object]) -> None:
    status = str(response.get("status") or "succeeded")
    run.status = "succeeded" if status in {"ok", "success", "succeeded"} else status
    run.error_code = _optional_str(response.get("errorCode"))
    run.error_summary = _optional_str(response.get("errorSummary") or response.get("message"))
    if run.status == "succeeded":
        run.error_code = None
        run.error_summary = None


def _operation_label(operation: str) -> str:
    if operation == UPDATE_APP_METADATA:
        return "商店元数据"
    if operation == UPDATE_MARKETING_PAGE:
        return "营销页面"
    return "版本说明"


def _request_hash(payload: dict[str, object]) -> str:
    raw = json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=True)
    return hashlib.sha256(raw.encode("utf-8")).hexdigest()


def _state_from_check(
    check: StorePreflightCheck,
    *,
    cached: bool,
    throttled: bool = False,
) -> PreflightState:
    return PreflightState(
        can_sync=check.can_sync,
        reason_code=check.reason_code,
        message=check.message,
        store_state=check.store_state_json,
        checked_at=check.checked_at,
        expires_at=check.expires_at,
        cached=cached,
        throttled=throttled,
    )


def _ephemeral_preflight(
    *,
    can_sync: bool,
    reason_code: str,
    message: str,
) -> PreflightState:
    now = datetime.now(UTC)
    return PreflightState(
        can_sync=can_sync,
        reason_code=reason_code,
        message=message,
        store_state={},
        checked_at=now,
        expires_at=now,
        cached=False,
    )


def _as_utc(value: datetime) -> datetime:
    return value if value.tzinfo else value.replace(tzinfo=UTC)


def _optional_str(value: Any) -> str | None:
    if value is None:
        return None
    text = str(value)
    return text if text else None

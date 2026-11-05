from __future__ import annotations

from dataclasses import dataclass
from datetime import UTC, datetime
from urllib.parse import quote

from sqlalchemy import func, select
from sqlalchemy.orm import Session, joinedload, selectinload

from testflying_api.admin.services import list_account_options, list_unassigned_apps
from testflying_api.schema import (
    App,
    Build,
    DeveloperAccount,
    DeveloperAccountApp,
    Device,
    Notification,
    StoreAppMetadataDraft,
    StoreImageSuite,
    StoreImageSuiteLocale,
    StoreMarketingPage,
    StoreMarketingPageLocale,
    StoreReleaseNoteDraft,
    StoreSyncRun,
)
from testflying_api.store_image_requirements import store_image_requirement
from testflying_api.store_sync import (
    CURRENT_METADATA_VERSION,
    DEFAULT_CONTENT_SET_ID,
    DEFAULT_CONTENT_SET_NAME,
    DEFAULT_LOCALE,
    UPDATE_APP_METADATA,
    UPDATE_MARKETING_PAGE,
    account_apps,
    account_connector,
    cached_preflight_for_app,
    current_metadata_drafts_for_app,
    draft_for_scope,
    get_or_refresh_preflight,
    latest_build_for_app,
    marketing_page_for_scope,
    marketing_page_locales,
    metadata_draft_for_scope,
    metadata_drafts_for_scope,
    recent_sync_runs,
    supported_locales_for_app,
)


@dataclass(frozen=True)
class StatCard:
    label: str
    value: str
    tone: str = "neutral"


def dashboard_context(session: Session) -> dict[str, object]:
    app_count = session.scalar(select(func.count(App.id))) or 0
    build_count = session.scalar(select(func.count(Build.id))) or 0
    ios_count = session.scalar(select(func.count(Build.id)).where(Build.platform == "ios")) or 0
    android_count = (
        session.scalar(select(func.count(Build.id)).where(Build.platform == "android")) or 0
    )
    renewal_count = session.scalar(
        select(func.count(DeveloperAccount.id)).where(DeveloperAccount.status != "ok")
    ) or 0
    return {
        "stats": [
            StatCard("应用", str(app_count), "blue"),
            StatCard("构建", str(build_count), "green"),
            StatCard("iOS / Android", f"{ios_count} / {android_count}", "neutral"),
            StatCard("续费提醒", str(renewal_count), "red" if renewal_count else "neutral"),
        ],
        "recent_builds": list_builds(session, limit=6),
        "recent_notifications": list_notifications(session, limit=8),
    }


def list_apps(session: Session) -> list[App]:
    return list(
        session.scalars(
            select(App)
            .options(selectinload(App.builds), selectinload(App.developer_account))
            .order_by(App.added_at.desc(), App.name.asc())
        )
    )


def upload_context(session: Session) -> dict[str, object]:
    return {"accounts": list_account_options(session)}


def list_builds(session: Session, *, limit: int | None = None) -> list[Build]:
    statement = (
        select(Build)
        .options(joinedload(Build.app), selectinload(Build.artifacts))
        .order_by(Build.uploaded_at.desc())
    )
    if limit is not None:
        statement = statement.limit(limit)
    return list(session.scalars(statement))


def list_devices(session: Session) -> list[Device]:
    return list(session.scalars(select(Device).order_by(Device.registered_at.desc())))


def list_accounts(session: Session) -> list[dict[str, object]]:
    accounts = list(
        session.scalars(select(DeveloperAccount).order_by(DeveloperAccount.expires_at.asc()))
    )
    app_rows = session.execute(
        select(DeveloperAccount.id, App.name)
        .select_from(DeveloperAccount)
        .join(App, App.developer_account_id == DeveloperAccount.id, isouter=True)
    )
    names_by_account: dict[str, list[str]] = {}
    for account_id, app_name in app_rows:
        if app_name:
            names_by_account.setdefault(account_id, []).append(app_name)
    legacy_rows = session.execute(
        select(DeveloperAccountApp.developer_account_id, App.name).join(App)
    )
    for account_id, app_name in legacy_rows:
        names = names_by_account.setdefault(account_id, [])
        if app_name not in names:
            names.append(app_name)
    sync_rows = session.scalars(
        select(StoreSyncRun).order_by(
            StoreSyncRun.developer_account_id.asc(),
            StoreSyncRun.started_at.desc(),
        )
    )
    latest_sync_by_account: dict[str, StoreSyncRun] = {}
    for run in sync_rows:
        latest_sync_by_account.setdefault(run.developer_account_id, run)
    return [
        {
            "account": account,
            "remaining_days": remaining_days(account.expires_at),
            "apps": names_by_account.get(account.id, []),
            "connector": account_connector(session, account.id),
            "latest_sync": latest_sync_by_account.get(account.id),
        }
        for account in accounts
    ]


def account_detail_context(session: Session, account_id: str) -> dict[str, object]:
    account = session.get(DeveloperAccount, account_id)
    apps = account_apps(session, account_id)
    connector = account_connector(session, account_id)
    account_store_platform = _single_store_platform(apps)
    return {
        "account": account,
        "remaining_days": remaining_days(account.expires_at) if account else 0,
        "apps": [
            {
                "app": app,
                "latest_build": latest_build_for_app(session, app.id),
            }
            for app in apps
        ],
        "connector": connector,
        "account_store_platform": account_store_platform,
        "unassigned_apps": list_unassigned_apps(session),
        "sync_runs": recent_sync_runs(session, account_id=account_id),
    }


def release_notes_context(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    version: str | None = None,
    locale: str = DEFAULT_LOCALE,
) -> dict[str, object]:
    account = session.get(DeveloperAccount, account_id)
    app = next((item for item in account_apps(session, account_id) if item.id == app_id), None)
    latest_build = latest_build_for_app(session, app_id)
    target_version = version or (latest_build.version if latest_build else "")
    draft = (
        draft_for_scope(
            session,
            account_id=account_id,
            app_id=app_id,
            platform=app.platform,
            version=target_version,
            locale=locale,
        )
        if app and target_version
        else None
    )
    preflight = (
        get_or_refresh_preflight(
            session,
            account_id=account_id,
            app_id=app_id,
            version=target_version,
            locale=locale,
        )
        if app and target_version
        else None
    )
    release_notes = draft.release_notes if draft else (latest_build.note if latest_build else "")
    return {
        "account": account,
        "app": app,
        "latest_build": latest_build,
        "version": target_version,
        "locale": locale,
        "draft": draft,
        "release_notes": release_notes,
        "preflight": preflight,
        "connector": account_connector(session, account_id),
        "sync_runs": recent_sync_runs(session, account_id=account_id, app_id=app_id),
    }


def store_metadata_context(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    version: str | None = None,
    locale: str = DEFAULT_LOCALE,
    content_set_id: str = DEFAULT_CONTENT_SET_ID,
    force_preflight_refresh: bool = False,
) -> dict[str, object]:
    account = session.get(DeveloperAccount, account_id)
    app = next((item for item in account_apps(session, account_id) if item.id == app_id), None)
    latest_build = latest_build_for_app(session, app_id)
    target_version = version or (latest_build.version if latest_build else "")
    drafts_by_locale: dict[str, StoreAppMetadataDraft] = {}
    if app:
        drafts_by_locale = current_metadata_drafts_for_app(
            session,
            account_id=account_id,
            app_id=app_id,
            platform=app.platform,
        )
        if not drafts_by_locale and target_version:
            drafts_by_locale = metadata_drafts_for_scope(
                session,
                account_id=account_id,
                app_id=app_id,
                platform=app.platform,
                version=target_version,
                content_set_id=DEFAULT_CONTENT_SET_ID,
            )
    release_note_drafts_by_locale = (
        _release_note_drafts_for_scope(
            session,
            account_id=account_id,
            app_id=app_id,
            platform=app.platform,
            version=target_version,
        )
        if app and target_version
        else {}
    )
    content_sets = [{"id": DEFAULT_CONTENT_SET_ID, "name": DEFAULT_CONTENT_SET_NAME}]
    image_suites: list[dict[str, object]] = []
    image_suite_locales_by_locale: dict[str, object] = {}
    active_content_set = {
        "id": DEFAULT_CONTENT_SET_ID,
        "name": DEFAULT_CONTENT_SET_NAME,
    }
    connector = account_connector(session, account_id)
    local_locales = _known_store_locales(
        fallback_locale=locale,
        historical_locales=(
            _historical_store_locales(
                session,
                account_id=account_id,
                app_id=app_id,
                platform=app.platform,
            )
            if app
            else []
        ),
        drafts_by_locale=drafts_by_locale,
        release_note_drafts_by_locale=release_note_drafts_by_locale,
        image_suite_locales_by_locale=image_suite_locales_by_locale,
    )
    supported_locales = local_locales
    if app and target_version and connector:
        supported_locales = supported_locales_for_app(
            session,
            account_id=account_id,
            app_id=app_id,
            version=target_version,
            fallback_locale=locale,
            fallback_locales=local_locales,
        )
    source_locale = _source_locale(supported_locales)
    active_locale = locale if locale in supported_locales else source_locale
    draft = (
        metadata_draft_for_scope(
            session,
            account_id=account_id,
            app_id=app_id,
            platform=app.platform,
            version=CURRENT_METADATA_VERSION,
            locale=source_locale,
            content_set_id=DEFAULT_CONTENT_SET_ID,
        )
        if app
        else None
    )
    base_metadata = _store_metadata_defaults(
        app=app,
        latest_build=latest_build,
        draft=draft,
    )
    preflight = (
        cached_preflight_for_app(
            session,
            account_id=account_id,
            app_id=app_id,
            version=target_version,
            locale=source_locale,
            operation=UPDATE_APP_METADATA,
        )
        if app and target_version and not force_preflight_refresh
        else None
    )
    if app and target_version and force_preflight_refresh:
        preflight = get_or_refresh_preflight(
            session,
            account_id=account_id,
            app_id=app_id,
            version=target_version,
            locale=source_locale,
            operation=UPDATE_APP_METADATA,
            force_refresh=True,
        )
    return {
        "account": account,
        "app": app,
        "latest_build": latest_build,
        "version": target_version,
        "locale": active_locale,
        "source_locale": source_locale,
        "content_set_id": active_content_set["id"],
        "content_set_name": active_content_set["name"],
        "content_sets": content_sets,
        "content_set_options": content_sets,
        "image_suites": image_suites,
        "draft": draft,
        "metadata": base_metadata,
        "metadata_fields": _metadata_fields(app.platform if app else ""),
        "store_image_slots": _store_image_slots(app.platform if app else ""),
        "store_label": _store_label(app.platform if app else ""),
        "supported_locales": supported_locales,
        "localized_metadata": _localized_metadata(
            supported_locales=supported_locales,
            drafts_by_locale=drafts_by_locale,
            release_note_drafts_by_locale=release_note_drafts_by_locale,
            image_suite_locales_by_locale=image_suite_locales_by_locale,
            base_metadata=base_metadata,
            source_locale=source_locale,
        ),
        "preflight": preflight,
        "connector": connector,
        "sync_runs": recent_sync_runs(session, account_id=account_id, app_id=app_id),
        "sync_history_groups": _sync_history_groups(
            recent_sync_runs(session, account_id=account_id, app_id=app_id, limit=50)
        ),
        "marketing_pages": _marketing_pages_for_app(
            session,
            account_id=account_id,
            app_id=app_id,
            platform=app.platform,
        )
        if app
        else [],
    }


def store_marketing_context(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    locale: str = DEFAULT_LOCALE,
) -> dict[str, object]:
    base = store_metadata_context(
        session,
        account_id=account_id,
        app_id=app_id,
        locale=locale,
    )
    app = base.get("app")
    raw_pages = (
        _marketing_pages_for_app(
            session,
            account_id=account_id,
            app_id=app_id,
            platform=app.platform,
        )
        if isinstance(app, App)
        else []
    )
    return {
        **base,
        "marketing_pages": [_marketing_page_summary(page) for page in raw_pages],
        "marketing_sync_runs": [
            run
            for run in recent_sync_runs(session, account_id=account_id, app_id=app_id, limit=30)
            if run.operation == UPDATE_MARKETING_PAGE
        ],
    }


def marketing_page_context(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    page_id: str,
    locale: str = DEFAULT_LOCALE,
    force_preflight_refresh: bool = False,
) -> dict[str, object]:
    account = session.get(DeveloperAccount, account_id)
    app = (
        account_apps(session, account_id)
        if account
        else []
    )
    scoped = next((item for item in app if item.id == app_id), None)
    page = (
        marketing_page_for_scope(
            session,
            account_id=account_id,
            app_id=app_id,
            page_id=page_id,
        )
        if scoped
        else None
    )
    connector = account_connector(session, account_id) if account else None
    locale_rows_by_locale = marketing_page_locales(session, page.id) if page else {}
    supported_from_connector = (
        supported_locales_for_app(
            session,
            account_id=account_id,
            app_id=app_id,
            version=page.page_id if page else "",
            fallback_locale=locale,
        )
        if scoped and page
        else [locale or DEFAULT_LOCALE]
    )
    supported_locales = _unique_locales(
        [
            *supported_from_connector,
            *locale_rows_by_locale.keys(),
            locale,
        ]
    )
    active_locale = locale if locale in supported_locales else _source_locale(supported_locales)
    source_locale = _source_locale(supported_locales)
    preflight = None
    if page and scoped and force_preflight_refresh:
        preflight = get_or_refresh_preflight(
            session,
            account_id=account_id,
            app_id=app_id,
            version=page.page_id,
            locale=active_locale,
            operation=UPDATE_MARKETING_PAGE,
            force_refresh=True,
        )
    elif page and scoped:
        preflight = cached_preflight_for_app(
            session,
            account_id=account_id,
            app_id=app_id,
            version=page.page_id,
            locale=active_locale,
            operation=UPDATE_MARKETING_PAGE,
        )
        if preflight is None:
            preflight = get_or_refresh_preflight(
                session,
                account_id=account_id,
                app_id=app_id,
                version=page.page_id,
                locale=active_locale,
                operation=UPDATE_MARKETING_PAGE,
            )
    return {
        "account": account,
        "app": scoped,
        "page": page,
        "connector": connector,
        "locale": active_locale,
        "source_locale": source_locale,
        "supported_locales": supported_locales,
        "localized_marketing_page": _localized_marketing_page(
            supported_locales=supported_locales,
            locale_rows_by_locale=locale_rows_by_locale,
            source_locale=source_locale,
        ),
        "store_image_slots": _store_image_slots(scoped.platform if scoped else ""),
        "store_label": _store_label(scoped.platform if scoped else ""),
        "preflight": preflight,
        "sync_runs": _marketing_page_sync_runs(
            session,
            account_id=account_id,
            app_id=app_id,
            page_id=page_id,
        )
        if page
        else [],
    }


def _store_metadata_defaults(
    *,
    app: App | None,
    latest_build: Build | None,
    draft: object | None,
) -> dict[str, str]:
    return {
        "keywords": draft.keywords if draft else "",
        "promotional_text": draft.promotional_text if draft else "",
        "description": draft.description if draft else (latest_build.note if latest_build else ""),
    }


def _known_store_locales(
    *,
    fallback_locale: str,
    historical_locales: list[str],
    drafts_by_locale: dict[str, object],
    release_note_drafts_by_locale: dict[str, StoreReleaseNoteDraft],
    image_suite_locales_by_locale: dict[str, object],
) -> list[str]:
    locales: list[str] = []
    seen: set[str] = set()
    for locale in [
        *drafts_by_locale.keys(),
        *release_note_drafts_by_locale.keys(),
        *image_suite_locales_by_locale.keys(),
        *historical_locales,
        fallback_locale,
    ]:
        normalized = str(locale or "").strip()
        if normalized and normalized not in seen:
            locales.append(normalized)
            seen.add(normalized)
    return locales or [DEFAULT_LOCALE]


def _historical_store_locales(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    platform: str,
) -> list[str]:
    locales: list[str] = []
    locales.extend(
        session.scalars(
            select(StoreAppMetadataDraft.locale).where(
                StoreAppMetadataDraft.developer_account_id == account_id,
                StoreAppMetadataDraft.app_id == app_id,
                StoreAppMetadataDraft.platform == platform,
            )
        )
    )
    locales.extend(
        session.scalars(
            select(StoreReleaseNoteDraft.locale).where(
                StoreReleaseNoteDraft.developer_account_id == account_id,
                StoreReleaseNoteDraft.app_id == app_id,
                StoreReleaseNoteDraft.platform == platform,
            )
        )
    )
    locales.extend(
        session.scalars(
            select(StoreImageSuiteLocale.locale)
            .join(StoreImageSuite)
            .where(
                StoreImageSuite.developer_account_id == account_id,
                StoreImageSuite.app_id == app_id,
                StoreImageSuite.platform == platform,
            )
        )
    )
    return locales


def _localized_metadata(
    *,
    supported_locales: list[str],
    drafts_by_locale: dict[str, object],
    release_note_drafts_by_locale: dict[str, StoreReleaseNoteDraft],
    image_suite_locales_by_locale: dict[str, object],
    base_metadata: dict[str, str],
    source_locale: str,
) -> list[dict[str, object]]:
    rows: list[dict[str, object]] = []
    source_draft = image_suite_locales_by_locale.get(source_locale) or drafts_by_locale.get(
        source_locale
    )
    source_store_images = _store_images(source_draft)
    for locale in supported_locales:
        draft = drafts_by_locale.get(locale)
        release_note_draft = release_note_drafts_by_locale.get(locale)
        image_suite_locale = image_suite_locales_by_locale.get(locale)
        defaults = base_metadata if locale == source_locale else _empty_metadata()
        own_store_images = _store_images(image_suite_locale or draft)
        rows.append(
            {
                "locale": locale,
                "is_source": locale == source_locale,
                "keywords": draft.keywords if draft else defaults["keywords"],
                "promotional_text": (
                    draft.promotional_text if draft else defaults["promotional_text"]
                ),
                "description": draft.description if draft else defaults["description"],
                "release_notes": release_note_draft.release_notes if release_note_draft else "",
                "store_images": _store_images_with_fallback(
                    own_store_images,
                    fallback_images=source_store_images,
                    source_locale=source_locale,
                    inherited=locale != source_locale,
                ),
            }
        )
    return rows


def _localized_marketing_page(
    *,
    supported_locales: list[str],
    locale_rows_by_locale: dict[str, StoreMarketingPageLocale],
    source_locale: str,
) -> list[dict[str, object]]:
    rows: list[dict[str, object]] = []
    source_store_images = _store_images(locale_rows_by_locale.get(source_locale))
    for locale in supported_locales:
        row = locale_rows_by_locale.get(locale)
        own_store_images = _store_images(row)
        rows.append(
            {
                "locale": locale,
                "is_source": locale == source_locale,
                "promotional_text": row.promotional_text if row else "",
                "store_images": _store_images_with_fallback(
                    own_store_images,
                    fallback_images=source_store_images,
                    source_locale=source_locale,
                    inherited=locale != source_locale,
                ),
            }
        )
    return rows


def _marketing_page_sync_runs(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    page_id: str,
) -> list[StoreSyncRun]:
    return list(
        session.scalars(
            select(StoreSyncRun)
            .where(
                StoreSyncRun.developer_account_id == account_id,
                StoreSyncRun.app_id == app_id,
                StoreSyncRun.operation == UPDATE_MARKETING_PAGE,
                StoreSyncRun.version == page_id,
            )
            .order_by(StoreSyncRun.started_at.desc())
            .limit(20)
        )
    )


def _unique_locales(raw_locales: list[str]) -> list[str]:
    locales: list[str] = []
    seen: set[str] = set()
    for raw_locale in raw_locales:
        locale = str(raw_locale or "").strip()
        if locale and locale not in seen:
            locales.append(locale)
            seen.add(locale)
    return locales or [DEFAULT_LOCALE]


def _merge_content_set_options(
    content_sets: list[dict[str, object]],
    image_suites: list[dict[str, object]],
) -> list[dict[str, object]]:
    merged: list[dict[str, object]] = []
    seen: set[str] = set()
    for item in [*content_sets, *image_suites]:
        item_id = str(item.get("id") or "").strip()
        if not item_id or item_id in seen:
            continue
        seen.add(item_id)
        merged.append({"id": item_id, "name": str(item.get("name") or item_id)})
    return merged


def _store_image_suites_for_app(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    platform: str,
) -> list[dict[str, object]]:
    suites = list(
        session.scalars(
            select(StoreImageSuite)
            .options(selectinload(StoreImageSuite.locales))
            .where(
                StoreImageSuite.developer_account_id == account_id,
                StoreImageSuite.app_id == app_id,
                StoreImageSuite.platform == platform,
            )
            .order_by(StoreImageSuite.updated_at.desc(), StoreImageSuite.suite_name.asc())
        )
    )
    rows = [
        {
            "id": suite.suite_id,
            "name": suite.suite_name,
            "source": suite.source,
            "locale_count": len(suite.locales),
            "updated_at": suite.updated_at,
            "locales_by_locale": {item.locale: item for item in suite.locales},
        }
        for suite in suites
    ]
    if not any(item["id"] == DEFAULT_CONTENT_SET_ID for item in rows):
        rows.insert(
            0,
            {
                "id": DEFAULT_CONTENT_SET_ID,
                "name": DEFAULT_CONTENT_SET_NAME,
                "source": "admin",
                "locale_count": 0,
                "updated_at": None,
                "locales_by_locale": {},
            },
        )
    return rows


def _marketing_pages_for_app(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    platform: str,
) -> list[StoreMarketingPage]:
    return list(
        session.scalars(
            select(StoreMarketingPage)
            .options(selectinload(StoreMarketingPage.locales))
            .where(
                StoreMarketingPage.developer_account_id == account_id,
                StoreMarketingPage.app_id == app_id,
                StoreMarketingPage.platform == platform,
            )
            .order_by(StoreMarketingPage.updated_at.desc(), StoreMarketingPage.page_name.asc())
        )
    )


def _marketing_page_summary(page: StoreMarketingPage) -> dict[str, object]:
    asset_count = 0
    filled_text_count = 0
    for locale in page.locales:
        if locale.promotional_text.strip():
            filled_text_count += 1
        store_images = locale.store_images_json or {}
        for value in store_images.values():
            if isinstance(value, dict):
                assets = value.get("assets")
                if isinstance(assets, list):
                    asset_count += len(assets)
            elif isinstance(value, list):
                asset_count += len(value)
    return {
        "page": page,
        "type_label": (
            "自定义产品页面"
            if page.page_type == "custom_product_page"
            else "产品页面优化"
        ),
        "status_label": _marketing_page_status_label(page),
        "status_tone": "green" if page.status == "synced" else "warn",
        "apple_page_id_label": page.apple_page_id or "未同步后回填",
        "language_count": len(page.locales),
        "filled_text_count": filled_text_count,
        "asset_count": asset_count,
    }


def _marketing_page_status_label(page: StoreMarketingPage) -> str:
    if page.status == "synced":
        return "已同步"
    if page.apple_page_id:
        return "待更新"
    return "未同步"


def _sync_history_groups(runs: list[StoreSyncRun]) -> list[dict[str, object]]:
    grouped: dict[str, list[StoreSyncRun]] = {}
    for run in runs:
        grouped.setdefault(run.version or "-", []).append(run)
    return [
        {
            "version": version,
            "runs": version_runs,
            "latest_at": version_runs[0].started_at if version_runs else None,
        }
        for version, version_runs in grouped.items()
    ]


def _release_note_drafts_for_scope(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    platform: str,
    version: str,
) -> dict[str, StoreReleaseNoteDraft]:
    drafts = session.scalars(
        select(StoreReleaseNoteDraft).where(
            StoreReleaseNoteDraft.developer_account_id == account_id,
            StoreReleaseNoteDraft.app_id == app_id,
            StoreReleaseNoteDraft.platform == platform,
            StoreReleaseNoteDraft.version == version,
        )
    )
    return {draft.locale: draft for draft in drafts}


def _empty_metadata() -> dict[str, str]:
    return {
        "keywords": "",
        "promotional_text": "",
        "description": "",
    }


def _store_images(draft: object | None) -> dict[str, object]:
    raw_images = getattr(draft, "store_images_json", None) if draft else None
    if not isinstance(raw_images, dict):
        return _empty_store_images()
    images = _empty_store_images()
    for key in ("feature_graphic_url", "phone_screenshots", "tablet_screenshots"):
        images[key] = _store_image_slot(raw_images.get(key))
    return images


def _empty_store_images() -> dict[str, object]:
    return {
        "feature_graphic_url": _store_image_slot(None),
        "phone_screenshots": _store_image_slot(None),
        "tablet_screenshots": _store_image_slot(None),
    }


def _store_image_slot(raw_value: object) -> dict[str, object]:
    if isinstance(raw_value, dict):
        urls = _string_list(raw_value.get("urls"))
        assets = _asset_list(raw_value.get("assets"))
    else:
        urls = _string_list(raw_value)
        assets = []
    asset_previews = [
        {
            "url": _admin_artifact_url(asset["storageKey"]) or asset["downloadUrl"],
            "fileName": asset["fileName"],
            "width": asset.get("width"),
            "height": asset.get("height"),
            "validationMessage": asset.get("validationMessage"),
            "matchedLabel": asset.get("matchedLabel"),
            "format": asset.get("format"),
            "storageKey": asset.get("storageKey"),
            "inherited": False,
            "sourceLocale": "",
            "canDelete": bool(asset.get("storageKey")),
        }
        for asset in assets
        if asset.get("storageKey") or asset.get("downloadUrl")
    ]
    url_previews = [
        {
            "url": url,
            "fileName": url.rsplit("/", 1)[-1],
            "width": None,
            "height": None,
            "validationMessage": "",
            "matchedLabel": "",
            "format": "",
            "storageKey": "",
            "inherited": False,
            "sourceLocale": "",
            "canDelete": False,
        }
        for url in urls
    ]
    preview_items = [*asset_previews, *url_previews]
    preview_urls = [
        *[item["url"] for item in preview_items],
    ]
    file_names = [
        *[item["fileName"] for item in preview_items],
    ]
    return {
        "urls": urls,
        "assets": assets,
        "value": "\n".join(urls),
        "preview_items": preview_items,
        "preview_urls": preview_urls,
        "file_names": file_names,
        "count": len(preview_urls),
        "inherited": False,
        "source_locale": "",
    }


def _store_images_with_fallback(
    images: dict[str, object],
    *,
    fallback_images: dict[str, object],
    source_locale: str,
    inherited: bool,
) -> dict[str, object]:
    if not inherited:
        return images
    merged: dict[str, object] = {}
    for key in ("feature_graphic_url", "phone_screenshots", "tablet_screenshots"):
        current = images.get(key)
        if _store_image_slot_has_preview(current):
            merged[key] = current
            continue
        fallback = fallback_images.get(key)
        merged[key] = (
            _inherited_store_image_slot(fallback, source_locale)
            if _store_image_slot_has_preview(fallback)
            else current or _store_image_slot(None)
        )
    return merged


def _store_image_slot_has_preview(slot: object) -> bool:
    if not isinstance(slot, dict):
        return False
    preview_items = slot.get("preview_items")
    if isinstance(preview_items, list | tuple) and preview_items:
        return True
    return bool(_string_list(slot.get("urls")) or _asset_list(slot.get("assets")))


def _inherited_store_image_slot(slot: object, source_locale: str) -> dict[str, object]:
    if not isinstance(slot, dict):
        return _store_image_slot(None)
    preview_items = [
        {
            **item,
            "inherited": True,
            "sourceLocale": source_locale,
            "canDelete": False,
        }
        for item in slot.get("preview_items", [])
        if isinstance(item, dict)
    ]
    preview_urls = [str(item.get("url") or "") for item in preview_items if item.get("url")]
    file_names = [str(item.get("fileName") or "") for item in preview_items if item.get("fileName")]
    return {
        "urls": [],
        "assets": [],
        "value": "",
        "preview_items": preview_items,
        "preview_urls": preview_urls,
        "file_names": file_names,
        "count": len(preview_urls),
        "inherited": True,
        "source_locale": source_locale,
    }


def _string_list(value: object) -> list[str]:
    if isinstance(value, list | tuple):
        candidates = [str(item or "") for item in value]
    else:
        candidates = str(value or "").splitlines()
    return [candidate.strip() for candidate in candidates if candidate.strip()]


def _asset_list(value: object) -> list[dict[str, object]]:
    if not isinstance(value, list | tuple):
        return []
    assets: list[dict[str, object]] = []
    for item in value:
        if not isinstance(item, dict):
            continue
        download_url = str(item.get("downloadUrl") or "").strip()
        storage_key = str(item.get("storageKey") or "").strip()
        if not download_url and not storage_key:
            continue
        assets.append(
            {
                "fileName": str(item.get("fileName") or "").strip(),
                "contentType": str(item.get("contentType") or "").strip(),
                "sizeBytes": int(item.get("sizeBytes") or 0),
                "storageKey": storage_key,
                "downloadUrl": download_url,
                "width": int(item.get("width") or 0) or None,
                "height": int(item.get("height") or 0) or None,
                "format": str(item.get("format") or "").strip(),
                "validationMessage": str(item.get("validationMessage") or "").strip(),
                "matchedLabel": str(item.get("matchedLabel") or "").strip(),
            }
        )
    return assets


def _admin_artifact_url(storage_key: object) -> str:
    value = str(storage_key or "").strip()
    if not value:
        return ""
    return "/admin/artifacts/" + "/".join(quote(part) for part in value.split("/"))


def _source_locale(supported_locales: list[str]) -> str:
    if "en-US" in supported_locales:
        return "en-US"
    if "en" in supported_locales:
        return "en"
    return supported_locales[0] if supported_locales else DEFAULT_LOCALE


def _metadata_fields(platform: str) -> list[dict[str, object]]:
    if platform == "android":
        return [
            {
                "key": "description",
                "name": "description",
                "label": "Full description（完整描述）",
                "short_label": "完整描述",
                "type": "textarea",
                "rows": 8,
                "required": True,
                "placeholder": "Google Play full description",
            },
        ]
    return [
        {
            "key": "promotional_text",
            "name": "promotionalText",
            "label": "Promotional Text（宣传文本）",
            "short_label": "宣传文本",
            "type": "textarea",
            "rows": 3,
            "required": False,
            "placeholder": "App Store Connect promotional text",
        },
        {
            "key": "description",
            "name": "description",
            "label": "Description（描述）",
            "short_label": "描述",
            "type": "textarea",
            "rows": 8,
            "required": True,
            "placeholder": "App Store Connect description",
        },
    ]


def _store_image_slots(platform: str) -> list[dict[str, object]]:
    if platform == "android":
        return [
            {
                "key": "feature_graphic_url",
                "name": "featureGraphicUrl",
                "label": "Feature graphic（功能宣传图）",
                "short_label": "功能宣传图",
                "hint": "Google Play 的横向展示图，常用尺寸为 1024 x 500。",
                "type": "input",
                "rows": 1,
                "placeholder": "Google Play feature graphic URL",
                "multiple": False,
                "requirement": store_image_requirement(platform, "feature_graphic_url"),
            },
            {
                "key": "phone_screenshots",
                "name": "phoneScreenshots",
                "label": "Phone screenshots（手机截图）",
                "short_label": "手机截图",
                "hint": "Google Play 手机设备截图。",
                "type": "textarea",
                "rows": 3,
                "placeholder": "一行一个 phone screenshot URL",
                "multiple": True,
                "requirement": store_image_requirement(platform, "phone_screenshots"),
            },
            {
                "key": "tablet_screenshots",
                "name": "tabletScreenshots",
                "label": "Tablet screenshots（平板截图）",
                "short_label": "平板截图",
                "hint": "Google Play 平板设备截图。",
                "type": "textarea",
                "rows": 3,
                "placeholder": "一行一个 tablet screenshot URL",
                "multiple": True,
                "requirement": store_image_requirement(platform, "tablet_screenshots"),
            },
        ]
    return [
        {
            "key": "phone_screenshots",
            "name": "phoneScreenshots",
            "label": "iPhone screenshots（iPhone 屏幕快照）",
            "short_label": "手机截图",
            "hint": "App Store Connect 的 iPhone 屏幕快照。",
            "type": "textarea",
            "rows": 3,
            "placeholder": "一行一个 iPhone screenshot URL",
            "multiple": True,
            "requirement": store_image_requirement(platform, "phone_screenshots"),
        },
        {
            "key": "tablet_screenshots",
            "name": "tabletScreenshots",
            "label": "iPad screenshots（iPad 屏幕快照）",
            "short_label": "平板截图",
            "hint": "App Store Connect 的 iPad 屏幕快照。",
            "type": "textarea",
            "rows": 3,
            "placeholder": "一行一个 iPad screenshot URL",
            "multiple": True,
            "requirement": store_image_requirement(platform, "tablet_screenshots"),
        },
    ]


def _single_store_platform(apps: list[App]) -> str:
    platforms = {app.platform for app in apps if app.platform in {"ios", "android"}}
    if len(platforms) == 1:
        return next(iter(platforms))
    return "mixed"


def _store_label(platform: str) -> str:
    if platform == "android":
        return "Google Play Console"
    if platform == "ios":
        return "App Store Connect"
    return "商店"


def list_notifications(session: Session, *, limit: int | None = None) -> list[Notification]:
    statement = select(Notification).order_by(Notification.created_at.desc())
    if limit is not None:
        statement = statement.limit(limit)
    return list(session.scalars(statement))


def format_datetime(value: datetime | None) -> str:
    if value is None:
        return "-"
    timestamp = value if value.tzinfo else value.replace(tzinfo=UTC)
    return timestamp.astimezone(UTC).strftime("%Y-%m-%d %H:%M")


def format_size(size_bytes: int | None) -> str:
    if not size_bytes:
        return "0 B"
    units = ["B", "KB", "MB", "GB"]
    value = float(size_bytes)
    unit_index = 0
    while value >= 1024 and unit_index < len(units) - 1:
        value /= 1024
        unit_index += 1
    return f"{value:.1f} {units[unit_index]}" if unit_index else f"{int(value)} B"


def environment_label(value: str) -> str:
    return "线上环境" if value == "production" else "开发环境"


def platform_label(value: str) -> str:
    return "iOS" if value == "ios" else "Android"


def account_status_label(value: str) -> str:
    return {
        "ok": "正常",
        "renewal_due": "需要续费",
        "expired": "已过期",
        "disabled": "已停用",
    }.get(value, value)


def remaining_days(expires_at: datetime) -> int:
    now = datetime.now(UTC)
    expires = expires_at if expires_at.tzinfo else expires_at.replace(tzinfo=UTC)
    return (expires - now).days

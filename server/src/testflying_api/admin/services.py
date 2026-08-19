from __future__ import annotations

import re
from datetime import UTC, datetime
from uuid import uuid4

from sqlalchemy import delete, select
from sqlalchemy.orm import Session

from testflying_api.errors import ApiError
from testflying_api.schema import App, DeveloperAccount, DeveloperAccountApp
from testflying_api.store_identifiers import normalize_store_identifiers

ACCOUNT_STATUSES = ("ok", "renewal_due", "expired", "disabled")


def list_account_options(session: Session) -> list[DeveloperAccount]:
    return list(
        session.scalars(select(DeveloperAccount).order_by(DeveloperAccount.team_name.asc()))
    )


def list_unassigned_apps(session: Session) -> list[App]:
    legacy_bound_ids = select(DeveloperAccountApp.app_id)
    return list(
        session.scalars(
            select(App)
            .where(App.developer_account_id.is_(None), App.id.not_in(legacy_bound_ids))
            .order_by(App.added_at.desc(), App.name.asc())
        )
    )


def save_developer_account(
    session: Session,
    *,
    account_id: str,
    team_name: str,
    expires_at: datetime,
    status: str,
    renewal_action_label: str,
) -> DeveloperAccount:
    normalized_id = _normalize_account_id(account_id)
    normalized_name = team_name.strip()
    normalized_status = status.strip() or "renewal_due"
    if not normalized_name:
        raise ApiError("invalid_account", "Team 名称不能为空", status_code=422)
    if normalized_status not in ACCOUNT_STATUSES:
        raise ApiError("invalid_account", "账号状态不合法", status_code=422)

    account = session.get(DeveloperAccount, normalized_id)
    if account is None:
        account = DeveloperAccount(
            id=normalized_id,
            team_name=normalized_name,
            expires_at=expires_at,
            status=normalized_status,
            renewal_action_label=renewal_action_label.strip() or "去续费",
        )
        session.add(account)
    else:
        account.team_name = normalized_name
        account.expires_at = expires_at
        account.status = normalized_status
        account.renewal_action_label = renewal_action_label.strip() or "去续费"
    session.flush()
    return account


def bind_app_to_account(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    store_app_id: str,
    store_package_name: str,
) -> App:
    account = session.get(DeveloperAccount, account_id)
    if account is None:
        raise ApiError("account_not_found", "开发者账号不存在", status_code=404)
    app = session.get(App, app_id)
    if app is None:
        raise ApiError("app_not_found", "App 不存在", status_code=404)
    if app.developer_account_id and app.developer_account_id != account_id:
        raise ApiError(
            "app_bound_to_other_account",
            "该 App 已绑定到其他开发者账号，请先解绑后再绑定。",
            status_code=422,
        )
    legacy_account_ids = set(
        session.scalars(
            select(DeveloperAccountApp.developer_account_id).where(
                DeveloperAccountApp.app_id == app_id
            )
        )
    )
    if legacy_account_ids and legacy_account_ids != {account_id}:
        raise ApiError(
            "app_bound_to_other_account",
            "该 App 已通过旧关系绑定到其他开发者账号，请先解绑后再绑定。",
            status_code=422,
        )

    normalized_store_app_id, normalized_store_package = normalize_store_identifiers(
        platform=app.platform,
        store_app_id=store_app_id,
        store_package_name=store_package_name,
    )
    app.developer_account_id = account_id
    app.store_app_id = normalized_store_app_id
    app.store_package_name = normalized_store_package
    session.execute(delete(DeveloperAccountApp).where(DeveloperAccountApp.app_id == app_id))
    session.flush()
    return app


def update_bound_app_store_settings(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    store_app_id: str,
    store_package_name: str,
) -> App:
    app = session.get(App, app_id)
    if app is None:
        raise ApiError("app_not_found", "App 不存在", status_code=404)
    if app.developer_account_id != account_id:
        legacy_link = session.get(DeveloperAccountApp, (account_id, app_id))
        if legacy_link is None or app.developer_account_id is not None:
            raise ApiError("app_not_found", "当前开发者账号下没有这个 App", status_code=404)
        app.developer_account_id = account_id
        session.execute(delete(DeveloperAccountApp).where(DeveloperAccountApp.app_id == app_id))
    normalized_store_app_id, normalized_store_package = normalize_store_identifiers(
        platform=app.platform,
        store_app_id=store_app_id,
        store_package_name=store_package_name,
    )
    app.store_app_id = normalized_store_app_id
    app.store_package_name = normalized_store_package
    session.flush()
    return app


def unbind_app_from_account(session: Session, *, account_id: str, app_id: str) -> None:
    app = session.get(App, app_id)
    if app is None:
        raise ApiError("app_not_found", "App 不存在", status_code=404)
    if app.developer_account_id == account_id:
        app.developer_account_id = None
        app.store_app_id = None
        app.store_package_name = None
    session.execute(
        delete(DeveloperAccountApp).where(
            DeveloperAccountApp.developer_account_id == account_id,
            DeveloperAccountApp.app_id == app_id,
        )
    )
    session.flush()


def parse_admin_datetime(value: str) -> datetime:
    normalized = value.strip()
    if not normalized:
        raise ApiError("invalid_account", "过期时间不能为空", status_code=422)
    try:
        parsed = datetime.fromisoformat(normalized)
    except ValueError as error:
        raise ApiError("invalid_account", "过期时间格式不合法", status_code=422) from error
    if parsed.tzinfo is None:
        return parsed.replace(tzinfo=UTC)
    return parsed.astimezone(UTC)


def next_account_id(team_name: str) -> str:
    slug = re.sub(r"[^a-zA-Z0-9]+", "-", team_name).strip("-").lower()[:42]
    return f"account-{slug or uuid4().hex[:10]}"


def _normalize_account_id(value: str) -> str:
    normalized = value.strip()
    if not normalized:
        raise ApiError("invalid_account", "账号 ID 不能为空", status_code=422)
    if not re.fullmatch(r"[a-zA-Z0-9][a-zA-Z0-9_.:-]{1,78}", normalized):
        raise ApiError(
            "invalid_account",
            "账号 ID 只能使用字母、数字、点、下划线、冒号和短横线。",
            status_code=422,
        )
    return normalized


def _empty_to_none(value: str) -> str | None:
    normalized = value.strip()
    return normalized or None

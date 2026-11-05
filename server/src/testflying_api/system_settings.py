from __future__ import annotations

from dataclasses import dataclass
from math import isfinite
from string import Formatter
from urllib.parse import urlsplit
from uuid import uuid4

from sqlalchemy import select
from sqlalchemy.orm import Session

from testflying_api.config import Settings
from testflying_api.schema import AuditLog, SystemSetting, utc_now

DATABASE_SETTING_KEYS = frozenset(
    {
        "connector_base_url_template",
        "dingtalk_enabled",
        "dingtalk_webhook_url",
        "dingtalk_secret",
        "dingtalk_timeout_seconds",
        "dingtalk_dispatch_interval_seconds",
    }
)


@dataclass(frozen=True)
class EffectiveBusinessSettings:
    connector_base_url_template: str | None
    dingtalk_enabled: bool
    dingtalk_webhook_url: str | None
    dingtalk_secret: str | None
    dingtalk_timeout_seconds: float
    dingtalk_dispatch_interval_seconds: float

    @property
    def dingtalk_configured(self) -> bool:
        return bool(
            self.dingtalk_enabled
            and self.dingtalk_webhook_url
            and self.dingtalk_secret
        )


def effective_business_settings(
    session: Session,
    environment: Settings,
) -> EffectiveBusinessSettings:
    rows = {
        row.key: row
        for row in session.scalars(select(SystemSetting))
        if row.key in DATABASE_SETTING_KEYS
    }
    return EffectiveBusinessSettings(
        connector_base_url_template=_optional_value(
            rows,
            "connector_base_url_template",
            environment.connector_base_url_template,
        ),
        dingtalk_enabled=_boolean_value(
            rows,
            "dingtalk_enabled",
            default=environment.dingtalk_configured,
        ),
        dingtalk_webhook_url=_optional_value(
            rows,
            "dingtalk_webhook_url",
            environment.dingtalk_webhook_url,
        ),
        dingtalk_secret=_optional_value(
            rows,
            "dingtalk_secret",
            environment.dingtalk_secret,
        ),
        dingtalk_timeout_seconds=_positive_float_value(
            rows,
            "dingtalk_timeout_seconds",
            environment.dingtalk_timeout_seconds,
        ),
        dingtalk_dispatch_interval_seconds=_positive_float_value(
            rows,
            "dingtalk_dispatch_interval_seconds",
            environment.dingtalk_dispatch_interval_seconds,
        ),
    )


def save_general_settings(
    session: Session,
    *,
    connector_base_url_template: str | None,
    actor: str,
) -> None:
    normalized = (connector_base_url_template or "").strip()
    _validate_connector_template(normalized)
    _upsert_setting(
        session,
        key="connector_base_url_template",
        value=normalized,
        is_secret=False,
    )
    _record_setting_audit(session, actor=actor, keys=["connector_base_url_template"])
    session.commit()


def save_notification_settings(
    session: Session,
    *,
    enabled: bool,
    webhook_url: str | None,
    secret: str | None,
    timeout_seconds: float,
    dispatch_interval_seconds: float,
    actor: str,
) -> None:
    normalized_timeout = _require_positive(timeout_seconds, "timeout_seconds")
    normalized_interval = _require_positive(
        dispatch_interval_seconds,
        "dispatch_interval_seconds",
    )
    normalized_webhook_url = (webhook_url or "").strip()
    if normalized_webhook_url:
        _validate_http_url(normalized_webhook_url, "webhook_url")
    _upsert_setting(session, key="dingtalk_enabled", value=str(enabled).lower())
    if normalized_webhook_url:
        _upsert_setting(
            session,
            key="dingtalk_webhook_url",
            value=normalized_webhook_url,
            is_secret=True,
        )
    if secret is not None and secret.strip():
        _upsert_setting(
            session,
            key="dingtalk_secret",
            value=secret.strip(),
            is_secret=True,
        )
    _upsert_setting(
        session,
        key="dingtalk_timeout_seconds",
        value=str(normalized_timeout),
    )
    _upsert_setting(
        session,
        key="dingtalk_dispatch_interval_seconds",
        value=str(normalized_interval),
    )
    _record_setting_audit(
        session,
        actor=actor,
        keys=[
            "dingtalk_enabled",
            "dingtalk_webhook_url",
            "dingtalk_secret",
            "dingtalk_timeout_seconds",
            "dingtalk_dispatch_interval_seconds",
        ],
    )
    session.commit()


def database_setting_keys(session: Session) -> set[str]:
    return set(
        session.scalars(
            select(SystemSetting.key).where(SystemSetting.key.in_(DATABASE_SETTING_KEYS))
        )
    )


def _optional_value(
    rows: dict[str, SystemSetting],
    key: str,
    default: str | None,
) -> str | None:
    row = rows.get(key)
    if row is None:
        return default
    value = row.value.strip()
    return value or None


def _boolean_value(
    rows: dict[str, SystemSetting],
    key: str,
    *,
    default: bool,
) -> bool:
    row = rows.get(key)
    if row is None:
        return default
    normalized = row.value.strip().lower()
    if normalized == "true":
        return True
    if normalized == "false":
        return False
    return default


def _positive_float_value(
    rows: dict[str, SystemSetting],
    key: str,
    default: float,
) -> float:
    row = rows.get(key)
    if row is None:
        return default
    try:
        value = float(row.value)
    except ValueError:
        return default
    return value if isfinite(value) and value > 0 else default


def _require_positive(value: float, field: str) -> float:
    normalized = float(value)
    if not isfinite(normalized) or normalized <= 0:
        raise ValueError(f"{field} must be a positive number")
    return normalized


def _validate_connector_template(value: str) -> None:
    if not value:
        return
    try:
        fields = list(Formatter().parse(value))
    except ValueError as error:
        raise ValueError("connector template only supports {account_id}") from error
    for _literal, field_name, format_spec, conversion in fields:
        if field_name is None:
            continue
        if field_name != "account_id" or format_spec or conversion:
            raise ValueError("connector template only supports {account_id}")


def _validate_http_url(value: str, field: str) -> None:
    parsed = urlsplit(value)
    if parsed.scheme not in {"http", "https"} or not parsed.hostname:
        raise ValueError(f"{field} must be an HTTP URL")


def _upsert_setting(
    session: Session,
    *,
    key: str,
    value: str,
    is_secret: bool = False,
) -> None:
    row = session.get(SystemSetting, key)
    if row is None:
        row = SystemSetting(key=key, value=value, is_secret=is_secret)
        session.add(row)
        return
    row.value = value
    row.is_secret = is_secret
    row.updated_at = utc_now()


def _record_setting_audit(
    session: Session,
    *,
    actor: str,
    keys: list[str],
) -> None:
    session.add(
        AuditLog(
            id=f"audit-{uuid4().hex[:12]}",
            developer_account_id=None,
            actor=actor,
            action="system_settings.update",
            target_type="system_settings",
            target_id=",".join(keys),
        )
    )

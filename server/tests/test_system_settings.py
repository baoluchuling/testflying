from __future__ import annotations

from dataclasses import replace

import pytest
from sqlalchemy import select
from sqlalchemy.orm import Session

from testflying_api.config import Settings
from testflying_api.schema import AuditLog, SystemSetting
from testflying_api.system_settings import (
    effective_business_settings,
    save_general_settings,
    save_notification_settings,
)


def test_effective_business_settings_prefers_database(
    db_session: Session,
    test_settings: Settings,
) -> None:
    db_session.add_all(
        [
            SystemSetting(key="dingtalk_enabled", value="false", is_secret=False),
            SystemSetting(
                key="dingtalk_webhook_url",
                value="https://db.example.test/robot/send",
                is_secret=True,
            ),
            SystemSetting(key="dingtalk_secret", value="SEC-db", is_secret=True),
        ]
    )
    db_session.commit()

    effective = effective_business_settings(db_session, test_settings)

    assert effective.dingtalk_enabled is False
    assert effective.dingtalk_webhook_url == "https://db.example.test/robot/send"
    assert effective.dingtalk_secret == "SEC-db"
    assert effective.dingtalk_configured is False


def test_effective_business_settings_falls_back_to_environment(
    db_session: Session,
    test_settings: Settings,
) -> None:
    settings = replace(
        test_settings,
        connector_base_url_template="https://connector-{account_id}.example.test",
        dingtalk_webhook_url="https://env.example.test/robot/send",
        dingtalk_secret="SEC-env",
        dingtalk_timeout_seconds=7.0,
        dingtalk_dispatch_interval_seconds=12.0,
    )

    effective = effective_business_settings(db_session, settings)

    assert effective.connector_base_url_template == (
        "https://connector-{account_id}.example.test"
    )
    assert effective.dingtalk_enabled is True
    assert effective.dingtalk_timeout_seconds == 7.0
    assert effective.dingtalk_dispatch_interval_seconds == 12.0


def test_save_notification_settings_keeps_blank_secret_and_records_audit(
    db_session: Session,
) -> None:
    db_session.add(
        SystemSetting(key="dingtalk_secret", value="SEC-existing", is_secret=True)
    )
    db_session.commit()

    save_notification_settings(
        db_session,
        enabled=True,
        webhook_url="https://oapi.example.test/robot/send",
        secret=" ",
        timeout_seconds=8,
        dispatch_interval_seconds=15,
        actor="admin",
    )

    assert db_session.get(SystemSetting, "dingtalk_secret").value == "SEC-existing"
    audit = db_session.scalar(select(AuditLog).where(AuditLog.target_type == "system_settings"))
    assert audit is not None
    assert audit.actor == "admin"
    assert audit.action == "system_settings.update"


def test_save_general_settings_normalizes_empty_value(db_session: Session) -> None:
    save_general_settings(
        db_session,
        connector_base_url_template="  ",
        actor="admin",
    )

    row = db_session.get(SystemSetting, "connector_base_url_template")
    assert row is not None
    assert row.value == ""


def test_save_general_settings_rejects_unknown_connector_placeholder(
    db_session: Session,
) -> None:
    with pytest.raises(ValueError, match="account_id"):
        save_general_settings(
            db_session,
            connector_base_url_template="https://connector-{accountId}.example.test",
            actor="admin",
        )

    assert db_session.get(SystemSetting, "connector_base_url_template") is None


def test_invalid_notification_numbers_do_not_mutate_rows(db_session: Session) -> None:
    with pytest.raises(ValueError, match="timeout_seconds"):
        save_notification_settings(
            db_session,
            enabled=True,
            webhook_url="https://oapi.example.test/robot/send",
            secret="SEC-new",
            timeout_seconds=0,
            dispatch_interval_seconds=10,
            actor="admin",
        )

    assert db_session.get(SystemSetting, "dingtalk_webhook_url") is None
    assert db_session.get(SystemSetting, "dingtalk_secret") is None


@pytest.mark.parametrize(
    ("timeout_seconds", "dispatch_interval_seconds", "field"),
    [
        (float("inf"), 10, "timeout_seconds"),
        (5, float("nan"), "dispatch_interval_seconds"),
    ],
)
def test_non_finite_notification_numbers_do_not_mutate_rows(
    db_session: Session,
    timeout_seconds: float,
    dispatch_interval_seconds: float,
    field: str,
) -> None:
    with pytest.raises(ValueError, match=field):
        save_notification_settings(
            db_session,
            enabled=True,
            webhook_url="https://oapi.example.test/robot/send",
            secret="SEC-new",
            timeout_seconds=timeout_seconds,
            dispatch_interval_seconds=dispatch_interval_seconds,
            actor="admin",
        )

    assert db_session.get(SystemSetting, "dingtalk_webhook_url") is None
    assert db_session.get(SystemSetting, "dingtalk_secret") is None


def test_invalid_notification_webhook_does_not_mutate_rows(db_session: Session) -> None:
    with pytest.raises(ValueError, match="webhook_url"):
        save_notification_settings(
            db_session,
            enabled=True,
            webhook_url="not-a-webhook?access_token=never-store",
            secret="SEC-new",
            timeout_seconds=5,
            dispatch_interval_seconds=10,
            actor="admin",
        )

    assert db_session.get(SystemSetting, "dingtalk_webhook_url") is None
    assert db_session.get(SystemSetting, "dingtalk_secret") is None

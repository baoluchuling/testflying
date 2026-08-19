from __future__ import annotations

from base64 import b64encode

from fastapi.testclient import TestClient
from sqlalchemy.orm import Session

from testflying_api.admin_api import routes as admin_routes
from testflying_api.schema import SystemSetting, WebhookDelivery


def _admin_headers(password: str = "dev-token") -> dict[str, str]:
    token = b64encode(f"admin:{password}".encode()).decode()
    return {"Authorization": f"Basic {token}"}


def test_settings_api_never_returns_secrets(
    client: TestClient,
    db_session: Session,
) -> None:
    client.app.state.settings = client.app.state.settings.__class__(
        **{
            **client.app.state.settings.__dict__,
            "database_url": "postgresql://secret-user:secret-password@db/testflying",
            "s3_secret_access_key": "s3-never-return",
        }
    )
    db_session.add_all(
        [
            SystemSetting(
                key="dingtalk_webhook_url",
                value="https://oapi.test/robot/send?access_token=never-return",
                is_secret=True,
            ),
            SystemSetting(key="dingtalk_secret", value="SEC-never-return", is_secret=True),
        ]
    )
    db_session.commit()

    response = client.get("/admin/api/settings", headers=_admin_headers())

    assert response.status_code == 200
    assert response.json()["notifications"]["webhookConfigured"] is True
    assert response.json()["notifications"]["secretConfigured"] is True
    for secret in (
        "never-return",
        "secret-user",
        "secret-password",
        "dev-token",
        "s3-never-return",
    ):
        assert secret not in response.text


def test_notification_settings_blank_secret_keeps_existing(
    client: TestClient,
    db_session: Session,
) -> None:
    db_session.add(
        SystemSetting(key="dingtalk_secret", value="SEC-existing", is_secret=True)
    )
    db_session.commit()

    response = client.put(
        "/admin/api/settings/notifications",
        headers=_admin_headers(),
        json={
            "enabled": True,
            "webhookUrl": "https://oapi.test/robot/send?access_token=updated",
            "secret": "",
            "timeoutSeconds": 5,
            "dispatchIntervalSeconds": 10,
        },
    )

    assert response.status_code == 200
    assert db_session.get(SystemSetting, "dingtalk_secret").value == "SEC-existing"
    assert "SEC-existing" not in response.text
    assert response.json()["message"] == "通知配置已保存"


def test_settings_api_rejects_invalid_connector_template(
    client: TestClient,
    db_session: Session,
) -> None:
    response = client.put(
        "/admin/api/settings/general",
        headers=_admin_headers(),
        json={"connectorBaseUrlTemplate": "https://connector-{accountId}.example.test"},
    )

    assert response.status_code == 422
    assert db_session.get(SystemSetting, "connector_base_url_template") is None


def test_settings_api_rejects_invalid_notification_webhook(
    client: TestClient,
    db_session: Session,
) -> None:
    response = client.put(
        "/admin/api/settings/notifications",
        headers=_admin_headers(),
        json={
            "enabled": True,
            "webhookUrl": "not-a-webhook?access_token=never-store",
            "secret": "SEC-never-store",
            "timeoutSeconds": 5,
            "dispatchIntervalSeconds": 10,
        },
    )

    assert response.status_code == 422
    assert db_session.get(SystemSetting, "dingtalk_webhook_url") is None
    assert db_session.get(SystemSetting, "dingtalk_secret") is None


def test_settings_api_rejects_non_finite_notification_interval(
    client: TestClient,
    db_session: Session,
) -> None:
    response = client.put(
        "/admin/api/settings/notifications",
        headers={**_admin_headers(), "Content-Type": "application/json"},
        content=(
            '{"enabled":true,'
            '"webhookUrl":"https://oapi.test/robot/send?access_token=valid",'
            '"secret":"SEC-never-store",'
            '"timeoutSeconds":5,'
            '"dispatchIntervalSeconds":Infinity}'
        ),
    )

    assert response.status_code == 422
    assert db_session.get(SystemSetting, "dingtalk_webhook_url") is None
    assert db_session.get(SystemSetting, "dingtalk_secret") is None


def test_notification_check_uses_effective_database_credentials(
    client: TestClient,
    db_session: Session,
    monkeypatch,
) -> None:
    db_session.add_all(
        [
            SystemSetting(key="dingtalk_enabled", value="true"),
            SystemSetting(
                key="dingtalk_webhook_url",
                value="https://oapi.test/robot/send?access_token=db-token",
                is_secret=True,
            ),
            SystemSetting(key="dingtalk_secret", value="SEC-db", is_secret=True),
        ]
    )
    db_session.commit()
    sent: list[dict[str, object]] = []
    monkeypatch.setattr(
        admin_routes,
        "send_dingtalk_markdown",
        lambda **payload: sent.append(payload),
        raising=False,
    )

    response = client.post(
        "/admin/api/settings/notifications/check",
        headers=_admin_headers(),
        json={},
    )

    assert response.status_code == 200
    assert response.json()["message"] == "钉钉配置检查消息已发送"
    assert sent[0]["title"] == "TestFlying 配置检查"
    assert sent[0]["secret"] == "SEC-db"


def test_settings_api_returns_delivery_counts(
    client: TestClient,
    db_session: Session,
) -> None:
    db_session.add_all(
        [
            WebhookDelivery(
                id="delivery-pending-settings",
                channel="dingtalk",
                event_key="settings:pending",
                status="pending",
                payload_json={},
            ),
            WebhookDelivery(
                id="delivery-dead-settings",
                channel="dingtalk",
                event_key="settings:dead",
                status="dead",
                payload_json={},
            ),
        ]
    )
    db_session.commit()

    response = client.get("/admin/api/settings", headers=_admin_headers())

    assert response.status_code == 200
    assert response.json()["notifications"]["pendingDeliveryCount"] == 1
    assert response.json()["notifications"]["deadDeliveryCount"] == 1

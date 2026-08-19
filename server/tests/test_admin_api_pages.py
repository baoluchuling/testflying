from __future__ import annotations

from base64 import b64encode
from dataclasses import replace

from fastapi.testclient import TestClient
from sqlalchemy.orm import Session

from testflying_api.schema import WebhookDelivery
from testflying_api.seed import seed_demo_catalog


def _admin_headers(password: str = "dev-token") -> dict[str, str]:
    token = b64encode(f"admin:{password}".encode()).decode()
    return {"Authorization": f"Basic {token}"}


def test_admin_api_dashboard_returns_overview(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.get("/admin/api/dashboard", headers=_admin_headers())

    assert response.status_code == 200
    payload = response.json()
    assert [item["label"] for item in payload["stats"]] == [
        "应用",
        "构建",
        "iOS / Android",
        "续费提醒",
    ]
    assert payload["recentBuilds"][0]["app"]["name"] == "DataFlow"
    assert payload["recentNotifications"][0]["title"] == "Apple 开发者账号即将到期"


def test_admin_api_builds_returns_artifact_links(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.get("/admin/api/builds", headers=_admin_headers())

    assert response.status_code == 200
    payload = response.json()
    assert payload["total"] == 3
    first = payload["builds"][0]
    assert first["app"]["name"] == "DataFlow"
    assert first["artifact"]["installUrl"].endswith("DataFlow.apk")
    assert first["environmentLabel"] == "开发环境"


def test_admin_api_devices_returns_registered_devices(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.get("/admin/api/devices", headers=_admin_headers())

    assert response.status_code == 200
    payload = response.json()
    assert payload["total"] == 2
    names = {device["name"] for device in payload["devices"]}
    assert names == {"iPhone 15 Pro", "Pixel 8"}


def test_admin_api_notifications_filters_by_type(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.get("/admin/api/notifications?type=device", headers=_admin_headers())

    assert response.status_code == 200
    payload = response.json()
    assert payload["activeType"] == "device"
    assert payload["total"] == 1
    assert payload["notifications"][0]["title"] == "iPhone 15 Pro 已登记"
    assert {item["type"] for item in payload["typeCounts"]} == {
        "all",
        "account",
        "build",
        "device",
    }


def test_admin_api_notifications_returns_dingtalk_status_without_secrets(
    client: TestClient,
    db_session: Session,
) -> None:
    client.app.state.settings = replace(
        client.app.state.settings,
        dingtalk_webhook_url=(
            "https://oapi.dingtalk.test/robot/send?access_token=never-return-this"
        ),
        dingtalk_secret="SEC-never-return-this",
    )
    db_session.add_all(
        [
            WebhookDelivery(
                id="delivery-pending",
                channel="dingtalk",
                event_key="build:pending:failed:dingtalk",
                status="pending",
                payload_json={},
            ),
            WebhookDelivery(
                id="delivery-dead",
                channel="dingtalk",
                event_key="build:dead:failed:dingtalk",
                status="dead",
                payload_json={},
            ),
        ]
    )
    db_session.commit()

    response = client.get("/admin/api/notifications", headers=_admin_headers())

    assert response.status_code == 200
    assert response.json()["dingtalk"] == {
        "configured": True,
        "webhookConfigured": True,
        "secretConfigured": True,
        "triggers": ["failed", "needs_human"],
        "pendingDeliveryCount": 1,
        "deadDeliveryCount": 1,
    }
    assert "never-return-this" not in response.text


def test_admin_api_docs_returns_public_endpoint_cards(client: TestClient) -> None:
    response = client.get("/admin/api/api-docs", headers=_admin_headers())

    assert response.status_code == 200
    payload = response.json()
    assert payload["downloadUrl"] == "/admin/api-docs/store-management.md"
    assert len(payload["endpoints"]) >= 10
    first = payload["endpoints"][0]
    assert first["method"] == "GET"
    assert first["title"] == "读取商店支持语言"
    assert first["curl"].startswith("curl")

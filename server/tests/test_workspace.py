from __future__ import annotations

from fastapi.testclient import TestClient
from sqlalchemy.orm import Session

from testflying_api.seed import seed_demo_catalog


def test_workspace_returns_client_contract_shape(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)
    response = client.get(
        "/v1/test-distribution/workspace",
        headers={
            "Authorization": "Bearer token-123",
            "X-Device-ID": "device-001",
            "X-Client-Platform": "ios",
        },
    )

    assert response.status_code == 200
    body = response.json()
    assert set(body) == {
        "apps",
        "builds",
        "devices",
        "developerAccounts",
        "notifications",
        "installTasks",
        "sortOrder",
        "profile",
    }
    assert body["apps"]
    assert body["builds"]
    assert body["developerAccounts"]
    assert body["notifications"]
    assert body["installTasks"] == []
    assert body["sortOrder"] == {"buildIds": []}
    assert any(device["id"] == "device-001" and device["isCurrent"] for device in body["devices"])
    assert body["profile"]["subtitle"] == "已连接内部测试分发服务"
    assert_no_client_state_keys(body)


def test_workspace_unknown_device_returns_empty_distribution_snapshot(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.get(
        "/v1/test-distribution/workspace",
        headers={
            "Authorization": "Bearer token-123",
            "X-Device-ID": "unknown-ios-device",
            "X-Client-Platform": "ios",
        },
    )

    assert response.status_code == 200
    body = response.json()
    assert body["apps"] == []
    assert body["builds"] == []
    assert body["installTasks"] == []
    assert body["sortOrder"] == {"buildIds": []}
    assert body["devices"][0]["id"] == "unknown-ios-device"
    assert body["devices"][0]["status"] == "pending"
    assert_no_client_state_keys(body)


def assert_no_client_state_keys(value: object) -> None:
    forbidden = {"isRead", "readAt", "installedAt", "installState", "progress"}
    if isinstance(value, dict):
        assert forbidden.isdisjoint(value)
        for item in value.values():
            assert_no_client_state_keys(item)
    if isinstance(value, list):
        for item in value:
            assert_no_client_state_keys(item)

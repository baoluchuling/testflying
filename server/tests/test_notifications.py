from __future__ import annotations

from fastapi.testclient import TestClient
from sqlalchemy.orm import Session

from testflying_api.seed import seed_demo_catalog


def test_notifications_return_feed_without_read_state(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.get("/v1/test-distribution/notifications")

    assert response.status_code == 200
    notifications = response.json()
    assert {item["type"] for item in notifications} >= {"build", "account", "device"}
    for item in notifications:
        assert "isRead" not in item
        assert "readAt" not in item


def test_notifications_can_filter_type(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.get("/v1/test-distribution/notifications?type=account")

    assert response.status_code == 200
    assert response.json()
    assert {item["type"] for item in response.json()} == {"account"}


def test_notification_read_write_routes_do_not_exist(client: TestClient) -> None:
    patch_response = client.patch("/v1/test-distribution/notifications/notice-account-renewal")
    mark_all_response = client.post("/v1/test-distribution/notifications/mark-all-read")

    assert patch_response.status_code in {404, 405}
    assert mark_all_response.status_code in {404, 405}

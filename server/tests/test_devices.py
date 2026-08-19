from __future__ import annotations

from fastapi.testclient import TestClient
from sqlalchemy.orm import Session

from testflying_api.seed import seed_demo_catalog


def test_current_device_returns_registered_device(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.get(
        "/v1/test-distribution/devices/current",
        headers={"X-Device-ID": "device-001"},
    )

    assert response.status_code == 200
    assert response.json()["id"] == "device-001"
    assert response.json()["isCurrent"] is True


def test_current_device_rejects_unregistered_device(client: TestClient) -> None:
    response = client.get(
        "/v1/test-distribution/devices/current",
        headers={"X-Device-ID": "unknown"},
    )

    assert response.status_code == 404
    assert response.json()["code"] == "device_not_registered"


def test_workspace_filters_builds_by_device_platform(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.get(
        "/v1/test-distribution/workspace",
        headers={"X-Device-ID": "device-android-001", "X-Client-Platform": "android"},
    )

    assert response.status_code == 200
    builds = response.json()["builds"]
    assert builds
    assert {build["installInfo"]["platform"] for build in builds} == {"android"}


def test_registration_link_does_not_auto_register_device(client: TestClient) -> None:
    response = client.post(
        "/v1/test-distribution/devices/registration-link",
        headers={"X-Device-ID": "new-device"},
    )

    assert response.status_code == 200
    assert response.json()["status"] == "pending_approval"

    current_response = client.get(
        "/v1/test-distribution/devices/current",
        headers={"X-Device-ID": "new-device"},
    )
    assert current_response.status_code == 404

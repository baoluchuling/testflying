from __future__ import annotations

from base64 import b64encode

from fastapi.testclient import TestClient
from sqlalchemy.orm import Session

from testflying_api.schema import Build
from testflying_api.seed import seed_demo_catalog


def _admin_headers(password: str = "dev-token") -> dict[str, str]:
    token = b64encode(f"admin:{password}".encode()).decode()
    return {"Authorization": f"Basic {token}"}


def test_admin_api_store_apps_lists_apps_and_stats(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.get("/admin/api/store-apps", headers=_admin_headers())

    assert response.status_code == 200
    payload = response.json()
    assert [item["id"] for item in payload["apps"]] == [
        "app-dataflow-android",
        "app-aurora-ios",
        "app-insight-ios",
    ]
    assert payload["selectedApp"]["id"] == "app-dataflow-android"
    assert payload["stats"] == {
        "total": 3,
        "ios": 2,
        "android": 1,
        "ready": 2,
        "needs": 1,
    }
    assert payload["accountSummary"] == {
        "totalAccounts": 1,
        "boundApps": 2,
        "connectorOk": 1,
        "connectorNeeds": 0,
        "renewalReminders": 1,
    }


def test_admin_api_store_apps_filters_needs(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.get("/admin/api/store-apps?filter=needs", headers=_admin_headers())

    assert response.status_code == 200
    payload = response.json()
    assert payload["filter"] == "needs"
    assert [item["id"] for item in payload["apps"]] == ["app-dataflow-android"]
    assert payload["apps"][0]["status"] == "needs_account"
    assert payload["apps"][0]["statusLabel"] == "未绑定账号"


def test_admin_api_store_apps_selects_requested_app(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.get(
        "/admin/api/store-apps?filter=ios&appId=app-insight-ios",
        headers=_admin_headers(),
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["filter"] == "ios"
    assert payload["selectedApp"]["id"] == "app-insight-ios"
    assert [item["selected"] for item in payload["apps"]] == [False, True]
    assert payload["selectedApp"]["storeManagementPath"].endswith(
        "/admin/accounts/account-apple-enterprise/apps/app-insight-ios/store"
    )
    assert payload["selectedApp"]["reviewsPath"] == (
        "/admin/store-reviews?accountId=account-apple-enterprise&appId=app-insight-ios"
    )


def test_admin_api_store_apps_handles_nullable_latest_build_metadata(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)
    latest_build = db_session.get(Build, "build-dataflow-android-54")
    assert latest_build is not None
    latest_build.version = None
    latest_build.build_number = None
    db_session.commit()

    response = client.get("/admin/api/store-apps", headers=_admin_headers())

    assert response.status_code == 200
    payload = response.json()
    assert payload["selectedApp"]["id"] == "app-dataflow-android"
    assert payload["selectedApp"]["latestBuild"] == {
        "version": "",
        "buildNumber": "",
        "environment": "development",
        "uploadedAt": latest_build.uploaded_at.isoformat(),
    }

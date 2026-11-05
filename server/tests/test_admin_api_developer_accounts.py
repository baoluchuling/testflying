from __future__ import annotations

from base64 import b64encode

from fastapi.testclient import TestClient
from sqlalchemy.orm import Session

from testflying_api.schema import StoreConnector
from testflying_api.seed import seed_demo_catalog


def _admin_headers(password: str = "dev-token") -> dict[str, str]:
    token = b64encode(f"admin:{password}".encode()).decode()
    return {"Authorization": f"Basic {token}"}


def test_admin_api_developer_accounts_lists_accounts(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.get("/admin/api/developer-accounts", headers=_admin_headers())

    assert response.status_code == 200
    payload = response.json()
    assert payload["stats"]["total"] == 1
    assert payload["accounts"][0]["id"] == "account-apple-enterprise"
    assert payload["accounts"][0]["detailPath"] == "/admin/accounts/account-apple-enterprise"
    assert "/admin/developer-accounts" not in response.text


def test_admin_api_developer_account_detail_exposes_spa_app_routes(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.get(
        "/admin/api/developer-accounts/account-apple-enterprise",
        headers=_admin_headers(),
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["account"]["id"] == "account-apple-enterprise"
    assert payload["connector"]["status"] == "ok"
    assert [item["id"] for item in payload["apps"]] == [
        "app-aurora-ios",
        "app-insight-ios",
    ]
    insight = payload["apps"][1]
    assert insight["storePath"] == (
        "/admin/accounts/account-apple-enterprise/apps/app-insight-ios/store"
    )
    assert insight["marketingPath"] == (
        "/admin/accounts/account-apple-enterprise/apps/app-insight-ios/marketing"
    )
    assert insight["releaseNotesPath"] == (
        "/admin/accounts/account-apple-enterprise/apps/app-insight-ios/release-notes"
    )
    assert insight["connectionPath"] == (
        "/admin/accounts/account-apple-enterprise/apps/app-insight-ios/connection"
    )
    assert "/admin/developer-accounts" not in response.text


def test_admin_api_developer_account_detail_auto_checks_connector(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)
    connector = db_session.query(StoreConnector).one()
    connector.status = "unknown"
    connector.last_checked_at = None
    db_session.commit()

    response = client.get(
        "/admin/api/developer-accounts/account-apple-enterprise",
        headers=_admin_headers(),
    )

    db_session.refresh(connector)
    assert response.status_code == 200
    assert response.json()["connector"]["status"] == "ok"
    assert connector.status == "ok"
    assert connector.last_checked_at is not None


def test_admin_api_developer_account_workspace_loads_store_state(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.get(
        "/admin/api/developer-accounts/account-apple-enterprise/apps/app-insight-ios/workspace",
        headers=_admin_headers(),
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["account"]["id"] == "account-apple-enterprise"
    assert payload["app"]["id"] == "app-insight-ios"
    assert payload["section"] == "store"
    assert payload["supportedLocales"]
    assert payload["localizedMetadata"]
    assert payload["app"]["connectionPath"] == (
        "/admin/accounts/account-apple-enterprise/apps/app-insight-ios/connection"
    )


def test_admin_api_developer_account_can_bind_update_and_unbind_app(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    bind_response = client.post(
        "/admin/api/developer-accounts/account-apple-enterprise/apps",
        headers=_admin_headers(),
        json={
            "appId": "app-dataflow-android",
            "storePackageName": "com.internal.dataflow",
        },
    )

    assert bind_response.status_code == 200
    bind_payload = bind_response.json()
    assert any(item["id"] == "app-dataflow-android" for item in bind_payload["state"]["apps"])

    update_response = client.patch(
        "/admin/api/developer-accounts/account-apple-enterprise/apps/app-dataflow-android/settings",
        headers=_admin_headers(),
        json={"storePackageName": "com.internal.dataflow.updated"},
    )

    assert update_response.status_code == 200
    updated = next(
        item
        for item in update_response.json()["state"]["apps"]
        if item["id"] == "app-dataflow-android"
    )
    assert updated["storePackageName"] == "com.internal.dataflow.updated"

    unbind_response = client.delete(
        "/admin/api/developer-accounts/account-apple-enterprise/apps/app-dataflow-android",
        headers=_admin_headers(),
    )

    assert unbind_response.status_code == 200
    assert all(
        item["id"] != "app-dataflow-android" for item in unbind_response.json()["state"]["apps"]
    )

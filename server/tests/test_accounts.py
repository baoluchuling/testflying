from __future__ import annotations

from fastapi.testclient import TestClient
from sqlalchemy.orm import Session

from testflying_api.seed import seed_demo_catalog


def test_developer_accounts_return_renewal_facts(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.get(
        "/v1/test-distribution/developer-accounts",
        headers={"X-Device-ID": "device-001", "X-Client-Platform": "ios"},
    )

    assert response.status_code == 200
    account = response.json()[0]
    assert account["expiresAt"]
    assert account["status"] == "renewal_due"
    assert account["appIds"]
    assert "isRead" not in account
    assert "dismissedAt" not in account


def test_renewals_filters_due_accounts(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.get(
        "/v1/test-distribution/developer-accounts/renewals",
        headers={"X-Device-ID": "device-001", "X-Client-Platform": "ios"},
    )

    assert response.status_code == 200
    assert response.json()
    assert response.json()[0]["remainingDays"] <= 30

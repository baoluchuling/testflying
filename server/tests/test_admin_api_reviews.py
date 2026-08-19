from __future__ import annotations

from base64 import b64encode

from fastapi.testclient import TestClient
from sqlalchemy import select
from sqlalchemy.orm import Session

from testflying_api.schema import StoreReview
from testflying_api.seed import seed_demo_catalog


def _admin_headers(password: str = "dev-token") -> dict[str, str]:
    token = b64encode(f"admin:{password}".encode()).decode()
    return {"Authorization": f"Basic {token}"}


def test_admin_api_store_reviews_state_lists_bound_apps(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.get("/admin/api/store-reviews", headers=_admin_headers())

    assert response.status_code == 200
    payload = response.json()
    assert payload["selectedAccountId"] == "account-apple-enterprise"
    assert payload["selectedAppId"] == "app-aurora-ios"
    assert [item["appId"] for item in payload["apps"]] == [
        "app-aurora-ios",
        "app-insight-ios",
    ]
    assert payload["apps"][0]["selected"] is True
    assert payload["stats"] == {"total": 0, "low": 0, "ios": 0, "android": 0}
    assert payload["analysisBoundaries"]


def test_admin_api_store_reviews_state_supports_app_switching(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.get(
        "/admin/api/store-reviews?accountId=account-apple-enterprise&appId=app-insight-ios",
        headers=_admin_headers(),
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["selectedAppId"] == "app-insight-ios"
    selected = [item for item in payload["apps"] if item["selected"]]
    assert [item["appId"] for item in selected] == ["app-insight-ios"]


def test_admin_api_store_reviews_state_filters_by_rating(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)
    client.post(
        "/admin/api/store-reviews/fetch",
        headers=_admin_headers(),
        json={"accountId": "account-apple-enterprise", "appId": "app-aurora-ios"},
    )

    response = client.get(
        "/admin/api/store-reviews"
        "?accountId=account-apple-enterprise&appId=app-aurora-ios&rating=3",
        headers=_admin_headers(),
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["rating"] == 3
    assert payload["reviews"]
    assert {review["rating"] for review in payload["reviews"]} == {3}


def test_admin_api_store_reviews_fetch_returns_state(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.post(
        "/admin/api/store-reviews/fetch",
        headers=_admin_headers(),
        json={"accountId": "account-apple-enterprise", "appId": "app-aurora-ios"},
    )

    stored_count = len(list(db_session.scalars(select(StoreReview))))
    payload = response.json()
    assert response.status_code == 200
    assert payload["message"] == "最新评论已拉取"
    assert payload["result"]["insertedCount"] == 2
    assert payload["result"]["stoppedReason"] == "no_more_pages"
    assert payload["state"]["reviews"]
    assert stored_count == 2


def test_admin_api_store_reviews_analyze_returns_friendly_error_when_llm_disabled(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)
    client.post(
        "/admin/api/store-reviews/fetch",
        headers=_admin_headers(),
        json={"accountId": "account-apple-enterprise", "appId": "app-aurora-ios"},
    )

    response = client.post(
        "/admin/api/store-reviews/analyze",
        headers=_admin_headers(),
        json={"accountId": "account-apple-enterprise", "appId": "app-aurora-ios"},
    )

    payload = response.json()
    assert response.status_code == 200
    assert payload["message"] == "评论分析失败"
    assert payload["error"] == {
        "code": "review_analysis_not_configured",
        "message": "评论分析 LLM 服务未配置",
    }
    assert payload["state"]["latestAnalysis"]["status"] == "failed"

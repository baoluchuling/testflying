from __future__ import annotations

from base64 import b64encode
from datetime import UTC, datetime

from fastapi.testclient import TestClient
from sqlalchemy import func, select
from sqlalchemy.orm import Session

from testflying_api.schema import StoreReview
from testflying_api.seed import seed_demo_catalog
from testflying_api.store_reviews import fetch_store_reviews_incremental


def _admin_headers(password: str = "dev-token") -> dict[str, str]:
    token = b64encode(f"admin:{password}".encode()).decode()
    return {"Authorization": f"Basic {token}"}


def _assert_admin_spa_shell(response) -> None:
    assert response.status_code == 200
    assert 'data-admin-app-root' in response.text
    assert "/assets/index-" in response.text


class FakeReviewClient:
    def __init__(self, pages: list[dict[str, object]]) -> None:
        self.pages = pages
        self.calls: list[dict[str, str]] = []

    def store_reviews(
        self,
        connector,
        *,
        account_id: str,
        app,
        store_app_id: str | None = None,
        package_name: str | None = None,
        store_query: dict[str, str] | None = None,
    ) -> dict[str, object]:
        self.calls.append(dict(store_query or {}))
        return self.pages[min(len(self.calls) - 1, len(self.pages) - 1)]


def test_initial_review_fetch_only_reads_one_page(db_session: Session) -> None:
    seed_demo_catalog(db_session)
    client = FakeReviewClient(
        [
            {
                "reviews": [
                    _review("review-1", "2026-06-29T10:00:00Z"),
                    _review("review-2", "2026-06-29T09:00:00Z"),
                ],
                "nextPageToken": "page-2",
            },
            {
                "reviews": [_review("review-3", "2026-06-28T10:00:00Z")],
                "nextPageToken": "",
            },
        ]
    )

    result = fetch_store_reviews_incremental(
        db_session,
        account_id="account-apple-enterprise",
        app_id="app-aurora-ios",
        client=client,
    )

    reviews = list(db_session.scalars(select(StoreReview).order_by(StoreReview.store_review_id)))
    assert len(client.calls) == 1
    assert result.inserted_count == 2
    assert result.fetched_count == 2
    assert result.stopped_reason == "initial_page_only"
    assert [review.store_review_id for review in reviews] == ["review-1", "review-2"]


def test_incremental_review_fetch_stops_on_existing_same_created_date(
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)
    db_session.add(
        StoreReview(
            id="store-review-existing",
            developer_account_id="account-apple-enterprise",
            app_id="app-aurora-ios",
            platform="ios",
            store_review_id="review-existing",
            rating=4,
            title="Existing",
            body="Existing review",
            author_name="user",
            locale="en-US",
            territory="US",
            app_version="1.0.0",
            created_at=datetime(2026, 6, 28, 10, 0, tzinfo=UTC),
            raw_json={},
        )
    )
    db_session.flush()
    client = FakeReviewClient(
        [
            {
                "reviews": [
                    _review("review-new", "2026-06-29T10:00:00Z"),
                    _review("review-existing", "2026-06-28T12:20:00Z"),
                    _review("review-older-gap", "2026-06-27T10:00:00Z"),
                ],
                "nextPageToken": "page-2",
            },
            {
                "reviews": [_review("review-should-not-load", "2026-06-26T10:00:00Z")],
                "nextPageToken": "",
            },
        ]
    )

    result = fetch_store_reviews_incremental(
        db_session,
        account_id="account-apple-enterprise",
        app_id="app-aurora-ios",
        client=client,
    )

    stored_ids = set(db_session.scalars(select(StoreReview.store_review_id)))
    assert len(client.calls) == 1
    assert result.inserted_count == 1
    assert result.duplicate_count == 1
    assert result.stopped_reason == "existing_review_same_created_date"
    assert "review-new" in stored_ids
    assert "review-existing" in stored_ids
    assert "review-older-gap" not in stored_ids
    assert "review-should-not-load" not in stored_ids


def test_admin_store_reviews_page_fetches_initial_reviews(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    page = client.get(
        "/admin/store-reviews?accountId=account-apple-enterprise&appId=app-aurora-ios",
        headers=_admin_headers(),
    )
    response = client.post(
        "/admin/api/store-reviews/fetch",
        headers=_admin_headers(),
        json={"accountId": "account-apple-enterprise", "appId": "app-aurora-ios"},
    )

    stored_count = db_session.scalar(select(func.count(StoreReview.id)))
    _assert_admin_spa_shell(page)
    assert response.status_code == 200
    assert response.json()["message"] == "最新评论已拉取"
    assert stored_count == 2


def test_admin_store_reviews_analysis_shows_config_error(
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

    assert response.status_code == 200
    assert response.json()["message"] == "评论分析失败"
    assert response.json()["error"]["message"] == "评论分析 LLM 服务未配置"


def _review(review_id: str, created_at: str) -> dict[str, object]:
    return {
        "id": review_id,
        "rating": 2,
        "title": f"Review {review_id}",
        "body": "Login state is lost after relaunch.",
        "authorName": "tester",
        "locale": "en-US",
        "territory": "US",
        "appVersion": "1.0.0",
        "createdAt": created_at,
        "updatedAt": created_at,
    }

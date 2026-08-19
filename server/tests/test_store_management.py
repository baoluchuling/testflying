from __future__ import annotations

import json

from fastapi.testclient import TestClient
from pytest import MonkeyPatch
from sqlalchemy.orm import Session

from testflying_api.routes.store_management import _reset_direct_sync_guards_for_tests
from testflying_api.schema import (
    DeveloperAccountApp,
    StoreAppMetadataDraft,
    StoreMarketingPage,
    StoreMarketingPageLocale,
    StoreReleaseNoteDraft,
    StoreSyncRun,
)
from testflying_api.seed import seed_demo_catalog
from testflying_api.store_sync import (
    CURRENT_METADATA_VERSION,
    UPDATE_APP_METADATA,
    UPDATE_MARKETING_PAGE,
    UPDATE_RELEASE_NOTES,
    sync_marketing_page,
)
from tests.fixtures import make_png_header_bytes


def test_store_management_import_requires_static_token(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.post(
        (
            "/v1/store-management/developer-accounts/account-apple-enterprise"
            "/apps/app-aurora-ios/store-versions/1.0.0/draft"
        ),
        json=_version_payload(),
    )

    assert response.status_code == 401
    assert response.json()["code"] == "invalid_static_token"


def test_store_management_imports_store_version_draft_without_store_sync(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.post(
        (
            "/v1/store-management/developer-accounts/account-apple-enterprise"
            "/apps/app-aurora-ios/store-versions/1.0.0/draft"
        ),
        headers={"Authorization": "Bearer dev-token"},
        json=_version_payload(),
    )

    assert response.status_code == 200
    assert response.json() == {
        "version": "1.0.0",
        "locales": ["en-US", "zh-Hant"],
        "savedMetadataDrafts": 2,
        "savedReleaseNoteDrafts": 2,
        "warnings": [],
    }
    assert db_session.query(StoreSyncRun).count() == 0

    metadata_drafts = (
        db_session.query(StoreAppMetadataDraft).order_by(StoreAppMetadataDraft.locale).all()
    )
    release_note_drafts = (
        db_session.query(StoreReleaseNoteDraft).order_by(StoreReleaseNoteDraft.locale).all()
    )
    assert {draft.locale for draft in metadata_drafts} == {"en-US", "zh-Hant"}
    assert {draft.locale for draft in release_note_drafts} == {"en-US", "zh-Hant"}

    en_us = next(draft for draft in metadata_drafts if draft.locale == "en-US")
    zh_hant = next(draft for draft in metadata_drafts if draft.locale == "zh-Hant")
    assert en_us.version == CURRENT_METADATA_VERSION
    assert zh_hant.version == CURRENT_METADATA_VERSION
    assert en_us.content_set_id == "default"
    assert en_us.description == "Long store description for import testing."
    assert all(
        not value["urls"] and not value["assets"] for value in en_us.store_images_json.values()
    )
    assert zh_hant.description == "Long store description for import testing."

    en_us_notes = next(draft for draft in release_note_drafts if draft.locale == "en-US")
    zh_hant_notes = next(draft for draft in release_note_drafts if draft.locale == "zh-Hant")
    assert en_us_notes.release_notes == "Fix bugs"
    assert zh_hant_notes.release_notes == "Fix bugs"


def test_store_management_direct_sync_default_store_page_from_existing_drafts(
    client: TestClient,
    db_session: Session,
) -> None:
    _reset_direct_sync_guards_for_tests()
    seed_demo_catalog(db_session)
    client.post(
        (
            "/v1/store-management/developer-accounts/account-apple-enterprise"
            "/apps/app-aurora-ios/store-versions/1.0.0/draft"
        ),
        headers={"Authorization": "Bearer dev-token"},
        json=_version_payload(),
    )

    response = client.post(
        (
            "/v1/store-management/developer-accounts/account-apple-enterprise"
            "/apps/app-aurora-ios/sync-runs"
        ),
        headers={"Authorization": "Bearer dev-token"},
        json={
            "version": "1.0.0",
            "locales": ["en-US", "zh-Hant"],
            "scopes": ["metadata", "release_notes", "store_images"],
            "actor": "third-party-computer",
        },
    )

    assert response.status_code == 200
    body = response.json()
    assert body["status"] == "succeeded"
    assert body["accountId"] == "account-apple-enterprise"
    assert body["appId"] == "app-aurora-ios"
    assert body["version"] == "1.0.0"
    assert body["scopes"] == ["metadata", "release_notes", "store_images"]
    assert body["locales"] == ["en-US", "zh-Hant"]
    assert body["idempotent"] is False
    assert len(body["runs"]) == 4
    assert {(item["operation"], item["locale"], item["status"]) for item in body["runs"]} == {
        (UPDATE_APP_METADATA, "en-US", "succeeded"),
        (UPDATE_APP_METADATA, "zh-Hant", "succeeded"),
        (UPDATE_RELEASE_NOTES, "en-US", "succeeded"),
        (UPDATE_RELEASE_NOTES, "zh-Hant", "succeeded"),
    }

    runs = db_session.query(StoreSyncRun).order_by(StoreSyncRun.locale).all()
    assert len(runs) == 4
    assert {run.operation for run in runs} == {UPDATE_APP_METADATA, UPDATE_RELEASE_NOTES}
    assert {run.sync_scopes_json["scopes"][0] for run in runs} == {
        "metadata",
        "release_notes",
    }


def test_store_management_direct_sync_android_short_description_scope(
    client: TestClient,
    db_session: Session,
) -> None:
    _reset_direct_sync_guards_for_tests()
    seed_demo_catalog(db_session)
    db_session.add(
        DeveloperAccountApp(
            developer_account_id="account-apple-enterprise",
            app_id="app-dataflow-android",
        )
    )
    db_session.commit()
    draft_response = client.post(
        (
            "/v1/store-management/developer-accounts/account-apple-enterprise"
            "/apps/app-dataflow-android/metadata-content-sets"
        ),
        headers={"Authorization": "Bearer dev-token"},
        data={"metadata": json.dumps(_android_metadata_payload())},
    )
    assert draft_response.status_code == 200

    response = client.post(
        (
            "/v1/store-management/developer-accounts/account-apple-enterprise"
            "/apps/app-dataflow-android/sync-runs"
        ),
        headers={"Authorization": "Bearer dev-token"},
        json={
            "version": "3.1.0",
            "locales": ["en-US"],
            "scopes": ["short_description"],
            "actor": "third-party-computer",
        },
    )

    assert response.status_code == 200
    body = response.json()
    assert body["scopes"] == ["short_description"]
    assert [(item["operation"], item["locale"], item["status"]) for item in body["runs"]] == [
        (UPDATE_APP_METADATA, "en-US", "succeeded")
    ]
    run = db_session.query(StoreSyncRun).filter_by(app_id="app-dataflow-android").one()
    assert run.sync_scopes_json == {"scopes": ["short_description"]}
    assert run.payload_snapshot_json["metadata"]["shortDescription"] == "Quick data flow."


def test_store_management_direct_sync_uses_explicit_ios_app_id(
    client: TestClient,
    db_session: Session,
) -> None:
    _reset_direct_sync_guards_for_tests()
    seed_demo_catalog(db_session)
    client.post(
        (
            "/v1/store-management/developer-accounts/account-apple-enterprise"
            "/apps/app-aurora-ios/store-versions/1.0.0/draft"
        ),
        headers={"Authorization": "Bearer dev-token"},
        json=_version_payload(),
    )

    response = client.post(
        (
            "/v1/store-management/developer-accounts/account-apple-enterprise"
            "/apps/app-aurora-ios/sync-runs"
        ),
        headers={"Authorization": "Bearer dev-token"},
        json={
            "version": "1.0.0",
            "locales": ["en-US"],
            "scopes": ["metadata"],
            "iosAppId": "9876543210",
            "actor": "third-party-computer",
        },
    )

    assert response.status_code == 200
    body = response.json()
    assert body["appId"] == "app-aurora-ios"
    run = db_session.query(StoreSyncRun).one()
    assert run.payload_snapshot_json["app"]["storeAppId"] == "9876543210"
    assert run.payload_snapshot_json["app"]["packageName"] == "com.internal.aurora"


def test_store_management_direct_sync_uses_explicit_android_package_name(
    client: TestClient,
    db_session: Session,
) -> None:
    _reset_direct_sync_guards_for_tests()
    seed_demo_catalog(db_session)
    db_session.add(
        DeveloperAccountApp(
            developer_account_id="account-apple-enterprise",
            app_id="app-dataflow-android",
        )
    )
    db_session.commit()
    client.post(
        (
            "/v1/store-management/developer-accounts/account-apple-enterprise"
            "/apps/app-dataflow-android/metadata-content-sets"
        ),
        headers={"Authorization": "Bearer dev-token"},
        data={"metadata": json.dumps(_android_metadata_payload())},
    )

    response = client.post(
        (
            "/v1/store-management/developer-accounts/account-apple-enterprise"
            "/apps/app-dataflow-android/sync-runs"
        ),
        headers={"Authorization": "Bearer dev-token"},
        json={
            "version": "3.1.0",
            "locales": ["en-US"],
            "scopes": ["short_description"],
            "packageName": "com.app.android.qw.newreadink",
            "actor": "third-party-computer",
        },
    )

    assert response.status_code == 200
    body = response.json()
    assert body["appId"] == "app-dataflow-android"
    run = db_session.query(StoreSyncRun).filter_by(app_id="app-dataflow-android").one()
    assert run.payload_snapshot_json["app"]["storeAppId"] is None
    assert run.payload_snapshot_json["app"]["packageName"] == "com.app.android.qw.newreadink"


def test_store_management_direct_sync_android_release_notes_accepts_store_version_code(
    client: TestClient,
    db_session: Session,
) -> None:
    _reset_direct_sync_guards_for_tests()
    seed_demo_catalog(db_session)
    db_session.add(
        DeveloperAccountApp(
            developer_account_id="account-apple-enterprise",
            app_id="app-dataflow-android",
        )
    )
    db_session.add(
        StoreReleaseNoteDraft(
            id="release-note-android-310-en",
            developer_account_id="account-apple-enterprise",
            app_id="app-dataflow-android",
            platform="android",
            version="3.1.0",
            locale="en-US",
            release_notes="Fix Android playback bugs.",
        )
    )
    db_session.commit()

    response = client.post(
        (
            "/v1/store-management/developer-accounts/account-apple-enterprise"
            "/apps/app-dataflow-android/sync-runs"
        ),
        headers={"Authorization": "Bearer dev-token"},
        json={
            "version": "3.1.0",
            "locales": ["en-US"],
            "scopes": ["release_notes"],
            "storeTrack": "production",
            "versionCode": 310,
            "actor": "third-party-computer",
        },
    )

    assert response.status_code == 200
    body = response.json()
    assert [(item["operation"], item["locale"], item["status"]) for item in body["runs"]] == [
        (UPDATE_RELEASE_NOTES, "en-US", "succeeded")
    ]
    run = db_session.query(StoreSyncRun).filter_by(app_id="app-dataflow-android").one()
    assert run.payload_snapshot_json["releaseNotes"] == "Fix Android playback bugs."
    assert run.payload_snapshot_json["storeRelease"] == {
        "track": "production",
        "versionCode": "310",
    }


def test_store_management_direct_sync_is_idempotent(
    client: TestClient,
    db_session: Session,
) -> None:
    _reset_direct_sync_guards_for_tests()
    seed_demo_catalog(db_session)
    client.post(
        (
            "/v1/store-management/developer-accounts/account-apple-enterprise"
            "/apps/app-aurora-ios/store-versions/1.0.0/draft"
        ),
        headers={"Authorization": "Bearer dev-token"},
        json=_version_payload(),
    )
    request_body = {
        "version": "1.0.0",
        "locales": ["en-US"],
        "scopes": ["release_notes"],
        "actor": "third-party-computer",
        "idempotencyKey": "build-123-release-notes-en-US",
    }

    first = client.post(
        (
            "/v1/store-management/developer-accounts/account-apple-enterprise"
            "/apps/app-aurora-ios/sync-runs"
        ),
        headers={"Authorization": "Bearer dev-token"},
        json=request_body,
    )
    second = client.post(
        (
            "/v1/store-management/developer-accounts/account-apple-enterprise"
            "/apps/app-aurora-ios/sync-runs"
        ),
        headers={"Authorization": "Bearer dev-token"},
        json=request_body,
    )

    assert first.status_code == 200
    assert second.status_code == 200
    assert first.json()["idempotent"] is False
    assert second.json()["idempotent"] is True
    assert first.json()["runs"] == second.json()["runs"]
    assert db_session.query(StoreSyncRun).count() == 1


def test_store_management_direct_sync_rate_limits_repeated_requests(
    client: TestClient,
    db_session: Session,
) -> None:
    _reset_direct_sync_guards_for_tests()
    seed_demo_catalog(db_session)
    client.post(
        (
            "/v1/store-management/developer-accounts/account-apple-enterprise"
            "/apps/app-aurora-ios/store-versions/1.0.0/draft"
        ),
        headers={"Authorization": "Bearer dev-token"},
        json=_version_payload(),
    )

    url = (
        "/v1/store-management/developer-accounts/account-apple-enterprise"
        "/apps/app-aurora-ios/sync-runs"
    )
    payload = {
        "version": "1.0.0",
        "locales": ["en-US"],
        "scopes": ["release_notes"],
    }
    for _ in range(10):
        response = client.post(url, headers={"Authorization": "Bearer dev-token"}, json=payload)
        assert response.status_code == 200

    limited = client.post(url, headers={"Authorization": "Bearer dev-token"}, json=payload)

    assert limited.status_code == 429
    assert limited.json()["code"] == "rate_limited"
    assert limited.json()["retryAfterSeconds"] > 0


def test_store_management_direct_sync_marketing_page_from_existing_draft(
    client: TestClient,
    db_session: Session,
) -> None:
    _reset_direct_sync_guards_for_tests()
    seed_demo_catalog(db_session)
    create_response = client.post(
        (
            "/v1/store-management/developer-accounts/account-apple-enterprise"
            "/apps/app-aurora-ios/marketing-pages"
        ),
        headers={"Authorization": "Bearer dev-token"},
        data={"metadata": json.dumps(_marketing_page_payload())},
    )
    page_id = create_response.json()["pageId"]

    response = client.post(
        (
            "/v1/store-management/developer-accounts/account-apple-enterprise"
            f"/apps/app-aurora-ios/marketing-pages/{page_id}/sync-runs"
        ),
        headers={"Authorization": "Bearer dev-token"},
        json={
            "locales": ["en-US", "zh-Hant"],
            "scopes": ["marketing_text", "store_images"],
            "actor": "third-party-computer",
        },
    )

    assert response.status_code == 200
    body = response.json()
    assert body["status"] == "succeeded"
    assert body["pageId"] == page_id
    assert body["version"] == page_id
    assert body["scopes"] == ["marketing_text", "store_images"]
    assert {(item["operation"], item["locale"], item["status"]) for item in body["runs"]} == {
        (UPDATE_MARKETING_PAGE, "en-US", "succeeded"),
        (UPDATE_MARKETING_PAGE, "zh-Hant", "succeeded"),
    }

    page = db_session.query(StoreMarketingPage).filter_by(page_id=page_id).one()
    assert page.status == "synced"
    assert db_session.query(StoreSyncRun).count() == 2


def test_store_management_lists_product_page_optimizations(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.get(
        (
            "/v1/store-management/developer-accounts/account-apple-enterprise"
            "/apps/app-aurora-ios/product-page-optimizations"
        ),
        headers={"Authorization": "Bearer dev-token"},
    )

    assert response.status_code == 200
    body = response.json()
    assert body["accountId"] == "account-apple-enterprise"
    assert body["appId"] == "app-aurora-ios"
    assert len(body["experiments"]) == 1
    experiment = body["experiments"][0]
    assert experiment["id"] == "ppo-app-aurora-ios"
    assert experiment["state"] == "PREPARE_FOR_SUBMISSION"
    assert experiment["trafficProportion"] == 50
    assert experiment["treatments"][0]["name"] == "Variant A"


def test_store_management_lists_current_store_locales(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.get(
        (
            "/v1/store-management/developer-accounts/account-apple-enterprise"
            "/apps/app-aurora-ios/store-locales?version=1.0.0"
        ),
        headers={"Authorization": "Bearer dev-token"},
    )

    assert response.status_code == 200
    body = response.json()
    assert body == {
        "accountId": "account-apple-enterprise",
        "appId": "app-aurora-ios",
        "platform": "ios",
        "version": "1.0.0",
        "locales": ["zh-Hans", "en-US", "ja", "ko"],
    }


def test_store_management_lists_current_store_listings(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.get(
        (
            "/v1/store-management/developer-accounts/account-apple-enterprise"
            "/apps/app-aurora-ios/store-listings?version=1.0.0"
        ),
        headers={"Authorization": "Bearer dev-token"},
    )

    assert response.status_code == 200
    body = response.json()
    assert body["accountId"] == "account-apple-enterprise"
    assert body["appId"] == "app-aurora-ios"
    assert body["platform"] == "ios"
    assert body["version"] == "1.0.0"
    assert [item["locale"] for item in body["listings"]] == ["zh-Hans", "en-US", "ja", "ko"]
    assert body["listings"][0]["description"] == "Mock store description."
    assert body["listings"][0]["promotionalText"] == "Mock promotional text."


def test_store_management_lists_current_store_images(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.get(
        (
            "/v1/store-management/developer-accounts/account-apple-enterprise"
            "/apps/app-aurora-ios/store-images?version=1.0.0"
        ),
        headers={"Authorization": "Bearer dev-token"},
    )

    assert response.status_code == 200
    body = response.json()
    assert body["accountId"] == "account-apple-enterprise"
    assert body["appId"] == "app-aurora-ios"
    assert body["platform"] == "ios"
    assert [item["locale"] for item in body["locales"]] == ["zh-Hans", "en-US", "ja", "ko"]
    first_image = body["locales"][0]["images"]["phone_screenshots"][0]
    assert first_image["fileName"] == "phone-1.png"
    assert first_image["width"] == 1290
    assert first_image["height"] == 2796


def test_store_management_store_read_apis_use_explicit_ios_app_id(
    client: TestClient,
    db_session: Session,
    monkeypatch: MonkeyPatch,
) -> None:
    seed_demo_catalog(db_session)
    captured: list[tuple[str, str | None, str | None]] = []

    def fake_supported_locales(
        self,
        connector,
        *,
        account_id: str,
        app,
        version: str,
        store_app_id: str | None = None,
        package_name: str | None = None,
    ) -> list[str]:
        captured.append(("store-locales", store_app_id, package_name))
        return ["en-US"]

    def fake_store_listings(
        self,
        connector,
        *,
        account_id: str,
        app,
        version: str = "",
        store_app_id: str | None = None,
        package_name: str | None = None,
    ) -> dict[str, object]:
        captured.append(("store-listings", store_app_id, package_name))
        return {"listings": []}

    def fake_store_images(
        self,
        connector,
        *,
        account_id: str,
        app,
        version: str = "",
        store_app_id: str | None = None,
        package_name: str | None = None,
    ) -> dict[str, object]:
        captured.append(("store-images", store_app_id, package_name))
        return {"locales": []}

    def fake_product_page_optimizations(
        self,
        connector,
        *,
        account_id: str,
        app,
        store_app_id: str | None = None,
    ) -> dict[str, object]:
        captured.append(("product-page-optimizations", store_app_id, None))
        return {"experiments": []}

    monkeypatch.setattr(
        "testflying_api.routes.store_management.StoreConnectorClient.supported_locales",
        fake_supported_locales,
    )
    monkeypatch.setattr(
        "testflying_api.routes.store_management.StoreConnectorClient.store_listings",
        fake_store_listings,
    )
    monkeypatch.setattr(
        "testflying_api.routes.store_management.StoreConnectorClient.store_images",
        fake_store_images,
    )
    monkeypatch.setattr(
        "testflying_api.routes.store_management.StoreConnectorClient.product_page_optimizations",
        fake_product_page_optimizations,
    )

    paths = [
        "store-locales?version=1.0.0&iosAppId=9876543210&packageName=ignored",
        "store-listings?version=1.0.0&storeAppId=9876543210&packageName=ignored",
        "store-images?version=1.0.0&appleAppId=9876543210&packageName=ignored",
        "product-page-optimizations?iosAppId=9876543210",
    ]
    for path in paths:
        response = client.get(
            (
                "/v1/store-management/developer-accounts/account-apple-enterprise"
                f"/apps/app-aurora-ios/{path}"
            ),
            headers={"Authorization": "Bearer dev-token"},
        )
        assert response.status_code == 200

    assert captured == [
        ("store-locales", "9876543210", None),
        ("store-listings", "9876543210", None),
        ("store-images", "9876543210", None),
        ("product-page-optimizations", "9876543210", None),
    ]


def test_store_management_store_read_apis_use_explicit_android_package_name(
    client: TestClient,
    db_session: Session,
    monkeypatch: MonkeyPatch,
) -> None:
    seed_demo_catalog(db_session)
    db_session.add(
        DeveloperAccountApp(
            developer_account_id="account-apple-enterprise",
            app_id="app-dataflow-android",
        )
    )
    db_session.commit()
    captured: list[tuple[str, str | None, str | None]] = []

    def fake_supported_locales(
        self,
        connector,
        *,
        account_id: str,
        app,
        version: str,
        store_app_id: str | None = None,
        package_name: str | None = None,
    ) -> list[str]:
        captured.append(("store-locales", store_app_id, package_name))
        return ["en-US"]

    def fake_store_listings(
        self,
        connector,
        *,
        account_id: str,
        app,
        version: str = "",
        store_app_id: str | None = None,
        package_name: str | None = None,
    ) -> dict[str, object]:
        captured.append(("store-listings", store_app_id, package_name))
        return {"listings": []}

    def fake_store_images(
        self,
        connector,
        *,
        account_id: str,
        app,
        version: str = "",
        store_app_id: str | None = None,
        package_name: str | None = None,
    ) -> dict[str, object]:
        captured.append(("store-images", store_app_id, package_name))
        return {"locales": []}

    def fake_store_releases(
        self,
        connector,
        *,
        account_id: str,
        app,
        version: str = "",
        store_app_id: str | None = None,
        package_name: str | None = None,
    ) -> dict[str, object]:
        captured.append(("store-releases", store_app_id, package_name))
        return {"releases": []}

    def fake_store_reviews(
        self,
        connector,
        *,
        account_id: str,
        app,
        store_app_id: str | None = None,
        package_name: str | None = None,
        store_query: dict[str, str] | None = None,
    ) -> dict[str, object]:
        captured.append(("store-reviews", store_app_id, package_name))
        return {"reviews": [], "nextPageToken": ""}

    monkeypatch.setattr(
        "testflying_api.routes.store_management.StoreConnectorClient.supported_locales",
        fake_supported_locales,
    )
    monkeypatch.setattr(
        "testflying_api.routes.store_management.StoreConnectorClient.store_listings",
        fake_store_listings,
    )
    monkeypatch.setattr(
        "testflying_api.routes.store_management.StoreConnectorClient.store_images",
        fake_store_images,
    )
    monkeypatch.setattr(
        "testflying_api.routes.store_management.StoreConnectorClient.store_releases",
        fake_store_releases,
    )
    monkeypatch.setattr(
        "testflying_api.routes.store_management.StoreConnectorClient.store_reviews",
        fake_store_reviews,
    )

    package_name = "com.app.android.qw.newreadink"
    paths = [
        f"store-locales?version=3.1.0&packageName={package_name}&iosAppId=ignored",
        f"store-listings?version=3.1.0&packageName={package_name}&iosAppId=ignored",
        f"store-images?version=3.1.0&packageName={package_name}&iosAppId=ignored",
        f"store-releases?version=3.1.0&packageName={package_name}&iosAppId=ignored",
        f"store-reviews?packageName={package_name}&iosAppId=ignored",
    ]
    for path in paths:
        response = client.get(
            (
                "/v1/store-management/developer-accounts/account-apple-enterprise"
                f"/apps/app-dataflow-android/{path}"
            ),
            headers={"Authorization": "Bearer dev-token"},
        )
        assert response.status_code == 200

    assert captured == [
        ("store-locales", None, package_name),
        ("store-listings", None, package_name),
        ("store-images", None, package_name),
        ("store-releases", None, package_name),
        ("store-reviews", None, package_name),
    ]


def test_store_management_lists_store_reviews_and_filters_in_center(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.get(
        (
            "/v1/store-management/developer-accounts/account-apple-enterprise"
            "/apps/app-aurora-ios/store-reviews"
            "?date=2026-06-24&timezone=UTC&rating=5&locale=en-US&pageSize=20"
        ),
        headers={"Authorization": "Bearer dev-token"},
    )

    assert response.status_code == 200
    body = response.json()
    assert body["accountId"] == "account-apple-enterprise"
    assert body["appId"] == "app-aurora-ios"
    assert body["platform"] == "ios"
    assert body["filters"]["timezone"] == "UTC"
    assert body["filters"]["rating"] == 5
    assert [item["id"] for item in body["reviews"]] == ["review-app-aurora-ios-1"]
    assert body["reviews"][0]["body"] == "The app works well for internal testing."


def test_store_management_store_reviews_rejects_conflicting_date_filters(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.get(
        (
            "/v1/store-management/developer-accounts/account-apple-enterprise"
            "/apps/app-aurora-ios/store-reviews"
            "?date=2026-06-24&startDate=2026-06-23"
        ),
        headers={"Authorization": "Bearer dev-token"},
    )

    assert response.status_code == 422
    assert response.json()["code"] == "invalid_review_date_filter"


def test_store_management_lists_google_play_store_releases(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)
    db_session.add(
        DeveloperAccountApp(
            developer_account_id="account-apple-enterprise",
            app_id="app-dataflow-android",
        )
    )
    db_session.commit()

    response = client.get(
        (
            "/v1/store-management/developer-accounts/account-apple-enterprise"
            "/apps/app-dataflow-android/store-releases"
        ),
        headers={"Authorization": "Bearer dev-token"},
    )

    assert response.status_code == 200
    body = response.json()
    assert body["accountId"] == "account-apple-enterprise"
    assert body["appId"] == "app-dataflow-android"
    assert body["platform"] == "android"
    assert [item["track"] for item in body["releases"]] == ["production", "internal"]
    assert body["releases"][0]["versionCodes"] == ["310"]
    assert body["releases"][0]["releaseNotes"][0]["language"] == "en-US"


def test_store_management_creates_product_page_optimization_idempotently(
    client: TestClient,
    db_session: Session,
) -> None:
    _reset_direct_sync_guards_for_tests()
    seed_demo_catalog(db_session)
    payload = {
        "name": "Summer Landing Test",
        "trafficProportion": 40,
        "locales": ["en-US", "zh-Hant"],
        "idempotencyKey": "ppo-summer-landing",
        "treatments": [
            {"name": "Variant A"},
            {"name": "Variant B", "locales": ["en-US"]},
        ],
    }

    first = client.post(
        (
            "/v1/store-management/developer-accounts/account-apple-enterprise"
            "/apps/app-aurora-ios/product-page-optimizations"
        ),
        headers={"Authorization": "Bearer dev-token"},
        json=payload,
    )
    second = client.post(
        (
            "/v1/store-management/developer-accounts/account-apple-enterprise"
            "/apps/app-aurora-ios/product-page-optimizations"
        ),
        headers={"Authorization": "Bearer dev-token"},
        json=payload,
    )

    assert first.status_code == 201
    assert second.status_code == 201
    first_body = first.json()
    second_body = second.json()
    assert first_body["idempotent"] is False
    assert second_body["idempotent"] is True
    assert first_body["experiment"] == second_body["experiment"]
    assert first_body["experiment"]["name"] == "Summer Landing Test"
    assert first_body["experiment"]["trafficProportion"] == 40
    assert [item["locales"] for item in first_body["experiment"]["treatments"]] == [
        ["en-US", "zh-Hant"],
        ["en-US"],
    ]
    assert db_session.query(StoreSyncRun).count() == 0


def test_store_management_create_product_page_optimization_uses_explicit_ios_app_id(
    client: TestClient,
    db_session: Session,
    monkeypatch: MonkeyPatch,
) -> None:
    _reset_direct_sync_guards_for_tests()
    seed_demo_catalog(db_session)
    captured: dict[str, str | None] = {}

    def fake_create_product_page_optimization(
        self,
        connector,
        *,
        account_id: str,
        app,
        name: str,
        traffic_proportion: int,
        locales: list[str],
        treatments: list[dict[str, object]],
        store_app_id: str | None = None,
    ) -> dict[str, object]:
        captured["store_app_id"] = store_app_id
        return {
            "experiment": {
                "id": "ppo-explicit-ios-app-id",
                "name": name,
                "platform": "ios",
                "state": "PREPARE_FOR_SUBMISSION",
                "trafficProportion": traffic_proportion,
                "reviewRequired": False,
                "startDate": "",
                "endDate": "",
                "treatments": [],
            }
        }

    monkeypatch.setattr(
        "testflying_api.routes.store_management.StoreConnectorClient."
        "create_product_page_optimization",
        fake_create_product_page_optimization,
    )

    response = client.post(
        (
            "/v1/store-management/developer-accounts/account-apple-enterprise"
            "/apps/app-aurora-ios/product-page-optimizations"
        ),
        headers={"Authorization": "Bearer dev-token"},
        json={
            "name": "Explicit Target Test",
            "trafficProportion": 25,
            "locales": ["en-US"],
            "iosAppId": "9876543210",
        },
    )

    assert response.status_code == 201
    assert response.json()["experiment"]["id"] == "ppo-explicit-ios-app-id"
    assert captured["store_app_id"] == "9876543210"


def test_store_management_rejects_product_page_optimization_for_android(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)
    db_session.add(
        DeveloperAccountApp(
            developer_account_id="account-apple-enterprise",
            app_id="app-dataflow-android",
        )
    )
    db_session.commit()

    response = client.get(
        (
            "/v1/store-management/developer-accounts/account-apple-enterprise"
            "/apps/app-dataflow-android/product-page-optimizations"
        ),
        headers={"Authorization": "Bearer dev-token"},
    )

    assert response.status_code == 422
    assert response.json()["code"] == "unsupported_product_page_optimization"


def test_store_management_store_image_suite_endpoint_is_removed(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.post(
        (
            "/v1/store-management/developer-accounts/account-apple-enterprise"
            "/apps/app-aurora-ios/store-image-suites"
        ),
        headers={"Authorization": "Bearer dev-token"},
        data={"metadata": "{}"},
    )

    assert response.status_code == 404


def test_store_management_imports_marketing_page_without_store_sync(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.post(
        (
            "/v1/store-management/developer-accounts/account-apple-enterprise"
            "/apps/app-aurora-ios/marketing-pages"
        ),
        headers={"Authorization": "Bearer dev-token"},
        data={"metadata": json.dumps(_marketing_page_payload())},
        files=[
            (
                "storeImageFiles__phone_screenshots__en-US",
                ("phone-1.png", make_png_header_bytes(1290, 2796), "image/png"),
            ),
            (
                "storeImageFiles__phoneScreenshots__en-US",
                ("phone-2.png", make_png_header_bytes(1320, 2868), "image/png"),
            ),
        ],
    )

    assert response.status_code == 200
    body = response.json()
    assert body["pageId"].startswith("page-")
    assert body == {
        "pageId": body["pageId"],
        "pageName": "API 自定义产品页",
        "pageType": "custom_product_page",
        "status": "draft",
        "locales": ["en-US", "zh-Hant"],
        "savedLocales": 2,
        "uploadedAssets": 2,
        "warnings": [],
    }
    assert db_session.query(StoreSyncRun).count() == 0

    page = db_session.query(StoreMarketingPage).one()
    assert page.page_id == body["pageId"]
    assert page.page_name == "API 自定义产品页"
    assert page.page_type == "custom_product_page"
    assert page.status == "draft"
    assert page.apple_page_id == ""
    assert page.deep_link_url == "anystories:///campaign"

    locales = (
        db_session.query(StoreMarketingPageLocale).order_by(StoreMarketingPageLocale.locale).all()
    )
    assert [item.locale for item in locales] == ["en-US", "zh-Hant"]
    en_us = next(item for item in locales if item.locale == "en-US")
    zh_hant = next(item for item in locales if item.locale == "zh-Hant")
    assert en_us.promotional_text == "Read better stories every day."
    assert zh_hant.promotional_text == "Read better stories every day."

    phone_assets = en_us.store_images_json["phone_screenshots"]["assets"]
    assert [asset["fileName"] for asset in phone_assets] == ["phone-1.png", "phone-2.png"]
    assert f"/{body['pageId']}/marketing/en-US/phone_screenshots/" in phone_assets[0]["storageKey"]


def test_store_management_marketing_sync_saves_apple_page_id(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)
    response = client.post(
        (
            "/v1/store-management/developer-accounts/account-apple-enterprise"
            "/apps/app-aurora-ios/marketing-pages"
        ),
        headers={"Authorization": "Bearer dev-token"},
        data={"metadata": json.dumps(_marketing_page_payload())},
    )
    page_id = response.json()["pageId"]

    run = sync_marketing_page(
        db_session,
        account_id="account-apple-enterprise",
        app_id="app-aurora-ios",
        page_id=page_id,
        locale="en-US",
        sync_scopes=["marketing_text"],
        actor="api",
        client=_MarketingSyncClient(),
    )
    db_session.commit()

    page = db_session.query(StoreMarketingPage).filter_by(page_id=page_id).one()
    assert run.status == "succeeded"
    assert page.status == "synced"
    assert page.apple_page_id == "cpp-apple-created"


def test_store_management_imports_metadata_content_set_without_store_sync(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.post(
        (
            "/v1/store-management/developer-accounts/account-apple-enterprise"
            "/apps/app-aurora-ios/metadata-content-sets"
        ),
        headers={"Authorization": "Bearer dev-token"},
        data={"metadata": json.dumps(_metadata_payload())},
        files=[
            (
                "storeImageFiles__phone_screenshots__en-US",
                ("phone-1.png", make_png_header_bytes(1290, 2796), "image/png"),
            ),
            (
                "storeImageFiles__phoneScreenshots__zh-Hant",
                ("phone-hant.png", make_png_header_bytes(1320, 2868), "image/png"),
            ),
        ],
    )

    assert response.status_code == 200
    body = response.json()
    assert body == {
        "contentSet": {"id": "summer-a", "name": "暑期截图方案 A"},
        "version": "1.0.0",
        "locales": ["en-US", "zh-Hant"],
        "savedDrafts": 2,
        "uploadedAssets": 2,
        "warnings": [],
    }

    drafts = db_session.query(StoreAppMetadataDraft).order_by(StoreAppMetadataDraft.locale).all()
    assert db_session.query(StoreSyncRun).count() == 0
    assert {draft.locale for draft in drafts} == {"en-US", "zh-Hant"}

    en_us = next(draft for draft in drafts if draft.locale == "en-US")
    zh_hant = next(draft for draft in drafts if draft.locale == "zh-Hant")
    assert en_us.version == CURRENT_METADATA_VERSION
    assert zh_hant.version == CURRENT_METADATA_VERSION
    assert en_us.content_set_id == "default"
    assert en_us.content_set_name == "默认上架内容"
    assert en_us.keywords == "novel,reader,story"
    assert en_us.promotional_text == "Read better stories every day."
    assert en_us.description == "Long store description for import testing."
    assert en_us.store_images_json["phone_screenshots"]["urls"] == [
        "https://cdn.example.test/source-phone.png"
    ]
    en_us_phone_assets = en_us.store_images_json["phone_screenshots"]["assets"]
    assert [asset["fileName"] for asset in en_us_phone_assets] == ["phone-1.png"]
    assert zh_hant.description == "Long store description for import testing."
    zh_hant_phone_assets = zh_hant.store_images_json["phone_screenshots"]["assets"]
    assert [asset["fileName"] for asset in zh_hant_phone_assets] == ["phone-hant.png"]


def test_store_management_import_rejects_invalid_store_image_size(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.post(
        (
            "/v1/store-management/developer-accounts/account-apple-enterprise"
            "/apps/app-aurora-ios/metadata-content-sets"
        ),
        headers={"Authorization": "Bearer dev-token"},
        data={"metadata": json.dumps(_metadata_payload())},
        files=[
            (
                "storeImageFiles__phone_screenshots__en-US",
                ("bad-phone.png", make_png_header_bytes(500, 500), "image/png"),
            )
        ],
    )

    assert response.status_code == 422
    assert response.json()["code"] == "store_image_invalid"
    assert "Apple 要求精确尺寸" in response.json()["message"]
    assert db_session.query(StoreAppMetadataDraft).count() == 0
    assert db_session.query(StoreSyncRun).count() == 0


def _version_payload() -> dict[str, object]:
    return {
        "sourceLocale": "en-US",
        "locales": [
            {
                "locale": "en-US",
                "keywords": "novel,reader,story",
                "promotionalText": "Read better stories every day.",
                "description": "Long store description for import testing.",
                "releaseNotes": "Fix bugs",
            },
            {
                "locale": "zh-Hant",
                "keywords": "",
                "promotionalText": "",
                "description": "",
                "releaseNotes": "",
            },
        ],
    }


def _marketing_page_payload() -> dict[str, object]:
    return {
        "pageName": "API 自定义产品页",
        "pageType": "custom_product_page",
        "sourceLocale": "en-US",
        "deepLinkUrl": "anystories:///campaign",
        "locales": [
            {
                "locale": "en-US",
                "promotionalText": "Read better stories every day.",
                "storeImages": {"phoneScreenshots": ["https://cdn.example.test/source-phone.png"]},
            },
            {
                "locale": "zh-Hant",
                "promotionalText": "",
                "storeImages": {},
            },
        ],
    }


def _metadata_payload() -> dict[str, object]:
    return {
        "version": "1.0.0",
        "contentSet": {
            "id": "summer-a",
            "name": "暑期截图方案 A",
        },
        "sourceLocale": "en-US",
        "locales": [
            {
                "locale": "en-US",
                "keywords": "novel,reader,story",
                "promotionalText": "Read better stories every day.",
                "description": "Long store description for import testing.",
                "storeImages": {"phoneScreenshots": ["https://cdn.example.test/source-phone.png"]},
            },
            {
                "locale": "zh-Hant",
                "keywords": "",
                "promotionalText": "",
                "description": "",
            },
        ],
    }


def _android_metadata_payload() -> dict[str, object]:
    return {
        "version": "3.1.0",
        "sourceLocale": "en-US",
        "locales": [
            {
                "locale": "en-US",
                "shortDescription": "Quick data flow.",
                "description": "Long Google Play description for import testing.",
            }
        ],
    }


class _MarketingSyncClient:
    def preflight(self, connector: object, payload: dict[str, object]) -> dict[str, object]:
        return {
            "canSync": True,
            "reasonCode": None,
            "message": "可同步",
            "storeState": {"versionExists": True, "editable": True},
        }

    def sync_store_operation(
        self,
        connector: object,
        payload: dict[str, object],
    ) -> dict[str, object]:
        return {
            "status": "succeeded",
            "message": "营销页面已同步。",
            "storeState": {"applePageId": "cpp-apple-created"},
        }

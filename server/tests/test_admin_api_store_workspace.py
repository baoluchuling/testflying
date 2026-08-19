from __future__ import annotations

from base64 import b64encode
from dataclasses import replace

from fastapi.testclient import TestClient
from sqlalchemy.orm import Session

from testflying_api.schema import StoreAppMetadataDraft, StoreConnector, StoreSyncRun
from testflying_api.seed import seed_demo_catalog


def _admin_headers(password: str = "dev-token") -> dict[str, str]:
    token = b64encode(f"admin:{password}".encode()).decode()
    return {"Authorization": f"Basic {token}"}


def _workspace_payload() -> dict[str, object]:
    return {
        "version": "1.9.3",
        "locale": "en-US",
        "locales": [
            {
                "locale": "en-US",
                "promotionalText": "Fast internal testing for release teams.",
                "description": "Insight Desk helps internal teams verify releases before launch.",
                "releaseNotes": "Fix bugs and improve diagnostics.",
                "storeImages": {
                    "phone_screenshots": {
                        "assets": [
                            {
                                "storageKey": "store-assets/test/en-US/phone/01.png",
                                "downloadUrl": "https://dist.example.test/01.png",
                            }
                        ]
                    }
                },
            }
        ],
    }


def _png_header(width: int, height: int) -> bytes:
    return (
        b"\x89PNG\r\n\x1a\n"
        + (13).to_bytes(4, "big")
        + b"IHDR"
        + width.to_bytes(4, "big")
        + height.to_bytes(4, "big")
        + b"\x08\x02\x00\x00\x00"
        + b"\x00\x00\x00\x00"
    )


def test_admin_api_store_workspace_saves_metadata_and_release_notes(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.put(
        "/admin/api/developer-accounts/account-apple-enterprise/apps/app-insight-ios/workspace/metadata",
        headers=_admin_headers(),
        json=_workspace_payload(),
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["message"] == "商店草稿已保存 1 个语言"
    saved_locale = next(
        item for item in payload["state"]["localizedMetadata"] if item["locale"] == "en-US"
    )
    assert saved_locale["description"].startswith("Insight Desk")
    draft = db_session.query(StoreAppMetadataDraft).one()
    assert draft.version == "__current__"
    assert draft.locale == "en-US"
    assert draft.description.startswith("Insight Desk")


def test_admin_api_store_workspace_uses_connector_supported_locales_for_live_connector(
    client: TestClient,
    db_session: Session,
    monkeypatch,
) -> None:
    seed_demo_catalog(db_session)
    connector = db_session.get(StoreConnector, "connector-apple-enterprise")
    assert connector is not None
    connector.base_url = "active://account-apple-enterprise"
    db_session.commit()

    def fake_supported_locales_for_app(*args, **kwargs):  # noqa: ANN002, ANN003, ARG001
        return ["en-US", "zh-Hant", "fr-FR"]

    monkeypatch.setattr(
        "testflying_api.admin.view_models.supported_locales_for_app",
        fake_supported_locales_for_app,
    )

    response = client.get(
        "/admin/api/developer-accounts/account-apple-enterprise/apps/app-insight-ios/workspace",
        headers=_admin_headers(),
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["supportedLocales"] == ["en-US", "zh-Hant", "fr-FR"]
    assert [item["locale"] for item in payload["localizedMetadata"]] == [
        "en-US",
        "zh-Hant",
        "fr-FR",
    ]


def test_admin_api_store_translation_returns_target_translations(client: TestClient) -> None:
    client.app.state.settings = replace(
        client.app.state.settings,
        translation_provider="mock",
    )

    response = client.post(
        "/admin/api/store-translation",
        headers=_admin_headers(),
        json={
            "sourceLocale": "en-US",
            "targetLocales": ["en-US", "zh-Hant", "fr-FR"],
            "field": "description",
            "text": "Read anywhere.",
        },
    )

    assert response.status_code == 200
    assert response.json() == {
        "translations": {
            "zh-Hant": "Read anywhere. [zh-Hant]",
            "fr-FR": "Read anywhere. [fr-FR]",
        }
    }


def test_admin_api_store_workspace_sync_requires_scope(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)
    payload = _workspace_payload()
    payload["syncScopes"] = []

    response = client.post(
        "/admin/api/store-workspace/account-apple-enterprise/app-insight-ios/metadata/sync",
        headers=_admin_headers(),
        json=payload,
    )

    assert response.status_code == 422
    assert response.json()["error"]["code"] == "missing_sync_scope"


def test_admin_api_store_workspace_syncs_selected_scopes(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)
    payload = _workspace_payload()
    payload["syncScopes"] = ["metadata", "release_notes"]

    response = client.post(
        "/admin/api/store-workspace/account-apple-enterprise/app-insight-ios/metadata/sync",
        headers=_admin_headers(),
        json=payload,
    )

    assert response.status_code == 200
    body = response.json()
    assert body["message"] == "已创建 2 个同步任务"
    assert [item["operation"] for item in body["syncRuns"]] == [
        "update_app_metadata",
        "update_release_notes",
    ]
    assert db_session.query(StoreSyncRun).count() == 2


def test_admin_api_store_workspace_deletes_center_store_image(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)
    client.put(
        "/admin/api/developer-accounts/account-apple-enterprise/apps/app-insight-ios/workspace/metadata",
        headers=_admin_headers(),
        json=_workspace_payload(),
    )

    response = client.request(
        "DELETE",
        "/admin/api/store-workspace/account-apple-enterprise/app-insight-ios/metadata/store-images",
        headers=_admin_headers(),
        json={
            "locale": "en-US",
            "slotKey": "phone_screenshots",
            "storageKey": "store-assets/test/en-US/phone/01.png",
        },
    )

    assert response.status_code == 200
    draft = db_session.query(StoreAppMetadataDraft).one()
    assert draft.store_images_json["phone_screenshots"]["assets"] == []


def test_admin_api_store_workspace_marks_source_store_images_as_inherited(
    client: TestClient,
    db_session: Session,
    monkeypatch,
) -> None:
    seed_demo_catalog(db_session)
    monkeypatch.setattr(
        "testflying_api.admin.view_models.supported_locales_for_app",
        lambda *args, **kwargs: ["en-US", "zh-Hant"],
    )
    payload = _workspace_payload()
    payload["locales"].append(
        {
            "locale": "zh-Hant",
            "promotionalText": "繁體文案",
            "description": "繁體描述",
            "releaseNotes": "繁體版本說明",
        }
    )
    save_response = client.put(
        "/admin/api/developer-accounts/account-apple-enterprise/apps/app-insight-ios/workspace/metadata",
        headers=_admin_headers(),
        json=payload,
    )
    assert save_response.status_code == 200
    saved_locales = sorted(draft.locale for draft in db_session.query(StoreAppMetadataDraft).all())
    assert saved_locales == ["en-US", "zh-Hant"]

    response = client.get(
        "/admin/api/developer-accounts/account-apple-enterprise/apps/app-insight-ios/workspace?locale=zh-Hant",
        headers=_admin_headers(),
    )

    assert response.status_code == 200
    localized = response.json()["localizedMetadata"]
    en_us = next(item for item in localized if item["locale"] == "en-US")
    zh_hant = next(item for item in localized if item["locale"] == "zh-Hant")
    en_us_item = en_us["storeImages"]["phone_screenshots"]["preview_items"][0]
    zh_hant_slot = zh_hant["storeImages"]["phone_screenshots"]
    zh_hant_item = zh_hant_slot["preview_items"][0]
    assert en_us_item["canDelete"] is True
    assert en_us_item["inherited"] is False
    assert zh_hant_slot["assets"] == []
    assert zh_hant_item["url"] == en_us_item["url"]
    assert zh_hant_item["sourceLocale"] == "en-US"
    assert zh_hant_item["inherited"] is True
    assert zh_hant_item["canDelete"] is False


def test_admin_api_store_workspace_keeps_shared_store_image_file_until_unreferenced(
    client: TestClient,
    db_session: Session,
    monkeypatch,
) -> None:
    seed_demo_catalog(db_session)
    payload = _workspace_payload()
    payload["locales"].append(
        {
            "locale": "zh-Hant",
            "promotionalText": "繁體文案",
            "description": "繁體描述",
            "releaseNotes": "繁體版本說明",
            "storeImages": {
                "phone_screenshots": {
                    "assets": [
                        {
                            "storageKey": "store-assets/test/en-US/phone/01.png",
                            "downloadUrl": "https://dist.example.test/01.png",
                        }
                    ]
                }
            },
        }
    )
    deleted: list[str] = []

    class FakeStorage:
        def delete(self, storage_key: str) -> None:
            deleted.append(storage_key)

    monkeypatch.setattr(client.app.state, "artifact_storage", FakeStorage())
    client.put(
        "/admin/api/developer-accounts/account-apple-enterprise/apps/app-insight-ios/workspace/metadata",
        headers=_admin_headers(),
        json=payload,
    )

    response = client.request(
        "DELETE",
        "/admin/api/store-workspace/account-apple-enterprise/app-insight-ios/metadata/store-images",
        headers=_admin_headers(),
        json={
            "locale": "zh-Hant",
            "slotKey": "phone_screenshots",
            "storageKey": "store-assets/test/en-US/phone/01.png",
        },
    )

    assert response.status_code == 200
    assert deleted == []


def test_admin_api_store_workspace_uploads_store_images(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.post(
        "/admin/api/store-workspace/account-apple-enterprise/app-insight-ios/metadata/store-images",
        headers=_admin_headers(),
        files=[
            (
                "storeImageFiles__phone_screenshots__en-US",
                ("iphone-69.png", _png_header(1290, 2796), "image/png"),
            )
        ],
    )

    assert response.status_code == 200
    body = response.json()
    assert body["message"] == "已上传 1 张中心后台商店图草稿"
    locale = next(item for item in body["state"]["localizedMetadata"] if item["locale"] == "en-US")
    assets = locale["storeImages"]["phone_screenshots"]["assets"]
    assert assets[0]["fileName"] == "iphone-69.png"
    draft = db_session.query(StoreAppMetadataDraft).filter_by(locale="en-US").one()
    assert draft.store_images_json["phone_screenshots"]["assets"][0]["width"] == 1290

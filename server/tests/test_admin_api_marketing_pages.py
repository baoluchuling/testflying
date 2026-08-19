from __future__ import annotations

from base64 import b64encode

from fastapi.testclient import TestClient
from sqlalchemy.orm import Session

from testflying_api.schema import StoreMarketingPage, StoreMarketingPageLocale, StoreSyncRun
from testflying_api.seed import seed_demo_catalog


def _admin_headers(password: str = "dev-token") -> dict[str, str]:
    token = b64encode(f"admin:{password}".encode()).decode()
    return {"Authorization": f"Basic {token}"}


def _marketing_payload() -> dict[str, object]:
    return {
        "pageId": "",
        "pageName": "冷启动投放页",
        "pageType": "custom_product_page",
        "deepLinkUrl": "anystories:///home",
        "locale": "en-US",
        "locales": [
            {
                "locale": "en-US",
                "promotionalText": "Read stories anytime.",
                "storeImages": {
                    "phone_screenshots": {
                        "assets": [
                            {
                                "storageKey": "store-assets/marketing/en-US/phone/01.png",
                                "downloadUrl": "https://dist.example.test/marketing-01.png",
                                "fileName": "marketing-01.png",
                            }
                        ]
                    }
                },
            },
            {
                "locale": "zh-Hant",
                "promotionalText": "隨時閱讀故事。",
                "storeImages": {},
            },
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


def test_admin_api_marketing_pages_create_with_content_and_open_detail(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.post(
        "/admin/api/developer-accounts/account-apple-enterprise"
        "/apps/app-aurora-ios/workspace/marketing-pages",
        headers=_admin_headers(),
        json=_marketing_payload(),
    )

    assert response.status_code == 200
    body = response.json()
    page_id = body["state"]["page"]["pageId"]
    assert body["message"] == "营销页面已创建"
    assert body["state"]["page"]["pageName"] == "冷启动投放页"
    en_us = next(item for item in body["state"]["localizedPage"] if item["locale"] == "en-US")
    assert en_us["promotionalText"] == "Read stories anytime."
    assert body["workspace"]["marketingPages"][0]["detailPath"].endswith(
        f"/marketing-pages/{page_id}"
    )

    detail = client.get(
        "/admin/api/store-workspace/account-apple-enterprise"
        f"/app-aurora-ios/marketing-pages/{page_id}",
        headers=_admin_headers(),
    )

    assert detail.status_code == 200
    assert detail.json()["page"]["applePageIdLabel"] == "未同步后回填"


def test_admin_api_marketing_pages_save_copy_delete_and_delete_image(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)
    create_response = client.post(
        "/admin/api/store-workspace/account-apple-enterprise"
        "/app-aurora-ios/marketing-pages",
        headers=_admin_headers(),
        json=_marketing_payload(),
    )
    page_id = create_response.json()["state"]["page"]["pageId"]

    save_payload = _marketing_payload()
    save_payload["pageName"] = "冷启动投放页 v2"
    save_payload["locales"] = [
        {
            "locale": "en-US",
            "promotionalText": "Read before launch.",
            "storeImages": {
                "phone_screenshots": {
                    "assets": [
                        {
                            "storageKey": "store-assets/marketing/en-US/phone/01.png",
                            "downloadUrl": "https://dist.example.test/marketing-01.png",
                            "fileName": "marketing-01.png",
                        }
                    ]
                }
            },
        }
    ]
    save_response = client.put(
        "/admin/api/store-workspace/account-apple-enterprise"
        f"/app-aurora-ios/marketing-pages/{page_id}",
        headers=_admin_headers(),
        json=save_payload,
    )

    assert save_response.status_code == 200
    assert save_response.json()["state"]["page"]["pageName"] == "冷启动投放页 v2"
    page = db_session.query(StoreMarketingPage).filter_by(page_id=page_id).one()
    assert page.apple_page_id == ""
    assert page.keywords == ""

    delete_image = client.request(
        "DELETE",
        "/admin/api/store-workspace/account-apple-enterprise"
        f"/app-aurora-ios/marketing-pages/{page_id}/store-images",
        headers=_admin_headers(),
        json={
            "locale": "en-US",
            "slotKey": "phone_screenshots",
            "storageKey": "store-assets/marketing/en-US/phone/01.png",
        },
    )

    assert delete_image.status_code == 200
    db_session.expire_all()
    locale = db_session.query(StoreMarketingPageLocale).filter_by(locale="en-US").one()
    assert locale.store_images_json["phone_screenshots"]["assets"] == []

    copy_response = client.post(
        "/admin/api/store-workspace/account-apple-enterprise"
        f"/app-aurora-ios/marketing-pages/{page_id}/copy",
        headers=_admin_headers(),
    )

    assert copy_response.status_code == 200
    assert db_session.query(StoreMarketingPage).count() == 2

    delete_response = client.request(
        "DELETE",
        "/admin/api/store-workspace/account-apple-enterprise"
        f"/app-aurora-ios/marketing-pages/{page_id}",
        headers=_admin_headers(),
    )

    assert delete_response.status_code == 200
    assert db_session.query(StoreMarketingPage).count() == 1


def test_admin_api_marketing_pages_keeps_shared_store_image_file_until_unreferenced(
    client: TestClient,
    db_session: Session,
    monkeypatch,
) -> None:
    seed_demo_catalog(db_session)
    payload = _marketing_payload()
    payload["locales"][1]["storeImages"] = {
        "phone_screenshots": {
            "assets": [
                {
                    "storageKey": "store-assets/marketing/en-US/phone/01.png",
                    "downloadUrl": "https://dist.example.test/marketing-01.png",
                    "fileName": "marketing-01.png",
                }
            ]
        }
    }
    create_response = client.post(
        "/admin/api/store-workspace/account-apple-enterprise"
        "/app-aurora-ios/marketing-pages",
        headers=_admin_headers(),
        json=payload,
    )
    page_id = create_response.json()["state"]["page"]["pageId"]
    deleted: list[str] = []

    class FakeStorage:
        def delete(self, storage_key: str) -> None:
            deleted.append(storage_key)

    monkeypatch.setattr(client.app.state, "artifact_storage", FakeStorage())

    response = client.request(
        "DELETE",
        "/admin/api/store-workspace/account-apple-enterprise"
        f"/app-aurora-ios/marketing-pages/{page_id}/store-images",
        headers=_admin_headers(),
        json={
            "locale": "zh-Hant",
            "slotKey": "phone_screenshots",
            "storageKey": "store-assets/marketing/en-US/phone/01.png",
        },
    )

    assert response.status_code == 200
    assert deleted == []


def test_admin_api_marketing_pages_sync_creates_runs(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)
    create_response = client.post(
        "/admin/api/store-workspace/account-apple-enterprise"
        "/app-aurora-ios/marketing-pages",
        headers=_admin_headers(),
        json=_marketing_payload(),
    )
    page_id = create_response.json()["state"]["page"]["pageId"]
    payload = _marketing_payload()
    payload["syncScopes"] = ["marketing_text", "store_images"]

    response = client.post(
        "/admin/api/store-workspace/account-apple-enterprise"
        f"/app-aurora-ios/marketing-pages/{page_id}/sync",
        headers=_admin_headers(),
        json=payload,
    )

    assert response.status_code == 200
    assert response.json()["message"] == "营销页面已同步 2 个语言"
    assert [item["operation"] for item in response.json()["syncRuns"]] == [
        "update_marketing_page",
        "update_marketing_page",
    ]
    assert db_session.query(StoreSyncRun).filter_by(operation="update_marketing_page").count() == 2


def test_admin_api_marketing_pages_uploads_store_images(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)
    create_response = client.post(
        "/admin/api/store-workspace/account-apple-enterprise"
        "/app-aurora-ios/marketing-pages",
        headers=_admin_headers(),
        json=_marketing_payload(),
    )
    page_id = create_response.json()["state"]["page"]["pageId"]

    response = client.post(
        "/admin/api/store-workspace/account-apple-enterprise"
        f"/app-aurora-ios/marketing-pages/{page_id}/store-images",
        headers=_admin_headers(),
        files=[
            (
                "storeImageFiles__phone_screenshots__zh-Hant",
                ("iphone-69.png", _png_header(1290, 2796), "image/png"),
            )
        ],
    )

    assert response.status_code == 200
    body = response.json()
    assert body["message"] == "已上传 1 张营销页面截图草稿"
    zh_hant = next(item for item in body["state"]["localizedPage"] if item["locale"] == "zh-Hant")
    assets = zh_hant["storeImages"]["phone_screenshots"]["assets"]
    assert assets[0]["fileName"] == "iphone-69.png"
    locale = db_session.query(StoreMarketingPageLocale).filter_by(locale="zh-Hant").one()
    assert locale.store_images_json["phone_screenshots"]["assets"][0]["width"] == 1290

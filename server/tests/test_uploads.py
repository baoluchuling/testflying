from __future__ import annotations

from fastapi.testclient import TestClient
from sqlalchemy.orm import Session

from testflying_api.schema import App, Build, DeviceBuildVisibility
from testflying_api.seed import seed_demo_catalog
from tests.fixtures import make_android_apk_bytes, make_ipa_bytes


def test_upload_ipa_creates_build_without_install_task(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.post(
        "/v1/test-distribution/uploads",
        data={
            "platform": "ios",
            "environment": "development",
            "changelog": "验证新上传链路",
        },
        files={"file": ("AuroraNew.ipa", make_ipa_bytes(), "application/octet-stream")},
    )

    assert response.status_code == 200
    body = response.json()
    assert body["build"]["installInfo"]["installUrl"].startswith("itms-services://")
    assert body["build"]["installInfo"]["manifestUrl"].endswith("/manifest.plist")
    assert body["build"]["environment"] == "development"

    workspace_response = client.get(
        "/v1/test-distribution/workspace",
        headers={"X-Device-ID": "device-001", "X-Client-Platform": "ios"},
    )
    workspace = workspace_response.json()
    assert body["build"]["id"] in {build["id"] for build in workspace["builds"]}
    assert workspace["installTasks"] == []
    assert (
        db_session.query(DeviceBuildVisibility).filter_by(build_id=body["build"]["id"]).count() >= 1
    )


def test_upload_android_parses_metadata_from_apk(
    client: TestClient,
    db_session: Session,
) -> None:
    response = client.post(
        "/v1/test-distribution/uploads",
        data={
            "platform": "android",
            "environment": "development",
            "changelog": "自动解析 Android metadata",
        },
        files={
            "file": (
                "app.apk",
                make_android_apk_bytes(),
                "application/vnd.android.package-archive",
            )
        },
    )

    assert response.status_code == 200
    body = response.json()
    assert body["app"]["name"] == "Auto Parsed"
    assert body["build"]["version"] == "4.5.6"
    assert body["build"]["buildNumber"] == "321"
    assert body["build"]["installInfo"]["installUrl"].endswith("/app.apk")

    app = db_session.get(App, body["app"]["id"])
    assert app is not None
    assert app.bundle_identifier == "com.example.autoparse"

    build = db_session.get(Build, body["build"]["id"])
    assert build is not None
    package_artifact = build.package_artifact()
    assert package_artifact is not None
    assert package_artifact.artifact_type == "package"
    assert package_artifact.metadata_json == {"source": "upload"}


def test_upload_can_attach_new_app_to_developer_account(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.post(
        "/v1/test-distribution/uploads",
        data={
            "platform": "android",
            "environment": "development",
            "developerAccountId": "account-apple-enterprise",
            "changelog": "带账号上传",
        },
        files={
            "file": (
                "app.apk",
                make_android_apk_bytes(),
                "application/vnd.android.package-archive",
            )
        },
    )

    assert response.status_code == 200
    body = response.json()
    app = db_session.get(App, body["app"]["id"])
    assert app is not None
    assert app.developer_account_id == "account-apple-enterprise"
    assert app.store_package_name == "com.example.autoparse"


def test_upload_rejects_wrong_store_identifier_for_android(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.post(
        "/v1/test-distribution/uploads",
        data={
            "platform": "android",
            "environment": "development",
            "developerAccountId": "account-apple-enterprise",
            "storeAppId": "1234567890",
            "changelog": "带错误商店标识上传",
        },
        files={
            "file": (
                "app.apk",
                make_android_apk_bytes(),
                "application/vnd.android.package-archive",
            )
        },
    )

    assert response.status_code == 422
    assert (
        response.json()["message"]
        == "Android App 只能填写 Google Play package name，不能填写 App Store Connect App ID。"
    )

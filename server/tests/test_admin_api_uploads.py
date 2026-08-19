from __future__ import annotations

from base64 import b64encode

from fastapi.testclient import TestClient
from sqlalchemy.orm import Session

from testflying_api.schema import App
from testflying_api.seed import seed_demo_catalog
from tests.fixtures import make_android_apk_bytes


def _admin_headers(password: str = "dev-token") -> dict[str, str]:
    token = b64encode(f"admin:{password}".encode()).decode()
    return {"Authorization": f"Basic {token}"}


def test_admin_api_upload_state_lists_developer_accounts(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.get("/admin/api/uploads", headers=_admin_headers())

    assert response.status_code == 200
    payload = response.json()
    assert payload["accounts"] == [
            {
                "id": "account-apple-enterprise",
                "teamName": "Internal Distribution Team",
                "status": "renewal_due",
                "platform": None,
            }
    ]


def test_admin_api_upload_package_returns_parsed_result(
    client: TestClient,
    db_session: Session,
) -> None:
    response = client.post(
        "/admin/api/uploads",
        headers=_admin_headers(),
        data={
            "platform": "android",
            "environment": "development",
            "changelog": "React admin upload",
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
    payload = response.json()
    assert payload["message"] == "上传成功，包信息已自动解析"
    assert payload["result"]["appName"] == "Auto Parsed"
    assert payload["result"]["bundleIdentifier"] == "com.example.autoparse"
    assert payload["result"]["platform"] == "android"
    assert payload["result"]["environment"] == "development"
    assert payload["result"]["version"] == "4.5.6"
    assert payload["result"]["buildNumber"] == "321"
    assert payload["result"]["storeIdentifier"] == "com.example.autoparse"
    assert payload["result"]["installUrl"].endswith("/app.apk")

    app = db_session.get(App, payload["result"]["appId"])
    assert app is not None
    assert app.bundle_identifier == "com.example.autoparse"

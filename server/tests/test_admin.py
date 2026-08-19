from __future__ import annotations

from base64 import b64encode

from fastapi.testclient import TestClient
from sqlalchemy.orm import Session

from testflying_api.seed import seed_demo_catalog


def _admin_headers(password: str = "dev-token") -> dict[str, str]:
    token = b64encode(f"admin:{password}".encode()).decode()
    return {"Authorization": f"Basic {token}"}


def _assert_admin_spa_shell(response) -> None:
    assert response.status_code == 200
    assert 'data-admin-app-root' in response.text
    assert "/assets/index-" in response.text
    assert "/static/admin/admin.css" not in response.text


def test_admin_requires_basic_auth(client: TestClient) -> None:
    response = client.get("/admin")

    assert response.status_code == 401
    assert response.headers["www-authenticate"] == "Basic"


def test_admin_serves_react_shell(client: TestClient) -> None:
    response = client.get("/admin", headers=_admin_headers())

    _assert_admin_spa_shell(response)
    assert "testflying" in response.text


def test_admin_spa_fallback_serves_nested_routes(client: TestClient) -> None:
    for path in (
        "/admin/apps",
        "/admin/store-reviews",
        "/admin/api-docs",
        "/admin/accounts/account-apple-enterprise/apps/app-insight-ios/store",
        "/admin/accounts/account-apple-enterprise/apps/app-insight-ios/marketing-pages/page-1",
    ):
        response = client.get(path, headers=_admin_headers())

        _assert_admin_spa_shell(response)


def test_admin_bootstrap_uses_canonical_admin_paths(client: TestClient) -> None:
    response = client.get("/admin/api/bootstrap", headers=_admin_headers())

    assert response.status_code == 200
    nav_paths = [item["path"] for item in response.json()["navItems"]]
    assert nav_paths[0] == "/admin"
    assert all(not path.startswith("/admin-next") for path in nav_paths)


def test_admin_dashboard_api_still_exposes_seeded_catalog(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_demo_catalog(db_session)

    response = client.get("/admin/api/dashboard", headers=_admin_headers())

    assert response.status_code == 200
    payload = response.json()
    assert payload["recentBuilds"]
    assert payload["recentNotifications"][0]["title"] == "Apple 开发者账号即将到期"


def test_admin_api_docs_markdown_download(client: TestClient) -> None:
    response = client.get("/admin/api-docs/store-management.md", headers=_admin_headers())

    assert response.status_code == 200
    assert response.headers["content-type"].startswith("text/markdown")
    assert 'filename="testflying-store-management-api.md"' in response.headers[
        "content-disposition"
    ]
    assert "# testflying 商店连接对外 API" in response.text
    assert "第三方电脑或外部系统调用" in response.text
    assert "/v1/connectors" not in response.text


def test_admin_next_redirects_to_canonical_admin(client: TestClient) -> None:
    expected = {
        "/admin-next": "/admin",
        "/admin-next/apps": "/admin/apps",
        "/admin-next/accounts/account-apple-enterprise/apps/app-insight-ios/store": (
            "/admin/accounts/account-apple-enterprise/apps/app-insight-ios/store"
        ),
    }

    for path, target in expected.items():
        response = client.get(path, headers=_admin_headers(), follow_redirects=False)

        assert response.status_code == 307
        assert response.headers["location"] == target

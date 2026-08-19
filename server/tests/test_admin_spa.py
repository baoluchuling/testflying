from __future__ import annotations

from fastapi.testclient import TestClient

from tests.test_admin import _admin_headers, _assert_admin_spa_shell


def test_admin_spa_requires_basic_auth(client: TestClient) -> None:
    response = client.get("/admin")

    assert response.status_code == 401
    assert response.headers["www-authenticate"] == "Basic"


def test_admin_spa_serves_react_shell(client: TestClient) -> None:
    response = client.get("/admin", headers=_admin_headers())

    _assert_admin_spa_shell(response)
    assert "testflying" in response.text


def test_admin_spa_fallback_serves_nested_routes(client: TestClient) -> None:
    for path in (
        "/admin/store-reviews",
        "/admin/builds/apps",
        "/admin/builds/history",
        "/admin/builds/runners",
        "/admin/settings/general",
        "/admin/settings/notifications",
        "/admin/settings/llm",
        "/admin/settings/runtime",
        "/admin/accounts",
        "/admin/accounts/account-apple-enterprise",
        "/admin/accounts/account-apple-enterprise/apps/app-insight-ios/store",
        "/admin/accounts/account-apple-enterprise/apps/app-insight-ios/marketing",
        "/admin/accounts/account-apple-enterprise/apps/app-insight-ios/connection",
        "/admin/accounts/account-apple-enterprise/apps/app-insight-ios/release-notes",
        "/admin/accounts/account-apple-enterprise/apps/app-insight-ios/marketing-pages/page-1",
    ):
        response = client.get(path, headers=_admin_headers())

        _assert_admin_spa_shell(response)


def test_admin_next_redirects_to_canonical_admin(client: TestClient) -> None:
    expected = {
        "/admin-next": "/admin",
        "/admin-next/apps": "/admin/apps",
        "/admin-next/uploads": "/admin/uploads",
        "/admin-next/accounts": "/admin/accounts",
        "/admin-next/accounts/account-apple-enterprise": "/admin/accounts/account-apple-enterprise",
        "/admin-next/accounts/account-apple-enterprise/apps/app-insight-ios/store": (
            "/admin/accounts/account-apple-enterprise/apps/app-insight-ios/store"
        ),
    }

    for path, target in expected.items():
        response = client.get(path, headers=_admin_headers(), follow_redirects=False)

        assert response.status_code == 307
        assert response.headers["location"] == target


def test_admin_resource_routes_do_not_render_spa_shell(client: TestClient) -> None:
    resources = {
        "/admin/api-docs/store-management.md": "text/markdown",
        "/admin/app-logs/events": "application/json",
        "/admin/app-logs/qr.svg": "image/svg+xml",
    }

    for path, content_type in resources.items():
        response = client.get(path, headers=_admin_headers(), follow_redirects=False)

        assert response.status_code == 200
        assert response.headers["content-type"].startswith(content_type)
        assert "data-admin-app-root" not in response.text

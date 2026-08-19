from __future__ import annotations

from base64 import b64encode

from fastapi.testclient import TestClient


def _admin_headers(password: str = "dev-token") -> dict[str, str]:
    token = b64encode(f"admin:{password}".encode()).decode()
    return {"Authorization": f"Basic {token}"}


def test_admin_api_bootstrap_requires_basic_auth(client: TestClient) -> None:
    response = client.get("/admin/api/bootstrap")

    assert response.status_code == 401
    assert response.headers["www-authenticate"] == "Basic"


def test_admin_api_bootstrap_returns_shell_metadata(client: TestClient) -> None:
    response = client.get("/admin/api/bootstrap", headers=_admin_headers())

    assert response.status_code == 200
    payload = response.json()
    assert payload["appName"] == "testflying"
    assert payload["health"] == {"state": "idle", "label": "未检查"}
    assert payload["navItems"] == [
        {"key": "dashboard", "label": "总览", "path": "/admin"},
        {"key": "uploads", "label": "上传", "path": "/admin/uploads"},
        {"key": "apps", "label": "商店管理", "path": "/admin/apps"},
        {"key": "store-reviews", "label": "商店评论", "path": "/admin/store-reviews"},
        {"key": "api-docs", "label": "接口文档", "path": "/admin/api-docs"},
        {"key": "builds", "label": "构建", "path": "/admin/builds/apps"},
        {"key": "devices", "label": "设备", "path": "/admin/devices"},
        {"key": "app-logs", "label": "App 日志", "path": "/admin/app-logs"},
        {"key": "notifications", "label": "通知", "path": "/admin/notifications"},
        {"key": "settings", "label": "设置", "path": "/admin/settings/general"},
    ]

from __future__ import annotations

from fastapi.testclient import TestClient


def test_web_origin_can_preflight_workspace_request(client: TestClient) -> None:
    response = client.options(
        "/v1/test-distribution/workspace",
        headers={
            "Origin": "http://localhost:8080",
            "Access-Control-Request-Method": "GET",
            "Access-Control-Request-Headers": ("authorization,x-device-id,x-client-platform"),
        },
    )

    assert response.status_code == 200
    assert response.headers["access-control-allow-origin"] == "http://localhost:8080"
    assert "GET" in response.headers["access-control-allow-methods"]
    assert "Authorization" in response.headers["access-control-allow-headers"]

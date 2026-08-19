from __future__ import annotations

from fastapi import FastAPI
from fastapi.testclient import TestClient

from testflying_api.errors import ApiError


def test_api_errors_use_client_contract_shape(app: FastAPI) -> None:
    @app.get("/boom")
    def boom() -> None:
        raise ApiError("build_not_found", "构建不存在", status_code=404)

    client = TestClient(app)
    response = client.get("/boom")

    assert response.status_code == 404
    assert response.json() == {
        "code": "build_not_found",
        "message": "构建不存在",
        "retryable": False,
    }

from __future__ import annotations

from base64 import b64encode

from fastapi.testclient import TestClient


def _admin_headers(password: str = "dev-token") -> dict[str, str]:
    token = b64encode(f"admin:{password}".encode()).decode()
    return {"Authorization": f"Basic {token}"}


def test_admin_api_llm_config_lists_protocols_and_features(client: TestClient) -> None:
    response = client.get("/admin/api/llm-config", headers=_admin_headers())

    assert response.status_code == 200
    payload = response.json()
    assert [item["key"] for item in payload["protocols"]] == [
        "openai_compatible",
        "claude_compatible",
    ]
    assert {item["featureKey"] for item in payload["featureBindings"]} == {
        "feedback_classification",
        "review_analysis",
        "translation",
    }
    assert all(item["primaryProfileId"] is None for item in payload["featureBindings"])


def test_admin_api_llm_config_creates_profile_and_binds_feature(
    client: TestClient,
) -> None:
    create_response = client.post(
        "/admin/api/llm-config/profiles",
        headers=_admin_headers(),
        json={
            "name": "小米 MiMo",
            "protocol": "openai_compatible",
            "baseUrl": "https://token-plan-cn.xiaomimimo.com/v1",
            "model": "mimo-v2.5-pro",
            "apiKey": "secret-123456",
            "authHeader": "api-key",
        },
    )

    assert create_response.status_code == 200
    created = create_response.json()
    profile = created["profile"]
    assert profile["protocol"] == "openai_compatible"
    assert profile["authHeader"] == "api-key"
    assert profile["apiKeySet"] is True
    assert profile["apiKeyPreview"] == "secr...3456"

    bind_response = client.put(
        "/admin/api/llm-config/bindings/review_analysis",
        headers=_admin_headers(),
        json={"primaryProfileId": profile["id"]},
    )

    assert bind_response.status_code == 200
    binding = bind_response.json()["binding"]
    assert binding["featureLabel"] == "评论分析"
    assert binding["primaryProfileId"] == profile["id"]
    assert binding["fallbackProfileId"] is None
    assert binding["status"] == "ready"


def test_admin_api_llm_config_keeps_existing_key_when_update_key_empty(
    client: TestClient,
) -> None:
    create_response = client.post(
        "/admin/api/llm-config/profiles",
        headers=_admin_headers(),
        json={
            "name": "Claude 兼容",
            "protocol": "claude_compatible",
            "baseUrl": "https://api.example.com",
            "model": "claude-like-model",
            "apiKey": "saved-secret",
            "authHeader": "x-api-key",
        },
    )
    profile_id = create_response.json()["profile"]["id"]

    update_response = client.patch(
        f"/admin/api/llm-config/profiles/{profile_id}",
        headers=_admin_headers(),
        json={
            "name": "Claude 兼容生产",
            "protocol": "claude_compatible",
            "baseUrl": "https://api.example.com",
            "model": "claude-like-model",
            "apiKey": "",
            "authHeader": "x-api-key",
        },
    )

    assert update_response.status_code == 200
    profile = update_response.json()["profile"]
    assert profile["name"] == "Claude 兼容生产"
    assert profile["apiKeySet"] is True
    assert profile["apiKeyPreview"] == "save...cret"

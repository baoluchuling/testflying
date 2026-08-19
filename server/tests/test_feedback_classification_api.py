from __future__ import annotations

import json
from base64 import b64encode
from typing import Any

from fastapi.testclient import TestClient

from testflying_api import feedback_classification


def _admin_headers(password: str = "dev-token") -> dict[str, str]:
    token = b64encode(f"admin:{password}".encode()).decode()
    return {"Authorization": f"Basic {token}"}


def _public_headers(token: str = "dev-token") -> dict[str, str]:
    return {"Authorization": f"Bearer {token}"}


def _bind_feedback_classifier(
    client: TestClient,
    *,
    protocol: str = "openai_compatible",
    base_url: str = "https://token-plan-cn.xiaomimimo.com/v1",
    model: str = "mimo-v2.5-pro",
    auth_header: str = "api-key",
) -> str:
    create_response = client.post(
        "/admin/api/llm-config/profiles",
        headers=_admin_headers(),
        json={
            "name": "小米 MiMo",
            "protocol": protocol,
            "baseUrl": base_url,
            "model": model,
            "apiKey": "secret-123456",
            "authHeader": auth_header,
        },
    )
    assert create_response.status_code == 200
    profile_id = create_response.json()["profile"]["id"]
    bind_response = client.put(
        "/admin/api/llm-config/bindings/feedback_classification",
        headers=_admin_headers(),
        json={"primaryProfileId": profile_id},
    )
    assert bind_response.status_code == 200
    return profile_id


def test_feedback_classification_requires_static_token(client: TestClient) -> None:
    response = client.post(
        "/v1/llm/feedback-classifications",
        json={"content": "打不开，一直闪退"},
    )

    assert response.status_code == 401
    assert response.json()["code"] == "invalid_static_token"


def test_feedback_classification_requires_llm_configuration(client: TestClient) -> None:
    response = client.post(
        "/v1/llm/feedback-classifications",
        headers=_public_headers(),
        json={"content": "打不开，一直闪退"},
    )

    assert response.status_code == 503
    assert response.json()["code"] == "feedback_classification_not_configured"


def test_feedback_classification_calls_openai_compatible_llm(
    client: TestClient,
    monkeypatch,
) -> None:
    _bind_feedback_classifier(client)
    captured: dict[str, Any] = {}

    class FakeResponse:
        def __enter__(self) -> FakeResponse:
            return self

        def __exit__(self, *args: object) -> None:
            return None

        def read(self) -> bytes:
            return json.dumps(
                {
                    "choices": [
                        {
                            "message": {
                                "content": json.dumps(
                                    {
                                        "feedbackId": "fb-1",
                                        "category": "crash",
                                        "categoryLabel": "闪退问题",
                                        "isBug": True,
                                        "isSuggestion": False,
                                        "severity": "high",
                                        "priority": "p1",
                                        "confidence": 0.88,
                                        "summary": "用户反馈新版本持续闪退。",
                                        "problem": "应用打开后异常退出。",
                                        "evidence": ["打不开", "一直闪退"],
                                        "suggestedAction": "优先排查新版本启动崩溃。",
                                        "routing": {
                                            "team": "client",
                                            "labels": ["crash", "ios"],
                                        },
                                        "needsHumanReview": False,
                                    },
                                    ensure_ascii=False,
                                )
                            }
                        }
                    ]
                },
                ensure_ascii=False,
            ).encode("utf-8")

    def fake_urlopen(request, timeout: int):  # noqa: ANN001
        captured["timeout"] = timeout
        captured["url"] = request.full_url
        captured["headers"] = dict(request.headers)
        captured["payload"] = json.loads(request.data.decode("utf-8"))
        return FakeResponse()

    monkeypatch.setattr(feedback_classification, "urlopen", fake_urlopen)

    response = client.post(
        "/v1/llm/feedback-classifications",
        headers=_public_headers(),
        json={
            "feedbackId": "fb-1",
            "content": "打不开，一直闪退，更新后就这样",
            "source": "app_store",
            "platform": "ios",
            "app": {"id": "com.example.app", "name": "Example", "version": "1.0.0"},
            "context": {"rating": 1},
        },
    )

    assert response.status_code == 200
    assert captured["timeout"] == 45
    assert captured["url"] == "https://token-plan-cn.xiaomimimo.com/v1/chat/completions"
    captured_headers = {key.lower(): value for key, value in captured["headers"].items()}
    assert captured_headers["api-key"] == "secret-123456"
    assert "所有输出必须使用简体中文" in captured["payload"]["messages"][0]["content"]
    assert captured["payload"]["messages"][1]["content"]

    payload = response.json()
    assert payload["feedbackId"] == "fb-1"
    assert payload["category"] == "crash"
    assert payload["isBug"] is True
    assert payload["isSuggestion"] is False
    assert payload["severity"] == "high"
    assert payload["priority"] == "p1"
    assert payload["routing"]["team"] == "client"
    assert payload["model"]["model"] == "mimo-v2.5-pro"


def test_feedback_classification_sends_images_to_openai_compatible_llm(
    client: TestClient,
    monkeypatch,
) -> None:
    _bind_feedback_classifier(client)
    captured: dict[str, Any] = {}

    class FakeResponse:
        def __enter__(self) -> FakeResponse:
            return self

        def __exit__(self, *args: object) -> None:
            return None

        def read(self) -> bytes:
            return json.dumps(
                {
                    "choices": [
                        {
                            "message": {
                                "content": json.dumps(
                                    {
                                        "feedbackId": "fb-image-1",
                                        "category": "usability",
                                        "categoryLabel": "体验问题",
                                        "isBug": False,
                                        "isSuggestion": True,
                                        "severity": "medium",
                                        "priority": "p2",
                                        "confidence": 0.8,
                                        "summary": "用户上传截图反馈按钮遮挡。",
                                        "problem": "截图显示界面元素可能遮挡操作。",
                                        "evidence": ["按钮挡住了内容"],
                                        "suggestedAction": "请产品和客户端确认截图对应页面布局。",
                                        "routing": {"team": "client", "labels": ["ui"]},
                                        "needsHumanReview": False,
                                    },
                                    ensure_ascii=False,
                                )
                            }
                        }
                    ]
                },
                ensure_ascii=False,
            ).encode("utf-8")

    def fake_urlopen(request, timeout: int):  # noqa: ANN001, ARG001
        captured["payload"] = json.loads(request.data.decode("utf-8"))
        return FakeResponse()

    monkeypatch.setattr(feedback_classification, "urlopen", fake_urlopen)

    response = client.post(
        "/v1/llm/feedback-classifications",
        headers=_public_headers(),
        json={
            "feedbackId": "fb-image-1",
            "images": [
                {
                    "url": "https://cdn.example.test/feedback/screen.png",
                    "name": "用户截图",
                    "mimeType": "image/png",
                    "detail": "high",
                }
            ],
        },
    )

    assert response.status_code == 200
    user_content = captured["payload"]["messages"][1]["content"]
    assert isinstance(user_content, list)
    assert user_content[0]["type"] == "text"
    text_payload = json.loads(user_content[0]["text"])
    assert text_payload["images"][0]["name"] == "用户截图"
    assert text_payload["images"][0]["url"] == "https://cdn.example.test/feedback/screen.png"
    assert user_content[1] == {
        "type": "image_url",
        "image_url": {
            "url": "https://cdn.example.test/feedback/screen.png",
            "detail": "high",
        },
    }


def test_feedback_classification_sends_data_url_images_to_claude_compatible_llm(
    client: TestClient,
    monkeypatch,
) -> None:
    _bind_feedback_classifier(
        client,
        protocol="claude_compatible",
        base_url="https://claude.example.test",
        model="claude-test",
        auth_header="x-api-key",
    )
    captured: dict[str, Any] = {}

    class FakeResponse:
        def __enter__(self) -> FakeResponse:
            return self

        def __exit__(self, *args: object) -> None:
            return None

        def read(self) -> bytes:
            return json.dumps(
                {
                    "content": [
                        {
                            "type": "text",
                            "text": json.dumps(
                                {
                                    "feedbackId": "fb-claude-image",
                                    "category": "bug",
                                    "categoryLabel": "缺陷",
                                    "isBug": True,
                                    "isSuggestion": False,
                                    "severity": "medium",
                                    "priority": "p2",
                                    "confidence": 0.78,
                                    "summary": "用户截图反馈页面异常。",
                                    "problem": "页面显示异常。",
                                    "evidence": ["截图显示异常"],
                                    "suggestedAction": "请结合截图复现。",
                                    "routing": {"team": "client", "labels": ["image"]},
                                    "needsHumanReview": False,
                                },
                                ensure_ascii=False,
                            ),
                        }
                    ]
                },
                ensure_ascii=False,
            ).encode("utf-8")

    def fake_urlopen(request, timeout: int):  # noqa: ANN001, ARG001
        captured["url"] = request.full_url
        captured["headers"] = dict(request.headers)
        captured["payload"] = json.loads(request.data.decode("utf-8"))
        return FakeResponse()

    monkeypatch.setattr(feedback_classification, "urlopen", fake_urlopen)

    response = client.post(
        "/v1/llm/feedback-classifications",
        headers=_public_headers(),
        json={
            "feedbackId": "fb-claude-image",
            "content": "页面显示异常，看截图",
            "images": [
                {
                    "url": "data:image/png;base64,aGVsbG8=",
                    "name": "异常截图",
                    "mimeType": "image/png",
                }
            ],
        },
    )

    assert response.status_code == 200
    assert captured["url"] == "https://claude.example.test/v1/messages"
    captured_headers = {key.lower(): value for key, value in captured["headers"].items()}
    assert captured_headers["x-api-key"] == "secret-123456"
    user_content = captured["payload"]["messages"][0]["content"]
    assert isinstance(user_content, list)
    assert user_content[0]["type"] == "text"
    text_payload = json.loads(user_content[0]["text"])
    assert text_payload["images"][0]["url"] == "data-url:image/png"
    assert user_content[1] == {
        "type": "image",
        "source": {
            "type": "base64",
            "media_type": "image/png",
            "data": "aGVsbG8=",
        },
    }


def test_feedback_classification_rejects_http_image_for_claude_compatible_llm(
    client: TestClient,
    monkeypatch,
) -> None:
    _bind_feedback_classifier(
        client,
        protocol="claude_compatible",
        base_url="https://claude.example.test",
        model="claude-test",
        auth_header="x-api-key",
    )

    def fake_urlopen(request, timeout: int):  # noqa: ANN001, ARG001
        raise AssertionError("Claude HTTP 图片应在请求 LLM 前被拦截")

    monkeypatch.setattr(feedback_classification, "urlopen", fake_urlopen)

    response = client.post(
        "/v1/llm/feedback-classifications",
        headers=_public_headers(),
        json={
            "content": "页面显示异常，看截图",
            "images": [{"url": "https://cdn.example.test/feedback/screen.png"}],
        },
    )

    assert response.status_code == 422
    assert response.json()["code"] == "feedback_image_unsupported"


def test_feedback_classification_rejects_invalid_image_url(client: TestClient) -> None:
    _bind_feedback_classifier(client)

    response = client.post(
        "/v1/llm/feedback-classifications",
        headers=_public_headers(),
        json={
            "content": "页面显示异常，看截图",
            "images": [{"url": "file:///tmp/screen.png"}],
        },
    )

    assert response.status_code == 422
    assert response.json()["code"] == "invalid_feedback_image_url"


def test_feedback_classification_rejects_invalid_llm_json(
    client: TestClient,
    monkeypatch,
) -> None:
    _bind_feedback_classifier(client)

    class FakeResponse:
        def __enter__(self) -> FakeResponse:
            return self

        def __exit__(self, *args: object) -> None:
            return None

        def read(self) -> bytes:
            return json.dumps(
                {"choices": [{"message": {"content": "不是 JSON"}}]},
                ensure_ascii=False,
            ).encode("utf-8")

    def fake_urlopen(request, timeout: int):  # noqa: ANN001, ARG001
        return FakeResponse()

    monkeypatch.setattr(feedback_classification, "urlopen", fake_urlopen)

    response = client.post(
        "/v1/llm/feedback-classifications",
        headers=_public_headers(),
        json={"content": "打不开，一直闪退"},
    )

    assert response.status_code == 502
    assert response.json()["code"] == "llm_invalid_response"

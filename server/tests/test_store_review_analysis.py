from __future__ import annotations

import json
from datetime import UTC, datetime
from typing import Any

from testflying_api import store_reviews
from testflying_api.llm_config import LlmRuntimeConfig
from testflying_api.schema import App, StoreReview


def test_openai_review_analysis_requests_chinese_structured_json(monkeypatch) -> None:
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
                                        "summary": "评论集中反馈登录失败。",
                                        "issues": [
                                            {
                                                "title": "登录失败影响进入",
                                                "severity": "high",
                                                "count": 1,
                                                "focus": "需要确认登录服务是否有回归。",
                                                "evidence": ["登录后一直转圈"],
                                                "suggestion": "优先复测最新版本登录链路。",
                                                "representativeReviewIds": ["review-1"],
                                            }
                                        ],
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
        captured["payload"] = json.loads(request.data.decode("utf-8"))
        return FakeResponse()

    monkeypatch.setattr(store_reviews, "urlopen", fake_urlopen)

    result = store_reviews._review_analysis_with_openai_compatible(
        LlmRuntimeConfig(
            provider="configured",
            protocol="openai_compatible",
            base_url="https://llm.example.test/v1",
            model="test-model",
            api_key="secret",
            auth_header="authorization_bearer",
        ),
        app=_app(),
        reviews=[_review()],
    )

    system_prompt = captured["payload"]["messages"][0]["content"]
    assert "所有输出必须使用简体中文" in system_prompt
    assert '"evidence"' in system_prompt
    assert '"suggestion"' in system_prompt
    assert result["summary"] == "评论集中反馈登录失败。"
    assert result["issues"][0]["evidence"] == ["登录后一直转圈"]


def test_openai_review_analysis_extracts_fenced_json_and_sorts_issues(monkeypatch) -> None:
    class FakeResponse:
        def __enter__(self) -> FakeResponse:
            return self

        def __exit__(self, *args: object) -> None:
            return None

        def read(self) -> bytes:
            content = (
                "分析结果如下：\n"
                "```json\n"
                + json.dumps(
                    {
                        "summary": "需要优先处理登录失败。",
                        "issues": [
                            {
                                "title": "轻微文案问题",
                                "severity": "low",
                                "count": 20,
                                "focus": "部分用户觉得文案不清楚。",
                            },
                            {
                                "title": "登录失败",
                                "severity": "high",
                                "count": 2,
                                "focus": "用户无法进入应用。",
                            },
                            {
                                "title": "加载慢",
                                "severity": "medium",
                                "count": 5,
                                "focus": "启动后加载时间较长。",
                            },
                        ],
                    },
                    ensure_ascii=False,
                )
                + "\n```"
            )
            return json.dumps(
                {"choices": [{"message": {"content": content}}]}, ensure_ascii=False
            ).encode("utf-8")

    def fake_urlopen(request, timeout: int):  # noqa: ANN001, ARG001
        return FakeResponse()

    monkeypatch.setattr(store_reviews, "urlopen", fake_urlopen)

    result = store_reviews._review_analysis_with_openai_compatible(
        LlmRuntimeConfig(
            provider="configured",
            protocol="openai_compatible",
            base_url="https://llm.example.test/v1",
            model="test-model",
            api_key="secret",
            auth_header="authorization_bearer",
        ),
        app=_app(),
        reviews=[_review()],
    )

    assert result["summary"] == "需要优先处理登录失败。"
    assert [issue["severity"] for issue in result["issues"]] == ["high", "medium", "low"]
    assert [issue["title"] for issue in result["issues"]] == ["登录失败", "加载慢", "轻微文案问题"]


def test_claude_review_analysis_requests_chinese_structured_json(monkeypatch) -> None:
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
                                    "summary": "暂未发现集中问题。",
                                    "issues": [],
                                },
                                ensure_ascii=False,
                            ),
                        }
                    ]
                },
                ensure_ascii=False,
            ).encode("utf-8")

    def fake_urlopen(request, timeout: int):  # noqa: ANN001
        captured["timeout"] = timeout
        captured["payload"] = json.loads(request.data.decode("utf-8"))
        return FakeResponse()

    monkeypatch.setattr(store_reviews, "urlopen", fake_urlopen)

    store_reviews._review_analysis_with_claude_compatible(
        LlmRuntimeConfig(
            provider="configured",
            protocol="claude_compatible",
            base_url="https://llm.example.test",
            model="test-model",
            api_key="secret",
            auth_header="x-api-key",
        ),
        app=_app(),
        reviews=[_review()],
    )

    system_prompt = captured["payload"]["system"]
    assert "所有输出必须使用简体中文" in system_prompt
    assert '"evidence"' in system_prompt
    assert '"suggestion"' in system_prompt


def _app() -> App:
    return App(
        id="app-ios",
        name="lookrva",
        platform="ios",
        bundle_identifier="com.example.lookrva",
    )


def _review() -> StoreReview:
    return StoreReview(
        id="review-row-1",
        developer_account_id="account-ios",
        app_id="app-ios",
        platform="ios",
        store_review_id="review-1",
        rating=1,
        title="登录失败",
        body="登录后一直转圈",
        locale="zh-Hans",
        app_version="1.0",
        created_at=datetime(2026, 7, 3, tzinfo=UTC),
    )

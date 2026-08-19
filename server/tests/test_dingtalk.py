from __future__ import annotations

import hashlib
import hmac
import json
from base64 import b64encode
from io import BytesIO
from typing import Any
from urllib.parse import parse_qs, urlsplit

import pytest

from testflying_api.dingtalk import (
    DingTalkDeliveryError,
    send_dingtalk_markdown,
    signed_webhook_url,
)


class FakeResponse:
    def __init__(self, payload: dict[str, Any]) -> None:
        self._body = BytesIO(json.dumps(payload).encode())

    def __enter__(self) -> FakeResponse:
        return self

    def __exit__(self, *_args: object) -> None:
        return None

    def read(self, limit: int = -1) -> bytes:
        return self._body.read(limit)


def test_signed_webhook_url_uses_dingtalk_hmac() -> None:
    timestamp = 1_700_000_000_123
    secret = "SEC-test"
    expected_signature = b64encode(
        hmac.new(
            secret.encode(),
            f"{timestamp}\n{secret}".encode(),
            hashlib.sha256,
        ).digest()
    ).decode()

    signed = signed_webhook_url(
        "https://oapi.dingtalk.test/robot/send?access_token=abc",
        secret,
        timestamp,
    )

    query = parse_qs(urlsplit(signed).query)
    assert query["access_token"] == ["abc"]
    assert query["timestamp"] == [str(timestamp)]
    assert query["sign"] == [expected_signature]


def test_send_dingtalk_markdown_posts_expected_payload() -> None:
    captured: dict[str, Any] = {}

    def opener(request, *, timeout: float):
        captured["request"] = request
        captured["timeout"] = timeout
        return FakeResponse({"errcode": 0, "errmsg": "ok"})

    send_dingtalk_markdown(
        webhook_url="https://oapi.dingtalk.test/robot/send?access_token=abc",
        secret="SEC-test",
        title="构建需要处理",
        markdown="**Demo** build failed",
        timeout_seconds=7.0,
        timestamp_ms=1_700_000_000_123,
        opener=opener,
    )

    request = captured["request"]
    assert captured["timeout"] == 7.0
    assert request.get_method() == "POST"
    assert request.get_header("Content-type") == "application/json"
    assert json.loads(request.data) == {
        "msgtype": "markdown",
        "markdown": {"title": "构建需要处理", "text": "**Demo** build failed"},
    }


def test_send_dingtalk_markdown_redacts_nonzero_response_error() -> None:
    def opener(_request, *, timeout: float):
        assert timeout == 5.0
        return FakeResponse({"errcode": 310000, "errmsg": "token=secret-value rejected"})

    with pytest.raises(DingTalkDeliveryError) as captured:
        send_dingtalk_markdown(
            webhook_url="https://oapi.dingtalk.test/robot/send?access_token=abc",
            secret="SEC-test",
            title="构建需要处理",
            markdown="failed",
            timeout_seconds=5.0,
            opener=opener,
        )

    assert "secret-value" not in str(captured.value)
    assert "token=[REDACTED]" in str(captured.value)


def test_send_dingtalk_markdown_hides_transport_details() -> None:
    def opener(_request, *, timeout: float):
        assert timeout == 5.0
        raise TimeoutError("https://oapi.dingtalk.test/?access_token=abc")

    with pytest.raises(DingTalkDeliveryError) as captured:
        send_dingtalk_markdown(
            webhook_url="https://oapi.dingtalk.test/robot/send?access_token=abc",
            secret="SEC-test",
            title="构建需要处理",
            markdown="failed",
            timeout_seconds=5.0,
            opener=opener,
        )

    assert str(captured.value) == "DingTalk request failed: TimeoutError"

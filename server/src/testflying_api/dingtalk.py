from __future__ import annotations

import hashlib
import hmac
import json
import time
from base64 import b64encode
from collections.abc import Callable
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.parse import parse_qsl, urlencode, urlsplit, urlunsplit
from urllib.request import Request, urlopen

from testflying_api.redaction import redact_text


class DingTalkDeliveryError(RuntimeError):
    pass


def signed_webhook_url(url: str, secret: str, timestamp_ms: int) -> str:
    string_to_sign = f"{timestamp_ms}\n{secret}".encode()
    digest = hmac.new(secret.encode(), string_to_sign, hashlib.sha256).digest()
    signature = b64encode(digest).decode()
    parts = urlsplit(url)
    query = parse_qsl(parts.query, keep_blank_values=True)
    query.extend((("timestamp", str(timestamp_ms)), ("sign", signature)))
    return urlunsplit(
        (parts.scheme, parts.netloc, parts.path, urlencode(query), parts.fragment),
    )


def send_dingtalk_markdown(
    *,
    webhook_url: str,
    secret: str,
    title: str,
    markdown: str,
    timeout_seconds: float,
    timestamp_ms: int | None = None,
    opener: Callable[..., Any] = urlopen,
) -> None:
    timestamp = timestamp_ms if timestamp_ms is not None else int(time.time() * 1000)
    payload = json.dumps(
        {
            "msgtype": "markdown",
            "markdown": {"title": title, "text": markdown},
        },
        ensure_ascii=False,
    ).encode()
    request = Request(
        signed_webhook_url(webhook_url, secret, timestamp),
        data=payload,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with opener(request, timeout=timeout_seconds) as response:
            result = json.loads(response.read(65_536))
    except (OSError, ValueError, HTTPError, URLError) as error:
        safe_name = type(error).__name__
        raise DingTalkDeliveryError(f"DingTalk request failed: {safe_name}") from error
    if not isinstance(result, dict) or result.get("errcode") != 0:
        message = result.get("errmsg") if isinstance(result, dict) else "invalid response"
        raise DingTalkDeliveryError(redact_text(str(message or "unknown"))[:500])

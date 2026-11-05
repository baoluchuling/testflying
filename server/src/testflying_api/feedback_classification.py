from __future__ import annotations

import json
import re
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

from sqlalchemy.orm import Session

from testflying_api.config import Settings
from testflying_api.errors import ApiError
from testflying_api.llm_config import (
    LLM_FEATURE_FEEDBACK_CLASSIFICATION,
    LlmRuntimeConfig,
    resolve_llm_runtime_config,
)

FEEDBACK_CATEGORIES = {
    "bug": "缺陷",
    "feature_request": "功能建议",
    "usability": "体验问题",
    "performance": "性能问题",
    "content": "内容问题",
    "billing": "付费问题",
    "account": "账号问题",
    "compatibility": "兼容问题",
    "crash": "闪退问题",
    "other": "其他问题",
}
FEEDBACK_SEVERITIES = {"low", "medium", "high", "critical"}
FEEDBACK_PRIORITIES = {"p0", "p1", "p2", "p3"}
MAX_FEEDBACK_IMAGES = 5
SUPPORTED_IMAGE_DETAILS = {"auto", "low", "high"}
_DATA_IMAGE_URL_RE = re.compile(
    r"^data:(image/(?:png|jpeg|jpg|webp|gif));base64,(.+)$",
    re.IGNORECASE | re.DOTALL,
)

FEEDBACK_CLASSIFICATION_SCHEMA_INSTRUCTION = (
    "你是面向内部产品、QA 和研发团队的用户反馈分类助手。"
    "所有输出必须使用简体中文。只返回 JSON，不要返回 Markdown、解释文字或代码块。"
    "根据用户反馈判断问题类型、是否是 bug、是否是建议，以及重要性。"
    "JSON 结构必须是："
    '{"feedbackId":"原样返回输入 feedbackId，没有则为空字符串",'
    '"category":"bug|feature_request|usability|performance|content|billing|account|compatibility|crash|other",'
    '"categoryLabel":"中文分类名",'
    '"isBug":true,'
    '"isSuggestion":false,'
    '"severity":"low|medium|high|critical",'
    '"priority":"p0|p1|p2|p3",'
    '"confidence":0.86,'
    '"summary":"一句话中文摘要",'
    '"problem":"用户实际遇到的问题",'
    '"evidence":["从原文截取的关键证据片段"],'
    '"suggestedAction":"建议内部下一步处理方式",'
    '"routing":{"team":"product|qa|client|server|ops|content|support",'
    '"labels":["中文或英文标签"]},'
    '"needsHumanReview":false}. '
    "priority 规则：critical 对应 p0，high 对应 p1，medium 对应 p2，low 对应 p3。"
    "如果证据不足、分类不确定或可能涉及安全/支付/数据丢失，needsHumanReview 返回 true。"
    "如果输入里包含图片，请结合图片内容和文字反馈判断；看不清或图片无法证明的内容不要编造。"
    "不要回复用户，不要编造不存在的事实，只能根据输入内容和上下文判断。"
)


def classify_feedback(
    session: Session,
    settings: Settings,
    *,
    feedback: dict[str, object],
) -> dict[str, object]:
    normalized_content = str(feedback.get("content") or "").strip()
    if len(normalized_content) > 8000:
        raise ApiError("invalid_feedback_content", "反馈内容不能超过 8000 个字符", status_code=422)
    feedback = dict(feedback)
    feedback["content"] = normalized_content
    images = _normalize_feedback_images(feedback.get("images"))
    if not normalized_content and not images:
        raise ApiError(
            "invalid_feedback_content",
            "反馈内容或图片至少需要提供一个",
            status_code=422,
        )
    if images:
        feedback["images"] = images
    else:
        feedback.pop("images", None)

    runtime = resolve_llm_runtime_config(
        session,
        settings,
        feature_key=LLM_FEATURE_FEEDBACK_CLASSIFICATION,
    )
    provider = runtime.provider.strip().lower()
    if provider in {"", "disabled", "none"}:
        raise ApiError(
            "feedback_classification_not_configured",
            "用户反馈分类 LLM 服务未配置",
            status_code=503,
        )
    if provider == "mock":
        result = _mock_feedback_classification(feedback)
    elif provider in {"openai", "configured"} and runtime.protocol == "openai_compatible":
        result = _classify_with_openai_compatible(runtime, feedback=feedback)
    elif provider == "configured" and runtime.protocol == "claude_compatible":
        result = _classify_with_claude_compatible(runtime, feedback=feedback)
    else:
        raise ApiError(
            "unsupported_feedback_classification_provider",
            "不支持的用户反馈分类服务配置",
            status_code=422,
        )
    result["model"] = {
        "provider": runtime.provider,
        "protocol": runtime.protocol,
        "model": runtime.model,
    }
    return result


def _classify_with_openai_compatible(
    runtime: LlmRuntimeConfig,
    *,
    feedback: dict[str, object],
) -> dict[str, object]:
    if not runtime.api_key:
        raise ApiError(
            "feedback_classification_not_configured",
            "用户反馈分类服务缺少 API Key",
            status_code=503,
        )
    payload = {
        "model": runtime.model,
        "temperature": 0.1,
        "response_format": {"type": "json_object"},
        "messages": [
            {
                "role": "system",
                "content": FEEDBACK_CLASSIFICATION_SCHEMA_INSTRUCTION,
            },
            {
                "role": "user",
                "content": _openai_user_content(feedback),
            },
        ],
    }
    request = Request(
        runtime.base_url.rstrip("/") + "/chat/completions",
        data=json.dumps(payload).encode("utf-8"),
        headers=_llm_headers(runtime),
        method="POST",
    )
    response_payload = _send_llm_request(request)
    return _decode_feedback_classification(_response_content(response_payload), feedback=feedback)


def _classify_with_claude_compatible(
    runtime: LlmRuntimeConfig,
    *,
    feedback: dict[str, object],
) -> dict[str, object]:
    if not runtime.api_key:
        raise ApiError(
            "feedback_classification_not_configured",
            "用户反馈分类服务缺少 API Key",
            status_code=503,
        )
    payload = {
        "model": runtime.model,
        "max_tokens": 1600,
        "temperature": 0.1,
        "system": FEEDBACK_CLASSIFICATION_SCHEMA_INSTRUCTION,
        "messages": [
            {
                "role": "user",
                "content": _claude_user_content(feedback),
            }
        ],
    }
    request = Request(
        _claude_messages_url(runtime.base_url),
        data=json.dumps(payload).encode("utf-8"),
        headers={
            **_llm_headers(runtime),
            "anthropic-version": "2023-06-01",
        },
        method="POST",
    )
    response_payload = _send_llm_request(request)
    return _decode_feedback_classification(_response_content(response_payload), feedback=feedback)


def _openai_user_content(feedback: dict[str, object]) -> str | list[dict[str, object]]:
    images = _feedback_images(feedback)
    text = _feedback_text_payload(feedback)
    if not images:
        return text
    content: list[dict[str, object]] = [{"type": "text", "text": text}]
    for image in images:
        content.append(
            {
                "type": "image_url",
                "image_url": {
                    "url": image["url"],
                    "detail": image.get("detail") or "auto",
                },
            }
        )
    return content


def _claude_user_content(feedback: dict[str, object]) -> str | list[dict[str, object]]:
    images = _feedback_images(feedback)
    text = _feedback_text_payload(feedback)
    if not images:
        return text
    content: list[dict[str, object]] = [{"type": "text", "text": text}]
    for image in images:
        content.append(
            {
                "type": "image",
                "source": _claude_image_source(image["url"]),
            }
        )
    return content


def _feedback_text_payload(feedback: dict[str, object]) -> str:
    sanitized = dict(feedback)
    images = _feedback_images(feedback)
    if images:
        sanitized["images"] = [_image_text_metadata(image) for image in images]
    return json.dumps(sanitized, ensure_ascii=False)


def _image_text_metadata(image: dict[str, str]) -> dict[str, str]:
    url = image["url"]
    metadata = {
        "name": image.get("name", ""),
        "mimeType": image.get("mimeType", ""),
        "detail": image.get("detail", "auto"),
        "kind": "data_url" if _is_data_image_url(url) else "url",
    }
    if _is_data_image_url(url):
        data_match = _DATA_IMAGE_URL_RE.match(url)
        metadata["url"] = f"data-url:{data_match.group(1).lower()}" if data_match else "data-url"
    else:
        metadata["url"] = url
    return metadata


def _normalize_feedback_images(value: object) -> list[dict[str, str]]:
    if value in (None, ""):
        return []
    if not isinstance(value, list):
        raise ApiError("invalid_feedback_images", "images 必须是数组", status_code=422)
    if len(value) > MAX_FEEDBACK_IMAGES:
        raise ApiError(
            "invalid_feedback_images",
            f"最多支持 {MAX_FEEDBACK_IMAGES} 张图片",
            status_code=422,
        )
    images: list[dict[str, str]] = []
    for index, item in enumerate(value, start=1):
        if not isinstance(item, dict):
            raise ApiError(
                "invalid_feedback_images",
                f"第 {index} 张图片格式不正确",
                status_code=422,
            )
        url = _text(item.get("url"))
        if not _is_supported_image_url(url):
            raise ApiError(
                "invalid_feedback_image_url",
                "图片地址只支持 http(s) URL 或 data:image/...;base64,...",
                status_code=422,
            )
        detail = (_text(item.get("detail")) or "auto").lower()
        if detail not in SUPPORTED_IMAGE_DETAILS:
            raise ApiError(
                "invalid_feedback_images",
                "图片 detail 只支持 auto、low、high",
                status_code=422,
            )
        images.append(
            {
                "url": url,
                "name": _text(item.get("name")),
                "mimeType": _text(item.get("mimeType")),
                "detail": detail,
            }
        )
    return images


def _feedback_images(feedback: dict[str, object]) -> list[dict[str, str]]:
    images = feedback.get("images")
    if not isinstance(images, list):
        return []
    return [image for image in images if isinstance(image, dict)]


def _is_supported_image_url(url: str) -> bool:
    normalized = url.lower()
    return (
        normalized.startswith("http://")
        or normalized.startswith("https://")
        or _is_data_image_url(url)
    )


def _is_data_image_url(url: str) -> bool:
    return bool(_DATA_IMAGE_URL_RE.match(url))


def _claude_image_source(url: str) -> dict[str, str]:
    data_match = _DATA_IMAGE_URL_RE.match(url)
    if data_match is None:
        raise ApiError(
            "feedback_image_unsupported",
            "Claude 兼容模式第一版只支持 data URL 图片；请使用 OpenAI 兼容模型传图片 URL，"
            "或传 data:image/...;base64,...",
            status_code=422,
        )
    media_type = data_match.group(1).lower()
    if media_type == "image/jpg":
        media_type = "image/jpeg"
    return {
        "type": "base64",
        "media_type": media_type,
        "data": re.sub(r"\s+", "", data_match.group(2)),
    }


def _send_llm_request(request: Request) -> dict[str, object]:
    try:
        with urlopen(request, timeout=45) as response:
            decoded = json.loads(response.read().decode("utf-8"))
    except HTTPError as error:
        raise ApiError(
            "feedback_classification_failed",
            f"用户反馈分类服务调用失败（HTTP {error.code}）",
            status_code=502,
        ) from error
    except (TimeoutError, URLError, json.JSONDecodeError) as error:
        raise ApiError(
            "feedback_classification_failed",
            "用户反馈分类服务调用失败，请稍后重试",
            status_code=502,
        ) from error
    if not isinstance(decoded, dict):
        raise ApiError(
            "feedback_classification_failed",
            "用户反馈分类服务返回格式不正确",
            status_code=502,
        )
    return decoded


def _decode_feedback_classification(
    content: str,
    *,
    feedback: dict[str, object],
) -> dict[str, object]:
    for candidate in _json_candidates(content):
        try:
            decoded = json.loads(candidate)
        except json.JSONDecodeError:
            continue
        if isinstance(decoded, dict):
            return _normalize_feedback_classification(decoded, feedback=feedback)
    raise ApiError(
        "llm_invalid_response",
        "用户反馈分类服务返回格式不正确",
        status_code=502,
    )


def _normalize_feedback_classification(
    value: dict[str, object],
    *,
    feedback: dict[str, object],
) -> dict[str, object]:
    category = _normalize_choice(value.get("category"), FEEDBACK_CATEGORIES, default="other")
    severity = _normalize_choice(value.get("severity"), FEEDBACK_SEVERITIES, default="medium")
    priority = _normalize_choice(
        value.get("priority"),
        FEEDBACK_PRIORITIES,
        default=_priority_for_severity(severity),
    )
    confidence = _clamp_float(value.get("confidence"), default=0.0)
    summary = _text(value.get("summary")) or _fallback_summary(feedback)
    problem = _text(value.get("problem")) or summary
    routing = value.get("routing") if isinstance(value.get("routing"), dict) else {}
    return {
        "feedbackId": _text(value.get("feedbackId")) or _text(feedback.get("feedbackId")),
        "category": category,
        "categoryLabel": _text(value.get("categoryLabel")) or FEEDBACK_CATEGORIES[category],
        "isBug": _bool(value.get("isBug"), default=category in {"bug", "crash"}),
        "isSuggestion": _bool(
            value.get("isSuggestion"),
            default=category == "feature_request",
        ),
        "severity": severity,
        "priority": priority,
        "confidence": confidence,
        "summary": summary,
        "problem": problem,
        "evidence": _string_list(value.get("evidence"))[:8],
        "suggestedAction": _text(value.get("suggestedAction")),
        "routing": {
            "team": _text(routing.get("team")) if isinstance(routing, dict) else "",
            "labels": _string_list(routing.get("labels")) if isinstance(routing, dict) else [],
        },
        "needsHumanReview": _bool(
            value.get("needsHumanReview"),
            default=confidence < 0.6 or severity == "critical",
        ),
    }


def _mock_feedback_classification(feedback: dict[str, object]) -> dict[str, object]:
    content = str(feedback.get("content") or "").strip()
    lowered = content.lower()
    category = "other"
    severity = "low"
    if any(keyword in lowered for keyword in ("crash", "闪退", "崩溃")):
        category = "crash"
        severity = "high"
    elif any(keyword in lowered for keyword in ("卡", "慢", "lag", "timeout")):
        category = "performance"
        severity = "medium"
    elif any(keyword in lowered for keyword in ("建议", "希望", "能不能", "feature")):
        category = "feature_request"
        severity = "low"
    elif any(keyword in lowered for keyword in ("支付", "扣费", "会员", "订阅")):
        category = "billing"
        severity = "high"
    return {
        "feedbackId": _text(feedback.get("feedbackId")),
        "category": category,
        "categoryLabel": FEEDBACK_CATEGORIES[category],
        "isBug": category in {"bug", "crash", "performance", "billing"},
        "isSuggestion": category == "feature_request",
        "severity": severity,
        "priority": _priority_for_severity(severity),
        "confidence": 0.7,
        "summary": _fallback_summary(feedback),
        "problem": content[:160],
        "evidence": [content[:120]] if content else [],
        "suggestedAction": "请人工确认反馈是否可复现，并结合日志或商店后台数据排查。",
        "routing": {"team": "qa", "labels": [category]},
        "needsHumanReview": False,
    }


def _json_candidates(content: str) -> list[str]:
    stripped = content.strip()
    candidates = [stripped] if stripped else []
    candidates.extend(
        match.group(1).strip()
        for match in re.finditer(r"```(?:json)?\s*(.*?)```", content, re.DOTALL | re.I)
    )
    start = stripped.find("{")
    end = stripped.rfind("}")
    if start != -1 and end > start:
        candidates.append(stripped[start : end + 1])
    unique: list[str] = []
    for candidate in candidates:
        if candidate and candidate not in unique:
            unique.append(candidate)
    return unique


def _response_content(response_payload: dict[str, object]) -> str:
    content = response_payload.get("content")
    if isinstance(content, list):
        text_parts = [
            str(item.get("text") or "")
            for item in content
            if isinstance(item, dict) and item.get("type") == "text"
        ]
        if text_parts:
            return "\n".join(text_parts).strip()
    choices = response_payload.get("choices")
    if not isinstance(choices, list) or not choices:
        return ""
    first_choice = choices[0]
    if not isinstance(first_choice, dict):
        return ""
    message = first_choice.get("message")
    if not isinstance(message, dict):
        return ""
    return str(message.get("content") or "").strip()


def _llm_headers(runtime: LlmRuntimeConfig) -> dict[str, str]:
    headers = {
        "Content-Type": "application/json",
        "Accept": "application/json",
    }
    if not runtime.api_key:
        return headers
    if runtime.auth_header == "api-key":
        headers["api-key"] = runtime.api_key
    elif runtime.auth_header == "x-api-key":
        headers["x-api-key"] = runtime.api_key
    else:
        headers["Authorization"] = f"Bearer {runtime.api_key}"
    return headers


def _claude_messages_url(base_url: str) -> str:
    normalized = base_url.rstrip("/")
    if normalized.endswith("/v1"):
        return normalized + "/messages"
    return normalized + "/v1/messages"


def _normalize_choice(value: object, allowed: set[str] | dict[str, str], *, default: str) -> str:
    normalized = str(value or "").strip().lower()
    allowed_values = set(allowed.keys()) if isinstance(allowed, dict) else allowed
    return normalized if normalized in allowed_values else default


def _priority_for_severity(severity: str) -> str:
    return {
        "critical": "p0",
        "high": "p1",
        "medium": "p2",
        "low": "p3",
    }.get(severity, "p2")


def _fallback_summary(feedback: dict[str, object]) -> str:
    content = str(feedback.get("content") or "").strip()
    if len(content) <= 80:
        return content or "用户反馈需要人工确认。"
    return content[:77] + "..."


def _text(value: object) -> str:
    return str(value or "").strip()


def _string_list(value: object) -> list[str]:
    if isinstance(value, list):
        return [_text(item) for item in value if _text(item)]
    if isinstance(value, str) and value.strip():
        return [value.strip()]
    return []


def _bool(value: object, *, default: bool) -> bool:
    if isinstance(value, bool):
        return value
    if isinstance(value, str):
        normalized = value.strip().lower()
        if normalized in {"true", "1", "yes", "y"}:
            return True
        if normalized in {"false", "0", "no", "n"}:
            return False
    return default


def _clamp_float(value: object, *, default: float) -> float:
    try:
        result = float(value)
    except (TypeError, ValueError):
        return default
    return min(1.0, max(0.0, result))

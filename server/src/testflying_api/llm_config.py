from __future__ import annotations

from dataclasses import dataclass
from datetime import UTC, datetime
from uuid import uuid4

from sqlalchemy import select
from sqlalchemy.orm import Session

from testflying_api.config import Settings
from testflying_api.errors import ApiError
from testflying_api.schema import LlmFeatureBinding, LlmProfile

LLM_FEATURE_REVIEW_ANALYSIS = "review_analysis"
LLM_FEATURE_TRANSLATION = "translation"
LLM_FEATURE_FEEDBACK_CLASSIFICATION = "feedback_classification"

LLM_FEATURES = (
    {
        "key": LLM_FEATURE_REVIEW_ANALYSIS,
        "label": "评论分析",
        "description": "商店评论增量拉取后的问题归纳。",
    },
    {
        "key": LLM_FEATURE_TRANSLATION,
        "label": "多语言翻译",
        "description": "商店文案从源语言生成其他语言草稿。",
    },
    {
        "key": LLM_FEATURE_FEEDBACK_CLASSIFICATION,
        "label": "用户反馈分类",
        "description": "根据用户反馈判断问题类型、是否缺陷、是否建议和处理优先级。",
    },
)

LLM_PROTOCOLS = (
    {
        "key": "openai_compatible",
        "label": "OpenAI 兼容",
        "defaultBaseUrl": "https://api.openai.com/v1",
        "defaultModel": "gpt-4o-mini",
        "defaultAuthHeader": "authorization_bearer",
    },
    {
        "key": "claude_compatible",
        "label": "Claude 兼容",
        "defaultBaseUrl": "https://api.anthropic.com",
        "defaultModel": "claude-3-5-haiku-latest",
        "defaultAuthHeader": "x-api-key",
    },
)

LLM_PRESETS = (
    {
        "key": "xiaomi_mimo_openai",
        "label": "小米 MiMo（OpenAI 兼容）",
        "protocol": "openai_compatible",
        "baseUrl": "https://token-plan-cn.xiaomimimo.com/v1",
        "model": "mimo-v2.5-pro",
        "authHeader": "api-key",
    },
    {
        "key": "xiaomi_mimo_claude",
        "label": "小米 MiMo（Claude 兼容）",
        "protocol": "claude_compatible",
        "baseUrl": "https://api.xiaomimimo.com/anthropic",
        "model": "mimo-v2.5-pro",
        "authHeader": "x-api-key",
    },
    {
        "key": "openai_compatible",
        "label": "OpenAI 兼容默认",
        "protocol": "openai_compatible",
        "baseUrl": "https://api.openai.com/v1",
        "model": "gpt-4o-mini",
        "authHeader": "authorization_bearer",
    },
    {
        "key": "claude_compatible",
        "label": "Claude 兼容默认",
        "protocol": "claude_compatible",
        "baseUrl": "https://api.anthropic.com",
        "model": "claude-3-5-haiku-latest",
        "authHeader": "x-api-key",
    },
)


@dataclass(frozen=True)
class LlmRuntimeConfig:
    provider: str
    protocol: str
    base_url: str
    model: str
    api_key: str | None
    auth_header: str


def resolve_llm_runtime_config(
    session: Session | None,
    settings: Settings,
    *,
    feature_key: str,
) -> LlmRuntimeConfig:
    if session is not None:
        profile = bound_profile_for_feature(session, feature_key)
        if profile is not None:
            return LlmRuntimeConfig(
                provider="configured",
                protocol=profile.protocol,
                base_url=profile.base_url,
                model=profile.model,
                api_key=profile.api_key or None,
                auth_header=_normalize_auth_header(profile.auth_header, profile.protocol),
            )
    return legacy_runtime_config(settings, feature_key=feature_key)


def legacy_runtime_config(settings: Settings, *, feature_key: str) -> LlmRuntimeConfig:
    if feature_key == LLM_FEATURE_TRANSLATION:
        provider = settings.translation_provider.strip().lower()
        return LlmRuntimeConfig(
            provider=provider,
            protocol="openai_compatible",
            base_url=settings.translation_openai_base_url,
            model=settings.translation_openai_model,
            api_key=settings.translation_openai_api_key,
            auth_header="authorization_bearer",
        )
    if feature_key == LLM_FEATURE_REVIEW_ANALYSIS:
        provider = settings.review_analysis_provider.strip().lower()
        return LlmRuntimeConfig(
            provider=provider,
            protocol="openai_compatible",
            base_url=settings.review_analysis_openai_base_url,
            model=settings.review_analysis_openai_model,
            api_key=settings.review_analysis_openai_api_key,
            auth_header="authorization_bearer",
        )
    if feature_key == LLM_FEATURE_FEEDBACK_CLASSIFICATION:
        provider = settings.review_analysis_provider.strip().lower()
        return LlmRuntimeConfig(
            provider=provider,
            protocol="openai_compatible",
            base_url=settings.review_analysis_openai_base_url,
            model=settings.review_analysis_openai_model,
            api_key=settings.review_analysis_openai_api_key,
            auth_header="authorization_bearer",
        )
    return LlmRuntimeConfig(
        provider="disabled",
        protocol="openai_compatible",
        base_url="",
        model="",
        api_key=None,
        auth_header="authorization_bearer",
    )


def bound_profile_for_feature(session: Session, feature_key: str) -> LlmProfile | None:
    binding = session.get(LlmFeatureBinding, feature_key)
    if binding is None or not binding.primary_profile_id:
        return None
    return session.get(LlmProfile, binding.primary_profile_id)


def save_llm_profile(
    session: Session,
    *,
    profile_id: str | None,
    name: str,
    protocol: str,
    base_url: str,
    model: str,
    api_key: str | None,
    auth_header: str,
) -> LlmProfile:
    normalized_protocol = _normalize_protocol(protocol)
    normalized_name = name.strip()
    normalized_base_url = base_url.strip().rstrip("/")
    normalized_model = model.strip()
    if not normalized_name:
        raise ApiError("invalid_llm_profile", "模型名称不能为空", status_code=422)
    if not normalized_base_url:
        raise ApiError("invalid_llm_profile", "Base URL 不能为空", status_code=422)
    if not normalized_model:
        raise ApiError("invalid_llm_profile", "模型 ID 不能为空", status_code=422)

    now = datetime.now(UTC)
    if profile_id:
        profile = session.get(LlmProfile, profile_id)
        if profile is None:
            raise ApiError("llm_profile_not_found", "LLM 模型不存在", status_code=404)
    else:
        profile = LlmProfile(
            id=f"llm-{uuid4().hex[:12]}",
            name=normalized_name,
            protocol=normalized_protocol,
            base_url=normalized_base_url,
            model=normalized_model,
            api_key="",
            auth_header="",
            status="unchecked",
            created_at=now,
            updated_at=now,
        )
        session.add(profile)

    profile.name = normalized_name
    profile.protocol = normalized_protocol
    profile.base_url = normalized_base_url
    profile.model = normalized_model
    if api_key is not None and api_key.strip():
        profile.api_key = api_key.strip()
    profile.auth_header = _normalize_auth_header(auth_header, normalized_protocol)
    profile.status = "configured" if profile.api_key else "missing_key"
    profile.updated_at = now
    session.flush()
    return profile


def save_feature_binding(
    session: Session,
    *,
    feature_key: str,
    primary_profile_id: str | None,
    fallback_profile_id: str | None = None,
) -> LlmFeatureBinding:
    if feature_key not in {feature["key"] for feature in LLM_FEATURES}:
        raise ApiError("invalid_llm_feature", "不支持的 LLM 功能", status_code=422)
    normalized_primary = _existing_profile_id(session, primary_profile_id)
    normalized_fallback = _existing_profile_id(session, fallback_profile_id)
    if normalized_primary and normalized_fallback and normalized_primary == normalized_fallback:
        normalized_fallback = None
    binding = session.get(LlmFeatureBinding, feature_key)
    if binding is None:
        binding = LlmFeatureBinding(
            feature_key=feature_key,
            primary_profile_id=normalized_primary,
            fallback_profile_id=normalized_fallback,
            updated_at=datetime.now(UTC),
        )
        session.add(binding)
    else:
        binding.primary_profile_id = normalized_primary
        binding.fallback_profile_id = normalized_fallback
        binding.updated_at = datetime.now(UTC)
    session.flush()
    return binding


def list_llm_profiles(session: Session) -> list[LlmProfile]:
    return list(session.scalars(select(LlmProfile).order_by(LlmProfile.created_at.asc())))


def list_llm_bindings(session: Session) -> dict[str, LlmFeatureBinding]:
    return {
        binding.feature_key: binding
        for binding in session.scalars(select(LlmFeatureBinding))
    }


def protocol_label(protocol: str) -> str:
    return next(
        (item["label"] for item in LLM_PROTOCOLS if item["key"] == protocol),
        protocol or "未知协议",
    )


def auth_header_label(auth_header: str) -> str:
    labels = {
        "authorization_bearer": "Authorization: Bearer",
        "api-key": "api-key",
        "x-api-key": "x-api-key",
    }
    return labels.get(auth_header, auth_header or "默认")


def mask_api_key(api_key: str | None) -> str:
    if not api_key:
        return ""
    if len(api_key) <= 8:
        return "****"
    return f"{api_key[:4]}...{api_key[-4:]}"


def _existing_profile_id(session: Session, profile_id: str | None) -> str | None:
    normalized = str(profile_id or "").strip()
    if not normalized:
        return None
    if session.get(LlmProfile, normalized) is None:
        raise ApiError("llm_profile_not_found", "绑定的 LLM 模型不存在", status_code=404)
    return normalized


def _normalize_protocol(protocol: str) -> str:
    normalized = protocol.strip()
    allowed = {item["key"] for item in LLM_PROTOCOLS}
    if normalized not in allowed:
        raise ApiError(
            "invalid_llm_protocol",
            "LLM 协议只支持 OpenAI 兼容或 Claude 兼容",
            status_code=422,
        )
    return normalized


def _normalize_auth_header(auth_header: str, protocol: str) -> str:
    normalized = auth_header.strip()
    if normalized in {"authorization_bearer", "api-key", "x-api-key"}:
        return normalized
    if protocol == "claude_compatible":
        return "x-api-key"
    return "authorization_bearer"

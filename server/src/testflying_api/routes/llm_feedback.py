from __future__ import annotations

import secrets
import time
from collections import deque
from threading import Lock
from typing import Annotated, Any

from fastapi import APIRouter, Depends, Request
from pydantic import BaseModel, ConfigDict, Field, field_validator, model_validator
from sqlalchemy.orm import Session

from testflying_api.database import get_db_session
from testflying_api.errors import ApiError
from testflying_api.feedback_classification import classify_feedback

router = APIRouter(prefix="/v1/llm", tags=["llm"])
SessionDep = Annotated[Session, Depends(get_db_session)]

FEEDBACK_CLASSIFICATION_RATE_LIMIT_PER_MINUTE = 30
FEEDBACK_CLASSIFICATION_RATE_LIMIT_PER_HOUR = 300

_rate_limit_lock = Lock()
_minute_events: dict[str, deque[float]] = {}
_hour_events: dict[str, deque[float]] = {}


class CamelModel(BaseModel):
    model_config = ConfigDict(
        alias_generator=lambda value: value.split("_")[0]
        + "".join(word[:1].upper() + word[1:] for word in value.split("_")[1:]),
        populate_by_name=True,
    )


class FeedbackAppContext(CamelModel):
    id: str = ""
    name: str = ""
    version: str = ""


class FeedbackImageInput(CamelModel):
    url: str = Field(min_length=1)
    name: str = ""
    mime_type: str = Field(default="", alias="mimeType")
    detail: str = "auto"

    @field_validator("url")
    @classmethod
    def _url_not_blank(cls, value: str) -> str:
        if not value.strip():
            raise ValueError("图片地址不能为空")
        return value.strip()

    @field_validator("detail")
    @classmethod
    def _detail_supported(cls, value: str) -> str:
        normalized = value.strip().lower() or "auto"
        if normalized not in {"auto", "low", "high"}:
            raise ValueError("图片 detail 只支持 auto、low、high")
        return normalized


class FeedbackClassificationRequest(CamelModel):
    feedback_id: str = Field(default="", alias="feedbackId")
    content: str = Field(default="", max_length=8000)
    title: str = ""
    source: str = "manual"
    platform: str = "unknown"
    app: FeedbackAppContext | None = None
    locale: str = "zh-CN"
    context: dict[str, Any] = Field(default_factory=dict)
    images: list[FeedbackImageInput] = Field(default_factory=list, max_length=5)

    @field_validator("content")
    @classmethod
    def _content_not_blank(cls, value: str) -> str:
        return value.strip()

    @model_validator(mode="after")
    def _content_or_images_required(self) -> FeedbackClassificationRequest:
        if not self.content and not self.images:
            raise ValueError("反馈内容或图片至少需要提供一个")
        return self


class FeedbackRouting(CamelModel):
    team: str = ""
    labels: list[str] = Field(default_factory=list)


class FeedbackModelInfo(CamelModel):
    provider: str
    protocol: str
    model: str


class FeedbackClassificationResponse(CamelModel):
    feedback_id: str = Field(default="", alias="feedbackId")
    category: str
    category_label: str = Field(alias="categoryLabel")
    is_bug: bool = Field(alias="isBug")
    is_suggestion: bool = Field(alias="isSuggestion")
    severity: str
    priority: str
    confidence: float
    summary: str
    problem: str
    evidence: list[str] = Field(default_factory=list)
    suggested_action: str = Field(alias="suggestedAction")
    routing: FeedbackRouting
    needs_human_review: bool = Field(alias="needsHumanReview")
    model: FeedbackModelInfo


@router.post(
    "/feedback-classifications",
    response_model=FeedbackClassificationResponse,
    response_model_by_alias=True,
)
def create_feedback_classification(
    payload: FeedbackClassificationRequest,
    request: Request,
    session: SessionDep,
) -> dict[str, object]:
    token = _require_static_token(request)
    _check_rate_limit(token)
    return classify_feedback(
        session,
        request.app.state.settings,
        feedback=payload.model_dump(by_alias=True, exclude_none=True),
    )


def _require_static_token(request: Request) -> str:
    expected = f"Bearer {request.app.state.settings.static_token}"
    authorization = request.headers.get("Authorization", "")
    if not secrets.compare_digest(authorization, expected):
        raise ApiError("invalid_static_token", "接口 token 不正确", status_code=401)
    return authorization


def _check_rate_limit(token: str) -> None:
    now = time.monotonic()
    with _rate_limit_lock:
        minute_events = _minute_events.setdefault(token, deque())
        hour_events = _hour_events.setdefault(token, deque())
        _drop_old_events(minute_events, now=now, window_seconds=60)
        _drop_old_events(hour_events, now=now, window_seconds=3600)

        waits: list[float] = []
        if len(minute_events) >= FEEDBACK_CLASSIFICATION_RATE_LIMIT_PER_MINUTE:
            waits.append(60 - (now - minute_events[0]))
        if len(hour_events) >= FEEDBACK_CLASSIFICATION_RATE_LIMIT_PER_HOUR:
            waits.append(3600 - (now - hour_events[0]))
        if waits:
            retry_after = max(1, int(max(waits)) + 1)
            raise ApiError(
                "rate_limited",
                "用户反馈分类请求过于频繁，请稍后重试",
                status_code=429,
                retryable=True,
                extra={"retryAfterSeconds": retry_after},
            )

        minute_events.append(now)
        hour_events.append(now)


def _drop_old_events(events: deque[float], *, now: float, window_seconds: float) -> None:
    while events and now - events[0] >= window_seconds:
        events.popleft()

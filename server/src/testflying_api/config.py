from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

DEFAULT_CORS_ALLOWED_ORIGINS = (
    "http://localhost:8080",
    "http://127.0.0.1:8080",
)


@dataclass(frozen=True)
class Settings:
    database_url: str
    public_base_url: str
    storage_backend: str
    storage_root: Path
    static_token: str
    s3_endpoint_url: str | None
    s3_public_base_url: str | None
    s3_bucket: str
    s3_access_key_id: str | None
    s3_secret_access_key: str | None
    cors_allowed_origins: tuple[str, ...]
    admin_username: str
    connector_base_url_template: str | None
    translation_provider: str
    translation_openai_api_key: str | None
    translation_openai_base_url: str
    translation_openai_model: str
    review_analysis_provider: str
    review_analysis_openai_api_key: str | None
    review_analysis_openai_base_url: str
    review_analysis_openai_model: str
    dingtalk_webhook_url: str | None
    dingtalk_secret: str | None
    dingtalk_timeout_seconds: float
    dingtalk_dispatch_interval_seconds: float
    runner_release_root: Path

    @property
    def dingtalk_configured(self) -> bool:
        return bool(self.dingtalk_webhook_url and self.dingtalk_secret)

    @classmethod
    def from_environment(cls) -> Settings:
        return cls(
            database_url=os.getenv(
                "TESTFLYING_DATABASE_URL",
                "sqlite:///./data/testflying.db",
            ),
            public_base_url=os.getenv(
                "TESTFLYING_PUBLIC_BASE_URL",
                "http://localhost:8000",
            ),
            storage_backend=os.getenv("TESTFLYING_STORAGE_BACKEND", "local"),
            storage_root=Path(
                os.getenv("TESTFLYING_STORAGE_ROOT", "./data/artifacts"),
            ),
            static_token=os.getenv("TESTFLYING_STATIC_TOKEN", "dev-token"),
            s3_endpoint_url=os.getenv("TESTFLYING_S3_ENDPOINT_URL"),
            s3_public_base_url=os.getenv("TESTFLYING_S3_PUBLIC_BASE_URL"),
            s3_bucket=os.getenv("TESTFLYING_S3_BUCKET", "testflying"),
            s3_access_key_id=os.getenv("TESTFLYING_S3_ACCESS_KEY_ID"),
            s3_secret_access_key=os.getenv("TESTFLYING_S3_SECRET_ACCESS_KEY"),
            cors_allowed_origins=_split_origins(
                os.getenv("TESTFLYING_CORS_ALLOWED_ORIGINS"),
            ),
            admin_username=os.getenv("TESTFLYING_ADMIN_USERNAME", "admin"),
            connector_base_url_template=_normalize_optional(
                os.getenv("TESTFLYING_CONNECTOR_BASE_URL_TEMPLATE"),
            ),
            translation_provider=os.getenv("TESTFLYING_TRANSLATION_PROVIDER", "disabled"),
            translation_openai_api_key=_normalize_optional(
                os.getenv("TESTFLYING_TRANSLATION_OPENAI_API_KEY"),
            ),
            translation_openai_base_url=os.getenv(
                "TESTFLYING_TRANSLATION_OPENAI_BASE_URL",
                "https://api.openai.com/v1",
            ),
            translation_openai_model=os.getenv(
                "TESTFLYING_TRANSLATION_OPENAI_MODEL",
                "gpt-4o-mini",
            ),
            review_analysis_provider=os.getenv(
                "TESTFLYING_REVIEW_ANALYSIS_PROVIDER",
                os.getenv("TESTFLYING_TRANSLATION_PROVIDER", "disabled"),
            ),
            review_analysis_openai_api_key=_normalize_optional(
                os.getenv("TESTFLYING_REVIEW_ANALYSIS_OPENAI_API_KEY")
                or os.getenv("TESTFLYING_TRANSLATION_OPENAI_API_KEY"),
            ),
            review_analysis_openai_base_url=os.getenv(
                "TESTFLYING_REVIEW_ANALYSIS_OPENAI_BASE_URL",
                os.getenv("TESTFLYING_TRANSLATION_OPENAI_BASE_URL", "https://api.openai.com/v1"),
            ),
            review_analysis_openai_model=os.getenv(
                "TESTFLYING_REVIEW_ANALYSIS_OPENAI_MODEL",
                os.getenv("TESTFLYING_TRANSLATION_OPENAI_MODEL", "gpt-4o-mini"),
            ),
            dingtalk_webhook_url=_normalize_optional(
                os.getenv("TESTFLYING_DINGTALK_WEBHOOK_URL"),
            ),
            dingtalk_secret=_normalize_optional(
                os.getenv("TESTFLYING_DINGTALK_SECRET"),
            ),
            dingtalk_timeout_seconds=_positive_float(
                os.getenv("TESTFLYING_DINGTALK_TIMEOUT_SECONDS"),
                default=5.0,
                field="TESTFLYING_DINGTALK_TIMEOUT_SECONDS",
            ),
            dingtalk_dispatch_interval_seconds=_positive_float(
                os.getenv("TESTFLYING_DINGTALK_DISPATCH_INTERVAL_SECONDS"),
                default=10.0,
                field="TESTFLYING_DINGTALK_DISPATCH_INTERVAL_SECONDS",
            ),
            runner_release_root=Path(
                os.getenv("TESTFLYING_RUNNER_RELEASE_ROOT", "./data/runner-releases"),
            ),
        )


def _split_origins(raw_value: str | None) -> tuple[str, ...]:
    if raw_value is None or not raw_value.strip():
        return DEFAULT_CORS_ALLOWED_ORIGINS
    return tuple(origin.strip() for origin in raw_value.split(",") if origin.strip())


def _normalize_optional(raw_value: str | None) -> str | None:
    if raw_value is None:
        return None
    value = raw_value.strip()
    return value or None


def _positive_float(raw_value: str | None, *, default: float, field: str) -> float:
    if raw_value is None or not raw_value.strip():
        return default
    try:
        value = float(raw_value)
    except ValueError as error:
        raise ValueError(f"{field} must be a positive number") from error
    if value <= 0:
        raise ValueError(f"{field} must be a positive number")
    return value

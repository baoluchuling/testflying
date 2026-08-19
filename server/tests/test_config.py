from __future__ import annotations

from pathlib import Path

import pytest

from testflying_api.config import Settings


def test_settings_reads_connector_base_url_template(monkeypatch) -> None:
    monkeypatch.setenv(
        "TESTFLYING_CONNECTOR_BASE_URL_TEMPLATE",
        " http://connector-{account_id}:8100 ",
    )

    settings = Settings.from_environment()

    assert settings.connector_base_url_template == "http://connector-{account_id}:8100"


def test_settings_normalizes_blank_connector_base_url_template(monkeypatch) -> None:
    monkeypatch.setenv("TESTFLYING_CONNECTOR_BASE_URL_TEMPLATE", " ")

    settings = Settings.from_environment()

    assert settings.connector_base_url_template is None


def test_settings_reads_translation_configuration(monkeypatch) -> None:
    monkeypatch.setenv("TESTFLYING_TRANSLATION_PROVIDER", "openai")
    monkeypatch.setenv("TESTFLYING_TRANSLATION_OPENAI_API_KEY", "test-key")
    monkeypatch.setenv("TESTFLYING_TRANSLATION_OPENAI_BASE_URL", "https://llm.example.test/v1")
    monkeypatch.setenv("TESTFLYING_TRANSLATION_OPENAI_MODEL", "translation-model")

    settings = Settings.from_environment()

    assert settings.translation_provider == "openai"
    assert settings.translation_openai_api_key == "test-key"
    assert settings.translation_openai_base_url == "https://llm.example.test/v1"
    assert settings.translation_openai_model == "translation-model"


def test_settings_reads_dingtalk_and_runner_release_configuration(
    monkeypatch,
    tmp_path: Path,
) -> None:
    monkeypatch.setenv(
        "TESTFLYING_DINGTALK_WEBHOOK_URL",
        " https://oapi.dingtalk.test/robot/send ",
    )
    monkeypatch.setenv("TESTFLYING_DINGTALK_SECRET", " SEC-test ")
    monkeypatch.setenv("TESTFLYING_DINGTALK_TIMEOUT_SECONDS", "7")
    monkeypatch.setenv("TESTFLYING_DINGTALK_DISPATCH_INTERVAL_SECONDS", "12")
    monkeypatch.setenv("TESTFLYING_RUNNER_RELEASE_ROOT", str(tmp_path / "releases"))

    settings = Settings.from_environment()

    assert settings.dingtalk_webhook_url == "https://oapi.dingtalk.test/robot/send"
    assert settings.dingtalk_secret == "SEC-test"
    assert settings.dingtalk_timeout_seconds == 7.0
    assert settings.dingtalk_dispatch_interval_seconds == 12.0
    assert settings.dingtalk_configured is True
    assert settings.runner_release_root == tmp_path / "releases"


@pytest.mark.parametrize("raw_value", ["0", "-1", "not-a-number"])
def test_settings_rejects_invalid_dingtalk_timeout(monkeypatch, raw_value: str) -> None:
    monkeypatch.setenv("TESTFLYING_DINGTALK_TIMEOUT_SECONDS", raw_value)

    with pytest.raises(ValueError, match="must be a positive number"):
        Settings.from_environment()

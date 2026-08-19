from __future__ import annotations

from package_agent.redaction import redact_text


def test_redacts_private_key_and_password() -> None:
    text = "PASSWORD=secret\n-----BEGIN PRIVATE KEY-----\nabc\n-----END PRIVATE KEY-----"

    redacted = redact_text(text)

    assert "secret" not in redacted
    assert "PRIVATE KEY" not in redacted
    assert "[REDACTED]" in redacted


def test_redacts_bearer_style_token_assignment() -> None:
    text = "api_key: sk-live-12345\nTOKEN = abc123"

    redacted = redact_text(text)

    assert "sk-live-12345" not in redacted
    assert "abc123" not in redacted

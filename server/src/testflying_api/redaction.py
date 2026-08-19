from __future__ import annotations

import re
from collections.abc import Mapping, Sequence
from typing import Any

SECRET_ASSIGNMENT_RE = re.compile(
    r"(?im)\b(password|token|secret|api[_-]?key|private[_-]?key)\b\s*[:=]\s*([^\r\n]+)"
)
PRIVATE_KEY_RE = re.compile(
    r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----.*?-----END [A-Z0-9 ]*PRIVATE KEY-----",
    re.DOTALL | re.IGNORECASE,
)
SECRET_QUERY_PARAMETER_RE = re.compile(
    r"(?i)([?&](?:access[_-]?token|token|secret|sign|signature|api[_-]?key)=)([^&#\s]+)"
)


def redact_text(value: str) -> str:
    redacted = PRIVATE_KEY_RE.sub("[REDACTED]", value)
    redacted = SECRET_QUERY_PARAMETER_RE.sub(r"\1[REDACTED]", redacted)
    return SECRET_ASSIGNMENT_RE.sub(_replace_secret_assignment, redacted)


def redact_json(value: Any) -> Any:
    if isinstance(value, str):
        return redact_text(value)
    if isinstance(value, Mapping):
        return {str(key): redact_json(item) for key, item in value.items()}
    if isinstance(value, Sequence) and not isinstance(value, bytes | bytearray):
        return [redact_json(item) for item in value]
    return value


def _replace_secret_assignment(match: re.Match[str]) -> str:
    return f"{match.group(1)}=[REDACTED]"

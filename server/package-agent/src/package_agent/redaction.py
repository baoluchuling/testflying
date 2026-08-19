from __future__ import annotations

import re

PRIVATE_KEY_RE = re.compile(
    r"-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----",
    re.DOTALL,
)
SECRET_ASSIGNMENT_RE = re.compile(
    r"(?im)\b(password|token|secret|api[_-]?key|private[_-]?key)\b\s*[:=]\s*([^\r\n]+)"
)


def redact_text(value: str) -> str:
    value = PRIVATE_KEY_RE.sub("[REDACTED]", value)
    return SECRET_ASSIGNMENT_RE.sub(_replace_secret_assignment, value)


def _replace_secret_assignment(match: re.Match[str]) -> str:
    return f"{match.group(1)}=[REDACTED]"

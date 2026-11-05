from __future__ import annotations

from fastapi import Request
from fastapi.responses import JSONResponse


class ApiError(Exception):
    def __init__(
        self,
        code: str,
        message: str,
        *,
        status_code: int = 400,
        retryable: bool = False,
        extra: dict[str, object] | None = None,
    ) -> None:
        self.code = code
        self.message = message
        self.status_code = status_code
        self.retryable = retryable
        self.extra = extra or {}


async def api_error_handler(_request: Request, error: ApiError) -> JSONResponse:
    content = {
        "code": error.code,
        "message": error.message,
        "retryable": error.retryable,
    }
    content.update(error.extra)
    return JSONResponse(
        status_code=error.status_code,
        content=content,
    )

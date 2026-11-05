from __future__ import annotations

from datetime import UTC, datetime
from typing import Annotated

from fastapi import APIRouter, Depends, Request
from pydantic import BaseModel, Field
from sqlalchemy.orm import Session

from testflying_api.active_connector import active_connector_hub
from testflying_api.database import get_db_session
from testflying_api.errors import ApiError
from testflying_api.store_sync import account_connector

router = APIRouter(prefix="/connector-agent", tags=["connector-agent"])
SessionDep = Annotated[Session, Depends(get_db_session)]


class AgentPollRequest(BaseModel):
    account_id: str = Field(alias="accountId")
    timeout_seconds: float = Field(default=25, alias="timeoutSeconds")


class AgentResultRequest(BaseModel):
    account_id: str = Field(alias="accountId")
    task_id: str = Field(alias="taskId")
    status_code: int = Field(alias="statusCode")
    body: str = ""


@router.post("/v1/poll")
def poll_connector_task(
    payload: AgentPollRequest,
    request: Request,
    session: SessionDep,
) -> dict[str, object]:
    connector = _active_connector_or_401(session, payload.account_id, request)
    now = datetime.now(UTC)
    connector.status = "ok"
    connector.last_checked_at = now
    session.commit()

    task = active_connector_hub.poll(
        account_id=payload.account_id,
        timeout_seconds=min(max(payload.timeout_seconds, 0), 30),
    )
    if task is None:
        return {"task": None}
    return {
        "task": {
            "id": task.id,
            "method": task.method,
            "path": task.path,
            "headers": task.headers,
            "body": task.body,
        }
    }


@router.post("/v1/results")
def complete_connector_task(
    payload: AgentResultRequest,
    request: Request,
    session: SessionDep,
) -> dict[str, object]:
    connector = _active_connector_or_401(session, payload.account_id, request)
    connector.status = "ok"
    connector.last_checked_at = datetime.now(UTC)
    session.commit()
    completed = active_connector_hub.complete(
        task_id=payload.task_id,
        status_code=payload.status_code,
        body=payload.body,
    )
    return {"ok": completed}


def _active_connector_or_401(session: Session, account_id: str, request: Request):
    connector = account_connector(session, account_id)
    if connector is None or not connector.base_url.startswith("active://"):
        raise ApiError("connector_not_active", "当前账号没有启用主动 Connector", status_code=404)
    authorization = request.headers.get("Authorization", "")
    if authorization != f"Bearer {connector.auth_token}":
        raise ApiError("invalid_connector_token", "Connector token 不正确", status_code=401)
    return connector

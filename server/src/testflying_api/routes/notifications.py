from __future__ import annotations

from typing import Annotated

from fastapi import APIRouter, Depends, Query
from sqlalchemy.orm import Session

from testflying_api.catalog_repository import CatalogRepository
from testflying_api.catalog_service import CatalogService
from testflying_api.database import get_db_session
from testflying_api.errors import ApiError
from testflying_api.models import NotificationResponse

router = APIRouter(prefix="/v1/test-distribution/notifications", tags=["notifications"])
SessionDep = Annotated[Session, Depends(get_db_session)]


@router.get("", response_model=list[NotificationResponse])
def list_notifications(
    session: SessionDep,
    notification_type: Annotated[str | None, Query(alias="type")] = None,
) -> list[NotificationResponse]:
    if notification_type in (None, "", "all"):
        types = None
    elif notification_type in {"build", "account", "device"}:
        types = {notification_type}
    else:
        raise ApiError("invalid_notification_type", "不支持的通知类型", status_code=422)
    return CatalogService(CatalogRepository(session)).notifications(types=types)

from __future__ import annotations

from typing import Annotated
from uuid import uuid4

from fastapi import APIRouter, Depends, Header, Request
from sqlalchemy.orm import Session

from testflying_api.catalog_repository import CatalogRepository
from testflying_api.catalog_service import CatalogService
from testflying_api.database import get_db_session
from testflying_api.errors import ApiError
from testflying_api.models import TestDevice

router = APIRouter(prefix="/v1/test-distribution/devices", tags=["devices"])
SessionDep = Annotated[Session, Depends(get_db_session)]


@router.get("/current", response_model=TestDevice)
def get_current_device(
    session: SessionDep,
    device_id: Annotated[str, Header(alias="X-Device-ID")],
) -> TestDevice:
    repository = CatalogRepository(session)
    device = repository.current_device(device_id)
    if device is None:
        raise ApiError("device_not_registered", "当前设备未登记", status_code=404)
    return CatalogService(repository).device_response(device, current_device_id=device_id)


@router.get("", response_model=list[TestDevice])
def list_devices(
    session: SessionDep,
    device_id: Annotated[str, Header(alias="X-Device-ID")] = "",
) -> list[TestDevice]:
    return CatalogService(CatalogRepository(session)).devices(current_device_id=device_id)


@router.post("/registration-link")
def create_registration_link(
    request: Request,
    device_id: Annotated[str, Header(alias="X-Device-ID")],
) -> dict[str, str]:
    settings = request.app.state.settings
    request_id = uuid4().hex
    return {
        "requestId": request_id,
        "deviceId": device_id,
        "status": "pending_approval",
        "registrationUrl": f"{settings.public_base_url}/devices/register/{request_id}",
    }

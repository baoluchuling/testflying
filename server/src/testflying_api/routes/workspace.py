from __future__ import annotations

from typing import Annotated

from fastapi import APIRouter, Depends, Header
from sqlalchemy.orm import Session

from testflying_api.catalog_repository import CatalogRepository
from testflying_api.catalog_service import CatalogService
from testflying_api.database import get_db_session
from testflying_api.models import WorkspaceResponse

router = APIRouter()
SessionDep = Annotated[Session, Depends(get_db_session)]


@router.get(
    "/v1/test-distribution/workspace",
    response_model=WorkspaceResponse,
)
def get_workspace(
    session: SessionDep,
    authorization: Annotated[str | None, Header()] = None,
    device_id: Annotated[str, Header(alias="X-Device-ID")] = "local",
    client_platform: Annotated[str, Header(alias="X-Client-Platform")] = "ios",
) -> WorkspaceResponse:
    service = CatalogService(CatalogRepository(session))
    workspace = service.workspace_for_device(
        device_id=device_id,
        platform=client_platform,
        has_token=bool(authorization),
    )
    if workspace.devices:
        return workspace
    return WorkspaceResponse.empty(
        device_id=device_id,
        platform=client_platform,
        has_token=bool(authorization),
    )

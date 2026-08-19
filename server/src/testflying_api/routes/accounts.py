from __future__ import annotations

from typing import Annotated

from fastapi import APIRouter, Depends, Header
from sqlalchemy.orm import Session

from testflying_api.catalog_repository import CatalogRepository
from testflying_api.catalog_service import CatalogService
from testflying_api.database import get_db_session
from testflying_api.models import DeveloperAccountResponse

router = APIRouter(prefix="/v1/test-distribution/developer-accounts", tags=["accounts"])
SessionDep = Annotated[Session, Depends(get_db_session)]


@router.get("", response_model=list[DeveloperAccountResponse])
def list_developer_accounts(
    session: SessionDep,
    device_id: Annotated[str, Header(alias="X-Device-ID")] = "local",
    client_platform: Annotated[str, Header(alias="X-Client-Platform")] = "ios",
) -> list[DeveloperAccountResponse]:
    return CatalogService(CatalogRepository(session)).developer_accounts(
        device_id=device_id,
        platform=client_platform,
    )


@router.get("/renewals", response_model=list[DeveloperAccountResponse])
def list_renewals(
    session: SessionDep,
    device_id: Annotated[str, Header(alias="X-Device-ID")] = "local",
    client_platform: Annotated[str, Header(alias="X-Client-Platform")] = "ios",
) -> list[DeveloperAccountResponse]:
    accounts = CatalogService(CatalogRepository(session)).developer_accounts(
        device_id=device_id,
        platform=client_platform,
    )
    return [account for account in accounts if account.remaining_days <= 30]

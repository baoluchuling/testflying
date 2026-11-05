from __future__ import annotations

from typing import Annotated

from fastapi import APIRouter, Depends, File, Form, Request, UploadFile
from sqlalchemy.orm import Session

from testflying_api.database import get_db_session
from testflying_api.models import UploadResponse
from testflying_api.upload_service import create_package_upload

router = APIRouter(prefix="/v1/test-distribution", tags=["uploads"])
SessionDep = Annotated[Session, Depends(get_db_session)]


@router.post("/uploads", response_model=UploadResponse)
async def upload_package(
    request: Request,
    session: SessionDep,
    file: Annotated[UploadFile, File()],
    platform: Annotated[str, Form()],
    environment: Annotated[str, Form()],
    changelog: Annotated[str, Form()] = "",
    app_name: Annotated[str | None, Form(alias="appName")] = None,
    developer_account_id: Annotated[str | None, Form(alias="developerAccountId")] = None,
    store_app_id: Annotated[str | None, Form(alias="storeAppId")] = None,
    store_package_name: Annotated[str | None, Form(alias="storePackageName")] = None,
) -> UploadResponse:
    return create_package_upload(
        session=session,
        storage=request.app.state.artifact_storage,
        content=await file.read(),
        file_name=file.filename or "",
        content_type=file.content_type or "",
        platform=platform,
        environment=environment,
        changelog=changelog,
        app_name=app_name,
        developer_account_id=developer_account_id,
        store_app_id=store_app_id,
        store_package_name=store_package_name,
    )

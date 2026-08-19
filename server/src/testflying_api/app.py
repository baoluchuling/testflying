from __future__ import annotations

import asyncio
from contextlib import asynccontextmanager
from pathlib import Path

from fastapi import FastAPI, Request
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import RedirectResponse
from fastapi.staticfiles import StaticFiles
from sqlalchemy.orm import Session, sessionmaker

from testflying_api import webhook_delivery
from testflying_api.admin_legacy_redirects import admin_next_redirect_path
from testflying_api.app_logs import AppLogHub
from testflying_api.config import Settings
from testflying_api.database import create_engine_for_url, create_session_factory
from testflying_api.errors import ApiError, api_error_handler
from testflying_api.schema import Base
from testflying_api.storage import ArtifactStorage, storage_from_settings


def create_app(
    settings: Settings | None = None,
    *,
    session_factory: sessionmaker[Session] | None = None,
    artifact_storage: ArtifactStorage | None = None,
) -> FastAPI:
    app_settings = settings or Settings.from_environment()

    @asynccontextmanager
    async def lifespan(application: FastAPI):
        stop_event = asyncio.Event()
        application.state.delivery_stop_event = stop_event
        delivery_task = asyncio.create_task(
            webhook_delivery.run_delivery_loop(
                application.state.session_factory,
                app_settings,
                stop_event,
            )
        )
        await asyncio.sleep(0)
        application.state.delivery_task = delivery_task
        try:
            yield
        finally:
            stop_event.set()
            await delivery_task

    app = FastAPI(
        title="testflying API",
        version="0.1.0",
        description="Backend API for internal app distribution workspace data.",
        lifespan=lifespan,
    )
    app.state.settings = app_settings
    app.state.app_log_hub = AppLogHub()
    if app_settings.cors_allowed_origins:
        app.add_middleware(
            CORSMiddleware,
            allow_origins=list(app_settings.cors_allowed_origins),
            allow_methods=["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"],
            allow_headers=[
                "Authorization",
                "Content-Type",
                "X-Device-ID",
                "X-Client-Platform",
                "Accept",
            ],
        )
    if session_factory is None:
        engine = create_engine_for_url(app_settings.database_url)
        Base.metadata.create_all(engine)
        session_factory = create_session_factory(engine)
        app.state.engine = engine
    app.state.session_factory = session_factory
    app.state.artifact_storage = artifact_storage or storage_from_settings(app_settings)
    package_dir = Path(__file__).parent

    @app.middleware("http")
    async def redirect_legacy_admin_pages(request: Request, call_next):
        if request.method == "GET":
            redirect_path = admin_next_redirect_path(request.url.path)
            if redirect_path is not None:
                target = redirect_path
                if request.url.query:
                    target = f"{target}?{request.url.query}"
                return RedirectResponse(target, status_code=307)
        return await call_next(request)

    if app_settings.storage_backend == "local":
        app_settings.storage_root.mkdir(parents=True, exist_ok=True)
        app.mount(
            "/artifacts",
            StaticFiles(directory=app_settings.storage_root),
            name="artifacts",
        )
    app.mount(
        "/static",
        StaticFiles(directory=package_dir / "static"),
        name="static",
    )

    app.add_exception_handler(ApiError, api_error_handler)

    from testflying_api import admin_spa
    from testflying_api.admin_api import routes as admin_api_routes
    from testflying_api.admin_api.errors import AdminApiError, admin_api_error_handler
    from testflying_api.routes import (
        accounts,
        app_logs,
        connector_agent,
        devices,
        health,
        llm_feedback,
        notifications,
        store_management,
        uploads,
        workspace,
    )

    app.add_exception_handler(AdminApiError, admin_api_error_handler)

    app.include_router(health.router)
    app.include_router(app_logs.router)
    app.include_router(connector_agent.router)
    app.include_router(admin_api_routes.router)
    app.include_router(admin_spa.router)
    app.include_router(workspace.router)
    app.include_router(uploads.router)
    app.include_router(llm_feedback.router)
    app.include_router(store_management.router)
    app.include_router(devices.router)
    app.include_router(accounts.router)
    app.include_router(notifications.router)
    return app

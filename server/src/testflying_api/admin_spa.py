from __future__ import annotations

from importlib import resources
from io import BytesIO
from pathlib import Path
from typing import Annotated

from fastapi import APIRouter, Depends, Request
from fastapi.responses import HTMLResponse, JSONResponse, Response

from testflying_api.admin.security import require_admin
from testflying_api.app_logs import LEVELS, build_app_log_connect_context

AdminDep = Annotated[None, Depends(require_admin)]

router = APIRouter(prefix="/admin", tags=["admin"])
ADMIN_APP_DIR = Path(__file__).parent / "static" / "admin-app"
PUBLIC_API_DOC_PATH = "docs/store-management-public-api.md"


@router.get("", response_class=HTMLResponse)
@router.get("/", response_class=HTMLResponse)
def admin_app(request: Request, _: AdminDep) -> HTMLResponse:
    return HTMLResponse(_admin_app_index())


@router.get("/api-docs/store-management.md")
def api_docs_markdown(_: AdminDep) -> Response:
    content = (
        resources.files("testflying_api")
        .joinpath(PUBLIC_API_DOC_PATH)
        .read_text(encoding="utf-8")
    )
    return Response(
        content=content,
        media_type="text/markdown; charset=utf-8",
        headers={
            "Content-Disposition": 'attachment; filename="testflying-store-management-api.md"'
        },
    )


@router.get("/app-logs/events")
def app_logs_events(
    request: Request,
    _: AdminDep,
    cursor: int = 0,
    limit: int = 500,
) -> JSONResponse:
    snapshot = request.app.state.app_log_hub.snapshot(cursor=cursor, limit=limit)
    return JSONResponse(
        {
            "cursor": snapshot.cursor,
            "devices": snapshot.devices,
            "logs": snapshot.logs,
            "errors": snapshot.errors,
            "levels": list(LEVELS),
        }
    )


@router.get("/app-logs/qr.svg")
def app_logs_qr(
    request: Request,
    _: AdminDep,
    host: str = "",
    port: str = "",
    name: str = "Mac",
) -> Response:
    import qrcode
    import qrcode.image.svg

    context = build_app_log_connect_context(request, host=host, port=port, name=name)
    image = qrcode.make(context["connect_url"], image_factory=qrcode.image.svg.SvgPathImage)
    stream = BytesIO()
    image.save(stream)
    return Response(
        content=stream.getvalue(),
        media_type="image/svg+xml",
        headers={"Cache-Control": "no-store"},
    )


@router.get("/artifacts/{storage_key:path}")
def admin_artifact(
    storage_key: str,
    request: Request,
    _: AdminDep,
) -> Response:
    artifact = request.app.state.artifact_storage.read(storage_key)
    return Response(
        content=artifact.content,
        media_type=artifact.content_type,
        headers={"Cache-Control": "private, max-age=300"},
    )


@router.get("/{path:path}", response_class=HTMLResponse)
def admin_app_fallback(request: Request, _: AdminDep, path: str = "") -> HTMLResponse:
    return HTMLResponse(_admin_app_index())


def _admin_app_index() -> str:
    index_path = ADMIN_APP_DIR / "index.html"
    if index_path.exists():
        return index_path.read_text(encoding="utf-8")
    return """<!doctype html>
<html lang="zh-CN">
  <head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>testflying 管理后台</title>
  </head>
  <body>
    <div id="root" data-admin-app-root data-admin-app-build="missing">
      <h1>testflying 新后台</h1>
      <p>Admin App 还没有构建。请运行 npm --prefix admin-web run build。</p>
    </div>
  </body>
</html>"""

from __future__ import annotations

import json
from pathlib import Path

from fastapi import APIRouter, Request, WebSocket, WebSocketDisconnect
from fastapi.responses import HTMLResponse
from fastapi.templating import Jinja2Templates

from testflying_api.app_logs import AppLogHub, build_app_log_connect_context

router = APIRouter(tags=["app-logs"])
templates = Jinja2Templates(directory=str(Path(__file__).parents[1] / "templates"))


@router.get("/app-logs/connect", response_class=HTMLResponse, name="app_log_connect_page")
def app_log_connect_page(
    request: Request,
    host: str = "",
    port: str = "",
    name: str = "Mac",
) -> HTMLResponse:
    return templates.TemplateResponse(
        request,
        "app_log_connect.html",
        {
            "request": request,
            "connect": build_app_log_connect_context(
                request,
                host=host,
                port=port,
                name=name,
            ),
        },
    )


@router.websocket("/push")
async def receive_app_logs(websocket: WebSocket) -> None:
    await websocket.accept()
    hub: AppLogHub = websocket.app.state.app_log_hub
    token, connection_id = hub.connect(websocket.query_params.get("token"))
    try:
        while True:
            message = await websocket.receive_text()
            try:
                payload = json.loads(message)
            except json.JSONDecodeError:
                hub.client_error(token, {"message": "WebSocket 消息不是合法 JSON"})
                continue
            if not isinstance(payload, dict):
                hub.client_error(token, {"message": "WebSocket JSON 根节点必须是对象"})
                continue

            message_type = str(payload.get("type") or "")
            if message_type == "hello":
                hub.hello(token, payload)
            elif message_type == "logs":
                hub.add_logs(token, payload)
            elif message_type == "client_error":
                hub.client_error(token, payload)
            else:
                hub.client_error(token, {"message": f"未知日志消息类型：{message_type or '-'}"})
    except WebSocketDisconnect:
        hub.disconnect(token, connection_id)

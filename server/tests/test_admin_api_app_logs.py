from __future__ import annotations

import json
from base64 import b64encode

from fastapi.testclient import TestClient


def _admin_headers(password: str = "dev-token") -> dict[str, str]:
    token = b64encode(f"admin:{password}".encode()).decode()
    return {"Authorization": f"Basic {token}"}


def test_admin_api_app_logs_state_contains_connect_context(client: TestClient) -> None:
    response = client.get("/admin/api/app-logs", headers=_admin_headers())

    assert response.status_code == 200
    payload = response.json()
    assert payload["connect"]["appName"] == "AnyStories"
    assert payload["connect"]["schemeUrl"].startswith("anystories:///connect?")
    assert payload["connect"]["websocketUrl"].startswith("ws://")
    assert payload["levels"] == ["跟踪", "调试", "信息", "警告", "错误", "致命"]
    assert payload["devices"] == []
    assert payload["logs"] == []


def test_admin_api_app_logs_events_return_hub_snapshot(client: TestClient) -> None:
    with client.websocket_connect("/push?token=device-admin-ios") as websocket:
        websocket.send_text(
            json.dumps(
                {
                    "type": "hello",
                    "protocol": 1,
                    "deviceId": "device-admin-ios",
                    "device": "Admin iPhone",
                    "platform": "ios",
                    "sentAt": "2026-06-18T12:00:00.000",
                }
            )
        )
        websocket.send_text(
            json.dumps(
                {
                    "type": "logs",
                    "history": False,
                    "lines": [
                        "2026-06-18 12:00:00.123 级别=信息 标签=播放器 "
                        "事件=播放 消息=开始播放 章节序号=3"
                    ],
                    "sentAt": "2026-06-18T12:00:01.000",
                }
            )
        )
        websocket.send_text(
            json.dumps(
                {
                    "type": "client_error",
                    "message": "读取日志失败",
                    "sentAt": "2026-06-18T12:00:02.000",
                }
            )
        )

        response = client.get("/admin/api/app-logs/events", headers=_admin_headers())

    assert response.status_code == 200
    payload = response.json()
    assert payload["devices"][0]["device"] == "Admin iPhone"
    assert payload["devices"][0]["connected"] is True
    assert payload["logs"][0]["message"] == "开始播放"
    assert payload["logs"][0]["fields"] == [{"key": "章节序号", "value": "3"}]
    assert payload["errors"][0]["message"] == "读取日志失败"

    cursor = payload["cursor"]
    next_response = client.get(
        f"/admin/api/app-logs/events?cursor={cursor}",
        headers=_admin_headers(),
    )
    assert next_response.status_code == 200
    assert next_response.json()["logs"] == []
    assert next_response.json()["errors"] == []

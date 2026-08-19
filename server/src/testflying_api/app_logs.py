from __future__ import annotations

import json
import re
from collections import deque
from dataclasses import dataclass, field
from datetime import UTC, datetime
from threading import RLock
from urllib.parse import urlencode
from uuid import uuid4

from fastapi import Request

LEVELS = ("跟踪", "调试", "信息", "警告", "错误", "致命")
UNKNOWN_TOKEN = "unknown"
LOG_TIMESTAMP_RE = re.compile(
    r"^(?P<timestamp>\d{4}-\d{2}-\d{2}\s+\d{2}:\d{2}:\d{2}(?:\.\d{1,6})?)\s+(?P<body>.*)$"
)
KEY_RE = re.compile(r"[\w\u4e00-\u9fff.-]+=")
CORE_FIELD_KEYS = {"级别", "标签", "事件", "消息"}
APP_LOG_SCHEME = "anystories"
APP_LOG_APP_NAME = "AnyStories"


def build_app_log_connect_context(
    request: Request,
    *,
    host: str = "",
    port: str = "",
    name: str = "Mac",
) -> dict[str, str]:
    normalized_host = (host or request.url.hostname or "127.0.0.1").strip()
    normalized_port = (port or str(request.url.port or _default_port(request))).strip()
    normalized_name = (name or "Mac").strip()
    query = urlencode(
        {
            "host": normalized_host,
            "port": normalized_port,
            "name": normalized_name,
        }
    )
    connect_page_base_url = str(request.url_for("app_log_connect_page"))
    connect_page_url = f"{connect_page_base_url}?{query}"
    return {
        "host": normalized_host,
        "port": normalized_port,
        "name": normalized_name,
        "app_scheme": APP_LOG_SCHEME,
        "app_name": APP_LOG_APP_NAME,
        "connect_url": connect_page_url,
        "connect_page_url": connect_page_url,
        "scheme_url": f"{APP_LOG_SCHEME}:///connect?{query}",
        "websocket_url": f"ws://{normalized_host}:{normalized_port}/push?token=<设备ID>",
    }


def _default_port(request: Request) -> int:
    return 443 if request.url.scheme == "https" else 80


@dataclass
class LogDevice:
    token: str
    device_id: str
    device_name: str
    platform: str
    connected: bool
    known_token: bool
    connected_at: datetime
    last_seen_at: datetime
    connection_id: str
    connection_count: int = 0
    error_count: int = 0
    log_count: int = 0

    def snapshot(self) -> dict[str, object]:
        return {
            "token": self.token,
            "deviceId": self.device_id,
            "device": self.device_name,
            "platform": self.platform,
            "connected": self.connected,
            "knownToken": self.known_token,
            "connectedAt": _isoformat(self.connected_at),
            "lastSeenAt": _isoformat(self.last_seen_at),
            "connectionCount": self.connection_count,
            "errorCount": self.error_count,
            "logCount": self.log_count,
        }


@dataclass(frozen=True)
class ParsedLogLine:
    timestamp: str
    level: str
    tag: str
    event: str
    message: str
    fields: list[dict[str, str]]


@dataclass(frozen=True)
class AppLogEntry:
    sequence: int
    token: str
    device_id: str
    device_name: str
    platform: str
    received_at: datetime
    sent_at: str
    history: bool
    raw: str
    parsed: ParsedLogLine

    def snapshot(self) -> dict[str, object]:
        return {
            "sequence": self.sequence,
            "token": self.token,
            "deviceId": self.device_id,
            "device": self.device_name,
            "platform": self.platform,
            "receivedAt": _isoformat(self.received_at),
            "sentAt": self.sent_at,
            "history": self.history,
            "raw": self.raw,
            "timestamp": self.parsed.timestamp,
            "level": self.parsed.level,
            "tag": self.parsed.tag,
            "event": self.parsed.event,
            "message": self.parsed.message,
            "fields": self.parsed.fields,
        }


@dataclass(frozen=True)
class AppClientError:
    sequence: int
    token: str
    device_id: str
    device_name: str
    received_at: datetime
    sent_at: str
    message: str

    def snapshot(self) -> dict[str, object]:
        return {
            "sequence": self.sequence,
            "token": self.token,
            "deviceId": self.device_id,
            "device": self.device_name,
            "receivedAt": _isoformat(self.received_at),
            "sentAt": self.sent_at,
            "message": self.message,
        }


@dataclass
class AppLogSnapshot:
    cursor: int
    devices: list[dict[str, object]]
    logs: list[dict[str, object]]
    errors: list[dict[str, object]]


@dataclass
class AppLogHub:
    max_entries: int = 5000
    max_errors: int = 500
    _lock: RLock = field(default_factory=RLock, init=False)
    _devices: dict[str, LogDevice] = field(default_factory=dict, init=False)
    _logs: deque[AppLogEntry] = field(default_factory=deque, init=False)
    _errors: deque[AppClientError] = field(default_factory=deque, init=False)
    _sequence: int = 0

    def connect(self, raw_token: str | None) -> tuple[str, str]:
        now = datetime.now(UTC)
        token = _normalize_token(raw_token)
        known_token = bool(raw_token and raw_token.strip())
        connection_id = uuid4().hex
        with self._lock:
            device = self._devices.get(token)
            if device is None:
                device = LogDevice(
                    token=token,
                    device_id=token if known_token else "",
                    device_name="未知设备",
                    platform="unknown",
                    connected=True,
                    known_token=known_token,
                    connected_at=now,
                    last_seen_at=now,
                    connection_id=connection_id,
                    connection_count=1,
                )
                self._devices[token] = device
            else:
                device.connected = True
                device.known_token = device.known_token or known_token
                if known_token and not device.device_id:
                    device.device_id = token
                device.connected_at = now
                device.last_seen_at = now
                device.connection_id = connection_id
                device.connection_count += 1
        return token, connection_id

    def disconnect(self, token: str, connection_id: str) -> None:
        with self._lock:
            device = self._devices.get(token)
            if device and device.connection_id == connection_id:
                device.connected = False
                device.last_seen_at = datetime.now(UTC)

    def hello(self, token: str, payload: dict[str, object]) -> None:
        with self._lock:
            device = self._device_or_create(token)
            device.device_id = _string_value(payload.get("deviceId")) or device.device_id
            device.device_name = _string_value(payload.get("device")) or device.device_name
            device.platform = _string_value(payload.get("platform")) or device.platform
            device.last_seen_at = datetime.now(UTC)

    def add_logs(self, token: str, payload: dict[str, object]) -> int:
        raw_lines = payload.get("lines")
        if not isinstance(raw_lines, list):
            return 0
        sent_at = _string_value(payload.get("sentAt"))
        history = bool(payload.get("history"))
        received_at = datetime.now(UTC)
        added = 0
        with self._lock:
            device = self._device_or_create(token)
            device.last_seen_at = received_at
            for raw_line in raw_lines:
                line = _string_value(raw_line)
                if not line:
                    continue
                self._sequence += 1
                self._logs.append(
                    AppLogEntry(
                        sequence=self._sequence,
                        token=token,
                        device_id=device.device_id,
                        device_name=device.device_name,
                        platform=device.platform,
                        received_at=received_at,
                        sent_at=sent_at,
                        history=history,
                        raw=line,
                        parsed=parse_log_line(line),
                    )
                )
                added += 1
                device.log_count += 1
            while len(self._logs) > self.max_entries:
                self._logs.popleft()
        return added

    def client_error(self, token: str, payload: dict[str, object]) -> None:
        received_at = datetime.now(UTC)
        with self._lock:
            device = self._device_or_create(token)
            device.last_seen_at = received_at
            device.error_count += 1
            self._sequence += 1
            self._errors.append(
                AppClientError(
                    sequence=self._sequence,
                    token=token,
                    device_id=device.device_id,
                    device_name=device.device_name,
                    received_at=received_at,
                    sent_at=_string_value(payload.get("sentAt")),
                    message=_string_value(payload.get("message")) or "手机端异常",
                )
            )
            while len(self._errors) > self.max_errors:
                self._errors.popleft()

    def snapshot(self, *, cursor: int = 0, limit: int = 500) -> AppLogSnapshot:
        normalized_limit = max(1, min(limit, 1000))
        with self._lock:
            logs = [entry for entry in self._logs if entry.sequence > cursor]
            errors = [entry for entry in self._errors if entry.sequence > cursor]
            latest_sequence = max(
                [
                    self._sequence,
                    cursor,
                    *[entry.sequence for entry in logs],
                    *[entry.sequence for entry in errors],
                ]
            )
            devices = sorted(
                (device.snapshot() for device in self._devices.values()),
                key=lambda item: (not bool(item["connected"]), str(item["device"]).lower()),
            )
        return AppLogSnapshot(
            cursor=latest_sequence,
            devices=devices,
            logs=[
                entry.snapshot()
                for entry in sorted(logs, key=lambda item: item.sequence, reverse=True)[
                    :normalized_limit
                ]
            ],
            errors=[
                entry.snapshot()
                for entry in sorted(errors, key=lambda item: item.sequence, reverse=True)[
                    :normalized_limit
                ]
            ],
        )

    def clear(self) -> None:
        with self._lock:
            self._logs.clear()
            self._errors.clear()
            self._devices.clear()
            self._sequence = 0

    def _device_or_create(self, token: str) -> LogDevice:
        device = self._devices.get(token)
        if device is not None:
            return device
        now = datetime.now(UTC)
        device = LogDevice(
            token=token,
            device_id="" if token == UNKNOWN_TOKEN else token,
            device_name="未知设备",
            platform="unknown",
            connected=True,
            known_token=token != UNKNOWN_TOKEN,
            connected_at=now,
            last_seen_at=now,
            connection_id="",
            connection_count=0,
        )
        self._devices[token] = device
        return device


def parse_log_line(line: str) -> ParsedLogLine:
    timestamp = ""
    body = line.strip()
    match = LOG_TIMESTAMP_RE.match(body)
    if match:
        timestamp = match.group("timestamp")
        body = match.group("body")

    fields = _parse_key_values(body)
    values = {field["key"]: field["value"] for field in fields}
    extra_fields = [field for field in fields if field["key"] not in CORE_FIELD_KEYS]
    return ParsedLogLine(
        timestamp=timestamp,
        level=values.get("级别", ""),
        tag=values.get("标签", ""),
        event=values.get("事件", ""),
        message=values.get("消息", line),
        fields=extra_fields,
    )


def _parse_key_values(body: str) -> list[dict[str, str]]:
    fields: list[dict[str, str]] = []
    position = 0
    while position < len(body):
        while position < len(body) and body[position].isspace():
            position += 1
        key_match = KEY_RE.match(body, position)
        if not key_match:
            break
        key = key_match.group()[:-1]
        position = key_match.end()
        value, position = _parse_value(body, position)
        fields.append({"key": key, "value": value})
    return fields


def _parse_value(body: str, position: int) -> tuple[str, int]:
    if position >= len(body):
        return "", position
    if body[position] == '"':
        try:
            value, next_position = json.JSONDecoder().raw_decode(body[position:])
            return str(value), position + next_position
        except json.JSONDecodeError:
            pass
    next_key = _find_next_key(body, position)
    if next_key == -1:
        return body[position:].strip(), len(body)
    return body[position:next_key].strip(), next_key


def _find_next_key(body: str, position: int) -> int:
    for match in KEY_RE.finditer(body, position):
        if match.start() > 0 and body[match.start() - 1].isspace():
            return match.start()
    return -1


def _normalize_token(raw_token: str | None) -> str:
    value = (raw_token or "").strip()
    return value or UNKNOWN_TOKEN


def _string_value(value: object) -> str:
    return str(value).strip() if value is not None else ""


def _isoformat(value: datetime) -> str:
    return value.astimezone(UTC).isoformat().replace("+00:00", "Z")

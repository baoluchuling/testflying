from __future__ import annotations

import json
import time
from collections import defaultdict, deque
from dataclasses import dataclass, field
from datetime import UTC, datetime
from threading import Condition, Event
from uuid import uuid4


@dataclass(frozen=True)
class ActiveConnectorTask:
    id: str
    account_id: str
    method: str
    path: str
    headers: dict[str, str]
    body: str
    created_at: datetime


@dataclass(frozen=True)
class ActiveConnectorResult:
    status_code: int
    body: str


@dataclass
class _PendingTask:
    task: ActiveConnectorTask
    event: Event = field(default_factory=Event)
    result: ActiveConnectorResult | None = None


class ActiveConnectorTimeoutError(RuntimeError):
    pass


class ActiveConnectorHub:
    def __init__(self) -> None:
        self._condition = Condition()
        self._queues: dict[str, deque[ActiveConnectorTask]] = defaultdict(deque)
        self._pending: dict[str, _PendingTask] = {}
        self._last_seen: dict[str, datetime] = {}

    def dispatch(
        self,
        *,
        account_id: str,
        method: str,
        path: str,
        headers: dict[str, str] | None = None,
        body: str | bytes | dict[str, object] | None = None,
        timeout_seconds: float = 30,
    ) -> ActiveConnectorResult:
        normalized_body = _normalize_body(body)
        task = ActiveConnectorTask(
            id=f"task-{uuid4().hex}",
            account_id=account_id,
            method=method.upper(),
            path=path,
            headers=dict(headers or {}),
            body=normalized_body,
            created_at=datetime.now(UTC),
        )
        pending = _PendingTask(task=task)
        with self._condition:
            self._pending[task.id] = pending
            self._queues[account_id].append(task)
            self._condition.notify_all()

        if not pending.event.wait(timeout_seconds):
            with self._condition:
                self._pending.pop(task.id, None)
                try:
                    self._queues[account_id].remove(task)
                except ValueError:
                    pass
            raise ActiveConnectorTimeoutError("active connector 暂无响应或执行超时")
        if pending.result is None:
            raise ActiveConnectorTimeoutError("active connector 没有返回执行结果")
        return pending.result

    def poll(self, *, account_id: str, timeout_seconds: float = 25) -> ActiveConnectorTask | None:
        deadline = time.monotonic() + max(timeout_seconds, 0)
        with self._condition:
            self.mark_seen(account_id)
            while not self._queues[account_id]:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    return None
                self._condition.wait(remaining)
                self.mark_seen(account_id)
            self.mark_seen(account_id)
            return self._queues[account_id].popleft()

    def complete(self, *, task_id: str, status_code: int, body: str) -> bool:
        with self._condition:
            pending = self._pending.pop(task_id, None)
        if pending is None:
            return False
        pending.result = ActiveConnectorResult(status_code=status_code, body=body)
        pending.event.set()
        return True

    def mark_seen(self, account_id: str) -> None:
        with self._condition:
            self._last_seen[account_id] = datetime.now(UTC)

    def last_seen_at(self, account_id: str) -> datetime | None:
        with self._condition:
            return self._last_seen.get(account_id)

    def reset(self) -> None:
        with self._condition:
            self._queues.clear()
            self._pending.clear()
            self._last_seen.clear()
            self._condition.notify_all()


def _normalize_body(body: str | bytes | dict[str, object] | None) -> str:
    if body is None:
        return ""
    if isinstance(body, bytes):
        return body.decode("utf-8")
    if isinstance(body, str):
        return body
    return json.dumps(body, ensure_ascii=False)


active_connector_hub = ActiveConnectorHub()

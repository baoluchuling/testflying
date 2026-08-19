from __future__ import annotations

import asyncio
from dataclasses import replace
from datetime import UTC, datetime, timedelta

from fastapi.testclient import TestClient
from sqlalchemy.orm import Session, sessionmaker

from testflying_api import webhook_delivery
from testflying_api.app import create_app
from testflying_api.config import Settings
from testflying_api.dingtalk import DingTalkDeliveryError
from testflying_api.schema import SystemSetting, WebhookDelivery
from testflying_api.storage import LocalArtifactStorage
from testflying_api.webhook_delivery import dispatch_due_deliveries, run_delivery_loop


def test_dispatch_due_deliveries_sends_only_due_pending_rows_once(
    session_factory: sessionmaker[Session],
    test_settings: Settings,
) -> None:
    now = datetime(2026, 7, 10, 8, 0, tzinfo=UTC)
    with session_factory() as session:
        session.add_all(
            [
                _delivery("due", next_attempt_at=now),
                _delivery("future", next_attempt_at=now + timedelta(hours=1)),
            ]
        )
        session.commit()
    sent: list[str] = []

    def sender(**payload: object) -> None:
        sent.append(str(payload["title"]))

    settings = replace(
        test_settings,
        dingtalk_webhook_url="https://oapi.dingtalk.test/robot/send?access_token=abc",
        dingtalk_secret="SEC-test",
    )

    assert dispatch_due_deliveries(
        session_factory,
        settings,
        now=now,
        sender=sender,
    ) == 1
    assert dispatch_due_deliveries(
        session_factory,
        settings,
        now=now,
        sender=sender,
    ) == 0

    with session_factory() as session:
        due = session.get(WebhookDelivery, "delivery-due")
        future = session.get(WebhookDelivery, "delivery-future")
        assert due is not None
        assert due.status == "delivered"
        assert due.attempt_count == 1
        assert due.delivered_at is not None
        assert due.delivered_at.replace(tzinfo=UTC) == now
        assert future is not None
        assert future.status == "pending"
    assert sent == ["Build due"]


def test_failed_delivery_retries_with_backoff_then_becomes_dead(
    session_factory: sessionmaker[Session],
    test_settings: Settings,
) -> None:
    now = datetime(2026, 7, 10, 8, 0, tzinfo=UTC)
    settings = replace(
        test_settings,
        dingtalk_webhook_url="https://oapi.dingtalk.test/robot/send?access_token=abc",
        dingtalk_secret="SEC-test",
    )
    with session_factory() as session:
        session.add(_delivery("retry", next_attempt_at=now))
        session.commit()

    def failing_sender(**_payload: object) -> None:
        raise DingTalkDeliveryError("token=secret-value rejected")

    expected_delays = [
        timedelta(minutes=1),
        timedelta(minutes=5),
        timedelta(minutes=30),
        timedelta(hours=2),
    ]
    current = now
    for attempt, delay in enumerate(expected_delays, start=1):
        assert dispatch_due_deliveries(
            session_factory,
            settings,
            now=current,
            sender=failing_sender,
        ) == 1
        with session_factory() as session:
            delivery = session.get(WebhookDelivery, "delivery-retry")
            assert delivery is not None
            assert delivery.status == "pending"
            assert delivery.attempt_count == attempt
            assert delivery.next_attempt_at is not None
            next_attempt = delivery.next_attempt_at.replace(tzinfo=UTC)
            assert next_attempt == current + delay
            assert "secret-value" not in (delivery.last_error or "")
            assert "token=[REDACTED]" in (delivery.last_error or "")
        current += delay

    assert dispatch_due_deliveries(
        session_factory,
        settings,
        now=current,
        sender=failing_sender,
    ) == 1
    with session_factory() as session:
        delivery = session.get(WebhookDelivery, "delivery-retry")
        assert delivery is not None
        assert delivery.status == "dead"
        assert delivery.attempt_count == 5


def test_failed_delivery_redacts_webhook_query_credentials(
    session_factory: sessionmaker[Session],
    test_settings: Settings,
) -> None:
    now = datetime(2026, 7, 10, 8, 0, tzinfo=UTC)
    settings = replace(
        test_settings,
        dingtalk_webhook_url=(
            "https://oapi.dingtalk.test/robot/send?access_token=token-value"
        ),
        dingtalk_secret="SEC-test",
    )
    with session_factory() as session:
        session.add(_delivery("query-secret", next_attempt_at=now))
        session.commit()

    def failing_sender(**payload: object) -> None:
        raise RuntimeError(
            f"request failed: {payload['webhook_url']}&timestamp=1&sign=signature-value"
        )

    assert dispatch_due_deliveries(
        session_factory,
        settings,
        now=now,
        sender=failing_sender,
    ) == 1

    with session_factory() as session:
        delivery = session.get(WebhookDelivery, "delivery-query-secret")
        assert delivery is not None
        assert "token-value" not in (delivery.last_error or "")
        assert "signature-value" not in (delivery.last_error or "")
        assert "access_token=[REDACTED]" in (delivery.last_error or "")
        assert "sign=[REDACTED]" in (delivery.last_error or "")


def test_delivery_loop_dispatches_immediately_and_stops(
    session_factory: sessionmaker[Session],
    test_settings: Settings,
) -> None:
    calls: list[bool] = []

    async def exercise() -> None:
        stop_event = asyncio.Event()

        def dispatcher(*_args: object, **_kwargs: object) -> int:
            calls.append(True)
            stop_event.set()
            return 0

        await run_delivery_loop(
            session_factory,
            test_settings,
            stop_event,
            dispatcher=dispatcher,
        )

    asyncio.run(exercise())

    assert calls == [True]


def test_database_setting_can_disable_environment_delivery(
    session_factory: sessionmaker[Session],
    test_settings: Settings,
) -> None:
    settings = replace(
        test_settings,
        dingtalk_webhook_url="https://oapi.dingtalk.test/robot/send?access_token=abc",
        dingtalk_secret="SEC-test",
    )
    with session_factory() as session:
        session.add_all(
            [
                SystemSetting(key="dingtalk_enabled", value="false"),
                _delivery("disabled", next_attempt_at=datetime.now(UTC)),
            ]
        )
        session.commit()

    assert dispatch_due_deliveries(session_factory, settings) == 0

    with session_factory() as session:
        delivery = session.get(WebhookDelivery, "delivery-disabled")
        assert delivery is not None
        assert delivery.status == "pending"


def test_app_lifespan_starts_and_stops_configured_delivery_loop(
    monkeypatch,
    session_factory: sessionmaker[Session],
    test_settings: Settings,
) -> None:
    calls: list[str] = []

    async def fake_loop(
        _session_factory: sessionmaker[Session],
        _settings: Settings,
        stop_event: asyncio.Event,
    ) -> None:
        calls.append("started")
        await stop_event.wait()
        calls.append("stopped")

    monkeypatch.setattr(webhook_delivery, "run_delivery_loop", fake_loop)
    settings = replace(
        test_settings,
        dingtalk_webhook_url="https://oapi.dingtalk.test/robot/send?access_token=abc",
        dingtalk_secret="SEC-test",
    )
    app = create_app(
        settings,
        session_factory=session_factory,
        artifact_storage=LocalArtifactStorage(
            root=settings.storage_root,
            public_base_url=settings.public_base_url,
        ),
    )

    with TestClient(app):
        assert calls == ["started"]

    assert calls == ["started", "stopped"]


def test_app_lifespan_starts_delivery_loop_before_notification_is_configured(
    monkeypatch,
    session_factory: sessionmaker[Session],
    test_settings: Settings,
) -> None:
    calls: list[str] = []

    async def fake_loop(
        _session_factory: sessionmaker[Session],
        _settings: Settings,
        stop_event: asyncio.Event,
    ) -> None:
        calls.append("started")
        await stop_event.wait()
        calls.append("stopped")

    monkeypatch.setattr(webhook_delivery, "run_delivery_loop", fake_loop)
    app = create_app(
        test_settings,
        session_factory=session_factory,
        artifact_storage=LocalArtifactStorage(
            root=test_settings.storage_root,
            public_base_url=test_settings.public_base_url,
        ),
    )

    with TestClient(app):
        assert calls == ["started"]

    assert calls == ["started", "stopped"]


def _delivery(name: str, *, next_attempt_at: datetime) -> WebhookDelivery:
    return WebhookDelivery(
        id=f"delivery-{name}",
        channel="dingtalk",
        event_key=f"build:{name}:failed:dingtalk",
        status="pending",
        payload_json={"title": f"Build {name}", "markdown": f"**{name}** failed"},
        attempt_count=0,
        next_attempt_at=next_attempt_at,
    )

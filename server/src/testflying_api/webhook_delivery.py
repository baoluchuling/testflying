from __future__ import annotations

import asyncio
from collections.abc import Callable
from datetime import UTC, datetime, timedelta

from sqlalchemy import select
from sqlalchemy.orm import Session, sessionmaker

from testflying_api.config import Settings
from testflying_api.dingtalk import send_dingtalk_markdown
from testflying_api.redaction import redact_text
from testflying_api.schema import WebhookDelivery
from testflying_api.system_settings import effective_business_settings

DeliverySender = Callable[..., None]
DeliveryDispatcher = Callable[..., int]
RETRY_DELAYS = (
    timedelta(0),
    timedelta(minutes=1),
    timedelta(minutes=5),
    timedelta(minutes=30),
    timedelta(hours=2),
)


def dispatch_due_deliveries(
    session_factory: sessionmaker[Session],
    settings: Settings,
    *,
    now: datetime | None = None,
    sender: DeliverySender = send_dingtalk_markdown,
) -> int:
    current = now or datetime.now(UTC)
    processed = 0
    with session_factory() as session:
        effective = effective_business_settings(session, settings)
        if not effective.dingtalk_configured:
            return 0
        deliveries = session.scalars(
            select(WebhookDelivery)
            .where(
                WebhookDelivery.channel == "dingtalk",
                WebhookDelivery.status == "pending",
                WebhookDelivery.next_attempt_at <= current,
            )
            .order_by(WebhookDelivery.next_attempt_at.asc(), WebhookDelivery.id.asc())
        ).all()
        for delivery in deliveries:
            payload = dict(delivery.payload_json or {})
            try:
                sender(
                    webhook_url=effective.dingtalk_webhook_url or "",
                    secret=effective.dingtalk_secret or "",
                    title=str(payload.get("title") or "TestFlying build notification"),
                    markdown=str(payload.get("markdown") or ""),
                    timeout_seconds=effective.dingtalk_timeout_seconds,
                )
            except Exception as error:  # noqa: BLE001 - delivery failures must stay isolated
                delivery.attempt_count += 1
                delivery.last_error = redact_text(str(error))[:500]
                if delivery.attempt_count >= len(RETRY_DELAYS):
                    delivery.status = "dead"
                else:
                    delivery.next_attempt_at = current + RETRY_DELAYS[delivery.attempt_count]
            else:
                delivery.attempt_count += 1
                delivery.status = "delivered"
                delivery.last_error = None
                delivery.delivered_at = current
            session.add(delivery)
            session.commit()
            processed += 1
    return processed


async def run_delivery_loop(
    session_factory: sessionmaker[Session],
    settings: Settings,
    stop_event: asyncio.Event,
    *,
    dispatcher: DeliveryDispatcher = dispatch_due_deliveries,
) -> None:
    while not stop_event.is_set():
        await asyncio.to_thread(dispatcher, session_factory, settings)
        if stop_event.is_set():
            break
        with session_factory() as session:
            effective = effective_business_settings(session, settings)
        try:
            await asyncio.wait_for(
                stop_event.wait(),
                timeout=effective.dingtalk_dispatch_interval_seconds,
            )
        except TimeoutError:
            continue

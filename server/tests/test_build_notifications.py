from __future__ import annotations

import pytest
from sqlalchemy import select
from sqlalchemy.orm import Session

from testflying_api.build_notifications import enqueue_terminal_build_notifications
from testflying_api.schema import App, Build, Notification, WebhookDelivery


def test_failed_agent_build_enqueues_redacted_notifications_once(db_session: Session) -> None:
    build = _agent_build(db_session, status="failed")
    build.failure_classification = "build_command_failed"
    build.failure_summary = "token=secret-value command failed"
    build.human_action = "Inspect signing settings."
    db_session.commit()

    for _ in range(2):
        enqueue_terminal_build_notifications(
            db_session,
            build,
            dingtalk_enabled=True,
            public_base_url="https://dist.example.test",
        )
        db_session.commit()

    notifications = list(
        db_session.scalars(select(Notification).where(Notification.build_id == build.id))
    )
    deliveries = list(
        db_session.scalars(
            select(WebhookDelivery).where(
                WebhookDelivery.event_key == f"build:{build.id}:failed:dingtalk"
            )
        )
    )
    assert len(notifications) == 1
    assert notifications[0].type == "build"
    assert notifications[0].app_id == build.app_id
    assert "secret-value" not in notifications[0].subtitle
    assert len(deliveries) == 1
    payload = deliveries[0].payload_json
    assert "Demo" in payload["markdown"]
    assert "development" in payload["markdown"]
    assert "main" in payload["markdown"]
    assert "Inspect signing settings." in payload["markdown"]
    assert "https://dist.example.test/admin/apps/app-ios-demo" in payload["markdown"]
    assert "secret-value" not in payload["markdown"]
    assert "token=[REDACTED]" in payload["markdown"]


def test_needs_human_keeps_in_app_notification_without_dingtalk(db_session: Session) -> None:
    build = _agent_build(db_session, status="needs_human")

    enqueue_terminal_build_notifications(
        db_session,
        build,
        dingtalk_enabled=False,
        public_base_url="https://dist.example.test",
    )
    db_session.commit()

    assert len(list(db_session.scalars(select(Notification)))) == 1
    assert len(list(db_session.scalars(select(WebhookDelivery)))) == 0


@pytest.mark.parametrize(
    ("status", "source"),
    [("succeeded", "agent"), ("cancelled", "agent"), ("failed", "upload")],
)
def test_non_boundary_builds_do_not_enqueue_notifications(
    db_session: Session,
    status: str,
    source: str,
) -> None:
    build = _agent_build(db_session, status=status)
    build.source = source
    db_session.commit()

    enqueue_terminal_build_notifications(
        db_session,
        build,
        dingtalk_enabled=True,
        public_base_url="https://dist.example.test",
    )
    db_session.commit()

    assert len(list(db_session.scalars(select(Notification)))) == 0
    assert len(list(db_session.scalars(select(WebhookDelivery)))) == 0


def _agent_build(session: Session, *, status: str) -> Build:
    app = App(
        id="app-ios-demo",
        name="Demo",
        bundle_identifier="com.example.demo",
        platform="ios",
        default_channel="dev",
    )
    build = Build(
        id=f"build-agent-{status}",
        app=app,
        channel="dev",
        environment="development",
        requested_environment="development",
        platform="ios",
        source="agent",
        lifecycle_status=status,
        git_url="git@example.com:demo.git",
        git_ref="main",
        note="",
        status="failed" if status == "failed" else "pending",
    )
    session.add_all([app, build])
    session.commit()
    return build

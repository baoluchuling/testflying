from __future__ import annotations

from hashlib import sha256
from urllib.parse import quote

from sqlalchemy import select
from sqlalchemy.orm import Session

from testflying_api.redaction import redact_text
from testflying_api.schema import Build, Notification, WebhookDelivery

NOTIFIABLE_STATUSES = {"failed", "needs_human"}


def enqueue_terminal_build_notifications(
    session: Session,
    build: Build,
    *,
    dingtalk_enabled: bool,
    public_base_url: str,
) -> None:
    if build.source != "agent" or build.lifecycle_status not in NOTIFIABLE_STATUSES:
        return
    app = build.app
    if app is None:
        return

    event_key = f"build:{build.id}:{build.lifecycle_status}:dingtalk"
    digest = sha256(event_key.encode()).hexdigest()[:20]
    notification_id = f"notice-build-{digest}"
    failure_summary = redact_text(build.failure_summary or build.note or "Build failed.")
    human_action = redact_text(build.human_action or "Inspect the build report and logs.")
    status_label = "需要人工处理" if build.lifecycle_status == "needs_human" else "构建失败"
    if session.get(Notification, notification_id) is None:
        session.add(
            Notification(
                id=notification_id,
                type="build",
                section="构建",
                icon_key="alert-triangle",
                title=f"{app.name} {status_label}",
                subtitle=failure_summary[:280],
                tag="需处理",
                tag_color="#B42318",
                app_id=app.id,
                build_id=build.id,
            )
        )

    if not dingtalk_enabled:
        return
    existing = session.scalar(
        select(WebhookDelivery).where(WebhookDelivery.event_key == event_key)
    )
    if existing is not None:
        return

    app_url = f"{public_base_url.rstrip('/')}/admin/apps/{quote(app.id, safe='')}"
    values = {
        "app": redact_text(app.name),
        "bundle": redact_text(app.bundle_identifier),
        "platform": redact_text(build.platform),
        "environment": redact_text(build.environment),
        "build_id": redact_text(build.id),
        "git_ref": redact_text(build.git_ref or "-"),
        "classification": redact_text(build.failure_classification or "-"),
        "summary": failure_summary,
        "human_action": human_action,
    }
    markdown = "\n".join(
        [
            f"## TestFlying {status_label}",
            f"- 应用：{values['app']} ({values['bundle']})",
            f"- 平台 / 环境：{values['platform']} / {values['environment']}",
            f"- 构建 ID：{values['build_id']}",
            f"- Git ref：{values['git_ref']}",
            f"- 分类：{values['classification']}",
            f"- 摘要：{values['summary']}",
            f"- 人工动作：{values['human_action']}",
            f"[查看应用详情]({app_url})",
        ]
    )
    session.add(
        WebhookDelivery(
            id=f"delivery-{digest}",
            channel="dingtalk",
            event_key=event_key,
            status="pending",
            payload_json={
                "title": f"{values['app']} {status_label}",
                "markdown": markdown,
            },
            attempt_count=0,
        )
    )

from __future__ import annotations

import sqlalchemy as sa

from alembic import op

revision = "20260710_0011"
down_revision = "20260710_0010"
branch_labels = None
depends_on = None


def upgrade() -> None:
    op.create_table(
        "webhook_deliveries",
        sa.Column("id", sa.String(length=80), nullable=False),
        sa.Column("channel", sa.String(length=30), nullable=False),
        sa.Column("event_key", sa.String(length=240), nullable=False),
        sa.Column("status", sa.String(length=30), nullable=False),
        sa.Column("payload_json", sa.JSON(), nullable=False),
        sa.Column("attempt_count", sa.Integer(), nullable=False),
        sa.Column("next_attempt_at", sa.DateTime(timezone=True), nullable=False),
        sa.Column("last_error", sa.Text(), nullable=True),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
        sa.Column("delivered_at", sa.DateTime(timezone=True), nullable=True),
        sa.PrimaryKeyConstraint("id"),
        sa.UniqueConstraint("event_key", name="uq_webhook_deliveries_event_key"),
    )
    op.create_index(
        "ix_webhook_deliveries_status_next_attempt",
        "webhook_deliveries",
        ["status", "next_attempt_at"],
        unique=False,
    )
    op.create_index(
        "ix_webhook_deliveries_channel",
        "webhook_deliveries",
        ["channel"],
        unique=False,
    )


def downgrade() -> None:
    op.drop_index("ix_webhook_deliveries_channel", table_name="webhook_deliveries")
    op.drop_index(
        "ix_webhook_deliveries_status_next_attempt",
        table_name="webhook_deliveries",
    )
    op.drop_table("webhook_deliveries")

from __future__ import annotations

import sqlalchemy as sa

from alembic import op

revision = "20260618_0002"
down_revision = "20260616_0001"
branch_labels = None
depends_on = None


def upgrade() -> None:
    with op.batch_alter_table("apps") as batch_op:
        batch_op.add_column(sa.Column("developer_account_id", sa.String(length=80), nullable=True))
        batch_op.add_column(sa.Column("store_app_id", sa.String(length=120), nullable=True))
        batch_op.add_column(sa.Column("store_package_name", sa.String(length=180), nullable=True))
        batch_op.create_foreign_key(
            "fk_apps_developer_account_id",
            "developer_accounts",
            ["developer_account_id"],
            ["id"],
            ondelete="SET NULL",
        )

    op.create_table(
        "store_connectors",
        sa.Column("id", sa.String(length=80), primary_key=True),
        sa.Column(
            "developer_account_id",
            sa.String(length=80),
            sa.ForeignKey("developer_accounts.id", ondelete="CASCADE"),
            nullable=False,
        ),
        sa.Column("name", sa.String(length=120), nullable=False),
        sa.Column("base_url", sa.String(length=400), nullable=False),
        sa.Column("auth_token", sa.String(length=240), nullable=False),
        sa.Column("status", sa.String(length=40), nullable=False),
        sa.Column("last_checked_at", sa.DateTime(timezone=True)),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
    )
    op.create_table(
        "store_release_note_drafts",
        sa.Column("id", sa.String(length=80), primary_key=True),
        sa.Column(
            "developer_account_id",
            sa.String(length=80),
            sa.ForeignKey("developer_accounts.id", ondelete="CASCADE"),
            nullable=False,
        ),
        sa.Column(
            "app_id",
            sa.String(length=80),
            sa.ForeignKey("apps.id", ondelete="CASCADE"),
            nullable=False,
        ),
        sa.Column("platform", sa.String(length=20), nullable=False),
        sa.Column("version", sa.String(length=60), nullable=False),
        sa.Column("locale", sa.String(length=40), nullable=False),
        sa.Column("release_notes", sa.Text(), nullable=False),
        sa.Column("updated_at", sa.DateTime(timezone=True), nullable=False),
        sa.UniqueConstraint(
            "developer_account_id",
            "app_id",
            "platform",
            "version",
            "locale",
            name="uq_store_release_note_drafts_scope",
        ),
    )
    op.create_table(
        "store_preflight_checks",
        sa.Column("id", sa.String(length=80), primary_key=True),
        sa.Column(
            "developer_account_id",
            sa.String(length=80),
            sa.ForeignKey("developer_accounts.id", ondelete="CASCADE"),
            nullable=False,
        ),
        sa.Column(
            "app_id",
            sa.String(length=80),
            sa.ForeignKey("apps.id", ondelete="CASCADE"),
            nullable=False,
        ),
        sa.Column(
            "connector_id",
            sa.String(length=80),
            sa.ForeignKey("store_connectors.id", ondelete="CASCADE"),
            nullable=False,
        ),
        sa.Column("platform", sa.String(length=20), nullable=False),
        sa.Column("operation", sa.String(length=60), nullable=False),
        sa.Column("version", sa.String(length=60), nullable=False),
        sa.Column("locale", sa.String(length=40), nullable=False),
        sa.Column("request_hash", sa.String(length=64), nullable=False),
        sa.Column("can_sync", sa.Boolean(), nullable=False),
        sa.Column("reason_code", sa.String(length=80)),
        sa.Column("message", sa.String(length=280), nullable=False),
        sa.Column("store_state_json", sa.JSON(), nullable=False),
        sa.Column("checked_at", sa.DateTime(timezone=True), nullable=False),
        sa.Column("expires_at", sa.DateTime(timezone=True), nullable=False),
    )
    op.create_table(
        "store_sync_runs",
        sa.Column("id", sa.String(length=80), primary_key=True),
        sa.Column(
            "developer_account_id",
            sa.String(length=80),
            sa.ForeignKey("developer_accounts.id", ondelete="CASCADE"),
            nullable=False,
        ),
        sa.Column(
            "app_id",
            sa.String(length=80),
            sa.ForeignKey("apps.id", ondelete="CASCADE"),
            nullable=False,
        ),
        sa.Column(
            "connector_id",
            sa.String(length=80),
            sa.ForeignKey("store_connectors.id", ondelete="CASCADE"),
            nullable=False,
        ),
        sa.Column(
            "draft_id",
            sa.String(length=80),
            sa.ForeignKey("store_release_note_drafts.id", ondelete="SET NULL"),
        ),
        sa.Column("platform", sa.String(length=20), nullable=False),
        sa.Column("operation", sa.String(length=60), nullable=False),
        sa.Column("version", sa.String(length=60), nullable=False),
        sa.Column("locale", sa.String(length=40), nullable=False),
        sa.Column("status", sa.String(length=40), nullable=False),
        sa.Column("started_at", sa.DateTime(timezone=True), nullable=False),
        sa.Column("finished_at", sa.DateTime(timezone=True)),
        sa.Column("error_code", sa.String(length=80)),
        sa.Column("error_summary", sa.String(length=280)),
    )
    op.create_table(
        "audit_logs",
        sa.Column("id", sa.String(length=80), primary_key=True),
        sa.Column(
            "developer_account_id",
            sa.String(length=80),
            sa.ForeignKey("developer_accounts.id", ondelete="SET NULL"),
        ),
        sa.Column("actor", sa.String(length=120), nullable=False),
        sa.Column("action", sa.String(length=120), nullable=False),
        sa.Column("target_type", sa.String(length=80), nullable=False),
        sa.Column("target_id", sa.String(length=120), nullable=False),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
    )


def downgrade() -> None:
    op.drop_table("audit_logs")
    op.drop_table("store_sync_runs")
    op.drop_table("store_preflight_checks")
    op.drop_table("store_release_note_drafts")
    op.drop_table("store_connectors")
    with op.batch_alter_table("apps") as batch_op:
        batch_op.drop_constraint("fk_apps_developer_account_id", type_="foreignkey")
        batch_op.drop_column("store_package_name")
        batch_op.drop_column("store_app_id")
        batch_op.drop_column("developer_account_id")

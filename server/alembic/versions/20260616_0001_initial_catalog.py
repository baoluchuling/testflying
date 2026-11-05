from __future__ import annotations

import sqlalchemy as sa

from alembic import op

revision = "20260616_0001"
down_revision = None
branch_labels = None
depends_on = None


def upgrade() -> None:
    op.create_table(
        "apps",
        sa.Column("id", sa.String(length=80), primary_key=True),
        sa.Column("name", sa.String(length=120), nullable=False),
        sa.Column("bundle_identifier", sa.String(length=180), nullable=False),
        sa.Column("platform", sa.String(length=20), nullable=False),
        sa.Column("default_channel", sa.String(length=20), nullable=False),
        sa.Column("icon_key", sa.String(length=40), nullable=False),
        sa.Column("icon_color", sa.String(length=20), nullable=False),
        sa.Column("added_at", sa.DateTime(timezone=True), nullable=False),
        sa.UniqueConstraint("platform", "bundle_identifier"),
    )
    op.create_table(
        "devices",
        sa.Column("id", sa.String(length=120), primary_key=True),
        sa.Column("name", sa.String(length=120), nullable=False),
        sa.Column("owner", sa.String(length=120), nullable=False),
        sa.Column("platform", sa.String(length=20), nullable=False),
        sa.Column("status", sa.String(length=40), nullable=False),
        sa.Column("status_color", sa.String(length=20), nullable=False),
        sa.Column("detail", sa.String(length=240), nullable=False),
        sa.Column("udid", sa.String(length=160), nullable=False),
        sa.Column("os_version", sa.String(length=80), nullable=False),
        sa.Column("certificate_status", sa.String(length=120), nullable=False),
        sa.Column("registered_at", sa.DateTime(timezone=True), nullable=False),
    )
    op.create_table(
        "developer_accounts",
        sa.Column("id", sa.String(length=80), primary_key=True),
        sa.Column("team_name", sa.String(length=160), nullable=False),
        sa.Column("expires_at", sa.DateTime(timezone=True), nullable=False),
        sa.Column("status", sa.String(length=40), nullable=False),
        sa.Column("renewal_action_label", sa.String(length=40), nullable=False),
    )
    op.create_table(
        "builds",
        sa.Column("id", sa.String(length=80), primary_key=True),
        sa.Column("app_id", sa.String(length=80), sa.ForeignKey("apps.id", ondelete="CASCADE")),
        sa.Column("version", sa.String(length=60), nullable=False),
        sa.Column("build_number", sa.String(length=80), nullable=False),
        sa.Column("channel", sa.String(length=20), nullable=False),
        sa.Column("environment", sa.String(length=30), nullable=False),
        sa.Column("platform", sa.String(length=20), nullable=False),
        sa.Column("uploaded_at", sa.DateTime(timezone=True), nullable=False),
        sa.Column("note", sa.Text(), nullable=False),
        sa.Column("status", sa.String(length=30), nullable=False),
        sa.Column("min_os_version", sa.String(length=80)),
        sa.Column("expires_at", sa.DateTime(timezone=True)),
    )
    op.create_table(
        "artifacts",
        sa.Column("id", sa.String(length=80), primary_key=True),
        sa.Column("build_id", sa.String(length=80), sa.ForeignKey("builds.id", ondelete="CASCADE")),
        sa.Column("file_name", sa.String(length=240), nullable=False),
        sa.Column("content_type", sa.String(length=120), nullable=False),
        sa.Column("storage_backend", sa.String(length=20), nullable=False),
        sa.Column("storage_key", sa.String(length=400), nullable=False),
        sa.Column("download_url", sa.String(length=800), nullable=False),
        sa.Column("manifest_url", sa.String(length=800)),
        sa.Column("install_url", sa.String(length=1000), nullable=False),
        sa.Column("size_bytes", sa.Integer(), nullable=False),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
        sa.UniqueConstraint("build_id"),
    )
    op.create_table(
        "device_build_visibility",
        sa.Column(
            "device_id",
            sa.String(length=120),
            sa.ForeignKey("devices.id", ondelete="CASCADE"),
        ),
        sa.Column("build_id", sa.String(length=80), sa.ForeignKey("builds.id", ondelete="CASCADE")),
        sa.Column("reason", sa.String(length=120), nullable=False),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
        sa.PrimaryKeyConstraint("device_id", "build_id"),
    )
    op.create_table(
        "developer_account_apps",
        sa.Column(
            "developer_account_id",
            sa.String(length=80),
            sa.ForeignKey("developer_accounts.id", ondelete="CASCADE"),
        ),
        sa.Column("app_id", sa.String(length=80), sa.ForeignKey("apps.id", ondelete="CASCADE")),
        sa.PrimaryKeyConstraint("developer_account_id", "app_id"),
    )
    op.create_table(
        "notifications",
        sa.Column("id", sa.String(length=80), primary_key=True),
        sa.Column("type", sa.String(length=30), nullable=False),
        sa.Column("section", sa.String(length=80), nullable=False),
        sa.Column("icon_key", sa.String(length=40), nullable=False),
        sa.Column("title", sa.String(length=180), nullable=False),
        sa.Column("subtitle", sa.String(length=280), nullable=False),
        sa.Column("tag", sa.String(length=40), nullable=False),
        sa.Column("tag_color", sa.String(length=20), nullable=False),
        sa.Column("app_id", sa.String(length=80), sa.ForeignKey("apps.id", ondelete="SET NULL")),
        sa.Column(
            "build_id",
            sa.String(length=80),
            sa.ForeignKey("builds.id", ondelete="SET NULL"),
        ),
        sa.Column(
            "device_id",
            sa.String(length=120),
            sa.ForeignKey("devices.id", ondelete="SET NULL"),
        ),
        sa.Column(
            "developer_account_id",
            sa.String(length=80),
            sa.ForeignKey("developer_accounts.id", ondelete="SET NULL"),
        ),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
    )


def downgrade() -> None:
    op.drop_table("notifications")
    op.drop_table("developer_account_apps")
    op.drop_table("device_build_visibility")
    op.drop_table("artifacts")
    op.drop_table("builds")
    op.drop_table("developer_accounts")
    op.drop_table("devices")
    op.drop_table("apps")

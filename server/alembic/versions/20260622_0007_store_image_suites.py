from __future__ import annotations

import sqlalchemy as sa

from alembic import op

revision = "20260622_0007"
down_revision = "20260618_0006"
branch_labels = None
depends_on = None


def upgrade() -> None:
    op.create_table(
        "store_image_suites",
        sa.Column("id", sa.String(length=80), nullable=False),
        sa.Column("developer_account_id", sa.String(length=80), nullable=False),
        sa.Column("app_id", sa.String(length=80), nullable=False),
        sa.Column("platform", sa.String(length=20), nullable=False),
        sa.Column("suite_id", sa.String(length=80), nullable=False, server_default="default"),
        sa.Column("suite_name", sa.String(length=120), nullable=False, server_default="默认商店图"),
        sa.Column("source", sa.String(length=40), nullable=False, server_default="api"),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
        sa.Column("updated_at", sa.DateTime(timezone=True), nullable=False),
        sa.ForeignKeyConstraint(
            ["app_id"],
            ["apps.id"],
            ondelete="CASCADE",
        ),
        sa.ForeignKeyConstraint(
            ["developer_account_id"],
            ["developer_accounts.id"],
            ondelete="CASCADE",
        ),
        sa.PrimaryKeyConstraint("id"),
        sa.UniqueConstraint(
            "developer_account_id",
            "app_id",
            "platform",
            "suite_id",
            name="uq_store_image_suites_scope",
        ),
    )
    op.create_table(
        "store_image_suite_locales",
        sa.Column("id", sa.String(length=80), nullable=False),
        sa.Column("image_suite_id", sa.String(length=80), nullable=False),
        sa.Column("locale", sa.String(length=40), nullable=False),
        sa.Column("store_images_json", sa.JSON(), nullable=False),
        sa.Column("updated_at", sa.DateTime(timezone=True), nullable=False),
        sa.ForeignKeyConstraint(
            ["image_suite_id"],
            ["store_image_suites.id"],
            ondelete="CASCADE",
        ),
        sa.PrimaryKeyConstraint("id"),
        sa.UniqueConstraint(
            "image_suite_id",
            "locale",
            name="uq_store_image_suite_locales_scope",
        ),
    )


def downgrade() -> None:
    op.drop_table("store_image_suite_locales")
    op.drop_table("store_image_suites")

from __future__ import annotations

import sqlalchemy as sa

from alembic import op

revision = "20260624_0008"
down_revision = "20260622_0007"
branch_labels = None
depends_on = None


def upgrade() -> None:
    op.add_column(
        "store_sync_runs",
        sa.Column("sync_scopes_json", sa.JSON(), nullable=False, server_default=sa.text("'{}'")),
    )
    op.add_column(
        "store_sync_runs",
        sa.Column(
            "payload_snapshot_json",
            sa.JSON(),
            nullable=False,
            server_default=sa.text("'{}'"),
        ),
    )

    op.create_table(
        "store_marketing_pages",
        sa.Column("id", sa.String(length=80), nullable=False),
        sa.Column("developer_account_id", sa.String(length=80), nullable=False),
        sa.Column("app_id", sa.String(length=80), nullable=False),
        sa.Column("platform", sa.String(length=20), nullable=False),
        sa.Column("page_id", sa.String(length=80), nullable=False),
        sa.Column("page_name", sa.String(length=160), nullable=False),
        sa.Column(
            "page_type",
            sa.String(length=60),
            nullable=False,
            server_default="custom_product_page",
        ),
        sa.Column("status", sa.String(length=40), nullable=False, server_default="draft"),
        sa.Column("apple_page_id", sa.String(length=120), nullable=False, server_default=""),
        sa.Column("deep_link_url", sa.String(length=500), nullable=False, server_default=""),
        sa.Column("keywords", sa.String(length=240), nullable=False, server_default=""),
        sa.Column("store_images_json", sa.JSON(), nullable=False),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
        sa.Column("updated_at", sa.DateTime(timezone=True), nullable=False),
        sa.ForeignKeyConstraint(["app_id"], ["apps.id"], ondelete="CASCADE"),
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
            "page_id",
            name="uq_store_marketing_pages_scope",
        ),
    )
    op.create_table(
        "store_marketing_page_locales",
        sa.Column("id", sa.String(length=80), nullable=False),
        sa.Column("marketing_page_id", sa.String(length=80), nullable=False),
        sa.Column("locale", sa.String(length=40), nullable=False),
        sa.Column("promotional_text", sa.String(length=240), nullable=False, server_default=""),
        sa.Column("store_images_json", sa.JSON(), nullable=False),
        sa.Column("updated_at", sa.DateTime(timezone=True), nullable=False),
        sa.ForeignKeyConstraint(
            ["marketing_page_id"],
            ["store_marketing_pages.id"],
            ondelete="CASCADE",
        ),
        sa.PrimaryKeyConstraint("id"),
        sa.UniqueConstraint(
            "marketing_page_id",
            "locale",
            name="uq_store_marketing_page_locales_scope",
        ),
    )


def downgrade() -> None:
    op.drop_table("store_marketing_page_locales")
    op.drop_table("store_marketing_pages")
    op.drop_column("store_sync_runs", "payload_snapshot_json")
    op.drop_column("store_sync_runs", "sync_scopes_json")

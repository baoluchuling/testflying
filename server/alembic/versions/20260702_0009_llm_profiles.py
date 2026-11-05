from __future__ import annotations

import sqlalchemy as sa

from alembic import op

revision = "20260702_0009"
down_revision = "20260624_0008"
branch_labels = None
depends_on = None


def upgrade() -> None:
    op.create_table(
        "llm_profiles",
        sa.Column("id", sa.String(length=80), nullable=False),
        sa.Column("name", sa.String(length=120), nullable=False),
        sa.Column("protocol", sa.String(length=40), nullable=False),
        sa.Column("base_url", sa.String(length=500), nullable=False),
        sa.Column("model", sa.String(length=160), nullable=False),
        sa.Column("api_key", sa.Text(), nullable=False, server_default=""),
        sa.Column("auth_header", sa.String(length=80), nullable=False, server_default=""),
        sa.Column("status", sa.String(length=40), nullable=False, server_default="unchecked"),
        sa.Column("last_checked_at", sa.DateTime(timezone=True), nullable=True),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
        sa.Column("updated_at", sa.DateTime(timezone=True), nullable=False),
        sa.PrimaryKeyConstraint("id"),
    )
    op.create_table(
        "llm_feature_bindings",
        sa.Column("feature_key", sa.String(length=80), nullable=False),
        sa.Column("primary_profile_id", sa.String(length=80), nullable=True),
        sa.Column("fallback_profile_id", sa.String(length=80), nullable=True),
        sa.Column("updated_at", sa.DateTime(timezone=True), nullable=False),
        sa.ForeignKeyConstraint(
            ["fallback_profile_id"],
            ["llm_profiles.id"],
            ondelete="SET NULL",
        ),
        sa.ForeignKeyConstraint(
            ["primary_profile_id"],
            ["llm_profiles.id"],
            ondelete="SET NULL",
        ),
        sa.PrimaryKeyConstraint("feature_key"),
    )


def downgrade() -> None:
    op.drop_table("llm_feature_bindings")
    op.drop_table("llm_profiles")

from __future__ import annotations

import sqlalchemy as sa

from alembic import op

revision = "20260714_0014"
down_revision = "20260714_0013"
branch_labels = None
depends_on = None


def upgrade() -> None:
    with op.batch_alter_table("app_build_settings", recreate="always") as batch_op:
        batch_op.drop_column("repo_subpath")


def downgrade() -> None:
    with op.batch_alter_table("app_build_settings", recreate="always") as batch_op:
        batch_op.add_column(
            sa.Column(
                "repo_subpath",
                sa.String(length=240),
                nullable=False,
                server_default=sa.text("''"),
            )
        )

    with op.batch_alter_table("app_build_settings") as batch_op:
        batch_op.alter_column(
            "repo_subpath",
            existing_type=sa.String(length=240),
            server_default=None,
        )

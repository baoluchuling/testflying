from __future__ import annotations

import sqlalchemy as sa

from alembic import op

revision = "20260618_0005"
down_revision = "20260618_0004"
branch_labels = None
depends_on = None


def upgrade() -> None:
    with op.batch_alter_table("store_app_metadata_drafts") as batch_op:
        batch_op.add_column(
            sa.Column(
                "store_images_json",
                sa.JSON(),
                nullable=False,
                server_default=sa.text("'{}'"),
            )
        )


def downgrade() -> None:
    with op.batch_alter_table("store_app_metadata_drafts") as batch_op:
        batch_op.drop_column("store_images_json")

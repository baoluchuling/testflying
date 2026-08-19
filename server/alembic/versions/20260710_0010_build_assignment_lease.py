from __future__ import annotations

import sqlalchemy as sa

from alembic import op

revision = "20260710_0010"
down_revision = "20260709_0003"
branch_labels = None
depends_on = None


def upgrade() -> None:
    with op.batch_alter_table("builds") as batch_op:
        batch_op.add_column(
            sa.Column("assignment_lease_expires_at", sa.DateTime(timezone=True), nullable=True)
        )


def downgrade() -> None:
    with op.batch_alter_table("builds") as batch_op:
        batch_op.drop_column("assignment_lease_expires_at")

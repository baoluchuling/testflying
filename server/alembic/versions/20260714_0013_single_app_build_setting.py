from __future__ import annotations

import sqlalchemy as sa

from alembic import op

revision = "20260714_0013"
down_revision = "20260713_0012"
branch_labels = None
depends_on = None


def upgrade() -> None:
    bind = op.get_bind()
    legacy = sa.table(
        "app_build_settings",
        sa.column("id", sa.String),
        sa.column("app_id", sa.String),
        sa.column("environment", sa.String),
        sa.column("updated_at", sa.DateTime(timezone=True)),
    )
    environment_priority = sa.case(
        (legacy.c.environment == "development", 0),
        else_=1,
    )
    rows = bind.execute(
        sa.select(legacy.c.id, legacy.c.app_id).order_by(
            legacy.c.app_id.asc(),
            legacy.c.updated_at.desc(),
            environment_priority.asc(),
            legacy.c.id.asc(),
        )
    ).all()
    retained_app_ids: set[str] = set()
    dropped_setting_ids: list[str] = []
    for row in rows:
        if row.app_id in retained_app_ids:
            dropped_setting_ids.append(row.id)
        else:
            retained_app_ids.add(row.app_id)
    if dropped_setting_ids:
        bind.execute(sa.delete(legacy).where(legacy.c.id.in_(dropped_setting_ids)))

    with op.batch_alter_table("app_build_settings", recreate="always") as batch_op:
        batch_op.drop_constraint("uq_app_build_settings_scope", type_="unique")
        batch_op.drop_column("environment")
        batch_op.create_unique_constraint("uq_app_build_settings_app_id", ["app_id"])


def downgrade() -> None:
    with op.batch_alter_table("app_build_settings", recreate="always") as batch_op:
        batch_op.drop_constraint("uq_app_build_settings_app_id", type_="unique")
        batch_op.add_column(
            sa.Column(
                "environment",
                sa.String(length=30),
                nullable=False,
                server_default="development",
            )
        )
        batch_op.create_unique_constraint(
            "uq_app_build_settings_scope",
            ["app_id", "environment"],
        )

    with op.batch_alter_table("app_build_settings") as batch_op:
        batch_op.alter_column(
            "environment",
            existing_type=sa.String(length=30),
            server_default=None,
        )

from __future__ import annotations

import sqlalchemy as sa

from alembic import op

revision = "20260618_0006"
down_revision = "20260618_0005"
branch_labels = None
depends_on = None


def upgrade() -> None:
    bind = op.get_bind()
    if bind.dialect.name == "postgresql":
        op.execute(
            """
            DO $$
            DECLARE
                old_constraint_name text;
            BEGIN
                SELECT con.conname
                INTO old_constraint_name
                FROM pg_constraint con
                JOIN pg_class rel ON rel.oid = con.conrelid
                WHERE rel.relname = 'store_app_metadata_drafts'
                  AND con.contype = 'u'
                  AND (
                    SELECT array_agg(att.attname::text ORDER BY keys.ordinality)
                    FROM unnest(con.conkey) WITH ORDINALITY AS keys(attnum, ordinality)
                    JOIN pg_attribute att
                      ON att.attrelid = rel.oid
                     AND att.attnum = keys.attnum
                  ) = ARRAY[
                    'developer_account_id',
                    'app_id',
                    'platform',
                    'version',
                    'locale'
                  ]::text[];

                IF old_constraint_name IS NOT NULL THEN
                    EXECUTE format(
                        'ALTER TABLE store_app_metadata_drafts DROP CONSTRAINT %I',
                        old_constraint_name
                    );
                END IF;
            END $$;
            """
        )
        op.execute(
            """
            ALTER TABLE store_app_metadata_drafts
            ADD COLUMN IF NOT EXISTS content_set_id varchar(80) NOT NULL DEFAULT 'default'
            """
        )
        op.execute(
            """
            ALTER TABLE store_app_metadata_drafts
            ADD COLUMN IF NOT EXISTS content_set_name varchar(120) NOT NULL DEFAULT '默认上架内容'
            """
        )
        op.execute(
            """
            DO $$
            BEGIN
                IF NOT EXISTS (
                    SELECT 1
                    FROM pg_constraint con
                    JOIN pg_class rel ON rel.oid = con.conrelid
                    WHERE rel.relname = 'store_app_metadata_drafts'
                      AND con.conname = 'uq_store_app_metadata_drafts_scope'
                ) THEN
                    ALTER TABLE store_app_metadata_drafts
                    ADD CONSTRAINT uq_store_app_metadata_drafts_scope
                    UNIQUE (
                        developer_account_id,
                        app_id,
                        platform,
                        version,
                        locale,
                        content_set_id
                    );
                END IF;
            END $$;
            """
        )
        return

    with op.batch_alter_table("store_app_metadata_drafts") as batch_op:
        batch_op.drop_constraint("uq_store_app_metadata_drafts_scope", type_="unique")
        batch_op.add_column(
            sa.Column(
                "content_set_id",
                sa.String(length=80),
                nullable=False,
                server_default="default",
            )
        )
        batch_op.add_column(
            sa.Column(
                "content_set_name",
                sa.String(length=120),
                nullable=False,
                server_default="默认上架内容",
            )
        )
        batch_op.create_unique_constraint(
            "uq_store_app_metadata_drafts_scope",
            [
                "developer_account_id",
                "app_id",
                "platform",
                "version",
                "locale",
                "content_set_id",
            ],
        )


def downgrade() -> None:
    with op.batch_alter_table("store_app_metadata_drafts") as batch_op:
        batch_op.drop_constraint("uq_store_app_metadata_drafts_scope", type_="unique")
        batch_op.create_unique_constraint(
            "uq_store_app_metadata_drafts_scope",
            ["developer_account_id", "app_id", "platform", "version", "locale"],
        )
        batch_op.drop_column("content_set_name")
        batch_op.drop_column("content_set_id")

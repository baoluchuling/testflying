from __future__ import annotations

from alembic import op

revision = "20260618_0004"
down_revision = "20260618_0003"
branch_labels = None
depends_on = None


def upgrade() -> None:
    bind = op.get_bind()
    if bind.dialect.name == "sqlite":
        _rewire_sqlite_connector_references()
        _deduplicate_sqlite_connectors()
    else:
        _rewire_postgresql_connector_references()
        _deduplicate_postgresql_connectors()
    with op.batch_alter_table("store_connectors") as batch_op:
        batch_op.create_unique_constraint(
            "uq_store_connectors_developer_account_id",
            ["developer_account_id"],
        )


def downgrade() -> None:
    with op.batch_alter_table("store_connectors") as batch_op:
        batch_op.drop_constraint(
            "uq_store_connectors_developer_account_id",
            type_="unique",
        )


def _deduplicate_postgresql_connectors() -> None:
    op.execute(
        """
        DELETE FROM store_connectors AS doomed
        USING store_connectors AS kept
        WHERE doomed.developer_account_id = kept.developer_account_id
          AND (
            kept.created_at < doomed.created_at
            OR (kept.created_at = doomed.created_at AND kept.id < doomed.id)
          )
        """
    )


def _rewire_postgresql_connector_references() -> None:
    for table_name in ("store_preflight_checks", "store_sync_runs"):
        op.execute(
            f"""
            WITH ranked AS (
              SELECT
                id,
                FIRST_VALUE(id) OVER (
                  PARTITION BY developer_account_id
                  ORDER BY created_at ASC, id ASC
                ) AS kept_id
              FROM store_connectors
            )
            UPDATE {table_name}
            SET connector_id = ranked.kept_id
            FROM ranked
            WHERE {table_name}.connector_id = ranked.id
              AND ranked.id <> ranked.kept_id
            """
        )


def _rewire_sqlite_connector_references() -> None:
    for table_name in ("store_preflight_checks", "store_sync_runs"):
        op.execute(
            f"""
            UPDATE {table_name}
            SET connector_id = (
              SELECT kept.id
              FROM store_connectors AS doomed
              JOIN store_connectors AS kept
                ON kept.developer_account_id = doomed.developer_account_id
              WHERE doomed.id = {table_name}.connector_id
              ORDER BY kept.created_at ASC, kept.id ASC
              LIMIT 1
            )
            WHERE connector_id IN (
              SELECT doomed.id
              FROM store_connectors AS doomed
              WHERE EXISTS (
                SELECT 1
                FROM store_connectors AS preferred
                WHERE preferred.developer_account_id = doomed.developer_account_id
                  AND (
                    preferred.created_at < doomed.created_at
                    OR (
                      preferred.created_at = doomed.created_at
                      AND preferred.id < doomed.id
                    )
                  )
              )
            )
            """
        )


def _deduplicate_sqlite_connectors() -> None:
    op.execute(
        """
        DELETE FROM store_connectors
        WHERE id NOT IN (
          SELECT id
          FROM (
            SELECT id
            FROM store_connectors AS candidate
            WHERE NOT EXISTS (
              SELECT 1
              FROM store_connectors AS preferred
              WHERE preferred.developer_account_id = candidate.developer_account_id
                AND (
                  preferred.created_at < candidate.created_at
                  OR (
                    preferred.created_at = candidate.created_at
                    AND preferred.id < candidate.id
                  )
                )
            )
          )
        )
        """
    )

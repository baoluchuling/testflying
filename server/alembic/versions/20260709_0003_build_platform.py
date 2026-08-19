from __future__ import annotations

import sqlalchemy as sa

from alembic import op

revision = "20260709_0003"
down_revision = "20260702_0009"
branch_labels = None
depends_on = None


def upgrade() -> None:
    with op.batch_alter_table("builds") as batch_op:
        batch_op.alter_column("version", existing_type=sa.String(length=60), nullable=True)
        batch_op.alter_column("build_number", existing_type=sa.String(length=80), nullable=True)
        batch_op.add_column(
            sa.Column(
                "requested_environment",
                sa.String(length=30),
                nullable=False,
                server_default="development",
            )
        )
        batch_op.add_column(
            sa.Column("source", sa.String(length=30), nullable=False, server_default="upload")
        )
        batch_op.add_column(
            sa.Column(
                "lifecycle_status",
                sa.String(length=40),
                nullable=False,
                server_default="succeeded",
            )
        )
        batch_op.add_column(sa.Column("git_url", sa.String(length=800), nullable=True))
        batch_op.add_column(sa.Column("git_ref", sa.String(length=240), nullable=True))
        batch_op.add_column(sa.Column("commit_sha", sa.String(length=80), nullable=True))
        batch_op.add_column(sa.Column("runner_id", sa.String(length=80), nullable=True))
        batch_op.add_column(sa.Column("runner_labels_json", sa.JSON(), nullable=True))
        batch_op.add_column(
            sa.Column("attempt_count", sa.Integer(), nullable=False, server_default="0")
        )
        batch_op.add_column(
            sa.Column("failure_classification", sa.String(length=80), nullable=True)
        )
        batch_op.add_column(sa.Column("failure_summary", sa.Text(), nullable=True))
        batch_op.add_column(sa.Column("human_action", sa.Text(), nullable=True))
        batch_op.add_column(sa.Column("started_at", sa.DateTime(timezone=True), nullable=True))
        batch_op.add_column(sa.Column("finished_at", sa.DateTime(timezone=True), nullable=True))
        batch_op.add_column(sa.Column("duration_seconds", sa.Integer(), nullable=True))

    _upgrade_artifacts()
    _upgrade_app_build_settings()
    _upgrade_build_runners()


def downgrade() -> None:
    _downgrade_build_runners()
    _downgrade_app_build_settings()
    _downgrade_artifacts()

    with op.batch_alter_table("builds") as batch_op:
        for column in [
            "duration_seconds",
            "finished_at",
            "started_at",
            "human_action",
            "failure_summary",
            "failure_classification",
            "attempt_count",
            "runner_labels_json",
            "runner_id",
            "commit_sha",
            "git_ref",
            "git_url",
            "lifecycle_status",
            "source",
            "requested_environment",
        ]:
            batch_op.drop_column(column)
        batch_op.alter_column("build_number", existing_type=sa.String(length=80), nullable=False)
        batch_op.alter_column("version", existing_type=sa.String(length=60), nullable=False)


def _upgrade_artifacts() -> None:
    bind = op.get_bind()
    if bind.dialect.name == "postgresql":
        _drop_artifacts_build_id_unique_postgresql()

    with op.batch_alter_table("artifacts", recreate="always") as batch_op:
        batch_op.add_column(
            sa.Column(
                "artifact_type",
                sa.String(length=30),
                nullable=False,
                server_default="package",
            )
        )
        batch_op.add_column(sa.Column("metadata_json", sa.JSON(), nullable=True))


def _upgrade_app_build_settings() -> None:
    op.create_table(
        "app_build_settings",
        sa.Column("id", sa.String(length=80), primary_key=True),
        sa.Column(
            "app_id",
            sa.String(length=80),
            sa.ForeignKey("apps.id", ondelete="CASCADE"),
            nullable=False,
        ),
        sa.Column("environment", sa.String(length=30), nullable=False),
        sa.Column("git_url", sa.String(length=800), nullable=False),
        sa.Column("repo_subpath", sa.String(length=240), nullable=False),
        sa.Column("runner_labels_json", sa.JSON(), nullable=False),
        sa.Column("credential_refs_json", sa.JSON(), nullable=False),
        sa.Column("artifact_type", sa.String(length=30), nullable=False),
        sa.Column("optional_defaults_json", sa.JSON(), nullable=False),
        sa.Column("updated_at", sa.DateTime(timezone=True), nullable=False),
        sa.UniqueConstraint("app_id", "environment", name="uq_app_build_settings_scope"),
    )


def _downgrade_artifacts() -> None:
    with op.batch_alter_table("artifacts", recreate="always") as batch_op:
        batch_op.drop_column("metadata_json")
        batch_op.drop_column("artifact_type")
        batch_op.create_unique_constraint("uq_artifacts_build_id", ["build_id"])


def _downgrade_app_build_settings() -> None:
    op.drop_table("app_build_settings")


def _upgrade_build_runners() -> None:
    op.create_table(
        "build_runners",
        sa.Column("id", sa.String(length=80), primary_key=True),
        sa.Column("name", sa.String(length=120), nullable=False),
        sa.Column("token_hash", sa.String(length=240), nullable=False),
        sa.Column("labels_json", sa.JSON(), nullable=False),
        sa.Column("capabilities_json", sa.JSON(), nullable=False),
        sa.Column("status", sa.String(length=40), nullable=False, server_default="offline"),
        sa.Column("version", sa.String(length=80), nullable=False, server_default=""),
        sa.Column("package_agent_version", sa.String(length=80), nullable=False, server_default=""),
        sa.Column("last_seen_at", sa.DateTime(timezone=True), nullable=True),
        sa.Column(
            "current_build_id",
            sa.String(length=80),
            sa.ForeignKey("builds.id", ondelete="SET NULL"),
            nullable=True,
        ),
    )
    op.create_table(
        "build_events",
        sa.Column("id", sa.String(length=80), primary_key=True),
        sa.Column(
            "build_id",
            sa.String(length=80),
            sa.ForeignKey("builds.id", ondelete="CASCADE"),
            nullable=False,
        ),
        sa.Column(
            "runner_id",
            sa.String(length=80),
            sa.ForeignKey("build_runners.id", ondelete="SET NULL"),
            nullable=True,
        ),
        sa.Column("type", sa.String(length=80), nullable=False),
        sa.Column("message", sa.String(length=500), nullable=False),
        sa.Column("payload_json", sa.JSON(), nullable=False),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
    )


def _downgrade_build_runners() -> None:
    op.drop_table("build_events")
    op.drop_table("build_runners")


def _drop_artifacts_build_id_unique_postgresql() -> None:
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
            WHERE rel.relname = 'artifacts'
              AND con.contype = 'u'
              AND (
                SELECT array_agg(att.attname::text ORDER BY keys.ordinality)
                FROM unnest(con.conkey) WITH ORDINALITY AS keys(attnum, ordinality)
                JOIN pg_attribute att
                  ON att.attrelid = rel.oid
                 AND att.attnum = keys.attnum
              ) = ARRAY['build_id']::text[];

            IF old_constraint_name IS NOT NULL THEN
                EXECUTE format('ALTER TABLE artifacts DROP CONSTRAINT %I', old_constraint_name);
            END IF;
        END
        $$;
        """
    )

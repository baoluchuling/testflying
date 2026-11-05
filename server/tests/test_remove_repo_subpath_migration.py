from __future__ import annotations

from pathlib import Path

import pytest
from alembic.config import Config
from sqlalchemy import create_engine, inspect, text

from alembic import command


def _alembic_config(database_url: str) -> Config:
    config = Config("alembic.ini")
    config.set_main_option("sqlalchemy.url", database_url)
    return config


def test_remove_repo_subpath_migration_round_trip(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    database_url = f"sqlite:///{tmp_path / 'migration.db'}"
    monkeypatch.setenv("TESTFLYING_DATABASE_URL", database_url)
    config = _alembic_config(database_url)
    command.upgrade(config, "20260714_0013")

    engine = create_engine(database_url)
    with engine.begin() as connection:
        connection.execute(
            text(
                "INSERT INTO apps "
                "(id, name, bundle_identifier, platform, default_channel, icon_key, "
                "icon_color, added_at) "
                "VALUES ('app-1', 'Demo', 'com.example.demo', 'ios', 'dev', "
                "'app', '#53606E', '2026-07-14 00:00:00')"
            )
        )
        connection.execute(
            text(
                "INSERT INTO app_build_settings "
                "(id, app_id, git_url, repo_subpath, runner_labels_json, "
                "credential_refs_json, artifact_type, optional_defaults_json, updated_at) "
                "VALUES ('setting-1', 'app-1', 'git://demo', 'legacy/ios', '[]', '{}', "
                "'ipa', '{}', '2026-07-14 01:00:00')"
            )
        )

    command.upgrade(config, "head")
    assert "repo_subpath" not in {
        column["name"] for column in inspect(engine).get_columns("app_build_settings")
    }

    command.downgrade(config, "20260714_0013")
    assert "repo_subpath" in {
        column["name"] for column in inspect(engine).get_columns("app_build_settings")
    }
    with engine.connect() as connection:
        repo_subpath = connection.execute(
            text("SELECT repo_subpath FROM app_build_settings WHERE id = 'setting-1'")
        ).scalar_one()
    assert repo_subpath == ""
    engine.dispose()

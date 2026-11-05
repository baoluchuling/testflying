from __future__ import annotations

from collections.abc import Iterable
from pathlib import Path

import pytest
from alembic.config import Config
from sqlalchemy import create_engine, inspect, text
from sqlalchemy.exc import IntegrityError

from alembic import command


def _alembic_config(database_url: str) -> Config:
    config = Config("alembic.ini")
    config.set_main_option("sqlalchemy.url", database_url)
    return config


def _prepare_legacy_database(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    *,
    rows: Iterable[dict[str, str]],
) -> tuple[str, Config]:
    database_url = f"sqlite:///{tmp_path / 'migration.db'}"
    monkeypatch.setenv("TESTFLYING_DATABASE_URL", database_url)
    config = _alembic_config(database_url)
    command.upgrade(config, "20260713_0012")
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
        for row in rows:
            connection.execute(
                text(
                    "INSERT INTO app_build_settings "
                    "(id, app_id, environment, git_url, repo_subpath, runner_labels_json, "
                    "credential_refs_json, artifact_type, optional_defaults_json, updated_at) "
                    "VALUES (:id, 'app-1', :environment, :git_url, '', '[]', '{}', "
                    "'ipa', '{}', :updated_at)"
                ),
                row,
            )
    engine.dispose()
    return database_url, config


def test_single_setting_migration_keeps_latest_row(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    database_url, config = _prepare_legacy_database(
        tmp_path,
        monkeypatch,
        rows=[
            {
                "id": "setting-dev",
                "environment": "development",
                "git_url": "git://old",
                "updated_at": "2026-07-14 01:00:00",
            },
            {
                "id": "setting-prod",
                "environment": "production",
                "git_url": "git://latest",
                "updated_at": "2026-07-14 02:00:00",
            },
        ],
    )

    command.upgrade(config, "20260714_0013")

    engine = create_engine(database_url)
    with engine.connect() as connection:
        rows = connection.execute(
            text("SELECT id, app_id, git_url FROM app_build_settings")
        ).mappings().all()
    assert rows == [
        {"id": "setting-prod", "app_id": "app-1", "git_url": "git://latest"}
    ]
    assert "environment" not in {
        column["name"] for column in inspect(engine).get_columns("app_build_settings")
    }
    engine.dispose()


def test_single_setting_migration_prefers_development_when_timestamps_match(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    database_url, config = _prepare_legacy_database(
        tmp_path,
        monkeypatch,
        rows=[
            {
                "id": "setting-prod",
                "environment": "production",
                "git_url": "git://production",
                "updated_at": "2026-07-14 01:00:00",
            },
            {
                "id": "setting-dev",
                "environment": "development",
                "git_url": "git://development",
                "updated_at": "2026-07-14 01:00:00",
            },
        ],
    )

    command.upgrade(config, "20260714_0013")

    engine = create_engine(database_url)
    with engine.connect() as connection:
        row = connection.execute(
            text("SELECT id, git_url FROM app_build_settings")
        ).mappings().one()
    assert row == {"id": "setting-dev", "git_url": "git://development"}
    engine.dispose()


def test_single_setting_migration_enforces_one_row_per_app(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    database_url, config = _prepare_legacy_database(
        tmp_path,
        monkeypatch,
        rows=[
            {
                "id": "setting-dev",
                "environment": "development",
                "git_url": "git://development",
                "updated_at": "2026-07-14 01:00:00",
            }
        ],
    )
    command.upgrade(config, "20260714_0013")

    engine = create_engine(database_url)
    with pytest.raises(IntegrityError), engine.begin() as connection:
        connection.execute(
            text(
                "INSERT INTO app_build_settings "
                "(id, app_id, git_url, repo_subpath, runner_labels_json, "
                "credential_refs_json, artifact_type, optional_defaults_json, updated_at) "
                "VALUES ('setting-second', 'app-1', 'git://second', '', '[]', '{}', "
                "'ipa', '{}', '2026-07-14 02:00:00')"
            )
        )
    engine.dispose()

from __future__ import annotations

import pytest
from sqlalchemy import inspect
from sqlalchemy.exc import IntegrityError
from sqlalchemy.orm import Session

from testflying_api.database import create_engine_for_url
from testflying_api.schema import App, AppBuildSetting, Base


def test_schema_contains_system_settings() -> None:
    engine = create_engine_for_url("sqlite:///:memory:")
    Base.metadata.create_all(engine)

    assert "system_settings" in inspect(engine).get_table_names()


def test_app_has_one_scalar_build_setting() -> None:
    engine = create_engine_for_url("sqlite:///:memory:")
    Base.metadata.create_all(engine)

    relationship = inspect(App).relationships["build_setting"]
    assert relationship.uselist is False
    columns = {
        column["name"] for column in inspect(engine).get_columns("app_build_settings")
    }
    assert "repo_subpath" not in columns

    with Session(engine) as session:
        app = App(
            id="app-build-setting",
            name="Demo",
            bundle_identifier="com.example.demo",
            platform="ios",
        )
        first = AppBuildSetting(
            id="setting-first",
            app_id=app.id,
            git_url="git@example.com:mobile/demo.git",
            runner_labels_json=[],
            credential_refs_json={},
            artifact_type="ipa",
            optional_defaults_json={},
        )
        app.build_setting = first
        session.add(app)
        session.commit()

        assert app.build_setting is first

        session.add(
            AppBuildSetting(
                id="setting-second",
                app_id=app.id,
                git_url="git@example.com:mobile/other.git",
                runner_labels_json=[],
                credential_refs_json={},
                artifact_type="ipa",
                optional_defaults_json={},
            )
        )
        with pytest.raises(IntegrityError):
            session.commit()

    engine.dispose()


def test_catalog_schema_contains_no_user_state_tables() -> None:
    engine = create_engine_for_url("sqlite:///:memory:")
    Base.metadata.create_all(engine)

    table_names = set(inspect(engine).get_table_names())

    assert {
        "apps",
        "builds",
        "artifacts",
        "devices",
        "developer_accounts",
        "developer_account_apps",
        "notifications",
        "webhook_deliveries",
        "device_build_visibility",
        "store_connectors",
        "store_release_note_drafts",
        "store_app_metadata_drafts",
        "store_image_suites",
        "store_image_suite_locales",
        "store_marketing_pages",
        "store_marketing_page_locales",
        "store_preflight_checks",
        "store_sync_runs",
        "store_reviews",
        "store_review_fetch_runs",
        "store_review_analysis_runs",
        "llm_profiles",
        "llm_feature_bindings",
        "audit_logs",
    }.issubset(table_names)
    assert "install_tasks" not in table_names
    assert "sort_orders" not in table_names
    assert "notification_reads" not in table_names


def test_webhook_delivery_event_key_is_unique() -> None:
    engine = create_engine_for_url("sqlite:///:memory:")
    Base.metadata.create_all(engine)

    constraints = inspect(engine).get_unique_constraints("webhook_deliveries")
    columns = {column["name"] for column in inspect(engine).get_columns("webhook_deliveries")}

    assert {
        "channel",
        "event_key",
        "status",
        "payload_json",
        "attempt_count",
        "next_attempt_at",
        "last_error",
        "created_at",
        "delivered_at",
    }.issubset(columns)
    assert any(constraint["column_names"] == ["event_key"] for constraint in constraints)


def test_store_connector_is_unique_per_account() -> None:
    engine = create_engine_for_url("sqlite:///:memory:")
    Base.metadata.create_all(engine)

    constraints = inspect(engine).get_unique_constraints("store_connectors")

    assert any(
        constraint["column_names"] == ["developer_account_id"] for constraint in constraints
    )


def test_store_metadata_drafts_include_image_settings() -> None:
    engine = create_engine_for_url("sqlite:///:memory:")
    Base.metadata.create_all(engine)

    columns = {
        column["name"]
        for column in inspect(engine).get_columns("store_app_metadata_drafts")
    }

    assert "store_images_json" in columns


def test_store_image_suites_are_unique_per_app_scope() -> None:
    engine = create_engine_for_url("sqlite:///:memory:")
    Base.metadata.create_all(engine)

    constraints = inspect(engine).get_unique_constraints("store_image_suites")

    assert any(
        constraint["column_names"] == [
            "developer_account_id",
            "app_id",
            "platform",
            "suite_id",
        ]
        for constraint in constraints
    )


def test_store_image_suite_locales_store_images_by_locale() -> None:
    engine = create_engine_for_url("sqlite:///:memory:")
    Base.metadata.create_all(engine)

    columns = {
        column["name"]
        for column in inspect(engine).get_columns("store_image_suite_locales")
    }
    constraints = inspect(engine).get_unique_constraints("store_image_suite_locales")

    assert "store_images_json" in columns
    assert any(
        constraint["column_names"] == ["image_suite_id", "locale"] for constraint in constraints
    )


def test_store_sync_runs_store_scope_and_payload_snapshots() -> None:
    engine = create_engine_for_url("sqlite:///:memory:")
    Base.metadata.create_all(engine)

    columns = {column["name"] for column in inspect(engine).get_columns("store_sync_runs")}

    assert "sync_scopes_json" in columns
    assert "payload_snapshot_json" in columns


def test_store_reviews_are_unique_per_store_review_scope() -> None:
    engine = create_engine_for_url("sqlite:///:memory:")
    Base.metadata.create_all(engine)

    constraints = inspect(engine).get_unique_constraints("store_reviews")
    fetch_columns = {
        column["name"] for column in inspect(engine).get_columns("store_review_fetch_runs")
    }
    analysis_columns = {
        column["name"] for column in inspect(engine).get_columns("store_review_analysis_runs")
    }

    assert any(
        constraint["column_names"] == [
            "developer_account_id",
            "app_id",
            "platform",
            "store_review_id",
        ]
        for constraint in constraints
    )
    assert {"inserted_count", "duplicate_count", "stopped_reason"}.issubset(fetch_columns)
    assert {"review_count", "issue_count", "analysis_json"}.issubset(analysis_columns)


def test_store_marketing_pages_are_unique_per_app_scope() -> None:
    engine = create_engine_for_url("sqlite:///:memory:")
    Base.metadata.create_all(engine)

    constraints = inspect(engine).get_unique_constraints("store_marketing_pages")

    assert any(
        constraint["column_names"] == [
            "developer_account_id",
            "app_id",
            "platform",
            "page_id",
        ]
        for constraint in constraints
    )


def test_store_marketing_page_locales_store_page_content_by_locale() -> None:
    engine = create_engine_for_url("sqlite:///:memory:")
    Base.metadata.create_all(engine)

    columns = {
        column["name"] for column in inspect(engine).get_columns("store_marketing_page_locales")
    }
    constraints = inspect(engine).get_unique_constraints("store_marketing_page_locales")

    assert {"promotional_text", "store_images_json"}.issubset(columns)
    assert any(
        constraint["column_names"] == ["marketing_page_id", "locale"]
        for constraint in constraints
    )

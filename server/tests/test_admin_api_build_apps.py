from __future__ import annotations

from base64 import b64encode
from datetime import UTC, datetime, timedelta

from fastapi.testclient import TestClient
from sqlalchemy.orm import Session

from testflying_api.schema import App, AppBuildSetting, Build, BuildRunner


def _admin_headers(password: str = "dev-token") -> dict[str, str]:
    token = b64encode(f"admin:{password}".encode()).decode()
    return {"Authorization": f"Basic {token}"}


def _app(session: Session, app_id: str, platform: str, *, name: str) -> App:
    app = App(
        id=app_id,
        name=name,
        bundle_identifier=f"com.example.{app_id}",
        platform=platform,
        default_channel="dev",
    )
    session.add(app)
    session.commit()
    return app


def _setting(
    session: Session,
    app_id: str,
    labels: list[str],
) -> AppBuildSetting:
    setting = AppBuildSetting(
        id=f"setting-{app_id}",
        app_id=app_id,
        git_url="git@example.com:mobile/demo.git",
        runner_labels_json=labels,
        credential_refs_json={"git": "git-main"},
        artifact_type="ipa",
        optional_defaults_json={},
    )
    session.add(setting)
    session.commit()
    return setting


def _runner(
    session: Session,
    runner_id: str,
    status: str,
    labels: list[str],
    platforms: list[str],
) -> BuildRunner:
    runner = BuildRunner(
        id=runner_id,
        name=runner_id,
        token_hash="hash",
        labels_json=labels,
        capabilities_json={"platforms": platforms},
        status=status,
        version="0.1.0",
        package_agent_version="0.1.0",
    )
    session.add(runner)
    session.commit()
    return runner


def test_build_apps_returns_only_configured_apps_with_runner_match(
    client: TestClient,
    db_session: Session,
) -> None:
    configured = _app(db_session, "app-configured", "android", name="Lookrva")
    unconfigured = _app(db_session, "app-unconfigured", "android", name="NovelGo")
    _setting(db_session, configured.id, ["ios-release"])
    _runner(db_session, "runner-online", "online", ["ios-release"], ["ios"])
    _runner(db_session, "runner-offline", "offline", ["ios-release"], ["ios"])

    response = client.get("/admin/api/builds/apps", headers=_admin_headers())

    assert response.status_code == 200
    assert response.json()["total"] == 1
    assert response.json()["availableApps"] == [
        {
            "id": unconfigured.id,
            "name": "NovelGo",
            "bundleIdentifier": "com.example.app-unconfigured",
            "platform": "android",
            "iconColor": "#53606E",
            "iconText": "NO",
        }
    ]
    item = response.json()["apps"][0]
    assert item["app"]["id"] == "app-configured"
    assert item["setting"]["gitUrl"] == "git@example.com:mobile/demo.git"
    assert item["matchingRunnerCount"] == 1
    assert item["hasOnlineRunner"] is True
    assert "environments" not in item


def test_build_apps_returns_latest_build_and_deterministic_order(
    client: TestClient,
    db_session: Session,
) -> None:
    second = _app(db_session, "app-z", "android", name="Zulu")
    first = _app(db_session, "app-a", "ios", name="Alpha")
    _setting(db_session, second.id, ["android-release"])
    _setting(db_session, first.id, [])
    older = datetime(2026, 7, 13, 1, 0, tzinfo=UTC)
    db_session.add_all(
        [
            Build(
                id="build-old",
                app_id=first.id,
                channel="dev",
                environment="development",
                requested_environment="development",
                platform="ios",
                source="agent",
                lifecycle_status="failed",
                uploaded_at=older,
            ),
            Build(
                id="build-new",
                app_id=first.id,
                channel="dev",
                environment="development",
                requested_environment="development",
                platform="ios",
                source="agent",
                lifecycle_status="queued",
                uploaded_at=older + timedelta(hours=1),
            ),
        ]
    )
    db_session.commit()

    response = client.get("/admin/api/builds/apps", headers=_admin_headers())

    assert response.status_code == 200
    assert [item["app"]["name"] for item in response.json()["apps"]] == [
        "Alpha",
        "Zulu",
    ]
    assert response.json()["apps"][0]["latestBuild"]["id"] == "build-new"

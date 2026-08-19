from __future__ import annotations

from fastapi.testclient import TestClient
from sqlalchemy.orm import Session

from testflying_api.build_platform import hash_runner_token
from testflying_api.schema import App, Artifact, Build, BuildRunner


def _runner_headers(token: str = "runner-token") -> dict[str, str]:
    return {"Authorization": f"Bearer {token}"}


def _create_assigned_build(db_session: Session) -> Build:
    app = App(
        id="app-ios-demo",
        name="Demo",
        bundle_identifier="com.example.demo",
        platform="ios",
        default_channel="dev",
    )
    runner = BuildRunner(
        id="runner-mac-1",
        name="Mac mini 1",
        token_hash=hash_runner_token("runner-token", token_pepper="dev-token"),
        labels_json=["ios-release"],
        capabilities_json={"platforms": ["ios"], "llmAdapters": ["codex"], "capacity": 1},
        status="busy",
        version="0.1.0",
        package_agent_version="0.1.0",
        current_build_id="build-agent-1",
    )
    build = Build(
        id="build-agent-1",
        app_id=app.id,
        channel="dev",
        environment="development",
        requested_environment="development",
        platform="ios",
        source="agent",
        lifecycle_status="assigned",
        runner_id="runner-mac-1",
        status="pending",
    )
    db_session.add_all([app, runner, build])
    db_session.commit()
    return build


def test_complete_runner_build_requires_package_symbols_report_and_log(
    client: TestClient,
    db_session: Session,
) -> None:
    build = _create_assigned_build(db_session)

    for artifact_type in ["package", "symbols", "report"]:
        db_session.add(
            Artifact(
                id=f"artifact-{artifact_type}",
                build_id=build.id,
                artifact_type=artifact_type,
                file_name=f"{artifact_type}.bin",
                content_type="application/octet-stream",
                storage_backend="local",
                storage_key=f"{build.id}/{artifact_type}.bin",
                download_url=f"https://dist.example.test/{artifact_type}.bin",
                install_url="",
                size_bytes=10,
            )
        )
    db_session.commit()

    response = client.post(
        f"/admin/api/build-runners/builds/{build.id}/complete",
        headers=_runner_headers(),
        json={
            "runnerId": "runner-mac-1",
            "status": "succeeded",
            "version": "1.0",
            "buildNumber": "45",
            "note": "done",
        },
    )

    assert response.status_code == 422
    assert response.json()["error"]["code"] == "missing_required_artifacts"

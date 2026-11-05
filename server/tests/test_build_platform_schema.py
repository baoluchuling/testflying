from __future__ import annotations

from sqlalchemy.orm import Session

from testflying_api.schema import App, Artifact, Build


def test_build_accepts_nullable_version_for_agent_run(db_session: Session) -> None:
    app = App(
        id="app-ios-com-example-demo",
        name="Demo",
        bundle_identifier="com.example.demo",
        platform="ios",
        default_channel="dev",
    )
    build = Build(
        id="build-agent-1",
        app_id=app.id,
        version=None,
        build_number=None,
        channel="dev",
        environment="development",
        requested_environment="development",
        platform="ios",
        source="agent",
        lifecycle_status="queued",
    )

    db_session.add(app)
    db_session.add(build)
    db_session.commit()

    persisted = db_session.get(Build, "build-agent-1")
    assert persisted is not None
    assert persisted.version is None
    assert persisted.build_number is None
    assert persisted.lifecycle_status == "queued"


def test_build_can_have_package_symbols_report_and_log_artifacts(db_session: Session) -> None:
    app = App(
        id="app-android-com-example-demo",
        name="Demo",
        bundle_identifier="com.example.demo",
        platform="android",
        default_channel="dev",
    )
    build = Build(
        id="build-agent-2",
        app_id=app.id,
        version="1.0",
        build_number="10",
        channel="dev",
        environment="development",
        requested_environment="development",
        platform="android",
        source="agent",
        lifecycle_status="succeeded",
    )
    db_session.add_all([app, build])
    for artifact_type in ["package", "symbols", "report", "log"]:
        db_session.add(
            Artifact(
                id=f"artifact-{artifact_type}",
                build_id=build.id,
                artifact_type=artifact_type,
                file_name=f"{artifact_type}.zip",
                content_type="application/octet-stream",
                storage_backend="local",
                storage_key=f"{build.id}/{artifact_type}.zip",
                download_url=f"https://dist.example.test/{artifact_type}.zip",
                install_url="",
                size_bytes=10,
            )
        )
    db_session.commit()

    persisted = db_session.get(Build, build.id)
    assert persisted is not None
    assert {artifact.artifact_type for artifact in persisted.artifacts} == {
        "package",
        "symbols",
        "report",
        "log",
    }
    package_artifact = persisted.package_artifact()
    assert package_artifact is not None
    assert package_artifact.artifact_type == "package"

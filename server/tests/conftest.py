from __future__ import annotations

from collections.abc import Generator
from io import BytesIO
from pathlib import Path

import pytest
from fastapi import FastAPI
from fastapi.testclient import TestClient
from sqlalchemy.orm import Session, sessionmaker

from testflying_api.app import create_app
from testflying_api.config import Settings
from testflying_api.database import create_engine_for_url, create_session_factory
from testflying_api.schema import Base
from testflying_api.storage import LocalArtifactStorage


@pytest.fixture
def session_factory() -> Generator[sessionmaker[Session]]:
    engine = create_engine_for_url("sqlite:///:memory:")
    Base.metadata.create_all(engine)
    factory = create_session_factory(engine)
    try:
        yield factory
    finally:
        Base.metadata.drop_all(engine)
        engine.dispose()


@pytest.fixture
def db_session(session_factory: sessionmaker[Session]) -> Generator[Session]:
    session = session_factory()
    try:
        yield session
    finally:
        session.close()


@pytest.fixture
def test_settings(tmp_path: Path) -> Settings:
    return Settings(
        database_url="sqlite:///:memory:",
        public_base_url="https://dist.example.test",
        storage_backend="local",
        storage_root=tmp_path / "artifacts",
        static_token="dev-token",
        s3_endpoint_url=None,
        s3_public_base_url=None,
        s3_bucket="testflying",
        s3_access_key_id=None,
        s3_secret_access_key=None,
        cors_allowed_origins=(
            "http://localhost:8080",
            "http://127.0.0.1:8080",
        ),
        admin_username="admin",
        connector_base_url_template=None,
        translation_provider="disabled",
        translation_openai_api_key=None,
        translation_openai_base_url="https://api.openai.com/v1",
        translation_openai_model="gpt-4o-mini",
        review_analysis_provider="disabled",
        review_analysis_openai_api_key=None,
        review_analysis_openai_base_url="https://api.openai.com/v1",
        review_analysis_openai_model="gpt-4o-mini",
        dingtalk_webhook_url=None,
        dingtalk_secret=None,
        dingtalk_timeout_seconds=5.0,
        dingtalk_dispatch_interval_seconds=10.0,
        runner_release_root=tmp_path / "runner-releases",
    )


@pytest.fixture
def app(
    session_factory: sessionmaker[Session],
    test_settings: Settings,
) -> FastAPI:
    storage = LocalArtifactStorage(
        root=test_settings.storage_root,
        public_base_url=test_settings.public_base_url,
    )
    return create_app(test_settings, session_factory=session_factory, artifact_storage=storage)


@pytest.fixture
def client(app: FastAPI) -> TestClient:
    return TestClient(app)


class FakeS3Client:
    def __init__(self) -> None:
        self.put_objects: list[dict[str, object]] = []
        self.deleted_objects: list[dict[str, object]] = []
        self.objects: dict[tuple[str, str], dict[str, object]] = {}

    def put_object(self, **kwargs: object) -> None:
        self.put_objects.append(kwargs)
        self.objects[(str(kwargs["Bucket"]), str(kwargs["Key"]))] = {
            "Body": kwargs["Body"],
            "ContentType": kwargs.get("ContentType"),
        }

    def get_object(self, **kwargs: object) -> dict[str, object]:
        item = self.objects[(str(kwargs["Bucket"]), str(kwargs["Key"]))]
        return {
            "Body": BytesIO(bytes(item["Body"])),
            "ContentType": item.get("ContentType"),
        }

    def delete_object(self, **kwargs: object) -> None:
        self.deleted_objects.append(kwargs)
        self.objects.pop((str(kwargs["Bucket"]), str(kwargs["Key"])), None)


@pytest.fixture
def fake_s3_client() -> FakeS3Client:
    return FakeS3Client()

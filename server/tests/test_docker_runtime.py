from __future__ import annotations

import tomllib
from pathlib import Path


def test_server_image_runs_migrations_before_api_start() -> None:
    dockerfile = Path("Dockerfile").read_text()
    start_script = Path("docker/start-api.sh").read_text()

    assert "COPY docker/start-api.sh /usr/local/bin/testflying-start-api" in dockerfile
    assert 'CMD ["testflying-start-api"]' in dockerfile
    assert "TESTFLYING_AUTO_MIGRATE" in start_script
    assert "alembic upgrade head" in start_script
    assert "exec uvicorn testflying_api.main:app" in start_script


def test_server_image_builds_admin_web_before_python_wheel() -> None:
    dockerfile = Path("Dockerfile").read_text()

    assert "FROM node:24-slim AS admin_web_builder" in dockerfile
    assert "COPY admin-web/package*.json ./" in dockerfile
    assert "RUN npm ci" in dockerfile
    assert "RUN npm run build" in dockerfile
    assert (
        "COPY --from=admin_web_builder /src/testflying_api/static/admin-app "
        "./src/testflying_api/static/admin-app"
    ) in dockerfile


def test_python_wheel_packages_admin_favicon() -> None:
    project = tomllib.loads(Path("pyproject.toml").read_text())

    package_data = project["tool"]["setuptools"]["package-data"]["testflying_api"]

    assert "static/admin-app/*.svg" in package_data

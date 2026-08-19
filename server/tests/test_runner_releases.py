from __future__ import annotations

import hashlib
import json
from pathlib import Path

import pytest
from fastapi.testclient import TestClient
from sqlalchemy.orm import Session

from testflying_api.build_platform import hash_runner_token
from testflying_api.errors import ApiError
from testflying_api.runner_releases import RunnerReleaseManifest, runner_release_status
from testflying_api.schema import BuildRunner


def test_runner_release_manifest_loads_contained_bundle(tmp_path: Path) -> None:
    bundle = _write_release(tmp_path)

    manifest = RunnerReleaseManifest.load(tmp_path, "darwin", "arm64")

    assert manifest.version == "0.2.0"
    assert manifest.runner_version == "0.2.0"
    assert manifest.package_agent_version == "0.2.0"
    assert manifest.bundle_path == bundle
    assert manifest.sha256 == hashlib.sha256(bundle.read_bytes()).hexdigest()


def test_runner_release_status_compares_both_runtime_versions(tmp_path: Path) -> None:
    _write_release(tmp_path)

    current = runner_release_status(
        tmp_path,
        platform="darwin",
        arch="arm64",
        runner_version="0.2.0",
        package_agent_version="0.2.0",
    )
    outdated = runner_release_status(
        tmp_path,
        platform="darwin",
        arch="arm64",
        runner_version="0.2.0",
        package_agent_version="0.1.0",
    )
    unknown = runner_release_status(
        tmp_path,
        platform="darwin",
        arch="amd64",
        runner_version="0.2.0",
        package_agent_version="0.2.0",
    )

    assert current == ("0.2.0", "current", "已是最新版本")
    assert outdated == ("0.2.0", "outdated", "可更新至 0.2.0")
    assert unknown == ("", "unknown", "未检测到发布版本")


def test_runner_update_check_and_download_require_provisioned_runner(
    client: TestClient,
    db_session: Session,
) -> None:
    bundle = _write_release(client.app.state.settings.runner_release_root)
    runner = BuildRunner(
        id="runner-mac-1",
        name="Mac mini 1",
        token_hash=hash_runner_token("runner-token", token_pepper="dev-token"),
        labels_json=["ios-release"],
        capabilities_json={"platforms": ["ios"], "capacity": 1},
        status="online",
    )
    db_session.add(runner)
    db_session.commit()
    headers = {"Authorization": "Bearer runner-token"}

    response = client.post(
        "/admin/api/build-runners/runner-mac-1/updates/check",
        headers=headers,
        json={
            "platform": "darwin",
            "arch": "arm64",
            "runnerVersion": "0.1.0",
            "packageAgentVersion": "0.1.0",
        },
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload == {
        "updateAvailable": True,
        "version": "0.2.0",
        "runnerVersion": "0.2.0",
        "packageAgentVersion": "0.2.0",
        "bundleUrl": ("/admin/api/build-runners/runner-mac-1/updates/darwin/arm64/0.2.0/bundle"),
        "sha256": hashlib.sha256(bundle.read_bytes()).hexdigest(),
    }
    download = client.get(payload["bundleUrl"], headers=headers)
    assert download.status_code == 200
    assert download.content == b"update-bundle"

    current = client.post(
        "/admin/api/build-runners/runner-mac-1/updates/check",
        headers=headers,
        json={
            "platform": "darwin",
            "arch": "arm64",
            "runnerVersion": "0.2.0",
            "packageAgentVersion": "0.2.0",
        },
    )
    assert current.status_code == 200
    assert current.json()["updateAvailable"] is False


@pytest.mark.parametrize(
    ("platform", "arch", "code"),
    [
        ("linux", "arm64", "unsupported_runner_platform"),
        ("darwin", "x86_64", "unsupported_runner_arch"),
    ],
)
def test_runner_release_rejects_unsupported_scope(
    tmp_path: Path,
    platform: str,
    arch: str,
    code: str,
) -> None:
    with pytest.raises(ApiError) as captured:
        RunnerReleaseManifest.load(tmp_path, platform, arch)

    assert captured.value.code == code


@pytest.mark.parametrize(
    ("field", "value", "code"),
    [
        ("bundleFile", "../escape.zip", "invalid_runner_release_bundle"),
        ("bundleFile", "/tmp/escape.zip", "invalid_runner_release_bundle"),
        ("sha256", "ABC", "invalid_runner_release_sha256"),
        ("version", "latest", "invalid_runner_release_version"),
    ],
)
def test_runner_release_rejects_invalid_manifest_fields(
    tmp_path: Path,
    field: str,
    value: str,
    code: str,
) -> None:
    _write_release(tmp_path)
    manifest_path = tmp_path / "darwin" / "arm64" / "release.json"
    payload = json.loads(manifest_path.read_text(encoding="utf-8"))
    payload[field] = value
    manifest_path.write_text(json.dumps(payload), encoding="utf-8")

    with pytest.raises(ApiError) as captured:
        RunnerReleaseManifest.load(tmp_path, "darwin", "arm64")

    assert captured.value.code == code


def test_runner_update_endpoints_reject_wrong_token_and_version(
    client: TestClient,
    db_session: Session,
) -> None:
    _write_release(client.app.state.settings.runner_release_root)
    db_session.add(
        BuildRunner(
            id="runner-mac-1",
            name="Mac mini 1",
            token_hash=hash_runner_token("runner-token", token_pepper="dev-token"),
            labels_json=[],
            capabilities_json={},
            status="online",
        )
    )
    db_session.commit()

    wrong_token = client.post(
        "/admin/api/build-runners/runner-mac-1/updates/check",
        headers={"Authorization": "Bearer wrong"},
        json={
            "platform": "darwin",
            "arch": "arm64",
            "runnerVersion": "0.1.0",
            "packageAgentVersion": "0.1.0",
        },
    )
    wrong_version = client.get(
        "/admin/api/build-runners/runner-mac-1/updates/darwin/arm64/0.1.0/bundle",
        headers={"Authorization": "Bearer runner-token"},
    )

    assert wrong_token.status_code == 401
    assert wrong_version.status_code == 404


def _write_release(root: Path) -> Path:
    release_dir = root / "darwin" / "arm64"
    release_dir.mkdir(parents=True)
    bundle = release_dir / "testflying-runner-0.2.0-darwin-arm64.zip"
    bundle.write_bytes(b"update-bundle")
    digest = hashlib.sha256(bundle.read_bytes()).hexdigest()
    (release_dir / "release.json").write_text(
        json.dumps(
            {
                "version": "0.2.0",
                "runnerVersion": "0.2.0",
                "packageAgentVersion": "0.2.0",
                "platform": "darwin",
                "arch": "arm64",
                "bundleFile": bundle.name,
                "sha256": digest,
            }
        ),
        encoding="utf-8",
    )
    return bundle

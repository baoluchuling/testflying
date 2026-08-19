from __future__ import annotations

from pathlib import Path

from testflying_api.storage import LocalArtifactStorage, S3ArtifactStorage


def test_storage_writes_file_and_returns_public_url(tmp_path: Path) -> None:
    storage = LocalArtifactStorage(root=tmp_path, public_base_url="https://dist.example.test")

    saved = storage.save("build-1", "app.ipa", b"ipa-bytes")

    assert saved.storage_path is not None
    assert saved.storage_path.exists()
    assert saved.download_url == "https://dist.example.test/artifacts/build-1/app.ipa"
    assert storage.read(saved.storage_key).content == b"ipa-bytes"
    assert storage.read(saved.storage_key).content_type == "application/octet-stream"
    assert storage.delete(saved.storage_key) is True
    assert saved.storage_path.exists() is False
    assert storage.delete(saved.storage_key) is False


def test_s3_storage_uses_configured_bucket_and_public_base_url(fake_s3_client: object) -> None:
    storage = S3ArtifactStorage(
        client=fake_s3_client,
        bucket="testflying",
        public_base_url="https://objects.example.test/testflying",
    )

    saved = storage.save("build-1", "app.ipa", b"ipa-bytes")

    assert saved.download_url == "https://objects.example.test/testflying/build-1/app.ipa"
    assert fake_s3_client.put_objects[0]["Bucket"] == "testflying"
    assert storage.read(saved.storage_key).content == b"ipa-bytes"
    assert storage.delete(saved.storage_key) is True
    assert fake_s3_client.deleted_objects[0] == {
        "Bucket": "testflying",
        "Key": "build-1/app.ipa",
    }

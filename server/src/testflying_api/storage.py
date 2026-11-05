from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Protocol
from urllib.parse import quote

import boto3

from testflying_api.config import Settings
from testflying_api.errors import ApiError


@dataclass(frozen=True)
class StoredArtifact:
    storage_key: str
    download_url: str
    storage_path: Path | None = None


@dataclass(frozen=True)
class ArtifactContent:
    content: bytes
    content_type: str


class ArtifactStorage(Protocol):
    backend: str

    def save(
        self,
        build_id: str,
        file_name: str,
        content: bytes,
        *,
        content_type: str = "application/octet-stream",
    ) -> StoredArtifact: ...

    def read(self, storage_key: str) -> ArtifactContent: ...

    def delete(self, storage_key: str) -> bool: ...


class LocalArtifactStorage:
    backend = "local"

    def __init__(self, *, root: Path, public_base_url: str) -> None:
        self._root = root
        self._public_base_url = public_base_url.rstrip("/")

    def save(
        self,
        build_id: str,
        file_name: str,
        content: bytes,
        *,
        content_type: str = "application/octet-stream",
    ) -> StoredArtifact:
        storage_key = _storage_key(build_id, file_name)
        storage_path = self._root / storage_key
        storage_path.parent.mkdir(parents=True, exist_ok=True)
        storage_path.write_bytes(content)
        return StoredArtifact(
            storage_key=storage_key,
            storage_path=storage_path,
            download_url=f"{self._public_base_url}/artifacts/{_quote_path(storage_key)}",
        )

    def read(self, storage_key: str) -> ArtifactContent:
        storage_path = (self._root / storage_key).resolve()
        root = self._root.resolve()
        if not storage_path.is_relative_to(root) or not storage_path.is_file():
            raise ApiError("artifact_not_found", "制品不存在", status_code=404)
        return ArtifactContent(
            content=storage_path.read_bytes(),
            content_type=_content_type_from_name(storage_path.name),
        )

    def delete(self, storage_key: str) -> bool:
        storage_path = (self._root / storage_key).resolve()
        root = self._root.resolve()
        if not storage_path.is_relative_to(root) or not storage_path.exists():
            return False
        if not storage_path.is_file():
            return False
        storage_path.unlink()
        return True


class S3ArtifactStorage:
    backend = "s3"

    def __init__(self, *, client: object, bucket: str, public_base_url: str) -> None:
        self._client = client
        self._bucket = bucket
        self._public_base_url = public_base_url.rstrip("/")

    def save(
        self,
        build_id: str,
        file_name: str,
        content: bytes,
        *,
        content_type: str = "application/octet-stream",
    ) -> StoredArtifact:
        storage_key = _storage_key(build_id, file_name)
        self._client.put_object(
            Bucket=self._bucket,
            Key=storage_key,
            Body=content,
            ContentType=content_type,
        )
        return StoredArtifact(
            storage_key=storage_key,
            download_url=f"{self._public_base_url}/{_quote_path(storage_key)}",
        )

    def read(self, storage_key: str) -> ArtifactContent:
        try:
            response = self._client.get_object(Bucket=self._bucket, Key=storage_key)
        except Exception as error:
            raise ApiError("artifact_not_found", "制品不存在", status_code=404) from error
        body = response.get("Body")
        content = body.read() if hasattr(body, "read") else bytes(body or b"")
        return ArtifactContent(
            content=content,
            content_type=str(response.get("ContentType") or _content_type_from_name(storage_key)),
        )

    def delete(self, storage_key: str) -> bool:
        try:
            self._client.delete_object(Bucket=self._bucket, Key=storage_key)
        except Exception:
            return False
        return True


def storage_from_settings(settings: Settings) -> ArtifactStorage:
    if settings.storage_backend == "local":
        return LocalArtifactStorage(
            root=settings.storage_root,
            public_base_url=settings.public_base_url,
        )
    if settings.storage_backend != "s3":
        raise ApiError("invalid_storage_backend", "不支持的制品存储后端")
    if not settings.s3_endpoint_url:
        raise ApiError("invalid_storage_config", "缺少 S3 endpoint 配置")
    if not settings.s3_public_base_url:
        raise ApiError("invalid_storage_config", "缺少 S3 public base URL 配置")

    client = boto3.client(
        "s3",
        endpoint_url=settings.s3_endpoint_url,
        aws_access_key_id=settings.s3_access_key_id,
        aws_secret_access_key=settings.s3_secret_access_key,
    )
    return S3ArtifactStorage(
        client=client,
        bucket=settings.s3_bucket,
        public_base_url=settings.s3_public_base_url,
    )


def _storage_key(build_id: str, file_name: str) -> str:
    safe_name = Path(file_name).name or "artifact.bin"
    return f"{build_id}/{safe_name}"


def _quote_path(path: str) -> str:
    return "/".join(quote(part) for part in path.split("/"))


def _content_type_from_name(file_name: str) -> str:
    suffix = Path(file_name).suffix.lower()
    if suffix == ".png":
        return "image/png"
    if suffix in {".jpg", ".jpeg"}:
        return "image/jpeg"
    if suffix == ".ipa":
        return "application/octet-stream"
    if suffix == ".apk":
        return "application/vnd.android.package-archive"
    if suffix == ".aab":
        return "application/octet-stream"
    if suffix == ".zip":
        return "application/zip"
    if suffix == ".json":
        return "application/json"
    if suffix == ".md":
        return "text/markdown"
    if suffix == ".log":
        return "text/plain"
    if suffix == ".plist":
        return "application/xml"
    return "application/octet-stream"

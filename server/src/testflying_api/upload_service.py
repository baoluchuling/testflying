from __future__ import annotations

import re
from datetime import UTC, datetime
from uuid import uuid4

from sqlalchemy import select
from sqlalchemy.orm import Session, joinedload

from testflying_api.catalog_repository import CatalogRepository
from testflying_api.catalog_service import CatalogService
from testflying_api.domain import (
    ArtifactType,
    BuildLifecycleStatus,
    BuildSource,
    channel_for_environment,
    normalize_environment,
    normalize_platform,
)
from testflying_api.errors import ApiError
from testflying_api.manifest import generate_ios_manifest, itms_services_url
from testflying_api.models import UploadResponse
from testflying_api.package_parser import (
    PackageMetadata,
    PackageParseError,
    parse_apk_metadata,
    parse_ipa_metadata,
)
from testflying_api.schema import (
    App,
    Artifact,
    Build,
    DeveloperAccount,
    Device,
    DeviceBuildVisibility,
    Notification,
)
from testflying_api.storage import ArtifactStorage
from testflying_api.store_identifiers import normalize_store_identifiers


def create_package_upload(
    *,
    session: Session,
    storage: ArtifactStorage,
    content: bytes,
    file_name: str,
    content_type: str,
    platform: str,
    environment: str,
    changelog: str = "",
    app_name: str | None = None,
    developer_account_id: str | None = None,
    store_app_id: str | None = None,
    store_package_name: str | None = None,
) -> UploadResponse:
    normalized_platform = _parse_platform(platform)
    normalized_environment = _parse_environment(environment)
    if not content:
        raise ApiError("empty_package", "上传包不能为空", status_code=422)

    metadata = _metadata_for_upload(
        platform=normalized_platform,
        content=content,
        app_name=app_name,
    )
    app = _upsert_app(
        session,
        metadata,
        environment=normalized_environment,
        developer_account_id=developer_account_id,
        store_app_id=store_app_id,
        store_package_name=store_package_name,
    )
    build = _create_build(
        session,
        app,
        metadata,
        environment=normalized_environment,
        changelog=changelog,
    )

    package_file_name = file_name or _default_file_name(metadata)
    effective_content_type = content_type or _content_type_for_platform(normalized_platform)
    saved_package = storage.save(
        build.id,
        package_file_name,
        content,
        content_type=effective_content_type,
    )

    manifest_url = None
    install_url = saved_package.download_url
    if normalized_platform == "ios":
        manifest_bytes = generate_ios_manifest(
            download_url=saved_package.download_url,
            bundle_identifier=metadata.bundle_identifier,
            version=metadata.version,
            title=metadata.app_name,
        )
        saved_manifest = storage.save(
            build.id,
            "manifest.plist",
            manifest_bytes,
            content_type="application/xml",
        )
        manifest_url = saved_manifest.download_url
        install_url = itms_services_url(manifest_url)

    session.add(
        Artifact(
            id=f"artifact-{build.id}",
            build_id=build.id,
            artifact_type=ArtifactType.PACKAGE.value,
            file_name=package_file_name,
            content_type=effective_content_type,
            storage_backend=storage.backend,
            storage_key=saved_package.storage_key,
            download_url=saved_package.download_url,
            manifest_url=manifest_url,
            install_url=install_url,
            size_bytes=len(content),
            metadata_json={"source": BuildSource.UPLOAD.value},
        )
    )
    _grant_build_to_registered_devices(session, build)
    session.add(
        Notification(
            id=f"notice-{build.id}",
            type="build",
            section="新构建",
            icon_key="rocket",
            title=f"{metadata.app_name} {metadata.version} 已上传",
            subtitle=(
                f"{_environment_label(normalized_environment)} build "
                f"{metadata.build_number} 可以安装验证。"
            ),
            tag=_environment_label(normalized_environment),
            tag_color="#2478FF" if normalized_environment == "development" else "#20864A",
            app_id=app.id,
            build_id=build.id,
            developer_account_id=app.developer_account_id,
            created_at=datetime.now(UTC),
        )
    )
    session.commit()

    created_build = session.scalars(
        select(Build)
        .options(joinedload(Build.app), joinedload(Build.artifacts))
        .where(Build.id == build.id)
    ).unique().one()
    service = CatalogService(CatalogRepository(session))
    build_response = service.build_response(created_build)
    return UploadResponse(
        app=service.app_response(app),
        build=build_response,
        install_info=build_response.install_info,
    )


def _parse_platform(value: str) -> str:
    try:
        return normalize_platform(value).value
    except ValueError as error:
        raise ApiError(
            "invalid_platform",
            "platform 必须是 ios 或 android",
            status_code=422,
        ) from error


def _parse_environment(value: str) -> str:
    try:
        return normalize_environment(value).value
    except ValueError as error:
        raise ApiError(
            "invalid_environment",
            "environment 必须是 development 或 production",
            status_code=422,
        ) from error


def _metadata_for_upload(
    *,
    platform: str,
    content: bytes,
    app_name: str | None,
) -> PackageMetadata:
    try:
        if platform == "ios":
            return parse_ipa_metadata(content)
        return parse_apk_metadata(content, app_name=app_name)
    except PackageParseError as error:
        raise ApiError("invalid_package", str(error), status_code=422) from error


def _upsert_app(
    session: Session,
    metadata: PackageMetadata,
    *,
    environment: str,
    developer_account_id: str | None,
    store_app_id: str | None,
    store_package_name: str | None,
) -> App:
    normalized_account_id = _normalize_optional(developer_account_id)
    if normalized_account_id and session.get(DeveloperAccount, normalized_account_id) is None:
        raise ApiError("account_not_found", "开发者账号不存在", status_code=404)
    normalized_store_app_id, normalized_store_package = normalize_store_identifiers(
        platform=metadata.platform,
        store_app_id=store_app_id,
        store_package_name=store_package_name,
    )
    if metadata.platform == "android" and normalized_store_package is None:
        normalized_store_package = metadata.bundle_identifier

    app = session.scalar(
        select(App).where(
            App.platform == metadata.platform,
            App.bundle_identifier == metadata.bundle_identifier,
        )
    )
    if app is not None:
        if normalized_account_id:
            if app.developer_account_id and app.developer_account_id != normalized_account_id:
                raise ApiError(
                    "app_bound_to_other_account",
                    "该 App 已绑定到其他开发者账号，请先在后台解绑后再上传。",
                    status_code=422,
                )
            app.developer_account_id = normalized_account_id
        _apply_store_identifiers(
            app,
            store_app_id=normalized_store_app_id,
            store_package_name=normalized_store_package,
        )
        app.name = metadata.app_name
        app.default_channel = channel_for_environment(environment)
        return app

    app = App(
        id=f"app-{metadata.platform}-{_slug(metadata.bundle_identifier)}",
        name=metadata.app_name,
        bundle_identifier=metadata.bundle_identifier,
        platform=metadata.platform,
        default_channel=channel_for_environment(environment),
        developer_account_id=normalized_account_id,
        store_app_id=normalized_store_app_id,
        store_package_name=normalized_store_package,
        icon_key="rocket" if metadata.platform == "ios" else "layers",
        icon_color="#2478FF" if metadata.platform == "ios" else "#B45309",
    )
    session.add(app)
    session.flush()
    return app


def _create_build(
    session: Session,
    app: App,
    metadata: PackageMetadata,
    *,
    environment: str,
    changelog: str,
) -> Build:
    build = Build(
        id=f"build-{_slug(metadata.bundle_identifier)}-{metadata.build_number}-{uuid4().hex[:8]}",
        app_id=app.id,
        version=metadata.version,
        build_number=metadata.build_number,
        channel=channel_for_environment(environment),
        environment=environment,
        requested_environment=environment,
        platform=metadata.platform,
        source=BuildSource.UPLOAD.value,
        lifecycle_status=BuildLifecycleStatus.SUCCEEDED.value,
        uploaded_at=datetime.now(UTC),
        finished_at=datetime.now(UTC),
        attempt_count=0,
        note=changelog,
        status="available",
    )
    session.add(build)
    session.flush()
    return build


def _grant_build_to_registered_devices(session: Session, build: Build) -> None:
    devices = session.scalars(select(Device).where(Device.platform == build.platform)).all()
    for device in devices:
        session.merge(DeviceBuildVisibility(device_id=device.id, build_id=build.id))


def _slug(value: str) -> str:
    return re.sub(r"[^a-zA-Z0-9]+", "-", value).strip("-").lower()[:48] or uuid4().hex[:12]


def _default_file_name(metadata: PackageMetadata) -> str:
    extension = "ipa" if metadata.platform == "ios" else "apk"
    return f"{_slug(metadata.bundle_identifier)}-{metadata.build_number}.{extension}"


def _content_type_for_platform(platform: str) -> str:
    if platform == "ios":
        return "application/octet-stream"
    return "application/vnd.android.package-archive"


def _environment_label(environment: str) -> str:
    return "线上环境" if environment == "production" else "开发环境"


def _apply_store_identifiers(
    app: App,
    *,
    store_app_id: str | None,
    store_package_name: str | None,
) -> None:
    if app.platform == "ios":
        app.store_package_name = None
        if store_app_id is not None:
            app.store_app_id = store_app_id
    elif app.platform == "android":
        app.store_app_id = None
        if store_package_name is not None:
            app.store_package_name = store_package_name


def _normalize_optional(value: str | None) -> str | None:
    if value is None:
        return None
    normalized = value.strip()
    return normalized or None

from __future__ import annotations

from sqlalchemy.orm import Session

from testflying_api.catalog_repository import CatalogRepository
from testflying_api.catalog_service import CatalogService
from testflying_api.schema import Artifact
from testflying_api.seed import seed_demo_catalog


def test_workspace_contains_distribution_facts_only(db_session: Session) -> None:
    seed_demo_catalog(db_session)
    workspace = CatalogService(CatalogRepository(db_session)).workspace_for_device(
        device_id="device-001",
        platform="ios",
    )

    assert workspace.apps
    assert workspace.builds
    assert {build.install_info.platform for build in workspace.builds} == {"ios"}
    assert workspace.install_tasks == []
    assert workspace.sort_order.build_ids == []


def test_workspace_uses_package_artifact_for_install_info(db_session: Session) -> None:
    seed_demo_catalog(db_session)
    db_session.add(
        Artifact(
            id="artifact-aurora-ios-120-log",
            build_id="build-aurora-ios-120",
            artifact_type="log",
            file_name="build.log",
            content_type="text/plain",
            storage_backend="local",
            storage_key="build-aurora-ios-120/build.log",
            download_url="https://dist.example.test/artifacts/build-aurora-ios-120/build.log",
            manifest_url=None,
            install_url="",
            size_bytes=512,
        )
    )
    db_session.commit()

    workspace = CatalogService(CatalogRepository(db_session)).workspace_for_device(
        device_id="device-001",
        platform="ios",
    )

    aurora_build = next(build for build in workspace.builds if build.id == "build-aurora-ios-120")
    assert aurora_build.install_info.install_url.startswith("itms-services://")
    assert aurora_build.install_info.download_url is not None
    assert aurora_build.install_info.download_url.endswith("Aurora-Mobile.ipa")

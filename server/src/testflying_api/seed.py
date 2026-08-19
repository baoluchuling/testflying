from __future__ import annotations

from datetime import UTC, datetime, timedelta

from sqlalchemy import select
from sqlalchemy.orm import Session

from testflying_api.schema import (
    App,
    Artifact,
    Build,
    DeveloperAccount,
    DeveloperAccountApp,
    Device,
    DeviceBuildVisibility,
    Notification,
    StoreConnector,
)


def seed_demo_catalog(session: Session) -> None:
    if session.scalar(select(App.id).limit(1)) is not None:
        return

    now = datetime.now(UTC)
    account = DeveloperAccount(
        id="account-apple-enterprise",
        team_name="Internal Distribution Team",
        expires_at=now + timedelta(days=12),
        status="renewal_due",
        renewal_action_label="去续费",
    )
    session.add(account)

    aurora = App(
        id="app-aurora-ios",
        name="Aurora Mobile",
        bundle_identifier="com.internal.aurora",
        platform="ios",
        developer_account_id=account.id,
        store_app_id="1234567890",
        default_channel="dev",
        icon_key="rocket",
        icon_color="#2478FF",
        added_at=now - timedelta(days=4),
    )
    insight = App(
        id="app-insight-ios",
        name="Insight Desk",
        bundle_identifier="com.internal.insight",
        platform="ios",
        developer_account_id=account.id,
        store_app_id="1234567891",
        default_channel="prod",
        icon_key="chart",
        icon_color="#20864A",
        added_at=now - timedelta(days=8),
    )
    dataflow = App(
        id="app-dataflow-android",
        name="DataFlow",
        bundle_identifier="com.internal.dataflow",
        platform="android",
        default_channel="dev",
        icon_key="layers",
        icon_color="#B45309",
        added_at=now - timedelta(days=2),
    )
    session.add_all([aurora, insight, dataflow])

    aurora_build = Build(
        id="build-aurora-ios-120",
        app_id=aurora.id,
        version="2.4.0",
        build_number="120",
        channel="dev",
        environment="development",
        platform="ios",
        uploaded_at=now - timedelta(hours=6),
        note="修复登录回跳并补充灰度日志。",
        status="available",
        min_os_version="iOS 16.0",
        expires_at=now + timedelta(days=42),
    )
    insight_build = Build(
        id="build-insight-ios-88",
        app_id=insight.id,
        version="1.9.3",
        build_number="88",
        channel="prod",
        environment="production",
        platform="ios",
        uploaded_at=now - timedelta(days=1, hours=3),
        note="线上环境签名包，用于回归验证。",
        status="available",
        min_os_version="iOS 15.0",
        expires_at=now + timedelta(days=21),
    )
    dataflow_build = Build(
        id="build-dataflow-android-54",
        app_id=dataflow.id,
        version="3.1.0",
        build_number="54",
        channel="dev",
        environment="development",
        platform="android",
        uploaded_at=now - timedelta(hours=2),
        note="补充上传链路和离线队列验证。",
        status="available",
        min_os_version="Android 10",
        expires_at=None,
    )
    session.add_all([aurora_build, insight_build, dataflow_build])

    session.add_all(
        [
            Artifact(
                id="artifact-aurora-ios-120",
                build_id=aurora_build.id,
                file_name="Aurora-Mobile.ipa",
                content_type="application/octet-stream",
                storage_backend="local",
                storage_key=f"{aurora_build.id}/Aurora-Mobile.ipa",
                download_url="https://dist.example.test/artifacts/build-aurora-ios-120/Aurora-Mobile.ipa",
                manifest_url="https://dist.example.test/artifacts/build-aurora-ios-120/manifest.plist",
                install_url=(
                    "itms-services://?action=download-manifest&url="
                    "https%3A%2F%2Fdist.example.test%2Fartifacts%2Fbuild-aurora-ios-120%2Fmanifest.plist"
                ),
                size_bytes=24_000_000,
            ),
            Artifact(
                id="artifact-insight-ios-88",
                build_id=insight_build.id,
                file_name="Insight-Desk.ipa",
                content_type="application/octet-stream",
                storage_backend="local",
                storage_key=f"{insight_build.id}/Insight-Desk.ipa",
                download_url="https://dist.example.test/artifacts/build-insight-ios-88/Insight-Desk.ipa",
                manifest_url="https://dist.example.test/artifacts/build-insight-ios-88/manifest.plist",
                install_url=(
                    "itms-services://?action=download-manifest&url="
                    "https%3A%2F%2Fdist.example.test%2Fartifacts%2Fbuild-insight-ios-88%2Fmanifest.plist"
                ),
                size_bytes=29_000_000,
            ),
            Artifact(
                id="artifact-dataflow-android-54",
                build_id=dataflow_build.id,
                file_name="DataFlow.apk",
                content_type="application/vnd.android.package-archive",
                storage_backend="local",
                storage_key=f"{dataflow_build.id}/DataFlow.apk",
                download_url="https://dist.example.test/artifacts/build-dataflow-android-54/DataFlow.apk",
                manifest_url=None,
                install_url="https://dist.example.test/artifacts/build-dataflow-android-54/DataFlow.apk",
                size_bytes=31_000_000,
            ),
        ]
    )

    ios_device = Device(
        id="device-001",
        name="iPhone 15 Pro",
        owner="QA Lab",
        platform="ios",
        status="registered",
        status_color="#20864A",
        detail="主测试机，允许安装开发环境和线上环境包",
        udid="00008110-001C2D123456801E",
        os_version="iOS 18.5",
        certificate_status="证书可用",
    )
    android_device = Device(
        id="device-android-001",
        name="Pixel 8",
        owner="QA Lab",
        platform="android",
        status="registered",
        status_color="#20864A",
        detail="Android 回归设备",
        udid="pixel8-lab-001",
        os_version="Android 15",
        certificate_status="无需 Apple 签名",
    )
    session.add_all([ios_device, android_device])
    session.add_all(
        [
            DeviceBuildVisibility(device_id=ios_device.id, build_id=aurora_build.id),
            DeviceBuildVisibility(device_id=ios_device.id, build_id=insight_build.id),
            DeviceBuildVisibility(device_id=android_device.id, build_id=dataflow_build.id),
        ]
    )

    session.add_all(
        [
            DeveloperAccountApp(developer_account_id=account.id, app_id=aurora.id),
            DeveloperAccountApp(developer_account_id=account.id, app_id=insight.id),
        ]
    )
    session.add(
        StoreConnector(
            id="connector-apple-enterprise",
            developer_account_id=account.id,
            name="Internal Store Connector",
            base_url="mock://account-apple-enterprise",
            auth_token="dev-connector-token",
            status="ok",
            last_checked_at=now - timedelta(minutes=4),
        )
    )
    session.add_all(
        [
            Notification(
                id="notice-build-aurora",
                type="build",
                section="新构建",
                icon_key="rocket",
                title="Aurora Mobile 2.4.0 已上传",
                subtitle="开发环境 build 120 可以安装验证。",
                tag="开发环境",
                tag_color="#2478FF",
                app_id=aurora.id,
                build_id=aurora_build.id,
                created_at=now - timedelta(hours=6),
            ),
            Notification(
                id="notice-account-renewal",
                type="account",
                section="账号续费",
                icon_key="wallet",
                title="Apple 开发者账号即将到期",
                subtitle="Internal Distribution Team 需要在 12 天内续费。",
                tag="续费",
                tag_color="#D92D20",
                developer_account_id=account.id,
                created_at=now - timedelta(hours=3),
            ),
            Notification(
                id="notice-device-ios",
                type="device",
                section="设备",
                icon_key="phone",
                title="iPhone 15 Pro 已登记",
                subtitle="设备已加入 iOS 内部测试池。",
                tag="设备",
                tag_color="#20864A",
                device_id=ios_device.id,
                created_at=now - timedelta(days=2),
            ),
        ]
    )
    session.commit()

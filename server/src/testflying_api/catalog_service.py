from __future__ import annotations

from datetime import UTC, datetime

from testflying_api.catalog_repository import CatalogRepository
from testflying_api.models import (
    AppResponse,
    BuildResponse,
    DeveloperAccountResponse,
    InstallInfo,
    NotificationResponse,
    ProfileAction,
    ProfileMetric,
    ProfilePreference,
    TestDevice,
    UserProfile,
    WorkspaceResponse,
    isoformat,
)
from testflying_api.schema import App, Build, DeveloperAccount, Device, Notification


class CatalogService:
    def __init__(self, repository: CatalogRepository) -> None:
        self._repository = repository

    def workspace_for_device(
        self,
        *,
        device_id: str,
        platform: str,
        has_token: bool = True,
    ) -> WorkspaceResponse:
        builds = self._repository.visible_builds(device_id=device_id, platform=platform)
        apps = self._repository.apps_for_builds(builds)
        app_ids = [app.id for app in apps]
        accounts = self._repository.developer_accounts_for_apps(app_ids)
        device = self._repository.current_device(device_id)

        return WorkspaceResponse(
            apps=[self.app_response(app) for app in apps],
            builds=[self.build_response(build) for build in builds],
            devices=self._workspace_devices(device=device, device_id=device_id, platform=platform),
            developer_accounts=[
                self.account_response(account, visible_app_ids=set(app_ids)) for account in accounts
            ],
            notifications=[
                self.notification_response(item) for item in self._repository.notifications()
            ],
            install_tasks=[],
            sort_order={"buildIds": []},
            profile=self.profile_response(has_token=has_token),
        )

    def apps(self, *, device_id: str, platform: str) -> list[AppResponse]:
        return [
            self.app_response(app)
            for app in self._repository.apps_for_builds(
                self._repository.visible_builds(device_id=device_id, platform=platform)
            )
        ]

    def builds(self, *, device_id: str, platform: str) -> list[BuildResponse]:
        return [
            self.build_response(build)
            for build in self._repository.visible_builds(device_id=device_id, platform=platform)
        ]

    def devices(self, *, current_device_id: str) -> list[TestDevice]:
        return [
            self.device_response(device, current_device_id=current_device_id)
            for device in self._repository.devices()
        ]

    def developer_accounts(
        self,
        *,
        device_id: str,
        platform: str,
    ) -> list[DeveloperAccountResponse]:
        builds = self._repository.visible_builds(device_id=device_id, platform=platform)
        app_ids = [build.app_id for build in builds]
        return [
            self.account_response(account, visible_app_ids=set(app_ids))
            for account in self._repository.developer_accounts_for_apps(app_ids)
        ]

    def notifications(self, *, types: set[str] | None = None) -> list[NotificationResponse]:
        return [
            self.notification_response(item) for item in self._repository.notifications(types=types)
        ]

    def app_response(self, app: App) -> AppResponse:
        return AppResponse(
            id=app.id,
            name=app.name,
            added_at=isoformat(app.added_at) or "",
            default_channel=app.default_channel,
            icon_key=app.icon_key,
            icon_color=app.icon_color,
        )

    def build_response(self, build: Build) -> BuildResponse:
        app = build.app
        artifact = build.package_artifact()
        download_url = artifact.download_url if artifact else None
        manifest_url = artifact.manifest_url if artifact else None
        install_url = artifact.install_url if artifact else ""
        expires_at = isoformat(build.expires_at)

        return BuildResponse(
            id=build.id,
            name=app.name,
            version=build.version or "",
            build_number=build.build_number or "",
            channel=build.channel,
            environment=build.environment,
            owner="",
            uploaded_at=isoformat(build.uploaded_at) or "",
            note=build.note,
            status=build.status,
            icon_key=app.icon_key,
            icon_color=app.icon_color,
            install_info=InstallInfo(
                platform=build.platform,
                install_url=install_url,
                manifest_url=manifest_url,
                download_url=download_url,
                min_os_version=build.min_os_version,
                expires_at=expires_at,
                is_installable=build.status != "expired",
                unavailable_reason="构建已过期" if build.status == "expired" else None,
            ),
        )

    def device_response(self, device: Device, *, current_device_id: str) -> TestDevice:
        return TestDevice(
            id=device.id,
            name=device.name,
            owner=device.owner,
            status=device.status,
            status_color=device.status_color,
            detail=device.detail,
            udid=device.udid,
            os_version=device.os_version,
            certificate_status=device.certificate_status,
            platform=device.platform,
            is_current=device.id == current_device_id,
        )

    def account_response(
        self,
        account: DeveloperAccount,
        *,
        visible_app_ids: set[str] | None = None,
    ) -> DeveloperAccountResponse:
        app_ids = self._repository.app_ids_for_developer_account(account.id)
        visible_ids = (
            app_ids
            if visible_app_ids is None
            else [item for item in app_ids if item in visible_app_ids]
        )
        app_name = self._repository.app_name(visible_ids[0]) if visible_ids else ""
        expires_at = account.expires_at
        if expires_at.tzinfo is None:
            expires_at = expires_at.replace(tzinfo=UTC)
        remaining_days = max((expires_at - datetime.now(UTC)).days, 0)
        return DeveloperAccountResponse(
            id=account.id,
            app_name=app_name,
            team_name=account.team_name,
            remaining_days=remaining_days,
            renewal_action_label=account.renewal_action_label,
            expires_at=isoformat(account.expires_at) or "",
            status=account.status,
            app_ids=visible_ids,
        )

    def notification_response(self, notification: Notification) -> NotificationResponse:
        return NotificationResponse(
            id=notification.id,
            type=notification.type,
            section=notification.section,
            icon_key=notification.icon_key,
            title=notification.title,
            subtitle=notification.subtitle,
            tag=notification.tag,
            tag_color=notification.tag_color,
            created_at=isoformat(notification.created_at) or "",
        )

    def profile_response(self, *, has_token: bool) -> UserProfile:
        return UserProfile(
            name="Internal Tester",
            initial="I",
            subtitle="已连接内部测试分发服务" if has_token else "本地调试模式",
            metrics=[
                ProfileMetric(label="应用", value="目录", icon_key="inventory"),
                ProfileMetric(label="设备", value="已登记", icon_key="phone"),
            ],
            actions=[
                ProfileAction(
                    icon_key="upload",
                    title="上传构建",
                    subtitle="通过 CI 或后台上传 IPA/APK",
                    message="上传入口由服务端提供，设备端不手动创建应用。",
                )
            ],
            preferences=[
                ProfilePreference(
                    title="本地安装状态",
                    subtitle="安装中、暂停和进度只保存在客户端",
                    value=True,
                    message="服务端不保存设备安装状态。",
                )
            ],
        )

    def _workspace_devices(
        self,
        *,
        device: Device | None,
        device_id: str,
        platform: str,
    ) -> list[TestDevice]:
        if device is None:
            return [
                TestDevice(
                    id=device_id,
                    name="未登记设备",
                    owner="",
                    status="pending",
                    status_color="#D92D20",
                    detail="当前设备尚未登记，无法查看可安装构建",
                    udid=device_id,
                    os_version="",
                    certificate_status="未登记",
                    platform=platform,
                    is_current=True,
                )
            ]
        return [
            self.device_response(item, current_device_id=device_id)
            for item in self._devices(device)
        ]

    def _devices(self, current_device: Device | None) -> list[Device]:
        devices = self._repository.devices()
        if current_device is not None:
            return devices
        return devices

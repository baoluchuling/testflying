from __future__ import annotations

from datetime import UTC, datetime

from pydantic import BaseModel, ConfigDict, Field


class CamelModel(BaseModel):
    model_config = ConfigDict(populate_by_name=True)


class AppSortOrder(CamelModel):
    build_ids: list[str] = Field(default_factory=list, alias="buildIds")


class ProfileMetric(CamelModel):
    label: str
    value: str
    icon_key: str = Field(alias="iconKey")


class ProfileAction(CamelModel):
    icon_key: str = Field(alias="iconKey")
    title: str
    subtitle: str
    message: str


class ProfilePreference(CamelModel):
    title: str
    subtitle: str
    value: bool
    message: str


class UserProfile(CamelModel):
    name: str
    initial: str
    subtitle: str
    metrics: list[ProfileMetric] = Field(default_factory=list)
    actions: list[ProfileAction] = Field(default_factory=list)
    preferences: list[ProfilePreference] = Field(default_factory=list)


class AppResponse(CamelModel):
    id: str
    name: str
    added_at: str = Field(alias="addedAt")
    default_channel: str = Field(alias="defaultChannel")
    icon_key: str = Field(alias="iconKey")
    icon_color: str = Field(alias="iconColor")


class InstallInfo(CamelModel):
    platform: str
    install_url: str = Field(alias="installUrl")
    manifest_url: str | None = Field(default=None, alias="manifestUrl")
    download_url: str | None = Field(default=None, alias="downloadUrl")
    min_os_version: str | None = Field(default=None, alias="minOsVersion")
    expires_at: str | None = Field(default=None, alias="expiresAt")
    is_installable: bool = Field(default=True, alias="isInstallable")
    unavailable_reason: str | None = Field(default=None, alias="unavailableReason")


class BuildResponse(CamelModel):
    id: str
    name: str
    version: str
    build_number: str = Field(alias="buildNumber")
    channel: str
    environment: str
    owner: str = ""
    uploaded_at: str = Field(alias="uploadedAt")
    note: str
    status: str
    icon_key: str = Field(alias="iconKey")
    icon_color: str = Field(alias="iconColor")
    install_info: InstallInfo = Field(alias="installInfo")


class TestDevice(CamelModel):
    id: str
    name: str
    owner: str = ""
    status: str
    status_color: str = Field(alias="statusColor")
    detail: str
    udid: str
    os_version: str = Field(alias="osVersion")
    certificate_status: str = Field(alias="certificateStatus")
    platform: str
    is_current: bool = Field(default=False, alias="isCurrent")


class DeveloperAccountResponse(CamelModel):
    id: str
    app_name: str = Field(alias="appName")
    team_name: str = Field(alias="teamName")
    remaining_days: int = Field(alias="remainingDays")
    renewal_action_label: str = Field(alias="renewalActionLabel")
    expires_at: str = Field(alias="expiresAt")
    status: str
    app_ids: list[str] = Field(default_factory=list, alias="appIds")


class NotificationResponse(CamelModel):
    id: str
    type: str
    section: str
    icon_key: str = Field(alias="iconKey")
    title: str
    subtitle: str
    tag: str
    tag_color: str = Field(alias="tagColor")
    created_at: str = Field(alias="createdAt")


class WorkspaceResponse(CamelModel):
    apps: list[AppResponse] = Field(default_factory=list)
    builds: list[BuildResponse] = Field(default_factory=list)
    devices: list[TestDevice] = Field(default_factory=list)
    developer_accounts: list[DeveloperAccountResponse] = Field(
        default_factory=list,
        alias="developerAccounts",
    )
    notifications: list[NotificationResponse] = Field(default_factory=list)
    install_tasks: list[dict[str, object]] = Field(default_factory=list, alias="installTasks")
    sort_order: AppSortOrder = Field(default_factory=AppSortOrder, alias="sortOrder")
    profile: UserProfile

    @classmethod
    def empty(cls, *, device_id: str, platform: str, has_token: bool) -> WorkspaceResponse:
        return cls(
            devices=[
                TestDevice(
                    id=device_id,
                    name="Local Test Device",
                    owner="",
                    platform=platform,
                    status="pending",
                    status_color="#53606E",
                    detail="等待服务端登记",
                    udid=device_id,
                    os_version="",
                    certificate_status="未登记",
                    is_current=True,
                )
            ],
            profile=UserProfile(
                name="Internal Tester",
                initial="I",
                subtitle="已连接" if has_token else "本地调试",
                metrics=[],
                actions=[],
                preferences=[],
            ),
        )


class UploadResponse(CamelModel):
    app: AppResponse
    build: BuildResponse
    install_info: InstallInfo = Field(alias="installInfo")


def isoformat(value: datetime | None) -> str | None:
    if value is None:
        return None
    if value.tzinfo is None:
        value = value.replace(tzinfo=UTC)
    return value.isoformat().replace("+00:00", "Z")

from __future__ import annotations

from typing import Any

from pydantic import BaseModel, ConfigDict, Field


def to_camel(value: str) -> str:
    words = value.split("_")
    return words[0] + "".join(word[:1].upper() + word[1:] for word in words[1:])


class AdminApiModel(BaseModel):
    model_config = ConfigDict(alias_generator=to_camel, populate_by_name=True)


class AdminNavItem(AdminApiModel):
    key: str
    label: str
    path: str


class AdminHealthState(AdminApiModel):
    state: str
    label: str


class AdminBootstrapResponse(AdminApiModel):
    app_name: str
    nav_items: list[AdminNavItem]
    health: AdminHealthState


class LlmProtocolItem(AdminApiModel):
    key: str
    label: str
    default_base_url: str
    default_model: str
    default_auth_header: str


class LlmPresetItem(AdminApiModel):
    key: str
    label: str
    protocol: str
    base_url: str
    model: str
    auth_header: str


class LlmProfileItem(AdminApiModel):
    id: str
    name: str
    protocol: str
    protocol_label: str
    base_url: str
    model: str
    auth_header: str
    auth_header_label: str
    api_key_set: bool
    api_key_preview: str
    status: str
    status_label: str
    updated_at_label: str


class LlmFeatureBindingItem(AdminApiModel):
    feature_key: str
    feature_label: str
    description: str
    primary_profile_id: str | None
    fallback_profile_id: str | None
    effective_profile_label: str
    status: str
    status_label: str


class LlmConfigState(AdminApiModel):
    protocols: list[LlmProtocolItem]
    presets: list[LlmPresetItem]
    profiles: list[LlmProfileItem]
    feature_bindings: list[LlmFeatureBindingItem]


class LlmProfileSaveRequest(AdminApiModel):
    name: str
    protocol: str
    base_url: str
    model: str
    api_key: str | None = None
    auth_header: str = ""


class LlmProfileSaveResponse(AdminApiModel):
    message: str
    profile: LlmProfileItem
    state: LlmConfigState


class LlmFeatureBindingSaveRequest(AdminApiModel):
    primary_profile_id: str | None = None
    fallback_profile_id: str | None = None


class LlmFeatureBindingSaveResponse(AdminApiModel):
    message: str
    binding: LlmFeatureBindingItem
    state: LlmConfigState


class ReviewAppItem(AdminApiModel):
    account_id: str
    app_id: str
    app_name: str
    bundle_identifier: str
    platform: str
    account_name: str
    icon_color: str
    review_count: int
    selected: bool


class ReviewStats(AdminApiModel):
    total: int
    low: int
    ios: int
    android: int


class ReviewItem(AdminApiModel):
    id: str
    store_review_id: str
    rating: int | None
    title: str
    body: str
    author_name: str
    locale: str
    territory: str
    app_version: str
    created_at: str


class ReviewFetchRunItem(AdminApiModel):
    id: str
    status: str
    page_count: int
    fetched_count: int
    inserted_count: int
    duplicate_count: int
    stopped_reason: str
    finished_at: str | None
    error_summary: str


class ReviewAnalysisRunItem(AdminApiModel):
    id: str
    status: str
    review_count: int
    low_rating_count: int
    issue_count: int
    summary: str
    finished_at: str | None
    error_summary: str


class ReviewAnalysisIssue(AdminApiModel):
    title: str
    severity: str
    count: int | None = None
    focus: str
    evidence: list[str] = Field(default_factory=list)
    suggestion: str = ""
    representative_review_ids: list[str] = Field(default_factory=list)


class StoreReviewsState(AdminApiModel):
    apps: list[ReviewAppItem]
    selected_account_id: str | None
    selected_app_id: str | None
    rating: int | None
    stats: ReviewStats
    reviews: list[ReviewItem]
    latest_fetch: ReviewFetchRunItem | None
    latest_analysis: ReviewAnalysisRunItem | None
    analysis_issues: list[ReviewAnalysisIssue]
    analysis_boundaries: list[str]


class ReviewScopeRequest(AdminApiModel):
    account_id: str
    app_id: str


class StoreReviewFetchResponse(AdminApiModel):
    message: str
    result: ReviewFetchRunItem
    state: StoreReviewsState


class StoreReviewAnalysisResponse(AdminApiModel):
    message: str
    result: ReviewAnalysisRunItem | None
    state: StoreReviewsState
    error: dict[str, Any] | None = None


class StoreAppBuildItem(AdminApiModel):
    version: str
    build_number: str
    environment: str
    uploaded_at: str


class StoreAppItem(AdminApiModel):
    id: str
    name: str
    bundle_identifier: str
    platform: str
    developer_account_id: str | None
    developer_account_name: str
    icon_color: str
    icon_text: str
    store_identifier: str
    status: str
    status_label: str
    latest_build: StoreAppBuildItem | None
    selected: bool
    store_management_path: str
    reviews_path: str


class StoreAppsStats(AdminApiModel):
    total: int
    ios: int
    android: int
    ready: int
    needs: int


class StoreAppsAccountSummary(AdminApiModel):
    total_accounts: int
    bound_apps: int
    connector_ok: int
    connector_needs: int
    renewal_reminders: int


class StoreAppsState(AdminApiModel):
    apps: list[StoreAppItem]
    selected_app: StoreAppItem | None
    filter: str
    stats: StoreAppsStats
    account_summary: StoreAppsAccountSummary


class DeveloperAccountSummary(AdminApiModel):
    id: str
    team_name: str
    status: str
    status_label: str
    expires_at: str
    expires_at_label: str
    remaining_days: int
    app_names: list[str]
    connector_name: str
    connector_status: str
    connector_status_label: str
    latest_sync_status: str
    latest_sync_at_label: str
    detail_path: str


class DeveloperAccountsStats(AdminApiModel):
    total: int
    ok: int
    needs: int
    bound_apps: int
    connector_needs: int


class DeveloperAccountsState(AdminApiModel):
    accounts: list[DeveloperAccountSummary]
    stats: DeveloperAccountsStats


class DeveloperAccountForm(AdminApiModel):
    account_id: str | None = None
    team_name: str
    expires_at: str
    status: str
    renewal_action_label: str = "去续费"


class DeveloperAccountSaveResponse(AdminApiModel):
    message: str
    account: DeveloperAccountSummary
    state: DeveloperAccountsState


class AccountAppItem(AdminApiModel):
    id: str
    name: str
    bundle_identifier: str
    platform: str
    platform_label: str
    icon_color: str
    icon_text: str
    store_app_id: str
    store_package_name: str
    latest_version_label: str
    store_path: str
    marketing_path: str
    release_notes_path: str
    connection_path: str


class UnassignedAppItem(AdminApiModel):
    id: str
    name: str
    bundle_identifier: str
    platform: str
    platform_label: str


class ConnectorState(AdminApiModel):
    name: str
    base_url: str
    auth_token: str
    status: str
    status_label: str
    checked_at_label: str


class SyncRunSummary(AdminApiModel):
    id: str
    operation: str
    status: str
    started_at_label: str
    summary: str


class DeveloperAccountDetailState(AdminApiModel):
    account: DeveloperAccountSummary
    connector: ConnectorState | None
    account_store_platform: str
    apps: list[AccountAppItem]
    unassigned_apps: list[UnassignedAppItem]
    sync_runs: list[SyncRunSummary]


class ConnectorSaveRequest(AdminApiModel):
    name: str
    base_url: str = ""
    auth_token: str = ""


class ConnectorActionResponse(AdminApiModel):
    message: str
    result: ConnectorState | None = None
    state: DeveloperAccountDetailState


class AccountAppBindRequest(AdminApiModel):
    app_id: str
    store_app_id: str = ""
    store_package_name: str = ""


class AccountAppSettingsRequest(AdminApiModel):
    store_app_id: str = ""
    store_package_name: str = ""


class AccountDetailActionResponse(AdminApiModel):
    message: str
    state: DeveloperAccountDetailState


class StoreLocaleContent(AdminApiModel):
    locale: str
    is_source: bool
    keywords: str
    promotional_text: str
    description: str
    release_notes: str
    store_images: dict[str, Any] = Field(default_factory=dict)


class StoreMarketingPageSummary(AdminApiModel):
    id: str
    page_id: str
    page_name: str
    page_type: str
    type_label: str
    status: str
    status_label: str
    apple_page_id_label: str
    deep_link_url: str = ""
    language_count: int
    filled_text_count: int
    asset_count: int
    detail_path: str


class MarketingPageLocaleContent(AdminApiModel):
    locale: str
    is_source: bool
    promotional_text: str
    store_images: dict[str, Any] = Field(default_factory=dict)


class MarketingPageDetailState(AdminApiModel):
    account: DeveloperAccountSummary
    app: AccountAppItem
    page: StoreMarketingPageSummary
    section: str = "marketing"
    locale: str
    source_locale: str
    supported_locales: list[str]
    localized_page: list[MarketingPageLocaleContent]
    connector: ConnectorState | None
    preflight_status: str
    preflight_label: str
    sync_runs: list[SyncRunSummary]


class MarketingPageLocaleInput(AdminApiModel):
    locale: str
    promotional_text: str = ""
    store_images: dict[str, Any] = Field(default_factory=dict)


class MarketingPageSaveRequest(AdminApiModel):
    page_name: str
    page_type: str = "custom_product_page"
    deep_link_url: str = ""
    locale: str = ""
    locales: list[MarketingPageLocaleInput]


class MarketingPageCreateRequest(MarketingPageSaveRequest):
    page_id: str = ""


class MarketingPageSyncRequest(MarketingPageSaveRequest):
    sync_scopes: list[str]


class MarketingPageActionResponse(AdminApiModel):
    message: str
    state: MarketingPageDetailState | None = None
    workspace: StoreWorkspaceState | None = None
    sync_runs: list[SyncRunSummary] = Field(default_factory=list)


class StoreWorkspaceState(AdminApiModel):
    account: DeveloperAccountSummary
    app: AccountAppItem
    section: str
    version: str
    locale: str
    source_locale: str
    supported_locales: list[str]
    localized_metadata: list[StoreLocaleContent]
    connector: ConnectorState | None
    preflight_status: str
    preflight_label: str
    sync_runs: list[SyncRunSummary]
    marketing_pages: list[StoreMarketingPageSummary]


class StoreLocaleContentInput(AdminApiModel):
    locale: str
    promotional_text: str = ""
    description: str = ""
    release_notes: str = ""
    store_images: dict[str, Any] = Field(default_factory=dict)


class StoreWorkspaceSaveRequest(AdminApiModel):
    version: str = ""
    locale: str = ""
    locales: list[StoreLocaleContentInput]


class StoreWorkspaceSyncRequest(AdminApiModel):
    version: str
    locale: str = ""
    sync_scopes: list[str]
    locales: list[StoreLocaleContentInput]


class StoreImageDeleteRequest(AdminApiModel):
    version: str = ""
    locale: str
    slot_key: str
    storage_key: str


class StoreWorkspaceActionResponse(AdminApiModel):
    message: str
    state: StoreWorkspaceState
    sync_runs: list[SyncRunSummary] = Field(default_factory=list)


class StoreTranslationRequest(AdminApiModel):
    source_locale: str
    target_locales: list[str]
    field: str
    text: str


class StoreTranslationResponse(AdminApiModel):
    translations: dict[str, str]


class UploadAccountOption(AdminApiModel):
    id: str
    team_name: str
    status: str
    platform: str | None = None


class UploadState(AdminApiModel):
    accounts: list[UploadAccountOption]


class UploadResult(AdminApiModel):
    app_id: str
    app_name: str
    bundle_identifier: str
    platform: str
    environment: str
    version: str
    build_number: str
    developer_account: str
    store_identifier: str
    install_url: str
    manifest_url: str | None = None
    download_url: str | None = None


class AdminUploadResponse(AdminApiModel):
    message: str
    result: UploadResult
    state: UploadState


class AppLogConnectInfo(AdminApiModel):
    host: str
    port: str
    name: str
    app_scheme: str
    app_name: str
    connect_url: str
    connect_page_url: str
    scheme_url: str
    websocket_url: str


class AppLogDeviceItem(AdminApiModel):
    token: str
    device_id: str
    device: str
    platform: str
    connected: bool
    known_token: bool
    connected_at: str
    last_seen_at: str
    connection_count: int
    error_count: int
    log_count: int


class AppLogFieldItem(AdminApiModel):
    key: str
    value: str


class AppLogEntryItem(AdminApiModel):
    sequence: int
    token: str
    device_id: str
    device: str
    platform: str
    received_at: str
    sent_at: str
    history: bool
    raw: str
    timestamp: str
    level: str
    tag: str
    event: str
    message: str
    fields: list[AppLogFieldItem]


class AppLogClientErrorItem(AdminApiModel):
    sequence: int
    token: str
    device_id: str
    device: str
    received_at: str
    sent_at: str
    message: str


class AppLogsState(AdminApiModel):
    connect: AppLogConnectInfo
    cursor: int
    devices: list[AppLogDeviceItem]
    logs: list[AppLogEntryItem]
    errors: list[AppLogClientErrorItem]
    levels: list[str]


class DashboardStatItem(AdminApiModel):
    label: str
    value: str
    tone: str


class BuildAppSummary(AdminApiModel):
    id: str
    name: str
    bundle_identifier: str
    platform: str
    icon_color: str
    icon_text: str


class BuildArtifactItem(AdminApiModel):
    artifact_type: str = ""
    artifact_type_label: str = ""
    file_name: str
    size_label: str
    install_url: str
    download_url: str
    manifest_url: str | None = None


class BuildItem(AdminApiModel):
    id: str
    app: BuildAppSummary
    version: str
    build_number: str
    source: str
    lifecycle_status: str
    git_ref: str
    platform: str
    platform_label: str
    environment: str
    environment_label: str
    status: str
    note: str
    min_os_version: str
    uploaded_at: str
    uploaded_at_label: str
    expires_at: str | None
    expires_at_label: str
    artifact: BuildArtifactItem | None
    artifacts: list[BuildArtifactItem] = Field(default_factory=list)
    failure_classification: str = ""
    failure_summary: str = ""
    human_action: str = ""
    recent_events: list[BuildEventItem] = Field(default_factory=list)


class BuildSettingItem(AdminApiModel):
    git_url: str
    runner_labels: list[str]
    credential_refs: dict[str, str]
    artifact_type: str
    optional_defaults: dict[str, Any]
    updated_at_label: str


class BuildAppItem(AdminApiModel):
    app: BuildAppSummary
    setting: BuildSettingItem
    matching_runner_count: int
    has_online_runner: bool
    latest_build: BuildItem | None


class BuildAppsState(AdminApiModel):
    apps: list[BuildAppItem]
    available_apps: list[BuildAppSummary]
    total: int


class BuildSettingSaveRequest(AdminApiModel):
    git_url: str
    runner_labels: list[str] = Field(default_factory=list)
    credential_refs: dict[str, str] = Field(default_factory=dict)
    artifact_type: str
    optional_defaults: dict[str, Any] = Field(default_factory=dict)


class AgentBuildCreateRequest(AdminApiModel):
    environment: str
    git_ref: str


class RunnerHeartbeatRequest(AdminApiModel):
    runner_id: str
    name: str
    labels: list[str] = Field(default_factory=list)
    version: str
    package_agent_version: str
    capabilities: dict[str, Any] = Field(default_factory=dict)


class RunnerProvisionRequest(AdminApiModel):
    runner_id: str
    name: str
    labels: list[str] = Field(default_factory=list)
    version: str = ""
    package_agent_version: str = ""
    capabilities: dict[str, Any] = Field(default_factory=dict)


class RunnerProvisionResponse(AdminApiModel):
    runner: BuildRunnerItem
    token: str


class RunnerPollRequest(AdminApiModel):
    runner_id: str
    timeout_seconds: float = 0


class RunnerBuildPayload(AdminApiModel):
    id: str
    app_id: str
    platform: str
    environment: str
    git_url: str
    git_ref: str
    artifact_type: str
    credential_refs: dict[str, str]


class RunnerPollResponse(AdminApiModel):
    build: RunnerBuildPayload | None


class RunnerUpdateCheckRequest(AdminApiModel):
    platform: str
    arch: str
    runner_version: str
    package_agent_version: str


class RunnerUpdateCheckResponse(AdminApiModel):
    update_available: bool
    version: str = ""
    runner_version: str = ""
    package_agent_version: str = ""
    bundle_url: str = ""
    sha256: str = ""


class BuildRunnerItem(AdminApiModel):
    id: str
    name: str
    status: str
    labels: list[str]
    version: str
    package_agent_version: str
    last_seen_at_label: str
    current_build_id: str | None
    capabilities: dict[str, Any]
    latest_version: str
    update_status: str
    update_status_label: str


class BuildRunnersState(AdminApiModel):
    runners: list[BuildRunnerItem]
    total: int


class BuildEventItem(AdminApiModel):
    type: str
    message: str
    created_at_label: str


class RunnerEventRequest(AdminApiModel):
    runner_id: str
    type: str
    message: str
    lifecycle_status: str | None = None
    payload: dict[str, Any] = Field(default_factory=dict)


class RunnerCompleteRequest(AdminApiModel):
    runner_id: str
    status: str
    version: str | None = None
    build_number: str | None = None
    commit_sha: str | None = None
    note: str | None = None
    failure_classification: str | None = None
    failure_summary: str | None = None
    human_action: str | None = None


class AppDetailState(AdminApiModel):
    app: BuildAppSummary
    builds: list[BuildItem]
    build_setting: BuildSettingItem | None


class AppBuildActionResponse(AdminApiModel):
    message: str
    build: BuildItem | None
    state: AppDetailState


class NotificationItem(AdminApiModel):
    id: str
    type: str
    section: str
    icon_key: str
    title: str
    subtitle: str
    tag: str
    tag_color: str
    created_at: str
    created_at_label: str


class DashboardState(AdminApiModel):
    stats: list[DashboardStatItem]
    recent_builds: list[BuildItem]
    recent_notifications: list[NotificationItem]


class BuildsState(AdminApiModel):
    builds: list[BuildItem]
    total: int


class DeviceItem(AdminApiModel):
    id: str
    name: str
    owner: str
    platform: str
    platform_label: str
    status: str
    status_color: str
    detail: str
    udid: str
    os_version: str
    certificate_status: str
    registered_at: str
    registered_at_label: str


class DevicesState(AdminApiModel):
    devices: list[DeviceItem]
    total: int


class NotificationTypeCount(AdminApiModel):
    type: str
    label: str
    count: int


class DingTalkConfigState(AdminApiModel):
    configured: bool
    webhook_configured: bool
    secret_configured: bool
    triggers: list[str]
    pending_delivery_count: int
    dead_delivery_count: int


class NotificationsState(AdminApiModel):
    notifications: list[NotificationItem]
    type_counts: list[NotificationTypeCount]
    active_type: str
    total: int
    dingtalk: DingTalkConfigState


class GeneralSettingsState(AdminApiModel):
    connector_base_url_template: str
    source: str


class NotificationSettingsState(AdminApiModel):
    enabled: bool
    configured: bool
    webhook_configured: bool
    secret_configured: bool
    timeout_seconds: float
    dispatch_interval_seconds: float
    pending_delivery_count: int
    dead_delivery_count: int
    source: str


class RuntimeEnvironmentItem(AdminApiModel):
    key: str
    label: str
    group: str
    source: str
    value_label: str
    configured: bool
    sensitive: bool
    restart_required: bool


class SettingsState(AdminApiModel):
    general: GeneralSettingsState
    notifications: NotificationSettingsState
    runtime: list[RuntimeEnvironmentItem]


class GeneralSettingsSaveRequest(AdminApiModel):
    connector_base_url_template: str | None = None


class NotificationSettingsSaveRequest(AdminApiModel):
    enabled: bool
    webhook_url: str | None = None
    secret: str | None = None
    timeout_seconds: float
    dispatch_interval_seconds: float


class SettingsActionResponse(AdminApiModel):
    message: str
    state: SettingsState


class ApiDocParamItem(AdminApiModel):
    name: str
    location: str
    required: str
    description: str


class ApiDocEndpointItem(AdminApiModel):
    anchor: str
    title: str
    method: str
    path: str
    summary: str
    params: list[ApiDocParamItem]
    curl: str
    response: str


class ApiDocsState(AdminApiModel):
    endpoints: list[ApiDocEndpointItem]
    download_url: str

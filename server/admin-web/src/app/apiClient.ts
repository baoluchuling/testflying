export type NavItem = {
  key: string;
  label: string;
  path: string;
};

export type BootstrapResponse = {
  appName: string;
  navItems: NavItem[];
  health: {
    state: 'idle' | 'ok' | 'error';
    label: string;
  };
};

export type ReviewAppItem = {
  accountId: string;
  appId: string;
  appName: string;
  bundleIdentifier: string;
  platform: string;
  accountName: string;
  iconColor: string;
  reviewCount: number;
  selected: boolean;
};

export type ReviewStats = {
  total: number;
  low: number;
  ios: number;
  android: number;
};

export type ReviewItem = {
  id: string;
  storeReviewId: string;
  rating: number | null;
  title: string;
  body: string;
  authorName: string;
  locale: string;
  territory: string;
  appVersion: string;
  createdAt: string;
};

export type ReviewFetchRun = {
  id: string;
  status: string;
  pageCount: number;
  fetchedCount: number;
  insertedCount: number;
  duplicateCount: number;
  stoppedReason: string;
  finishedAt: string | null;
  errorSummary: string;
};

export type ReviewAnalysisRun = {
  id: string;
  status: string;
  reviewCount: number;
  lowRatingCount: number;
  issueCount: number;
  summary: string;
  finishedAt: string | null;
  errorSummary: string;
};

export type ReviewAnalysisIssue = {
  title: string;
  severity: string;
  count: number | null;
  focus: string;
  evidence: string[];
  suggestion: string;
  representativeReviewIds: string[];
};

export type StoreReviewsState = {
  apps: ReviewAppItem[];
  selectedAccountId: string | null;
  selectedAppId: string | null;
  rating: number | null;
  stats: ReviewStats;
  reviews: ReviewItem[];
  latestFetch: ReviewFetchRun | null;
  latestAnalysis: ReviewAnalysisRun | null;
  analysisIssues: ReviewAnalysisIssue[];
  analysisBoundaries: string[];
};

export type StoreReviewActionResponse = {
  message: string;
  result: ReviewFetchRun | ReviewAnalysisRun | null;
  state: StoreReviewsState;
  error?: {
    code: string;
    message: string;
  } | null;
};

export type StoreAppBuild = {
  version: string;
  buildNumber: string;
  environment: string;
  uploadedAt: string;
};

export type StoreAppItem = {
  id: string;
  name: string;
  bundleIdentifier: string;
  platform: string;
  developerAccountId: string | null;
  developerAccountName: string;
  iconColor: string;
  iconText: string;
  storeIdentifier: string;
  status: string;
  statusLabel: string;
  latestBuild: StoreAppBuild | null;
  selected: boolean;
  storeManagementPath: string;
  reviewsPath: string;
};

export type StoreAppsStats = {
  total: number;
  ios: number;
  android: number;
  ready: number;
  needs: number;
};

export type StoreAppsAccountSummary = {
  totalAccounts: number;
  boundApps: number;
  connectorOk: number;
  connectorNeeds: number;
  renewalReminders: number;
};

export type StoreAppsState = {
  apps: StoreAppItem[];
  selectedApp: StoreAppItem | null;
  filter: string;
  stats: StoreAppsStats;
  accountSummary: StoreAppsAccountSummary;
};

export type LlmProtocolItem = {
  key: string;
  label: string;
  defaultBaseUrl: string;
  defaultModel: string;
  defaultAuthHeader: string;
};

export type LlmPresetItem = {
  key: string;
  label: string;
  protocol: string;
  baseUrl: string;
  model: string;
  authHeader: string;
};

export type LlmProfileItem = {
  id: string;
  name: string;
  protocol: string;
  protocolLabel: string;
  baseUrl: string;
  model: string;
  authHeader: string;
  authHeaderLabel: string;
  apiKeySet: boolean;
  apiKeyPreview: string;
  status: string;
  statusLabel: string;
  updatedAtLabel: string;
};

export type LlmFeatureBindingItem = {
  featureKey: string;
  featureLabel: string;
  description: string;
  primaryProfileId: string | null;
  fallbackProfileId: string | null;
  effectiveProfileLabel: string;
  status: string;
  statusLabel: string;
};

export type LlmConfigState = {
  protocols: LlmProtocolItem[];
  presets: LlmPresetItem[];
  profiles: LlmProfileItem[];
  featureBindings: LlmFeatureBindingItem[];
};

export type LlmProfilePayload = {
  name: string;
  protocol: string;
  baseUrl: string;
  model: string;
  apiKey?: string;
  authHeader: string;
};

export type LlmProfileSaveResponse = {
  message: string;
  profile: LlmProfileItem;
  state: LlmConfigState;
};

export type LlmFeatureBindingSaveResponse = {
  message: string;
  binding: LlmFeatureBindingItem;
  state: LlmConfigState;
};

export type DeveloperAccountSummary = {
  id: string;
  teamName: string;
  status: string;
  statusLabel: string;
  expiresAt: string;
  expiresAtLabel: string;
  remainingDays: number;
  appNames: string[];
  connectorName: string;
  connectorStatus: string;
  connectorStatusLabel: string;
  latestSyncStatus: string;
  latestSyncAtLabel: string;
  detailPath: string;
};

export type DeveloperAccountsStats = {
  total: number;
  ok: number;
  needs: number;
  boundApps: number;
  connectorNeeds: number;
};

export type DeveloperAccountsState = {
  accounts: DeveloperAccountSummary[];
  stats: DeveloperAccountsStats;
};

export type DeveloperAccountForm = {
  accountId?: string | null;
  teamName: string;
  expiresAt: string;
  status: string;
  renewalActionLabel: string;
};

export type AccountAppItem = {
  id: string;
  name: string;
  bundleIdentifier: string;
  platform: string;
  platformLabel: string;
  iconColor: string;
  iconText: string;
  storeAppId: string;
  storePackageName: string;
  latestVersionLabel: string;
  storePath: string;
  marketingPath: string;
  releaseNotesPath: string;
  connectionPath: string;
};

export type UnassignedAppItem = {
  id: string;
  name: string;
  bundleIdentifier: string;
  platform: string;
  platformLabel: string;
};

export type ConnectorState = {
  name: string;
  baseUrl: string;
  authToken: string;
  status: string;
  statusLabel: string;
  checkedAtLabel: string;
};

export type SyncRunSummary = {
  id: string;
  operation: string;
  status: string;
  startedAtLabel: string;
  summary: string;
};

export type DeveloperAccountDetailState = {
  account: DeveloperAccountSummary;
  connector: ConnectorState | null;
  accountStorePlatform: string;
  apps: AccountAppItem[];
  unassignedApps: UnassignedAppItem[];
  syncRuns: SyncRunSummary[];
};

export type DeveloperAccountSaveResponse = {
  message: string;
  account: DeveloperAccountSummary;
  state: DeveloperAccountsState;
};

export type ConnectorActionResponse = {
  message: string;
  result: ConnectorState | null;
  state: DeveloperAccountDetailState;
};

export type AccountDetailActionResponse = {
  message: string;
  state: DeveloperAccountDetailState;
};

export type StoreLocaleContent = {
  locale: string;
  isSource: boolean;
  keywords: string;
  promotionalText: string;
  description: string;
  releaseNotes: string;
  storeImages: Record<string, unknown>;
};

export type StoreMarketingPageSummary = {
  id: string;
  pageId: string;
  pageName: string;
  pageType: string;
  typeLabel: string;
  status: string;
  statusLabel: string;
  applePageIdLabel: string;
  deepLinkUrl: string;
  languageCount: number;
  filledTextCount: number;
  assetCount: number;
  detailPath: string;
};

export type StoreWorkspaceState = {
  account: DeveloperAccountSummary;
  app: AccountAppItem;
  section: string;
  version: string;
  locale: string;
  sourceLocale: string;
  supportedLocales: string[];
  localizedMetadata: StoreLocaleContent[];
  connector: ConnectorState | null;
  preflightStatus: string;
  preflightLabel: string;
  syncRuns: SyncRunSummary[];
  marketingPages: StoreMarketingPageSummary[];
};

export type StoreLocaleContentInput = {
  locale: string;
  promotionalText: string;
  description: string;
  releaseNotes: string;
  storeImages: Record<string, unknown>;
};

export type StoreWorkspaceActionResponse = {
  message: string;
  state: StoreWorkspaceState;
  syncRuns: SyncRunSummary[];
};

export type StoreTranslationRequest = {
  sourceLocale: string;
  targetLocales: string[];
  field: string;
  text: string;
};

export type StoreTranslationResponse = {
  translations: Record<string, string>;
};

export type MarketingPageLocaleContent = {
  locale: string;
  isSource: boolean;
  promotionalText: string;
  storeImages: Record<string, unknown>;
};

export type MarketingPageDetailState = {
  account: DeveloperAccountSummary;
  app: AccountAppItem;
  page: StoreMarketingPageSummary;
  section: string;
  locale: string;
  sourceLocale: string;
  supportedLocales: string[];
  localizedPage: MarketingPageLocaleContent[];
  connector: ConnectorState | null;
  preflightStatus: string;
  preflightLabel: string;
  syncRuns: SyncRunSummary[];
};

export type MarketingPageLocaleInput = {
  locale: string;
  promotionalText: string;
  storeImages: Record<string, unknown>;
};

export type MarketingPagePayload = {
  pageId?: string;
  pageName: string;
  pageType: string;
  deepLinkUrl: string;
  locale: string;
  locales: MarketingPageLocaleInput[];
  syncScopes?: string[];
};

export type MarketingPageActionResponse = {
  message: string;
  state: MarketingPageDetailState | null;
  workspace: StoreWorkspaceState | null;
  syncRuns: SyncRunSummary[];
};

export type UploadAccountOption = {
  id: string;
  teamName: string;
  status: string;
  platform: string | null;
};

export type UploadState = {
  accounts: UploadAccountOption[];
};

export type UploadResult = {
  appId: string;
  appName: string;
  bundleIdentifier: string;
  platform: string;
  environment: string;
  version: string;
  buildNumber: string;
  developerAccount: string;
  storeIdentifier: string;
  installUrl: string;
  manifestUrl: string | null;
  downloadUrl: string | null;
};

export type AdminUploadResponse = {
  message: string;
  result: UploadResult;
  state: UploadState;
};

export type AppLogConnectInfo = {
  host: string;
  port: string;
  name: string;
  appScheme: string;
  appName: string;
  connectUrl: string;
  connectPageUrl: string;
  schemeUrl: string;
  websocketUrl: string;
};

export type AppLogDevice = {
  token: string;
  deviceId: string;
  device: string;
  platform: string;
  connected: boolean;
  knownToken: boolean;
  connectedAt: string;
  lastSeenAt: string;
  connectionCount: number;
  errorCount: number;
  logCount: number;
};

export type AppLogField = {
  key: string;
  value: string;
};

export type AppLogEntry = {
  sequence: number;
  token: string;
  deviceId: string;
  device: string;
  platform: string;
  receivedAt: string;
  sentAt: string;
  history: boolean;
  raw: string;
  timestamp: string;
  level: string;
  tag: string;
  event: string;
  message: string;
  fields: AppLogField[];
};

export type AppLogClientError = {
  sequence: number;
  token: string;
  deviceId: string;
  device: string;
  receivedAt: string;
  sentAt: string;
  message: string;
};

export type AppLogsState = {
  connect: AppLogConnectInfo;
  cursor: number;
  devices: AppLogDevice[];
  logs: AppLogEntry[];
  errors: AppLogClientError[];
  levels: string[];
};

export type DashboardStat = {
  label: string;
  value: string;
  tone: string;
};

export type BuildAppSummary = {
  id: string;
  name: string;
  bundleIdentifier: string;
  platform: string;
  iconColor: string;
  iconText: string;
};

export type BuildArtifact = {
  artifactType?: string;
  artifactTypeLabel?: string;
  fileName: string;
  sizeLabel: string;
  installUrl: string;
  downloadUrl: string;
  manifestUrl: string | null;
};

export type BuildItem = {
  id: string;
  app: BuildAppSummary;
  version: string;
  buildNumber: string;
  source?: string;
  sourceLabel?: string;
  lifecycleStatus?: string;
  lifecycleStatusLabel?: string;
  gitRef?: string;
  platform: string;
  platformLabel: string;
  environment: string;
  environmentLabel: string;
  status: string;
  note: string;
  minOsVersion: string;
  uploadedAt: string;
  uploadedAtLabel: string;
  expiresAt: string | null;
  expiresAtLabel: string;
  artifact: BuildArtifact | null;
  artifacts: BuildArtifact[];
  failureClassification: string;
  failureSummary: string;
  humanAction: string;
  recentEvents: {
    type: string;
    message: string;
    createdAtLabel: string;
  }[];
};

export type BuildSettingItem = {
  gitUrl: string;
  runnerLabels: string[];
  credentialRefs: Record<string, string>;
  artifactType: string;
  optionalDefaults: Record<string, unknown>;
  updatedAtLabel: string;
};

export type AppDetailState = {
  app: BuildAppSummary;
  builds: BuildItem[];
  buildSetting: BuildSettingItem | null;
};

export type AgentBuildCreateInput = {
  environment: 'development' | 'production';
  gitRef: string;
};

export type BuildSettingSavePayload = {
  gitUrl: string;
  runnerLabels: string[];
  credentialRefs: Record<string, string>;
  artifactType: string;
  optionalDefaults: Record<string, unknown>;
};

export type AppBuildActionResponse = {
  message: string;
  build: BuildItem | null;
  state: AppDetailState;
};

export type NotificationItem = {
  id: string;
  type: string;
  section: string;
  iconKey: string;
  title: string;
  subtitle: string;
  tag: string;
  tagColor: string;
  createdAt: string;
  createdAtLabel: string;
};

export type DashboardState = {
  stats: DashboardStat[];
  recentBuilds: BuildItem[];
  recentNotifications: NotificationItem[];
};

export type BuildsState = {
  builds: BuildItem[];
  total: number;
};

export type BuildAppItem = {
  app: BuildAppSummary;
  setting: BuildSettingItem;
  matchingRunnerCount: number;
  hasOnlineRunner: boolean;
  latestBuild: BuildItem | null;
};

export type BuildAppsState = {
  apps: BuildAppItem[];
  availableApps: BuildAppSummary[];
  total: number;
};

export type BuildRunnerItem = {
  id: string;
  name: string;
  status: string;
  labels: string[];
  version: string;
  packageAgentVersion: string;
  lastSeenAtLabel: string;
  currentBuildId: string | null;
  capabilities: Record<string, unknown>;
  latestVersion: string;
  updateStatus: 'current' | 'outdated' | 'unknown';
  updateStatusLabel: string;
};

export type BuildRunnersState = {
  runners: BuildRunnerItem[];
  total: number;
};

export type RunnerProvisionPayload = {
  runnerId: string;
  name: string;
  labels: string[];
  version: string;
  packageAgentVersion: string;
  capabilities: Record<string, unknown>;
};

export type RunnerProvisionResponse = {
  runner: BuildRunnerItem;
  token: string;
};

export type DeviceItem = {
  id: string;
  name: string;
  owner: string;
  platform: string;
  platformLabel: string;
  status: string;
  statusColor: string;
  detail: string;
  udid: string;
  osVersion: string;
  certificateStatus: string;
  registeredAt: string;
  registeredAtLabel: string;
};

export type DevicesState = {
  devices: DeviceItem[];
  total: number;
};

export type NotificationTypeCount = {
  type: string;
  label: string;
  count: number;
};

export type DingTalkConfigState = {
  configured: boolean;
  webhookConfigured: boolean;
  secretConfigured: boolean;
  triggers: string[];
  pendingDeliveryCount: number;
  deadDeliveryCount: number;
};

export type NotificationsState = {
  notifications: NotificationItem[];
  typeCounts: NotificationTypeCount[];
  activeType: string;
  total: number;
  dingtalk: DingTalkConfigState;
};

export type GeneralSettingsState = {
  connectorBaseUrlTemplate: string;
  source: string;
};

export type NotificationSettingsState = {
  enabled: boolean;
  configured: boolean;
  webhookConfigured: boolean;
  secretConfigured: boolean;
  timeoutSeconds: number;
  dispatchIntervalSeconds: number;
  pendingDeliveryCount: number;
  deadDeliveryCount: number;
  source: string;
};

export type RuntimeEnvironmentItem = {
  key: string;
  label: string;
  group: string;
  source: string;
  valueLabel: string;
  configured: boolean;
  sensitive: boolean;
  restartRequired: boolean;
};

export type SettingsState = {
  general: GeneralSettingsState;
  notifications: NotificationSettingsState;
  runtime: RuntimeEnvironmentItem[];
};

export type GeneralSettingsPayload = {
  connectorBaseUrlTemplate: string | null;
};

export type NotificationSettingsPayload = {
  enabled: boolean;
  webhookUrl: string | null;
  secret: string | null;
  timeoutSeconds: number;
  dispatchIntervalSeconds: number;
};

export type SettingsActionResponse = {
  message: string;
  state: SettingsState;
};

export type ApiDocParam = {
  name: string;
  location: string;
  required: string;
  description: string;
};

export type ApiDocEndpoint = {
  anchor: string;
  title: string;
  method: string;
  path: string;
  summary: string;
  params: ApiDocParam[];
  curl: string;
  response: string;
};

export type ApiDocsState = {
  endpoints: ApiDocEndpoint[];
  downloadUrl: string;
};

export class AdminApiError extends Error {
  code: string;
  detail: unknown;

  constructor(message: string, code = 'admin_api_error', detail: unknown = null) {
    super(message);
    this.name = 'AdminApiError';
    this.code = code;
    this.detail = detail;
  }
}

export async function getJson<T>(path: string): Promise<T> {
  const response = await fetch(path, {
    headers: { Accept: 'application/json' },
    cache: 'no-store'
  });
  const payload = await response.json().catch(() => null);
  if (!response.ok) {
    const error = payload?.error;
    throw new AdminApiError(
      error?.message || `请求失败：HTTP ${response.status}`,
      error?.code || 'http_error',
      error?.detail || null
    );
  }
  return payload as T;
}

export async function postJson<T>(path: string, body: unknown): Promise<T> {
  const response = await fetch(path, {
    method: 'POST',
    headers: {
      Accept: 'application/json',
      'Content-Type': 'application/json'
    },
    body: JSON.stringify(body)
  });
  const payload = await response.json().catch(() => null);
  if (!response.ok) {
    const error = payload?.error;
    throw new AdminApiError(
      error?.message || `请求失败：HTTP ${response.status}`,
      error?.code || 'http_error',
      error?.detail || null
    );
  }
  return payload as T;
}

export async function patchJson<T>(path: string, body: unknown): Promise<T> {
  const response = await fetch(path, {
    method: 'PATCH',
    headers: {
      Accept: 'application/json',
      'Content-Type': 'application/json'
    },
    body: JSON.stringify(body)
  });
  const payload = await response.json().catch(() => null);
  if (!response.ok) {
    const error = payload?.error;
    throw new AdminApiError(
      error?.message || `请求失败：HTTP ${response.status}`,
      error?.code || 'http_error',
      error?.detail || null
    );
  }
  return payload as T;
}

export async function putJson<T>(path: string, body: unknown): Promise<T> {
  const response = await fetch(path, {
    method: 'PUT',
    headers: {
      Accept: 'application/json',
      'Content-Type': 'application/json'
    },
    body: JSON.stringify(body)
  });
  const payload = await response.json().catch(() => null);
  if (!response.ok) {
    const error = payload?.error;
    throw new AdminApiError(
      error?.message || `请求失败：HTTP ${response.status}`,
      error?.code || 'http_error',
      error?.detail || null
    );
  }
  return payload as T;
}

export async function deleteJson<T>(path: string, body?: unknown): Promise<T> {
  const response = await fetch(path, {
    method: 'DELETE',
    headers: body === undefined
      ? { Accept: 'application/json' }
      : { Accept: 'application/json', 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body)
  });
  const payload = await response.json().catch(() => null);
  if (!response.ok) {
    const error = payload?.error;
    throw new AdminApiError(
      error?.message || `请求失败：HTTP ${response.status}`,
      error?.code || 'http_error',
      error?.detail || null
    );
  }
  return payload as T;
}

export async function postFormJson<T>(path: string, body: FormData): Promise<T> {
  const response = await fetch(path, {
    method: 'POST',
    headers: { Accept: 'application/json' },
    body
  });
  const payload = await response.json().catch(() => null);
  if (!response.ok) {
    const error = payload?.error;
    throw new AdminApiError(
      error?.message || `请求失败：HTTP ${response.status}`,
      error?.code || 'http_error',
      error?.detail || null
    );
  }
  return payload as T;
}

export function bootstrapAdmin(): Promise<BootstrapResponse> {
  return getJson<BootstrapResponse>('/admin/api/bootstrap');
}

export function loadDashboardState(): Promise<DashboardState> {
  return getJson<DashboardState>('/admin/api/dashboard');
}

export function loadStoreApps(pathAndQuery: string): Promise<StoreAppsState> {
  return getJson<StoreAppsState>(`/admin/api/store-apps${pathAndQuery}`);
}

export function loadLlmConfig(): Promise<LlmConfigState> {
  return getJson<LlmConfigState>('/admin/api/llm-config');
}

export function createLlmProfile(payload: LlmProfilePayload): Promise<LlmProfileSaveResponse> {
  return postJson<LlmProfileSaveResponse>('/admin/api/llm-config/profiles', payload);
}

export function updateLlmProfile(
  profileId: string,
  payload: LlmProfilePayload
): Promise<LlmProfileSaveResponse> {
  return patchJson<LlmProfileSaveResponse>(`/admin/api/llm-config/profiles/${profileId}`, payload);
}

export function updateLlmFeatureBinding(
  featureKey: string,
  payload: { primaryProfileId: string | null; fallbackProfileId?: string | null }
): Promise<LlmFeatureBindingSaveResponse> {
  return putJson<LlmFeatureBindingSaveResponse>(
    `/admin/api/llm-config/bindings/${featureKey}`,
    payload
  );
}

export function loadDeveloperAccounts(): Promise<DeveloperAccountsState> {
  return getJson<DeveloperAccountsState>('/admin/api/developer-accounts');
}

export function loadDeveloperAccount(accountId: string): Promise<DeveloperAccountDetailState> {
  return getJson<DeveloperAccountDetailState>(`/admin/api/developer-accounts/${accountId}`);
}

export function loadStoreWorkspace(
  accountId: string,
  appId: string,
  section: string,
  locale = ''
): Promise<StoreWorkspaceState> {
  const params = new URLSearchParams({ section });
  if (locale) params.set('locale', locale);
  return getJson<StoreWorkspaceState>(
    `/admin/api/developer-accounts/${accountId}/apps/${appId}/workspace?${params.toString()}`
  );
}

export function createDeveloperAccount(
  payload: DeveloperAccountForm
): Promise<DeveloperAccountSaveResponse> {
  return postJson<DeveloperAccountSaveResponse>('/admin/api/developer-accounts', payload);
}

export function updateDeveloperAccount(
  accountId: string,
  payload: DeveloperAccountForm
): Promise<DeveloperAccountSaveResponse> {
  return patchJson<DeveloperAccountSaveResponse>(
    `/admin/api/developer-accounts/${accountId}`,
    payload
  );
}

export function saveAccountConnector(
  accountId: string,
  payload: { name: string; baseUrl: string; authToken: string }
): Promise<ConnectorActionResponse> {
  return postJson<ConnectorActionResponse>(
    `/admin/api/developer-accounts/${accountId}/connector`,
    payload
  );
}

export function checkAccountConnector(accountId: string): Promise<ConnectorActionResponse> {
  return postJson<ConnectorActionResponse>(
    `/admin/api/developer-accounts/${accountId}/connector/check`,
    {}
  );
}

export function bindAccountApp(
  accountId: string,
  payload: { appId: string; storeAppId: string; storePackageName: string }
): Promise<AccountDetailActionResponse> {
  return postJson<AccountDetailActionResponse>(
    `/admin/api/developer-accounts/${accountId}/apps`,
    payload
  );
}

export function updateAccountAppSettings(
  accountId: string,
  appId: string,
  payload: { storeAppId: string; storePackageName: string }
): Promise<AccountDetailActionResponse> {
  return patchJson<AccountDetailActionResponse>(
    `/admin/api/developer-accounts/${accountId}/apps/${appId}/settings`,
    payload
  );
}

export function unbindAccountApp(
  accountId: string,
  appId: string
): Promise<AccountDetailActionResponse> {
  return deleteJson<AccountDetailActionResponse>(
    `/admin/api/developer-accounts/${accountId}/apps/${appId}`
  );
}

export function saveStoreWorkspaceMetadata(
  accountId: string,
  appId: string,
  payload: { version: string; locale: string; locales: StoreLocaleContentInput[] }
): Promise<StoreWorkspaceActionResponse> {
  return putJson<StoreWorkspaceActionResponse>(
    `/admin/api/developer-accounts/${accountId}/apps/${appId}/workspace/metadata`,
    payload
  );
}

export function saveStoreWorkspaceReleaseNotes(
  accountId: string,
  appId: string,
  payload: { version: string; locale: string; locales: StoreLocaleContentInput[] }
): Promise<StoreWorkspaceActionResponse> {
  return putJson<StoreWorkspaceActionResponse>(
    `/admin/api/developer-accounts/${accountId}/apps/${appId}/workspace/release-notes`,
    payload
  );
}

export function translateStoreText(
  payload: StoreTranslationRequest
): Promise<StoreTranslationResponse> {
  return postJson<StoreTranslationResponse>('/admin/api/store-translation', payload);
}

export function checkStoreWorkspacePreflight(
  accountId: string,
  appId: string,
  payload: { version: string; locale: string; syncScopes: string[]; locales: StoreLocaleContentInput[] }
): Promise<StoreWorkspaceActionResponse> {
  return postJson<StoreWorkspaceActionResponse>(
    `/admin/api/developer-accounts/${accountId}/apps/${appId}/workspace/metadata/preflight`,
    payload
  );
}

export function syncStoreWorkspaceMetadata(
  accountId: string,
  appId: string,
  payload: { version: string; locale: string; syncScopes: string[]; locales: StoreLocaleContentInput[] }
): Promise<StoreWorkspaceActionResponse> {
  return postJson<StoreWorkspaceActionResponse>(
    `/admin/api/developer-accounts/${accountId}/apps/${appId}/workspace/metadata/sync`,
    payload
  );
}

export function deleteStoreWorkspaceImage(
  accountId: string,
  appId: string,
  payload: { locale: string; slotKey: string; storageKey: string; version?: string }
): Promise<StoreWorkspaceActionResponse> {
  return deleteJson<StoreWorkspaceActionResponse>(
    `/admin/api/developer-accounts/${accountId}/apps/${appId}/workspace/metadata/store-images`,
    payload
  );
}

export function uploadStoreWorkspaceImages(
  accountId: string,
  appId: string,
  formData: FormData
): Promise<StoreWorkspaceActionResponse> {
  return postFormJson<StoreWorkspaceActionResponse>(
    `/admin/api/developer-accounts/${accountId}/apps/${appId}/workspace/metadata/store-images`,
    formData
  );
}

export function loadMarketingPage(
  accountId: string,
  appId: string,
  pageId: string,
  locale = ''
): Promise<MarketingPageDetailState> {
  const params = new URLSearchParams();
  if (locale) params.set('locale', locale);
  const query = params.toString();
  return getJson<MarketingPageDetailState>(
    `/admin/api/developer-accounts/${accountId}/apps/${appId}/workspace/marketing-pages/${pageId}${query ? `?${query}` : ''}`
  );
}

export function createMarketingPage(
  accountId: string,
  appId: string,
  payload: MarketingPagePayload
): Promise<MarketingPageActionResponse> {
  return postJson<MarketingPageActionResponse>(
    `/admin/api/developer-accounts/${accountId}/apps/${appId}/workspace/marketing-pages`,
    payload
  );
}

export function saveMarketingPage(
  accountId: string,
  appId: string,
  pageId: string,
  payload: MarketingPagePayload
): Promise<MarketingPageActionResponse> {
  return putJson<MarketingPageActionResponse>(
    `/admin/api/developer-accounts/${accountId}/apps/${appId}/workspace/marketing-pages/${pageId}`,
    payload
  );
}

export function copyMarketingPage(
  accountId: string,
  appId: string,
  pageId: string
): Promise<MarketingPageActionResponse> {
  return postJson<MarketingPageActionResponse>(
    `/admin/api/developer-accounts/${accountId}/apps/${appId}/workspace/marketing-pages/${pageId}/copy`,
    {}
  );
}

export function deleteMarketingPage(
  accountId: string,
  appId: string,
  pageId: string
): Promise<MarketingPageActionResponse> {
  return deleteJson<MarketingPageActionResponse>(
    `/admin/api/developer-accounts/${accountId}/apps/${appId}/workspace/marketing-pages/${pageId}`
  );
}

export function checkMarketingPagePreflight(
  accountId: string,
  appId: string,
  pageId: string,
  payload: MarketingPagePayload
): Promise<MarketingPageActionResponse> {
  return postJson<MarketingPageActionResponse>(
    `/admin/api/developer-accounts/${accountId}/apps/${appId}/workspace/marketing-pages/${pageId}/preflight`,
    payload
  );
}

export function syncMarketingPage(
  accountId: string,
  appId: string,
  pageId: string,
  payload: MarketingPagePayload
): Promise<MarketingPageActionResponse> {
  return postJson<MarketingPageActionResponse>(
    `/admin/api/developer-accounts/${accountId}/apps/${appId}/workspace/marketing-pages/${pageId}/sync`,
    payload
  );
}

export function deleteMarketingPageImage(
  accountId: string,
  appId: string,
  pageId: string,
  payload: { locale: string; slotKey: string; storageKey: string }
): Promise<MarketingPageActionResponse> {
  return deleteJson<MarketingPageActionResponse>(
    `/admin/api/developer-accounts/${accountId}/apps/${appId}/workspace/marketing-pages/${pageId}/store-images`,
    payload
  );
}

export function uploadMarketingPageImages(
  accountId: string,
  appId: string,
  pageId: string,
  formData: FormData
): Promise<MarketingPageActionResponse> {
  return postFormJson<MarketingPageActionResponse>(
    `/admin/api/developer-accounts/${accountId}/apps/${appId}/workspace/marketing-pages/${pageId}/store-images`,
    formData
  );
}

export function loadStoreReviews(pathAndQuery: string): Promise<StoreReviewsState> {
  return getJson<StoreReviewsState>(`/admin/api/store-reviews${pathAndQuery}`);
}

export function loadUploadState(): Promise<UploadState> {
  return getJson<UploadState>('/admin/api/uploads');
}

export function loadBuildsState(): Promise<BuildsState> {
  return getJson<BuildsState>('/admin/api/builds');
}

export function loadBuildAppsState(): Promise<BuildAppsState> {
  return getJson<BuildAppsState>('/admin/api/builds/apps');
}

export function loadBuildRunnersState(): Promise<BuildRunnersState> {
  return getJson<BuildRunnersState>('/admin/api/build-runners');
}

export function loadSettingsState(): Promise<SettingsState> {
  return getJson<SettingsState>('/admin/api/settings');
}

export function saveGeneralSettings(
  payload: GeneralSettingsPayload
): Promise<SettingsActionResponse> {
  return putJson<SettingsActionResponse>('/admin/api/settings/general', payload);
}

export function saveNotificationSettings(
  payload: NotificationSettingsPayload
): Promise<SettingsActionResponse> {
  return putJson<SettingsActionResponse>('/admin/api/settings/notifications', payload);
}

export function checkNotificationSettings(): Promise<SettingsActionResponse> {
  return postJson<SettingsActionResponse>('/admin/api/settings/notifications/check', {});
}

export function provisionBuildRunner(
  payload: RunnerProvisionPayload
): Promise<RunnerProvisionResponse> {
  return postJson<RunnerProvisionResponse>('/admin/api/build-runners/provision', payload);
}

export function loadAppDetailState(appId: string): Promise<AppDetailState> {
  return getJson<AppDetailState>(`/admin/api/apps/${encodeURIComponent(appId)}`);
}

export function createAgentBuild(
  appId: string,
  payload: AgentBuildCreateInput
): Promise<AppBuildActionResponse> {
  return postJson<AppBuildActionResponse>(
    `/admin/api/apps/${encodeURIComponent(appId)}/builds`,
    payload
  );
}

export function saveAppBuildSetting(
  appId: string,
  payload: BuildSettingSavePayload
): Promise<AppBuildActionResponse> {
  return putJson<AppBuildActionResponse>(
    `/admin/api/apps/${encodeURIComponent(appId)}/build-setting`,
    payload
  );
}

export function loadDevicesState(): Promise<DevicesState> {
  return getJson<DevicesState>('/admin/api/devices');
}

export function loadNotificationsState(pathAndQuery: string): Promise<NotificationsState> {
  return getJson<NotificationsState>(`/admin/api/notifications${pathAndQuery}`);
}

export function loadApiDocsState(): Promise<ApiDocsState> {
  return getJson<ApiDocsState>('/admin/api/api-docs');
}

export function loadAppLogs(): Promise<AppLogsState> {
  return getJson<AppLogsState>('/admin/api/app-logs');
}

export function loadAppLogEvents(cursor: number): Promise<AppLogsState> {
  return getJson<AppLogsState>(`/admin/api/app-logs/events?cursor=${cursor}`);
}

export function fetchStoreReviews(accountId: string, appId: string) {
  return postJson<StoreReviewActionResponse>('/admin/api/store-reviews/fetch', {
    accountId,
    appId
  });
}

export function analyzeStoreReviews(accountId: string, appId: string) {
  return postJson<StoreReviewActionResponse>('/admin/api/store-reviews/analyze', {
    accountId,
    appId
  });
}

export function uploadPackage(
  formData: FormData,
  onProgress: (percent: number) => void
): Promise<AdminUploadResponse> {
  return new Promise((resolve, reject) => {
    const request = new XMLHttpRequest();
    request.open('POST', '/admin/api/uploads');
    request.setRequestHeader('Accept', 'application/json');
    request.upload.addEventListener('progress', (event) => {
      if (event.lengthComputable && event.total > 0) {
        onProgress(Math.min(99, Math.round((event.loaded / event.total) * 100)));
      }
    });
    request.addEventListener('load', () => {
      const payload = parseJson(request.responseText);
      if (request.status < 200 || request.status >= 300) {
        const error = payload?.error;
        reject(
          new AdminApiError(
            error?.message || `请求失败：HTTP ${request.status}`,
            error?.code || 'http_error',
            error?.detail || null
          )
        );
        return;
      }
      onProgress(100);
      resolve(payload as AdminUploadResponse);
    });
    request.addEventListener('error', () => {
      reject(new AdminApiError('上传失败，请检查网络后重试', 'upload_network_error'));
    });
    request.send(formData);
  });
}

function parseJson(value: string): any {
  try {
    return JSON.parse(value);
  } catch {
    return null;
  }
}

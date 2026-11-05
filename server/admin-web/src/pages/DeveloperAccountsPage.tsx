import { useEffect, useMemo, useState, type FormEvent } from 'react';
import {
  AdminApiError,
  bindAccountApp,
  checkAccountConnector,
  checkMarketingPagePreflight,
  checkStoreWorkspacePreflight,
  copyMarketingPage,
  createMarketingPage,
  createDeveloperAccount,
  deleteMarketingPage,
  deleteMarketingPageImage,
  deleteStoreWorkspaceImage,
  loadDeveloperAccount,
  loadDeveloperAccounts,
  loadMarketingPage,
  loadStoreWorkspace,
  saveAccountConnector,
  saveMarketingPage,
  saveStoreWorkspaceMetadata,
  saveStoreWorkspaceReleaseNotes,
  syncMarketingPage,
  syncStoreWorkspaceMetadata,
  translateStoreText,
  unbindAccountApp,
  updateAccountAppSettings,
  updateDeveloperAccount,
  uploadMarketingPageImages,
  uploadStoreWorkspaceImages,
  type AccountAppItem,
  type DeveloperAccountDetailState,
  type DeveloperAccountForm,
  type DeveloperAccountSummary,
  type DeveloperAccountsState,
  type MarketingPageDetailState,
  type MarketingPageLocaleContent,
  type MarketingPageLocaleInput,
  type MarketingPagePayload,
  type StoreLocaleContent,
  type StoreLocaleContentInput,
  type StoreMarketingPageSummary,
  type StoreWorkspaceState
} from '../app/apiClient';

type StoreImageUploadRequest = { locale: string; slotKey: string; files: File[] };
type StoreTextField = 'promotionalText' | 'description' | 'releaseNotes';
type SyncScopeOption = { value: string; label: string; description: string };
type PendingSyncConfirmation =
  | {
      kind: 'workspace';
      title: string;
      targetLabel: string;
      versionLabel: string;
      scopeOptions: SyncScopeOption[];
      localeLabels: string[];
      locales: StoreLocaleContentInput[];
      syncScopes: string[];
    }
  | {
      kind: 'marketing';
      title: string;
      targetLabel: string;
      versionLabel: string;
      scopeOptions: SyncScopeOption[];
      localeLabels: string[];
      syncScopes: string[];
      pageId: string;
      payload: MarketingPagePayload;
    };

type AccountRoute =
  | { kind: 'list' }
  | { kind: 'new' }
  | { kind: 'detail'; accountId: string }
  | { kind: 'edit'; accountId: string }
  | { kind: 'app'; accountId: string; appId: string; section: StoreSection; pageId?: string };

type StoreSection = 'store' | 'marketing' | 'connection';

const accountStatuses = [
  { value: 'ok', label: '正常' },
  { value: 'renewal_due', label: '需要续费' },
  { value: 'expired', label: '已过期' },
  { value: 'disabled', label: '已停用' }
];

const storeSections: Array<{ key: StoreSection; label: string; description: string }> = [
  { key: 'store', label: '默认商店页', description: '编辑当前商店页的版本说明、宣传文本、描述和截图。同步前再选择本次提交范围。' },
  { key: 'marketing', label: '营销页面', description: '自定义产品页面和产品页面优化' },
  { key: 'connection', label: '商店连接', description: 'Connector、商店标识和连接检查' }
];

const syncScopeLabels: Record<string, string> = {
  metadata: '宣传文本和描述',
  store_images: '商店图',
  release_notes: '版本说明',
  marketing_text: '营销页文案'
};

const syncScopeDescriptions: Record<string, string> = {
  metadata: '同步当前商店页的宣传文本和描述。',
  store_images: '同步当前草稿中的手机截图和平板截图。',
  release_notes: '同步当前商店最新版本的版本说明。',
  marketing_text: '同步当前营销页面的宣传文本。'
};

export function DeveloperAccountsPage() {
  const [route, setRoute] = useState<AccountRoute>(() => parseAccountRoute(location.pathname));
  const [accountsState, setAccountsState] = useState<DeveloperAccountsState | null>(null);
  const [detailState, setDetailState] = useState<DeveloperAccountDetailState | null>(null);
  const [loading, setLoading] = useState(false);
  const [message, setMessage] = useState('');
  const [error, setError] = useState('');

  useEffect(() => {
    void loadFromLocation();
    const onPopState = () => void loadFromLocation();
    window.addEventListener('popstate', onPopState);
    return () => window.removeEventListener('popstate', onPopState);
  }, []);

  async function loadFromLocation() {
    const nextRoute = parseAccountRoute(location.pathname);
    setRoute(nextRoute);
    setLoading(true);
    setError('');
    try {
      if (nextRoute.kind === 'list' || nextRoute.kind === 'new') {
        setDetailState(null);
        setAccountsState(await loadDeveloperAccounts());
      } else {
        setDetailState(await loadDeveloperAccount(nextRoute.accountId));
      }
    } catch (requestError) {
      setError(errorMessage(requestError));
    } finally {
      setLoading(false);
    }
  }

  function navigate(path: string) {
    history.pushState({ adminRoute: 'accounts' }, '', path);
    void loadFromLocation();
  }

  function showMessage(value: string) {
    setMessage(value);
    window.setTimeout(() => setMessage(''), 2400);
  }

  async function submitAccount(payload: DeveloperAccountForm, accountId?: string) {
    setError('');
    try {
      const response = accountId
        ? await updateDeveloperAccount(accountId, payload)
        : await createDeveloperAccount(payload);
      setAccountsState(response.state);
      showMessage(response.message);
      navigate(response.account.detailPath);
    } catch (requestError) {
      setError(errorMessage(requestError));
    }
  }

  async function refreshDetail(responseMessage?: string) {
    const nextRoute = parseAccountRoute(location.pathname);
    if (nextRoute.kind === 'detail' || nextRoute.kind === 'edit' || nextRoute.kind === 'app') {
      setDetailState(await loadDeveloperAccount(nextRoute.accountId));
      if (responseMessage) showMessage(responseMessage);
    }
  }

  async function saveConnector(payload: { name: string; baseUrl: string; authToken: string }) {
    if (!detailState) return;
    setError('');
    try {
      const response = await saveAccountConnector(detailState.account.id, payload);
      setDetailState(response.state);
      showMessage(response.message);
    } catch (requestError) {
      setError(errorMessage(requestError));
    }
  }

  async function checkConnector() {
    if (!detailState) return;
    setError('');
    try {
      const response = await checkAccountConnector(detailState.account.id);
      setDetailState(response.state);
      showMessage(response.message);
    } catch (requestError) {
      setError(errorMessage(requestError));
    }
  }

  async function bindApp(payload: { appId: string; storeAppId: string; storePackageName: string }) {
    if (!detailState) return;
    setError('');
    try {
      const response = await bindAccountApp(detailState.account.id, payload);
      setDetailState(response.state);
      showMessage(response.message);
    } catch (requestError) {
      setError(errorMessage(requestError));
    }
  }

  async function saveAppSettings(
    app: AccountAppItem,
    payload: { storeAppId: string; storePackageName: string }
  ) {
    if (!detailState) return;
    setError('');
    try {
      const response = await updateAccountAppSettings(detailState.account.id, app.id, payload);
      setDetailState(response.state);
      showMessage(response.message);
    } catch (requestError) {
      setError(errorMessage(requestError));
    }
  }

  async function removeApp(app: AccountAppItem) {
    if (!detailState) return;
    setError('');
    try {
      const response = await unbindAccountApp(detailState.account.id, app.id);
      setDetailState(response.state);
      showMessage(response.message);
      await refreshDetail();
    } catch (requestError) {
      setError(errorMessage(requestError));
    }
  }

  const hasLoadedRouteContent =
    route.kind === 'list' || route.kind === 'new' ? Boolean(accountsState) : Boolean(detailState);

  return (
    <div className="accounts-page">
      {loading && hasLoadedRouteContent ? (
        <div className="page-busy-pill" role="status" aria-live="polite">
          加载中
        </div>
      ) : null}
      {message ? <div className="notice ok">{message}</div> : null}
      {error ? <div className="notice error">{error}</div> : null}
      {loading && !hasLoadedRouteContent ? (
        <div className="empty-state">正在加载开发者账号...</div>
      ) : null}

      {route.kind === 'list' ? (
        <AccountsList state={accountsState} onNavigate={navigate} />
      ) : null}
      {route.kind === 'new' ? (
        <AccountEditor
          title="新增开发者账号"
          onCancel={() => navigate('/admin/accounts')}
          onSubmit={(payload) => void submitAccount(payload)}
        />
      ) : null}
      {route.kind === 'detail' && detailState ? (
        <AccountDetail
          state={detailState}
          onNavigate={navigate}
          onCheckConnector={() => void checkConnector()}
          onSaveConnector={(payload) => void saveConnector(payload)}
          onBindApp={(payload) => void bindApp(payload)}
          onSaveAppSettings={(app, payload) => void saveAppSettings(app, payload)}
          onRemoveApp={(app) => void removeApp(app)}
        />
      ) : null}
      {route.kind === 'edit' && detailState ? (
        <AccountEditor
          title="编辑开发者账号"
          account={detailState.account}
          onCancel={() => navigate(`/admin/accounts/${detailState.account.id}`)}
          onSubmit={(payload) => void submitAccount(payload, detailState.account.id)}
        />
      ) : null}
      {route.kind === 'app' && detailState ? (
        <StoreWorkspace
          state={detailState}
          appId={route.appId}
          section={route.section}
          pageId={route.pageId}
          onNavigate={navigate}
          onCheckConnector={() => void checkConnector()}
          onSaveConnector={(payload) => void saveConnector(payload)}
          onSaveAppSettings={(app, payload) => void saveAppSettings(app, payload)}
        />
      ) : null}
    </div>
  );
}

function AccountsList({
  state,
  onNavigate
}: {
  state: DeveloperAccountsState | null;
  onNavigate: (path: string) => void;
}) {
  const accounts = state?.accounts ?? [];
  return (
    <div className="accounts-layout">
      <section className="panel accounts-table-panel">
        <div className="panel-head compact">
          <strong>开发者账号</strong>
          <button className="button primary" type="button" onClick={() => onNavigate('/admin/accounts/new')}>
            新增账号
          </button>
        </div>
        <div className="account-stats-row">
          <Metric label="账号" value={state?.stats.total ?? 0} />
          <Metric label="正常" value={state?.stats.ok ?? 0} />
          <Metric label="需处理" value={state?.stats.needs ?? 0} />
          <Metric label="绑定应用" value={state?.stats.boundApps ?? 0} />
        </div>
        <div className="account-table">
          <div className="account-row header">
            <span>账号</span>
            <span>应用</span>
            <span>Connector</span>
            <span>到期</span>
            <span>操作</span>
          </div>
          {accounts.map((account) => (
            <button
              key={account.id}
              className="account-row"
              type="button"
              onClick={() => onNavigate(account.detailPath)}
            >
              <span>
                <strong>{account.teamName}</strong>
                <small>{account.id}</small>
              </span>
              <span>{account.appNames.length ? account.appNames.join('、') : '未绑定 App'}</span>
              <span className={`tag ${account.connectorStatus === 'ok' ? 'ok' : 'warn'}`}>
                {account.connectorStatusLabel}
              </span>
              <span>{account.expiresAtLabel}</span>
              <span className="button slim">打开</span>
            </button>
          ))}
        </div>
        {!accounts.length ? <div className="empty-state">还没有开发者账号。</div> : null}
      </section>
    </div>
  );
}

function AccountDetail({
  state,
  onNavigate,
  onCheckConnector,
  onSaveConnector,
  onBindApp,
  onSaveAppSettings,
  onRemoveApp
}: {
  state: DeveloperAccountDetailState;
  onNavigate: (path: string) => void;
  onCheckConnector: () => void;
  onSaveConnector: (payload: { name: string; baseUrl: string; authToken: string }) => void;
  onBindApp: (payload: { appId: string; storeAppId: string; storePackageName: string }) => void;
  onSaveAppSettings: (
    app: AccountAppItem,
    payload: { storeAppId: string; storePackageName: string }
  ) => void;
  onRemoveApp: (app: AccountAppItem) => void;
}) {
  return (
    <div className="account-detail-layout">
      <section className="panel account-overview-card">
        <div className="panel-head compact">
          <div>
            <strong>{state.account.teamName}</strong>
            <p className="muted">{state.account.id}</p>
          </div>
          <button
            className="button"
            type="button"
            onClick={() => onNavigate(`/admin/accounts/${state.account.id}/edit`)}
          >
            编辑账号
          </button>
        </div>
        <div className="account-stats-row">
          <Metric label="账号状态" value={state.account.statusLabel} />
          <Metric label="到期时间" value={state.account.expiresAtLabel} />
          <Metric label="绑定应用" value={state.apps.length} />
          <Metric label="Connector" value={state.connector?.statusLabel ?? '未配置'} />
        </div>
      </section>

      <section className="panel connector-panel">
        <div className="panel-head compact">
          <strong>Connector</strong>
          <button className="button" type="button" onClick={onCheckConnector}>
            检查连接
          </button>
        </div>
        <ConnectorForm connector={state.connector} onSubmit={onSaveConnector} />
      </section>

      <section className="panel account-apps-panel">
        <div className="panel-head compact">
          <strong>已绑定 App</strong>
          <span>{state.apps.length} 个</span>
        </div>
        <div className="account-app-list">
          {state.apps.map((app) => (
            <BoundAppCard
              key={app.id}
              accountId={state.account.id}
              app={app}
              onNavigate={onNavigate}
              onSave={(payload) => onSaveAppSettings(app, payload)}
              onRemove={() => onRemoveApp(app)}
            />
          ))}
        </div>
        {!state.apps.length ? <div className="empty-state">这个账号还没有绑定 App。</div> : null}
      </section>

      <section className="panel">
        <div className="panel-head compact">
          <strong>绑定新 App</strong>
          <span>{state.unassignedApps.length} 个可选</span>
        </div>
        <BindAppForm state={state} onSubmit={onBindApp} />
      </section>

      <section className="panel">
        <div className="panel-head compact">
          <strong>最近同步</strong>
          <span>{state.syncRuns.length} 条</span>
        </div>
        <div className="sync-run-list">
          {state.syncRuns.map((run) => (
            <div key={run.id} className="sync-run-row">
              <strong>{run.operation || '同步任务'}</strong>
              <span>{run.status}</span>
              <span>{run.startedAtLabel}</span>
              <small>{run.summary}</small>
            </div>
          ))}
        </div>
        {!state.syncRuns.length ? <div className="empty-state">暂无同步记录。</div> : null}
      </section>
    </div>
  );
}

function StoreWorkspace({
  state,
  appId,
  section,
  pageId,
  onNavigate,
  onCheckConnector,
  onSaveConnector,
  onSaveAppSettings
}: {
  state: DeveloperAccountDetailState;
  appId: string;
  section: StoreSection;
  pageId?: string;
  onNavigate: (path: string) => void;
  onCheckConnector: () => void;
  onSaveConnector: (payload: { name: string; baseUrl: string; authToken: string }) => void;
  onSaveAppSettings: (
    app: AccountAppItem,
    payload: { storeAppId: string; storePackageName: string }
  ) => void;
}) {
  const app = state.apps.find((item) => item.id === appId) ?? null;
  const sectionLabel = storeSections.find((item) => item.key === section)?.label ?? '商店工作区';
  const [workspace, setWorkspace] = useState<StoreWorkspaceState | null>(null);
  const [marketingPage, setMarketingPage] = useState<MarketingPageDetailState | null>(null);
  const [workspaceError, setWorkspaceError] = useState('');
  const [workspaceNotice, setWorkspaceNotice] = useState('');
  const [workspaceLoading, setWorkspaceLoading] = useState(false);
  const [marketingPageLoading, setMarketingPageLoading] = useState(false);
  const [workspaceBusy, setWorkspaceBusy] = useState(false);
  const [syncBusy, setSyncBusy] = useState(false);
  const [syncConfirmation, setSyncConfirmation] = useState<PendingSyncConfirmation | null>(null);
  const [syncConfirmationError, setSyncConfirmationError] = useState('');

  useEffect(() => {
    let active = true;
    setWorkspaceError('');
    setWorkspaceLoading(true);
    if (section !== 'marketing' || !pageId) setMarketingPage(null);
    setSyncConfirmation(null);
    setSyncConfirmationError('');
    loadStoreWorkspace(state.account.id, appId, section)
      .then((payload) => {
        if (active) setWorkspace(payload);
      })
      .catch((requestError) => {
        if (active) setWorkspaceError(errorMessage(requestError));
      })
      .finally(() => {
        if (active) setWorkspaceLoading(false);
      });
    return () => {
      active = false;
    };
  }, [appId, pageId, section, state.account.id]);

  useEffect(() => {
    if (!pageId || section !== 'marketing') {
      setMarketingPageLoading(false);
      return;
    }
    let active = true;
    setWorkspaceError('');
    setMarketingPageLoading(true);
    loadMarketingPage(state.account.id, appId, pageId)
      .then((payload) => {
        if (active) setMarketingPage(payload);
      })
      .catch((requestError) => {
        if (active) setWorkspaceError(errorMessage(requestError));
      })
      .finally(() => {
        if (active) setMarketingPageLoading(false);
      });
    return () => {
      active = false;
    };
  }, [appId, pageId, section, state.account.id]);

  function showWorkspaceNotice(value: string) {
    setWorkspaceNotice(value);
    window.setTimeout(() => setWorkspaceNotice(''), 2400);
  }

  async function saveMetadata(locales: StoreLocaleContentInput[]) {
    if (!workspace) return;
    setWorkspaceBusy(true);
    setWorkspaceError('');
    try {
      const response = await saveStoreWorkspaceMetadata(state.account.id, appId, {
        version: workspace.version,
        locale: workspace.locale || workspace.sourceLocale,
        locales
      });
      setWorkspace(response.state);
      showWorkspaceNotice(response.message);
    } catch (requestError) {
      setWorkspaceError(errorMessage(requestError));
    } finally {
      setWorkspaceBusy(false);
    }
  }

  async function saveReleaseNotes(locales: StoreLocaleContentInput[]) {
    if (!workspace) return;
    setWorkspaceBusy(true);
    setWorkspaceError('');
    try {
      const response = await saveStoreWorkspaceReleaseNotes(state.account.id, appId, {
        version: workspace.version,
        locale: workspace.locale || workspace.sourceLocale,
        locales
      });
      setWorkspace(response.state);
      showWorkspaceNotice(response.message);
    } catch (requestError) {
      setWorkspaceError(errorMessage(requestError));
    } finally {
      setWorkspaceBusy(false);
    }
  }

  async function saveDefaultStore(locales: StoreLocaleContentInput[]) {
    if (!workspace) return;
    setWorkspaceBusy(true);
    setWorkspaceError('');
    try {
      await saveStoreWorkspaceMetadata(state.account.id, appId, {
        version: workspace.version,
        locale: workspace.locale || workspace.sourceLocale,
        locales
      });
      const response = await saveStoreWorkspaceReleaseNotes(state.account.id, appId, {
        version: workspace.version,
        locale: workspace.locale || workspace.sourceLocale,
        locales
      });
      setWorkspace(response.state);
      showWorkspaceNotice(response.message);
    } catch (requestError) {
      setWorkspaceError(errorMessage(requestError));
    } finally {
      setWorkspaceBusy(false);
    }
  }

  async function checkPreflight(locales: StoreLocaleContentInput[], syncScopes: string[]) {
    if (!workspace) return;
    setWorkspaceBusy(true);
    setWorkspaceError('');
    try {
      const response = await checkStoreWorkspacePreflight(state.account.id, appId, {
        version: workspace.version,
        locale: workspace.locale || workspace.sourceLocale,
        syncScopes,
        locales
      });
      setWorkspace(response.state);
      showWorkspaceNotice(response.message);
    } catch (requestError) {
      setWorkspaceError(errorMessage(requestError));
    } finally {
      setWorkspaceBusy(false);
    }
  }

  async function syncWorkspace(locales: StoreLocaleContentInput[], syncScopes: string[]) {
    if (!workspace) return;
    setSyncConfirmationError('');
    const onlyReleaseNotes = syncScopes.length === 1 && syncScopes[0] === 'release_notes';
    setSyncConfirmation({
      kind: 'workspace',
      title: onlyReleaseNotes ? '同步版本说明' : '同步商店页',
      targetLabel: app?.name ?? appId,
      versionLabel: workspace.version || '未确认',
      scopeOptions: syncScopeOptions(syncScopes),
      localeLabels: locales.map((item) => item.locale),
      locales,
      syncScopes
    });
  }

  async function confirmSync(selectedScopes: string[]) {
    if (!syncConfirmation) return;
    if (selectedScopes.length === 0) {
      setSyncConfirmationError('请选择至少一个同步项目。');
      return;
    }
    setWorkspaceBusy(true);
    setSyncBusy(true);
    setWorkspaceError('');
    setSyncConfirmationError('');
    try {
      if (syncConfirmation.kind === 'workspace') {
        if (!workspace) return;
        const response = await syncStoreWorkspaceMetadata(state.account.id, appId, {
          version: workspace.version,
          locale: workspace.locale || workspace.sourceLocale,
          syncScopes: selectedScopes,
          locales: syncConfirmation.locales
        });
        setWorkspace(response.state);
        showWorkspaceNotice(response.message);
      } else {
        const response = await syncMarketingPage(
          state.account.id,
          appId,
          syncConfirmation.pageId,
          { ...syncConfirmation.payload, syncScopes: selectedScopes }
        );
        if (response.workspace) setWorkspace(response.workspace);
        if (response.state) setMarketingPage(response.state);
        showWorkspaceNotice(response.message);
      }
      setSyncConfirmation(null);
    } catch (requestError) {
      const message = errorMessage(requestError);
      setWorkspaceError(message);
      setSyncConfirmationError(message);
    } finally {
      setWorkspaceBusy(false);
      setSyncBusy(false);
    }
  }

  async function deleteImage(payload: { locale: string; slotKey: string; storageKey: string }) {
    if (!workspace) return;
    setWorkspaceBusy(true);
    setWorkspaceError('');
    try {
      const response = await deleteStoreWorkspaceImage(state.account.id, appId, {
        ...payload,
        version: workspace.version
      });
      setWorkspace(response.state);
      showWorkspaceNotice(response.message);
    } catch (requestError) {
      setWorkspaceError(errorMessage(requestError));
    } finally {
      setWorkspaceBusy(false);
    }
  }

  async function uploadImages(payload: StoreImageUploadRequest) {
    if (!workspace || payload.files.length === 0) return;
    setWorkspaceBusy(true);
    setWorkspaceError('');
    try {
      const response = await uploadStoreWorkspaceImages(
        state.account.id,
        appId,
        storeImageFormData(payload)
      );
      setWorkspace(response.state);
      showWorkspaceNotice(response.message);
    } catch (requestError) {
      setWorkspaceError(errorMessage(requestError));
    } finally {
      setWorkspaceBusy(false);
    }
  }

  async function createMarketing(payload: MarketingPagePayload) {
    setWorkspaceBusy(true);
    setWorkspaceError('');
    try {
      const response = await createMarketingPage(state.account.id, appId, payload);
      if (response.workspace) setWorkspace(response.workspace);
      if (response.state) {
        setMarketingPage(response.state);
        onNavigate(response.state.page.detailPath);
      }
      showWorkspaceNotice(response.message);
    } catch (requestError) {
      setWorkspaceError(errorMessage(requestError));
    } finally {
      setWorkspaceBusy(false);
    }
  }

  async function saveMarketing(payload: MarketingPagePayload) {
    if (!marketingPage) return;
    setWorkspaceBusy(true);
    setWorkspaceError('');
    try {
      const response = await saveMarketingPage(
        state.account.id,
        appId,
        marketingPage.page.pageId,
        payload
      );
      if (response.workspace) setWorkspace(response.workspace);
      if (response.state) setMarketingPage(response.state);
      showWorkspaceNotice(response.message);
    } catch (requestError) {
      setWorkspaceError(errorMessage(requestError));
    } finally {
      setWorkspaceBusy(false);
    }
  }

  async function copyMarketing() {
    if (!marketingPage) return;
    setWorkspaceBusy(true);
    setWorkspaceError('');
    try {
      const response = await copyMarketingPage(state.account.id, appId, marketingPage.page.pageId);
      if (response.workspace) setWorkspace(response.workspace);
      if (response.state) {
        setMarketingPage(response.state);
        onNavigate(response.state.page.detailPath);
      }
      showWorkspaceNotice(response.message);
    } catch (requestError) {
      setWorkspaceError(errorMessage(requestError));
    } finally {
      setWorkspaceBusy(false);
    }
  }

  async function removeMarketing() {
    if (!marketingPage) return;
    if (!window.confirm(`确认删除「${marketingPage.page.pageName}」？只会删除中心后台草稿。`)) {
      return;
    }
    setWorkspaceBusy(true);
    setWorkspaceError('');
    try {
      const response = await deleteMarketingPage(state.account.id, appId, marketingPage.page.pageId);
      if (response.workspace) setWorkspace(response.workspace);
      setMarketingPage(null);
      showWorkspaceNotice(response.message);
      onNavigate(`/admin/accounts/${state.account.id}/apps/${appId}/marketing`);
    } catch (requestError) {
      setWorkspaceError(errorMessage(requestError));
    } finally {
      setWorkspaceBusy(false);
    }
  }

  async function checkMarketing(payload: MarketingPagePayload) {
    if (!marketingPage) return;
    setWorkspaceBusy(true);
    setWorkspaceError('');
    try {
      const response = await checkMarketingPagePreflight(
        state.account.id,
        appId,
        marketingPage.page.pageId,
        payload
      );
      if (response.state) setMarketingPage(response.state);
      showWorkspaceNotice(response.message);
    } catch (requestError) {
      setWorkspaceError(errorMessage(requestError));
    } finally {
      setWorkspaceBusy(false);
    }
  }

  async function syncMarketing(payload: MarketingPagePayload) {
    if (!marketingPage) return;
    setSyncConfirmationError('');
    setSyncConfirmation({
      kind: 'marketing',
      title: '同步营销页面',
      targetLabel: marketingPage.page.pageName,
      versionLabel: marketingPage.page.applePageIdLabel || '未同步',
      scopeOptions: syncScopeOptions(payload.syncScopes ?? []),
      localeLabels: payload.locales.map((item) => item.locale),
      syncScopes: payload.syncScopes ?? [],
      pageId: marketingPage.page.pageId,
      payload
    });
  }

  async function deleteMarketingImage(payload: { locale: string; slotKey: string; storageKey: string }) {
    if (!marketingPage) return;
    setWorkspaceBusy(true);
    setWorkspaceError('');
    try {
      const response = await deleteMarketingPageImage(
        state.account.id,
        appId,
        marketingPage.page.pageId,
        payload
      );
      if (response.state) setMarketingPage(response.state);
      if (response.workspace) setWorkspace(response.workspace);
      showWorkspaceNotice(response.message);
    } catch (requestError) {
      setWorkspaceError(errorMessage(requestError));
    } finally {
      setWorkspaceBusy(false);
    }
  }

  async function uploadMarketingImages(payload: StoreImageUploadRequest) {
    if (!marketingPage || payload.files.length === 0) return;
    setWorkspaceBusy(true);
    setWorkspaceError('');
    try {
      const response = await uploadMarketingPageImages(
        state.account.id,
        appId,
        marketingPage.page.pageId,
        storeImageFormData(payload)
      );
      if (response.state) setMarketingPage(response.state);
      if (response.workspace) setWorkspace(response.workspace);
      showWorkspaceNotice(response.message);
    } catch (requestError) {
      setWorkspaceError(errorMessage(requestError));
    } finally {
      setWorkspaceBusy(false);
    }
  }

  if (!app) {
    return <div className="notice error">当前账号下没有这个 App。</div>;
  }

  return (
    <div className="compact-page compact-store-editor store-workspace-page">
      <div className="compact-context">
        <div className="compact-title">
          <strong>Store Content</strong>
          <h2>应用商店管理</h2>
          <span>
            {app.name} · {app.bundleIdentifier} · {app.platformLabel} · 当前草稿 · {workspace?.supportedLocales.length ?? 0} 个语言 · 当前商店最新版本（含未发布）：{workspace?.version || app.latestVersionLabel || '未确认'} · {workspace?.preflightLabel || '等待检查'}
          </span>
        </div>
        <div className="compact-actions">
          {workspaceLoading || marketingPageLoading ? (
            <span className="inline-busy-pill" role="status" aria-live="polite">
              加载中
            </span>
          ) : null}
          {section !== 'store' ? (
            <button className="button" type="button" onClick={() => onNavigate('/admin/apps')}>
              商店应用
            </button>
          ) : null}
          {section === 'connection' ? (
            <button className="button" type="button" onClick={() => onNavigate(`/admin/accounts/${state.account.id}`)}>
              返回账号
            </button>
          ) : null}
          {section === 'connection' ? (
            <button className="button primary" type="button" onClick={onCheckConnector}>
              检查连接
            </button>
          ) : null}
        </div>
      </div>

      <div className="compact-body">
        <div className="compact-editor-grid">
          <aside className="module-nav compact-editor-nav" aria-label="应用商店模块">
            <div className="module-head">
              <strong>应用模块</strong>
              <span>只保留当前应用相关操作</span>
            </div>
            <div className="module-list">
              {storeSections.map((item) => (
                <button
                  key={item.key}
                  className={item.key === section ? 'module-link active' : 'module-link'}
                  type="button"
                  onClick={() => onNavigate(`/admin/accounts/${state.account.id}/apps/${app.id}/${item.key}`)}
                >
                  {storeModuleIcon(item.key)}
                  <span>{item.label}</span>
                </button>
              ))}
            </div>
          </aside>

          <section className="content compact-editor-content">
            <div className="content-head">
              <div>
                <h2>{sectionLabel}</h2>
                <p>{storeSections.find((item) => item.key === section)?.description}</p>
              </div>
              <div className="compact-editor-meta">
                <span>当前语言：<strong>{workspace?.locale || workspace?.sourceLocale || '-'}</strong></span>
                <span>{workspace?.supportedLocales.length ?? 0} 个语言</span>
              </div>
            </div>

            <div className="compact-editor-panel">
              {(workspaceLoading || marketingPageLoading) && workspace ? (
                <div className="workspace-busy-pill" role="status" aria-live="polite">
                  正在刷新
                </div>
              ) : null}
              {section === 'connection' ? (
                <div className="connection-workspace">
                  <ConnectorForm connector={state.connector} onSubmit={onSaveConnector} />
                  <BoundAppSettings app={app} onSave={(payload) => onSaveAppSettings(app, payload)} />
                </div>
              ) : null}

              {workspaceError ? <div className="notice error">{workspaceError}</div> : null}
              {workspaceNotice ? <div className="notice ok">{workspaceNotice}</div> : null}
              {!workspace && workspaceLoading && !workspaceError && section !== 'connection' ? (
                <div className="workspace-loading-card" role="status" aria-live="polite">
                  <span />
                  <span />
                  <span />
                </div>
              ) : null}

              {workspace && section === 'store' ? (
                <StoreDefaultPanel
                  workspace={workspace}
                  busy={workspaceBusy}
                  syncing={syncBusy}
                  onBackToApps={() => onNavigate('/admin/apps')}
                  onSave={saveDefaultStore}
                  onCheck={checkPreflight}
                  onSync={syncWorkspace}
                  onDeleteImage={deleteImage}
                  onUploadImages={uploadImages}
                />
              ) : null}
              {workspace && section === 'marketing' ? (
                pageId && !marketingPage && marketingPageLoading ? (
                  <div className="workspace-loading-card" role="status" aria-live="polite">
                    <span />
                    <span />
                    <span />
                  </div>
                ) : pageId ? (
                  <MarketingPageDetailPanel
                    detail={marketingPage}
                    busy={workspaceBusy}
                    syncing={syncBusy}
                    onBack={() => onNavigate(`/admin/accounts/${state.account.id}/apps/${app.id}/marketing`)}
                    onSave={saveMarketing}
                    onCheck={checkMarketing}
                    onSync={syncMarketing}
                    onCopy={() => void copyMarketing()}
                    onDelete={() => void removeMarketing()}
                    onDeleteImage={deleteMarketingImage}
                    onUploadImages={uploadMarketingImages}
                  />
                ) : (
                  <MarketingPagesPanel
                    workspace={workspace}
                    busy={workspaceBusy}
                    onNavigate={onNavigate}
                    onCreate={createMarketing}
                  />
                )
              ) : null}
            </div>
          </section>

          <StoreWorkspaceSide
            account={state.account}
            app={app}
            connector={state.connector}
            workspace={workspace}
            marketingPage={marketingPage}
            section={section}
          />
        </div>

        {syncConfirmation ? (
          <SyncConfirmDialog
            confirmation={syncConfirmation}
            busy={syncBusy}
            error={syncConfirmationError}
            onCancel={() => {
              if (syncBusy) return;
              setSyncConfirmation(null);
              setSyncConfirmationError('');
            }}
            onConfirm={(selectedScopes) => void confirmSync(selectedScopes)}
          />
        ) : null}
      </div>
    </div>
  );
}

function StoreWorkspaceSide({
  account,
  app,
  connector,
  workspace,
  marketingPage,
  section
}: {
  account: DeveloperAccountSummary;
  app: AccountAppItem;
  connector: DeveloperAccountDetailState['connector'];
  workspace: StoreWorkspaceState | null;
  marketingPage: MarketingPageDetailState | null;
  section: StoreSection;
}) {
  const preflightLabel = section === 'marketing'
    ? marketingPage?.preflightLabel || workspace?.preflightLabel
    : workspace?.preflightLabel;
  const preflightStatus = section === 'marketing'
    ? marketingPage?.preflightStatus || workspace?.preflightStatus
    : workspace?.preflightStatus;
  const connectorOk = connector?.status === 'ok';
  const syncScopeSummary = section === 'marketing'
    ? '默认勾选文案和商店图；点击同步时可在确认清单里调整。'
    : '默认勾选版本说明、宣传文本、描述和商店图；点击同步时可在确认清单里调整。';
  return (
    <aside className="compact-editor-side">
      <div className="compact-column-head">
        <strong>{section === 'connection' ? '连接状态' : '同步前检查'}</strong>
        <span className={`tag ${preflightStatus === 'ok' ? 'ok' : 'warn'}`}>
          {preflightLabel || '等待检查'}
        </span>
      </div>
      <div className="compact-side-list">
        {section === 'connection' ? (
          <>
            <div className="compact-side-card">
              <strong>开发者账号</strong>
              <span>{account.teamName} · {account.statusLabel}</span>
              <span>{account.expiresAtLabel}</span>
            </div>
            <div className="compact-side-card">
              <strong>商店标识</strong>
              <span>{app.platform === 'ios' ? `App ID：${app.storeAppId || '未填写'}` : `Package：${app.storePackageName || app.bundleIdentifier}`}</span>
            </div>
          </>
        ) : (
          <>
            <div className="compact-side-card">
              <strong>商店版本</strong>
              <span>
                {preflightStatus === 'ok'
                  ? `当前商店最新版本（含未发布）：${workspace?.version || app.latestVersionLabel || '未确认'}。`
                  : '商店版本还没有创建，暂时不能同步。点击同步时会再次弹出确认清单。'}
              </span>
              <span className={`tag ${preflightStatus === 'ok' ? 'ok' : 'warn'}`}>
                {preflightStatus === 'ok' ? '可同步' : '阻断'}
              </span>
            </div>
            <div className="compact-side-card">
              <strong>Connector 连接</strong>
              <span>
                {connector?.name || '未配置'}
                {connector?.checkedAtLabel ? ` · ${connector.checkedAtLabel}` : ''}
              </span>
              <span className={`tag ${connectorOk ? 'ok' : 'warn'}`}>
                {connector?.statusLabel || '未检查'}
              </span>
            </div>
            <div className="compact-side-card">
              <strong>本次同步范围</strong>
              <span>{syncScopeSummary}</span>
            </div>
            <div className="compact-side-card">
              <strong>语言来源</strong>
              <span>{workspace?.supportedLocales.length ? workspace.supportedLocales.join('、') : '等待从商店接口同步'}</span>
            </div>
          </>
        )}
      </div>
    </aside>
  );
}

function SyncConfirmDialog({
  confirmation,
  busy,
  error,
  onCancel,
  onConfirm
}: {
  confirmation: PendingSyncConfirmation;
  busy: boolean;
  error: string;
  onCancel: () => void;
  onConfirm: (selectedScopes: string[]) => void;
}) {
  const [selectedScopes, setSelectedScopes] = useState(confirmation.syncScopes);

  useEffect(() => {
    setSelectedScopes(confirmation.syncScopes);
  }, [confirmation]);

  function toggleScope(value: string) {
    setSelectedScopes((current) =>
      current.includes(value)
        ? current.filter((item) => item !== value)
        : [...current, value]
    );
  }

  const hasScopeSelection = selectedScopes.length > 0;

  return (
    <div className="modal-backdrop" role="presentation">
      <section
        aria-labelledby="sync-confirm-title"
        aria-modal="true"
        className="sync-confirm-dialog"
        role="dialog"
      >
        <header>
          <div>
            <p className="eyebrow">同步确认</p>
            <h3 id="sync-confirm-title">{confirmation.title}</h3>
          </div>
          <button className="button slim" type="button" onClick={onCancel} disabled={busy}>
            关闭
          </button>
        </header>
        <p className="sync-confirm-summary">
          勾选本次要提交到商店后台的项目。同步过程中请不要关闭页面。
        </p>
        <fieldset className="sync-scope-options">
          <legend>同步项目</legend>
          {confirmation.scopeOptions.map((option) => (
            <label key={option.value} className="sync-scope-option">
              <input
                type="checkbox"
                checked={selectedScopes.includes(option.value)}
                onChange={() => toggleScope(option.value)}
                disabled={busy}
              />
              <span>
                <strong>{option.label}</strong>
                <small>{option.description}</small>
              </span>
            </label>
          ))}
          {!hasScopeSelection ? (
            <p className="sync-scope-warning">请选择至少一个同步项目。</p>
          ) : null}
        </fieldset>
        <dl className="sync-confirm-meta">
          <div>
            <dt>目标</dt>
            <dd>{confirmation.targetLabel}</dd>
          </div>
          <div>
            <dt>{confirmation.kind === 'marketing' ? '页面 ID' : '版本'}</dt>
            <dd>{confirmation.versionLabel}</dd>
          </div>
          <div>
            <dt>语言</dt>
            <dd>{confirmation.localeLabels.join('、') || '无'}</dd>
          </div>
        </dl>
        {error ? <div className="notice error compact">{error}</div> : null}
        <footer>
          <button className="button" type="button" onClick={onCancel} disabled={busy}>
            取消
          </button>
          <button
            className="button primary"
            type="button"
            onClick={() => onConfirm(selectedScopes)}
            disabled={busy || !hasScopeSelection}
          >
            {busy ? '同步中...' : '确认同步'}
          </button>
        </footer>
      </section>
    </div>
  );
}

function AccountEditor({
  title,
  account,
  onCancel,
  onSubmit
}: {
  title: string;
  account?: DeveloperAccountSummary;
  onCancel: () => void;
  onSubmit: (payload: DeveloperAccountForm) => void;
}) {
  const [accountId, setAccountId] = useState(account?.id ?? '');
  const [teamName, setTeamName] = useState(account?.teamName ?? '');
  const [expiresAt, setExpiresAt] = useState(toDateTimeInput(account?.expiresAt ?? ''));
  const [status, setStatus] = useState(account?.status ?? 'renewal_due');
  const [renewalActionLabel, setRenewalActionLabel] = useState('去续费');

  function submit(event: FormEvent) {
    event.preventDefault();
    onSubmit({
      accountId: account ? account.id : accountId,
      teamName,
      expiresAt,
      status,
      renewalActionLabel
    });
  }

  return (
    <section className="panel account-editor">
      <div className="panel-head compact">
        <strong>{title}</strong>
      </div>
      <form className="form-grid" onSubmit={submit}>
        <label>
          <span>账号 ID</span>
          <input value={accountId} onChange={(event) => setAccountId(event.target.value)} disabled={Boolean(account)} />
        </label>
        <label>
          <span>Team 名称</span>
          <input value={teamName} onChange={(event) => setTeamName(event.target.value)} />
        </label>
        <label>
          <span>到期时间</span>
          <input
            type="datetime-local"
            value={expiresAt}
            onChange={(event) => setExpiresAt(event.target.value)}
          />
        </label>
        <label>
          <span>状态</span>
          <select value={status} onChange={(event) => setStatus(event.target.value)}>
            {accountStatuses.map((item) => (
              <option key={item.value} value={item.value}>
                {item.label}
              </option>
            ))}
          </select>
        </label>
        <label>
          <span>提醒动作</span>
          <input
            value={renewalActionLabel}
            onChange={(event) => setRenewalActionLabel(event.target.value)}
          />
        </label>
        <div className="form-actions">
          <button className="button" type="button" onClick={onCancel}>
            取消
          </button>
          <button className="button primary" type="submit">
            保存账号
          </button>
        </div>
      </form>
    </section>
  );
}

function ConnectorForm({
  connector,
  onSubmit
}: {
  connector: DeveloperAccountDetailState['connector'];
  onSubmit: (payload: { name: string; baseUrl: string; authToken: string }) => void;
}) {
  const [name, setName] = useState(connector?.name ?? 'Connector');
  const [baseUrl, setBaseUrl] = useState(connector?.baseUrl ?? '');
  const [authToken, setAuthToken] = useState(connector?.authToken ?? '');

  useEffect(() => {
    setName(connector?.name ?? 'Connector');
    setBaseUrl(connector?.baseUrl ?? '');
    setAuthToken(connector?.authToken ?? '');
  }, [connector]);

  function submit(event: FormEvent) {
    event.preventDefault();
    onSubmit({ name, baseUrl, authToken });
  }

  return (
    <form className="form-grid" onSubmit={submit}>
      <label>
        <span>名称</span>
        <input value={name} onChange={(event) => setName(event.target.value)} />
      </label>
      <label>
        <span>地址</span>
        <input value={baseUrl} onChange={(event) => setBaseUrl(event.target.value)} />
      </label>
      <label>
        <span>Token</span>
        <input value={authToken} onChange={(event) => setAuthToken(event.target.value)} />
      </label>
      <div className="form-actions">
        <span className={`tag ${connector?.status === 'ok' ? 'ok' : 'warn'}`}>
          {connector?.statusLabel ?? '未配置'}
        </span>
        <button className="button" type="submit">
          保存 Connector
        </button>
      </div>
    </form>
  );
}

function BindAppForm({
  state,
  onSubmit
}: {
  state: DeveloperAccountDetailState;
  onSubmit: (payload: { appId: string; storeAppId: string; storePackageName: string }) => void;
}) {
  const [appId, setAppId] = useState(state.unassignedApps[0]?.id ?? '');
  const [storeAppId, setStoreAppId] = useState('');
  const [storePackageName, setStorePackageName] = useState('');
  const selectedApp = useMemo(
    () => state.unassignedApps.find((app) => app.id === appId) ?? null,
    [appId, state.unassignedApps]
  );

  useEffect(() => {
    if (!appId && state.unassignedApps[0]) setAppId(state.unassignedApps[0].id);
  }, [appId, state.unassignedApps]);

  function submit(event: FormEvent) {
    event.preventDefault();
    if (!appId) return;
    onSubmit({ appId, storeAppId, storePackageName });
    setStoreAppId('');
    setStorePackageName('');
  }

  return (
    <form className="form-grid" onSubmit={submit}>
      <label>
        <span>App</span>
        <select value={appId} onChange={(event) => setAppId(event.target.value)}>
          {state.unassignedApps.map((app) => (
            <option key={app.id} value={app.id}>
              {app.name} / {app.platformLabel}
            </option>
          ))}
        </select>
      </label>
      <label>
        <span>iOS App ID</span>
        <input
          value={storeAppId}
          onChange={(event) => setStoreAppId(event.target.value)}
          disabled={selectedApp?.platform === 'android'}
        />
      </label>
      <label>
        <span>Android Package</span>
        <input
          value={storePackageName}
          onChange={(event) => setStorePackageName(event.target.value)}
          disabled={selectedApp?.platform === 'ios'}
        />
      </label>
      <div className="form-actions">
        <button className="button primary" type="submit" disabled={!appId}>
          绑定 App
        </button>
      </div>
    </form>
  );
}

function BoundAppCard({
  accountId,
  app,
  onNavigate,
  onSave,
  onRemove
}: {
  accountId: string;
  app: AccountAppItem;
  onNavigate: (path: string) => void;
  onSave: (payload: { storeAppId: string; storePackageName: string }) => void;
  onRemove: () => void;
}) {
  return (
    <article className="bound-app-card">
      <div className="selected-store-app-title">
        <span className="app-avatar" style={{ backgroundColor: app.iconColor }}>
          {app.iconText}
        </span>
        <div>
          <strong>{app.name}</strong>
          <span>{app.bundleIdentifier}</span>
        </div>
      </div>
      <div className="bound-app-actions">
        <button className="button" type="button" onClick={() => onNavigate(app.storePath)}>
          商店管理
        </button>
        <button className="button" type="button" onClick={() => onNavigate(app.marketingPath)}>
          营销页面
        </button>
        <button className="button" type="button" onClick={() => onNavigate(app.connectionPath)}>
          连接设置
        </button>
        <button className="button" type="button" onClick={onRemove}>
          解绑
        </button>
      </div>
      <BoundAppSettings app={{ ...app, connectionPath: `/admin/accounts/${accountId}/apps/${app.id}/connection` }} onSave={onSave} />
    </article>
  );
}

function BoundAppSettings({
  app,
  onSave
}: {
  app: AccountAppItem;
  onSave: (payload: { storeAppId: string; storePackageName: string }) => void;
}) {
  const [storeAppId, setStoreAppId] = useState(app.storeAppId);
  const [storePackageName, setStorePackageName] = useState(app.storePackageName);

  useEffect(() => {
    setStoreAppId(app.storeAppId);
    setStorePackageName(app.storePackageName);
  }, [app]);

  function submit(event: FormEvent) {
    event.preventDefault();
    onSave({ storeAppId, storePackageName });
  }

  return (
    <form className="app-settings-row" onSubmit={submit}>
      <label>
        <span>iOS App ID</span>
        <input
          value={storeAppId}
          onChange={(event) => setStoreAppId(event.target.value)}
          disabled={app.platform === 'android'}
        />
      </label>
      <label>
        <span>Android Package</span>
        <input
          value={storePackageName}
          onChange={(event) => setStorePackageName(event.target.value)}
          disabled={app.platform === 'ios'}
        />
      </label>
      <button className="button" type="submit">
        保存标识
      </button>
    </form>
  );
}

function StoreDefaultPanel({
  workspace,
  busy,
  syncing,
  onBackToApps,
  onSave,
  onCheck,
  onSync,
  onDeleteImage,
  onUploadImages
}: {
  workspace: StoreWorkspaceState;
  busy: boolean;
  syncing: boolean;
  onBackToApps: () => void;
  onSave: (locales: StoreLocaleContentInput[]) => void;
  onCheck: (locales: StoreLocaleContentInput[], syncScopes: string[]) => void;
  onSync: (locales: StoreLocaleContentInput[], syncScopes: string[]) => void;
  onDeleteImage: (payload: { locale: string; slotKey: string; storageKey: string }) => void;
  onUploadImages: (payload: StoreImageUploadRequest) => void;
}) {
  const [rows, setRows] = useState<StoreLocaleContent[]>(workspace.localizedMetadata);
  const [translatingField, setTranslatingField] = useState<StoreTextField | null>(null);
  const [translationError, setTranslationError] = useState<{
    field: StoreTextField | null;
    message: string;
  }>({ field: null, message: '' });

  useEffect(() => {
    setRows(workspace.localizedMetadata);
    setTranslationError({ field: null, message: '' });
    setTranslatingField(null);
  }, [workspace]);

  const currentLocale = workspace.locale || workspace.sourceLocale || rows[0]?.locale || '';
  const current = rows.find((item) => item.locale === currentLocale) ?? rows[0] ?? null;
  const payload = serializeStoreRows(rows);

  function updateField(field: StoreTextField, value: string, locale = currentLocale) {
    setRows((currentRows) =>
      currentRows.map((item) =>
        item.locale === locale ? { ...item, [field]: value } : item
      )
    );
  }

  async function translateField(field: StoreTextField) {
    const source = rows.find((item) => item.isSource) ?? current;
    const sourceLocale = source?.locale || currentLocale;
    const sourceText = String(source?.[field] || '').trim();
    const targetLocales = rows
      .filter((item) => item.locale !== sourceLocale)
      .map((item) => item.locale);
    if (!sourceText) {
      setTranslationError({ field, message: '源文案为空，无法翻译。' });
      return;
    }
    if (!targetLocales.length) {
      setTranslationError({ field, message: '没有需要翻译的目标语言。' });
      return;
    }
    setTranslatingField(field);
    setTranslationError({ field: null, message: '' });
    try {
      const response = await translateStoreText({
        sourceLocale,
        targetLocales,
        field,
        text: sourceText
      });
      setRows((currentRows) =>
        currentRows.map((item) => {
          const translated = response.translations[item.locale];
          return translated ? { ...item, [field]: translated } : item;
        })
      );
    } catch (requestError) {
      setTranslationError({ field, message: errorMessage(requestError) });
    } finally {
      setTranslatingField(null);
    }
  }

  return (
    <div className="workspace-content-grid">
      <section className="store-summary-strip store-page-actions">
        <button className="button" type="button" onClick={onBackToApps}>
          商店应用
        </button>
        <button className="button" type="button" onClick={() => onSave(payload)} disabled={busy}>
          保存草稿
        </button>
        <button
          className="button primary"
          type="button"
          onClick={() => onSync(payload, ['release_notes', 'metadata', 'store_images'])}
          disabled={busy}
        >
          {syncing ? '同步中...' : '同步到商店'}
        </button>
        <button
          className="button"
          type="button"
          onClick={() => onCheck(payload, ['release_notes', 'metadata', 'store_images'])}
          disabled={busy}
        >
          实时查询
        </button>
      </section>
      <StoreTextSection
        title="版本说明"
        subtitle="Release Notes"
        meta="跟随当前商店最新版本"
        icon="release"
        value={current?.releaseNotes || ''}
        placeholder="fix bugs"
        locales={rows}
        field="releaseNotes"
        onChange={(value) => updateField('releaseNotes', value)}
        onLocaleChange={(locale, value) => updateField('releaseNotes', value, locale)}
        onTranslate={() => translateField('releaseNotes')}
        translateBusy={translatingField === 'releaseNotes'}
        translateError={translationError.field === 'releaseNotes' ? translationError.message : ''}
      />
      <StoreTextSection
        title="宣传文本"
        subtitle="Promotional Text"
        meta={`当前语言：${currentLocale}`}
        icon="promo"
        value={current?.promotionalText || ''}
        placeholder="App Store Connect promotional text"
        locales={rows}
        field="promotionalText"
        onChange={(value) => updateField('promotionalText', value)}
        onLocaleChange={(locale, value) => updateField('promotionalText', value, locale)}
        onTranslate={() => translateField('promotionalText')}
        translateBusy={translatingField === 'promotionalText'}
        translateError={
          translationError.field === 'promotionalText' ? translationError.message : ''
        }
      />
      <StoreTextSection
        title="描述"
        subtitle="Description"
        meta={`当前语言：${currentLocale}`}
        icon="description"
        value={current?.description || ''}
        placeholder="App Store Connect description"
        locales={rows}
        field="description"
        onChange={(value) => updateField('description', value)}
        onLocaleChange={(locale, value) => updateField('description', value, locale)}
        onTranslate={() => translateField('description')}
        translateBusy={translatingField === 'description'}
        translateError={translationError.field === 'description' ? translationError.message : ''}
      />
      <ImageOverview
        locales={rows}
        currentLocale={currentLocale}
        busy={busy}
        onDeleteImage={onDeleteImage}
        onUploadImages={onUploadImages}
      />
    </div>
  );
}

function MarketingPagesPanel({
  workspace,
  busy,
  onNavigate,
  onCreate
}: {
  workspace: StoreWorkspaceState;
  busy: boolean;
  onNavigate: (path: string) => void;
  onCreate: (payload: MarketingPagePayload) => void;
}) {
  const [pageName, setPageName] = useState('新的自定义产品页面');
  const sourceLocale = workspace.sourceLocale || workspace.locale || workspace.supportedLocales[0] || 'en-US';

  function createPage(event: FormEvent) {
    event.preventDefault();
    onCreate({
      pageName,
      pageType: 'custom_product_page',
      deepLinkUrl: '',
      locale: sourceLocale,
      locales: workspace.supportedLocales.map((locale) => ({
        locale,
        promotionalText: locale === sourceLocale ? '' : '',
        storeImages: {}
      }))
    });
  }

  return (
    <div className="workspace-content-grid">
      <section className="store-summary-strip">
        <span>{workspace.marketingPages.length} 个营销页面</span>
        <span>{workspace.supportedLocales.length} 个语言</span>
        <span>{workspace.preflightLabel}</span>
      </section>
      <form className="marketing-create-row" onSubmit={createPage}>
        <label>
          <span>新建自定义产品页面</span>
          <input value={pageName} onChange={(event) => setPageName(event.target.value)} />
        </label>
        <button className="button primary" type="submit" disabled={busy || !pageName.trim()}>
          新建页面
        </button>
      </form>
      <div className="marketing-page-list">
        {workspace.marketingPages.map((page) => (
          <button
            key={page.id}
            className="marketing-page-row"
            type="button"
            onClick={() => onNavigate(page.detailPath)}
          >
            <span>
              <strong>{page.pageName}</strong>
              <small>{page.typeLabel} / {page.pageId}</small>
            </span>
            <span>{page.languageCount} 语言</span>
            <span>{page.assetCount} 张图</span>
            <span className={`tag ${page.status === 'synced' ? 'ok' : 'warn'}`}>
              {page.statusLabel}
            </span>
          </button>
        ))}
      </div>
      {!workspace.marketingPages.length ? (
        <div className="empty-state">还没有营销页面。</div>
      ) : null}
    </div>
  );
}

function MarketingPageDetailPanel({
  detail,
  busy,
  syncing,
  onBack,
  onSave,
  onCheck,
  onSync,
  onCopy,
  onDelete,
  onDeleteImage,
  onUploadImages
}: {
  detail: MarketingPageDetailState | null;
  busy: boolean;
  syncing: boolean;
  onBack: () => void;
  onSave: (payload: MarketingPagePayload) => void;
  onCheck: (payload: MarketingPagePayload) => void;
  onSync: (payload: MarketingPagePayload) => void;
  onCopy: () => void;
  onDelete: () => void;
  onDeleteImage: (payload: { locale: string; slotKey: string; storageKey: string }) => void;
  onUploadImages: (payload: StoreImageUploadRequest) => void;
}) {
  const [pageName, setPageName] = useState('');
  const [deepLinkUrl, setDeepLinkUrl] = useState('');
  const [rows, setRows] = useState<MarketingPageLocaleContent[]>([]);
  const [translating, setTranslating] = useState(false);
  const [translationError, setTranslationError] = useState('');

  useEffect(() => {
    setPageName(detail?.page.pageName ?? '');
    setDeepLinkUrl(detail?.page.deepLinkUrl ?? '');
    setRows(detail?.localizedPage ?? []);
    setTranslationError('');
    setTranslating(false);
  }, [detail]);

  if (!detail) {
    return <div className="empty-state">正在加载营销页面...</div>;
  }

  const currentLocale = detail.locale || detail.sourceLocale || rows[0]?.locale || '';
  const current = rows.find((item) => item.locale === currentLocale) ?? rows[0] ?? null;
  const payload = marketingPayloadFromRows(detail, pageName, deepLinkUrl, rows);

  function updatePromotionalText(value: string, locale = currentLocale) {
    setRows((currentRows) =>
      currentRows.map((item) =>
        item.locale === locale ? { ...item, promotionalText: value } : item
      )
    );
  }

  async function translatePromotionalText() {
    const source = rows.find((item) => item.isSource) ?? current;
    const sourceLocale = source?.locale || currentLocale;
    const sourceText = String(source?.promotionalText || '').trim();
    const targetLocales = rows
      .filter((item) => item.locale !== sourceLocale)
      .map((item) => item.locale);
    if (!sourceText) {
      setTranslationError('源文案为空，无法翻译。');
      return;
    }
    if (!targetLocales.length) {
      setTranslationError('没有需要翻译的目标语言。');
      return;
    }
    setTranslating(true);
    setTranslationError('');
    try {
      const response = await translateStoreText({
        sourceLocale,
        targetLocales,
        field: 'promotionalText',
        text: sourceText
      });
      setRows((currentRows) =>
        currentRows.map((item) => {
          const translated = response.translations[item.locale];
          return translated ? { ...item, promotionalText: translated } : item;
        })
      );
    } catch (requestError) {
      setTranslationError(errorMessage(requestError));
    } finally {
      setTranslating(false);
    }
  }

  return (
    <div className="workspace-content-grid">
      <section className="store-summary-strip">
        <button className="button" type="button" onClick={onBack}>
          返回营销页面
        </button>
        <span>{detail.page.typeLabel}</span>
        <span>Apple 页面 ID：{detail.page.applePageIdLabel}</span>
        <span>{detail.preflightLabel}</span>
        <button className="button" type="button" onClick={() => onSave(payload)} disabled={busy}>
          保存草稿
        </button>
        <button
          className="button"
          type="button"
          onClick={() => onCheck({ ...payload, syncScopes: ['marketing_text', 'store_images'] })}
          disabled={busy}
        >
          实时查询
        </button>
        <button
          className="button primary"
          type="button"
          onClick={() => onSync({ ...payload, syncScopes: ['marketing_text', 'store_images'] })}
          disabled={busy}
        >
          {syncing ? '同步中...' : '同步营销页'}
        </button>
      </section>

      <section className="field-card marketing-detail-fields">
        <header>
          <div>
            <strong>页面基础信息</strong>
            <span>中心后台草稿，不会自动同步到商店</span>
          </div>
          <div className="button-row">
            <button className="button" type="button" onClick={onCopy} disabled={busy}>
              复制页面
            </button>
            <button className="button danger" type="button" onClick={onDelete} disabled={busy}>
              删除页面
            </button>
          </div>
        </header>
        <div className="form-grid">
          <label>
            <span>页面名称</span>
            <input value={pageName} onChange={(event) => setPageName(event.target.value)} />
          </label>
          <label>
            <span>Deep Link URL</span>
            <input value={deepLinkUrl} onChange={(event) => setDeepLinkUrl(event.target.value)} />
          </label>
        </div>
      </section>

      <MarketingTextCard
        title="宣传文本"
        value={current?.promotionalText || ''}
        locales={rows}
        onChange={updatePromotionalText}
        onLocaleChange={(locale, value) => updatePromotionalText(value, locale)}
        onTranslate={translatePromotionalText}
        translateBusy={translating}
        translateError={translationError}
      />
      <MarketingImageOverview
        locales={rows}
        busy={busy}
        onDeleteImage={onDeleteImage}
        onUploadImages={onUploadImages}
      />
    </div>
  );
}

function MarketingTextCard({
  title,
  value,
  locales,
  onChange,
  onLocaleChange,
  onTranslate,
  translateBusy,
  translateError
}: {
  title: string;
  value: string;
  locales: MarketingPageLocaleContent[];
  onChange: (value: string) => void;
  onLocaleChange: (locale: string, value: string) => void;
  onTranslate: () => void;
  translateBusy: boolean;
  translateError: string;
}) {
  const [expanded, setExpanded] = useState(false);
  return (
    <article className="field-card">
      <header>
        <div>
          <strong>{title}</strong>
          <span>{locales.length} 个语言</span>
        </div>
        <div className="field-card-actions">
          <button
            aria-label={`翻译${title}到其他语言`}
            className="button slim"
            type="button"
            onClick={onTranslate}
            disabled={translateBusy}
          >
            {translateBusy ? '翻译中...' : '翻译'}
          </button>
          <button
            aria-label={expanded ? `收起${title}多语言` : `展开${title}多语言`}
            className="button"
            type="button"
            onClick={() => setExpanded((current) => !current)}
          >
            {expanded ? '收起多语言' : '展开多语言'}
          </button>
        </div>
      </header>
      <textarea
        aria-label={`${title} 当前语言`}
        value={value}
        placeholder="App Store Connect promotional text"
        onChange={(event) => onChange(event.target.value)}
      />
      {expanded ? (
        <div className="locale-expanded-block">
          {translateError ? <div className="notice error compact">{translateError}</div> : null}
          <div className="locale-content-list">
            {locales.map((locale) => (
              <div key={locale.locale} className="locale-content-row">
                <strong>{locale.locale}</strong>
                <span className={locale.isSource ? 'tag ok' : 'tag'}>
                  {locale.isSource ? '源文案' : '翻译'}
                </span>
                <textarea
                  aria-label={`${locale.locale} ${title}`}
                  value={locale.promotionalText}
                  placeholder="未填写"
                  onChange={(event) => onLocaleChange(locale.locale, event.target.value)}
                />
              </div>
            ))}
          </div>
        </div>
      ) : null}
    </article>
  );
}

function MarketingImageOverview({
  locales,
  busy,
  onDeleteImage,
  onUploadImages
}: {
  locales: MarketingPageLocaleContent[];
  busy: boolean;
  onDeleteImage: (payload: { locale: string; slotKey: string; storageKey: string }) => void;
  onUploadImages: (payload: StoreImageUploadRequest) => void;
}) {
  return (
    <article className="field-card">
      <header>
        <div>
          <strong>营销页截图</strong>
          <span>{locales.length} 个语言</span>
        </div>
      </header>
      <div className="locale-content-list">
        {locales.map((locale) => {
          const assets = imageAssets(locale.storeImages);
          return (
            <div key={locale.locale} className="locale-content-row image-locale-row">
              <strong>{locale.locale}</strong>
              <span>{assets.length} 张</span>
              <ImageUploadActions
                locale={locale.locale}
                busy={busy}
                onUploadImages={onUploadImages}
              />
              <div className="store-image-assets">
                {assets.length ? (
                  assets.map((asset) => (
                    <div key={asset.id} className="store-image-asset-row">
                      {asset.downloadUrl ? <img src={asset.downloadUrl} alt={`${locale.locale} 营销页截图`} /> : null}
                      <span>{asset.fileName || asset.storageKey}</span>
                      {asset.canDelete ? (
                        <button
                          className="button slim"
                          type="button"
                          onClick={() =>
                            onDeleteImage({
                              locale: locale.locale,
                              slotKey: asset.slotKey,
                              storageKey: asset.storageKey
                            })
                          }
                        >
                          删除
                        </button>
                      ) : null}
                    </div>
                  ))
                ) : (
                  <p>还没有上传图片</p>
                )}
              </div>
            </div>
          );
        })}
      </div>
    </article>
  );
}

function StoreTextSection({
  title,
  subtitle,
  meta,
  icon,
  value,
  locales,
  field,
  placeholder,
  onChange,
  onLocaleChange,
  onTranslate,
  translateBusy,
  translateError
}: {
  title: string;
  subtitle: string;
  meta: string;
  icon: 'release' | 'promo' | 'description';
  value: string;
  locales: StoreLocaleContent[];
  field: StoreTextField;
  placeholder: string;
  onChange: (value: string) => void;
  onLocaleChange: (locale: string, value: string) => void;
  onTranslate: () => void;
  translateBusy: boolean;
  translateError: string;
}) {
  const [expanded, setExpanded] = useState(false);
  const filledCount = locales.filter((locale) => String(locale[field] || '').trim()).length;
  return (
    <section className="section" data-section={field}>
      <div className="section-head">
        <div className="section-title">
          <span className="title-icon">{storeSectionIcon(icon)}</span>
          <div>
            <h3>{title}</h3>
            <p className="meta">
              <span>{subtitle}</span>
              <span>{meta}</span>
            </p>
          </div>
        </div>
        <div className="section-tools">
          <span className="pill">
            <span className="dot" />
            {filledCount}/{locales.length} 已填写
          </span>
          <button
            aria-label={`翻译${title}到其他语言`}
            className="button"
            type="button"
            onClick={onTranslate}
            disabled={translateBusy}
          >
            {translateBusy ? '翻译中...' : '翻译'}
          </button>
          <button
            aria-label={expanded ? `收起${title}多语言` : `展开${title}多语言`}
            className="button"
            type="button"
            onClick={() => setExpanded((current) => !current)}
          >
            {expanded ? '收起多语言' : '展开多语言'}
          </button>
        </div>
      </div>
      <textarea
        aria-label={`${title} 当前语言`}
        value={value}
        placeholder={placeholder}
        onChange={(event) => onChange(event.target.value)}
      />
      {expanded ? (
        <div className="item-language-panel">
          <div className="item-language-panel-title">
            <span>{title}的所有语言（{locales.length}）</span>
            <span>{filledCount}/{locales.length} 已填写</span>
          </div>
          {translateError ? <div className="notice error compact">{translateError}</div> : null}
          <div className="language-list">
            {locales.map((locale) => {
              const filled = String(locale[field] || '').trim();
              return (
                <div key={locale.locale} className="language-row language-edit-row">
                  <span className="drag">⋮</span>
                  <span className="lang">{locale.locale}</span>
                  <span className={locale.isSource ? 'status source' : 'status'}>
                    {locale.isSource ? '源文案' : filled ? '已填写' : '待填写'}
                  </span>
                  <textarea
                    aria-label={`${locale.locale} ${title}`}
                    value={locale[field]}
                    placeholder={placeholder}
                    onChange={(event) => onLocaleChange(locale.locale, event.target.value)}
                  />
                </div>
              );
            })}
          </div>
        </div>
      ) : null}
    </section>
  );
}

function storeSectionIcon(icon: 'release' | 'promo' | 'description' | 'image') {
  if (icon === 'release') {
    return (
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M6 20h12" />
        <path d="M8 4h8v16H8z" />
        <path d="M10 8h4" />
      </svg>
    );
  }
  if (icon === 'promo') {
    return (
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M3 11v2h4l10 5V6L7 11z" />
        <path d="M17 9.5a3 3 0 0 1 0 5" />
      </svg>
    );
  }
  if (icon === 'image') {
    return (
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <rect x="3" y="5" width="18" height="14" rx="2" />
        <path d="m3 16 5-5 4 4 3-3 6 6" />
      </svg>
    );
  }
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M6 3h9l3 3v15H6z" />
      <path d="M14 3v4h4" />
    </svg>
  );
}

function storeModuleIcon(section: StoreSection) {
  if (section === 'marketing') {
    return (
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M4 18V6l8 6 8-6v12" />
      </svg>
    );
  }
  if (section === 'connection') {
    return (
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M10 13a5 5 0 0 0 7 0l2-2a5 5 0 0 0-7-7l-1 1" />
        <path d="M14 11a5 5 0 0 0-7 0l-2 2a5 5 0 0 0 7 7l1-1" />
      </svg>
    );
  }
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M6 3h9l3 3v15H6z" />
      <path d="M14 3v4h4" />
    </svg>
  );
}

function EditableFieldCard({
  title,
  value,
  locales,
  field,
  placeholder,
  onChange,
  onLocaleChange,
  onTranslate,
  translateBusy,
  translateError
}: {
  title: string;
  value: string;
  locales: StoreLocaleContent[];
  field: StoreTextField;
  placeholder: string;
  onChange: (value: string) => void;
  onLocaleChange: (locale: string, value: string) => void;
  onTranslate: () => void;
  translateBusy: boolean;
  translateError: string;
}) {
  const [expanded, setExpanded] = useState(false);
  return (
    <article className="field-card">
      <header>
        <div>
          <strong>{title}</strong>
          <span>{locales.length} 个语言</span>
        </div>
        <div className="field-card-actions">
          <button
            aria-label={`翻译${title}到其他语言`}
            className="button slim"
            type="button"
            onClick={onTranslate}
            disabled={translateBusy}
          >
            {translateBusy ? '翻译中...' : '翻译'}
          </button>
          <button
            aria-label={expanded ? `收起${title}多语言` : `展开${title}多语言`}
            className="button"
            type="button"
            onClick={() => setExpanded((current) => !current)}
          >
            {expanded ? '收起多语言' : '展开多语言'}
          </button>
        </div>
      </header>
      <textarea
        aria-label={`${title} 当前语言`}
        value={value}
        placeholder={placeholder}
        onChange={(event) => onChange(event.target.value)}
      />
      {expanded ? (
        <div className="locale-expanded-block">
          {translateError ? <div className="notice error compact">{translateError}</div> : null}
          <div className="locale-content-list">
            {locales.map((locale) => (
              <div key={locale.locale} className="locale-content-row">
                <strong>{locale.locale}</strong>
                <span className={locale.isSource ? 'tag ok' : 'tag'}>
                  {locale.isSource ? '源文案' : '翻译'}
                </span>
                <textarea
                  aria-label={`${locale.locale} ${title}`}
                  value={locale[field]}
                  placeholder={placeholder}
                  onChange={(event) => onLocaleChange(locale.locale, event.target.value)}
                />
              </div>
            ))}
          </div>
        </div>
      ) : null}
    </article>
  );
}

function ImageOverview({
  locales,
  currentLocale,
  busy,
  onDeleteImage,
  onUploadImages
}: {
  locales: StoreLocaleContent[];
  currentLocale: string;
  busy: boolean;
  onDeleteImage: (payload: { locale: string; slotKey: string; storageKey: string }) => void;
  onUploadImages: (payload: StoreImageUploadRequest) => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const current = locales.find((locale) => locale.locale === currentLocale) ?? locales[0] ?? null;
  const phoneCount = locales.reduce(
    (total, locale) =>
      total + (imageAssetsBySlot(locale.storeImages).phone_screenshots.length > 0 ? 1 : 0),
    0
  );
  const tabletCount = locales.reduce(
    (total, locale) =>
      total + (imageAssetsBySlot(locale.storeImages).tablet_screenshots.length > 0 ? 1 : 0),
    0
  );

  return (
    <section className="section" data-section="screenshots">
      <div className="section-head">
        <div className="section-title">
          <span className="title-icon">{storeSectionIcon('image')}</span>
          <div>
            <h3>商店图</h3>
            <p className="meta">
              <span>手机截图 {phoneCount}/{locales.length}</span>
              <span>平板截图 {tabletCount}/{locales.length}</span>
            </p>
          </div>
        </div>
        <div className="section-tools">
          <button
            aria-label={expanded ? '收起商店图多语言' : '展开商店图多语言'}
            className="button"
            type="button"
            onClick={() => setExpanded((value) => !value)}
          >
            {expanded ? '收起多语言' : '展开多语言'}
          </button>
        </div>
      </div>
      <div className="shot-list">
        {current ? (
          <StoreImageLocaleRow
            locale={current}
            busy={busy}
            onDeleteImage={onDeleteImage}
            onUploadImages={onUploadImages}
          />
        ) : null}
      </div>
      {expanded ? (
        <div className="item-language-panel">
          <div className="item-language-panel-title">
            <span>商店图的所有语言（{locales.length}）</span>
            <span>只删除中心后台草稿图片</span>
          </div>
          <div className="shot-list">
            {locales.map((locale) => (
              <StoreImageLocaleRow
                key={locale.locale}
                locale={locale}
                busy={busy}
                onDeleteImage={onDeleteImage}
                onUploadImages={onUploadImages}
              />
            ))}
          </div>
        </div>
      ) : null}
    </section>
  );
}

function StoreImageLocaleRow({
  locale,
  busy,
  onDeleteImage,
  onUploadImages
}: {
  locale: StoreLocaleContent;
  busy: boolean;
  onDeleteImage: (payload: { locale: string; slotKey: string; storageKey: string }) => void;
  onUploadImages: (payload: StoreImageUploadRequest) => void;
}) {
  const assetsBySlot = imageAssetsBySlot(locale.storeImages);

  return (
    <div className="shot-row">
      <div className="shot-lang">{locale.locale}</div>
      <div className="shot-slot-list">
        {STORE_IMAGE_SLOT_CONFIG.map((slot) => {
          const assets = assetsBySlot[slot.key] ?? [];
          return (
            <div key={slot.key} className="shot-slot" role="group" aria-label={`${locale.locale} ${slot.label}`}>
              <div className="shot-slot-head">
                <strong>{slot.label}</strong>
                <span>{assets.length} 张</span>
              </div>
              <div className="shot-strip">
                {assets.map((asset) => (
                  <div key={asset.id} className="shot uploaded">
                    {asset.downloadUrl ? (
                      <img src={asset.downloadUrl} alt={`${locale.locale} ${slot.label}`} />
                    ) : null}
                    {asset.canDelete ? (
                      <button
                        className="shot-delete"
                        type="button"
                        onClick={() =>
                          onDeleteImage({
                            locale: locale.locale,
                            slotKey: asset.slotKey,
                            storageKey: asset.storageKey
                          })
                        }
                      >
                        删除
                      </button>
                    ) : null}
                  </div>
                ))}
                <ImageUploadAction
                  locale={locale.locale}
                  slotKey={slot.key}
                  label={slot.label}
                  busy={busy}
                  onUploadImages={onUploadImages}
                />
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

function ImageUploadAction({
  locale,
  slotKey,
  label,
  busy,
  onUploadImages
}: {
  locale: string;
  slotKey: string;
  label: string;
  busy: boolean;
  onUploadImages: (payload: StoreImageUploadRequest) => void;
}) {
  function selectFiles(files: FileList | null) {
    const selected = Array.from(files ?? []);
    if (selected.length === 0) return;
    onUploadImages({ locale, slotKey, files: selected });
  }

  return (
    <label className={busy ? 'shot empty shot-upload disabled' : 'shot empty shot-upload'}>
      <span>+</span>
      <small>上传</small>
      <input
        aria-label={`${locale} ${label}上传`}
        type="file"
        multiple
        accept="image/png,image/jpeg"
        disabled={busy}
        onChange={(event) => {
          selectFiles(event.currentTarget.files);
          event.currentTarget.value = '';
        }}
      />
    </label>
  );
}

function ImageUploadActions({
  locale,
  busy,
  onUploadImages
}: {
  locale: string;
  busy: boolean;
  onUploadImages: (payload: StoreImageUploadRequest) => void;
}) {
  function selectFiles(slotKey: string, files: FileList | null) {
    const selected = Array.from(files ?? []);
    if (selected.length === 0) return;
    onUploadImages({ locale, slotKey, files: selected });
  }

  return (
    <div className="store-image-upload-actions">
      {STORE_IMAGE_SLOT_CONFIG.map((slot) => (
        <label key={slot.key} className={busy ? 'button slim disabled' : 'button slim'}>
          {slot.label}
          <input
            aria-label={`${locale} ${slot.label}上传`}
            type="file"
            multiple
            accept="image/png,image/jpeg"
            disabled={busy}
            onChange={(event) => {
              selectFiles(slot.key, event.currentTarget.files);
              event.currentTarget.value = '';
            }}
          />
        </label>
      ))}
    </div>
  );
}

function Metric({ label, value }: { label: string; value: number | string }) {
  return (
    <div className="account-metric">
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function parseAccountRoute(pathname: string): AccountRoute {
  const parts = pathname.replace(/^\/admin\/?/, '').split('/').filter(Boolean);
  if (parts[0] !== 'accounts') return { kind: 'list' };
  if (!parts[1]) return { kind: 'list' };
  if (parts[1] === 'new') return { kind: 'new' };
  if (parts[2] === 'edit') return { kind: 'edit', accountId: parts[1] };
  if (parts[2] === 'apps' && parts[3]) {
    if (parts[4] === 'marketing-pages' && parts[5]) {
      return {
        kind: 'app',
        accountId: parts[1],
        appId: parts[3],
        section: 'marketing',
        pageId: parts[5]
      };
    }
    const section = parts[4] === 'release-notes' ? 'store' : isStoreSection(parts[4]) ? parts[4] : 'store';
    return { kind: 'app', accountId: parts[1], appId: parts[3], section };
  }
  return { kind: 'detail', accountId: parts[1] };
}

function isStoreSection(value: string | undefined): value is StoreSection {
  return value === 'store' || value === 'marketing' || value === 'connection';
}

function syncScopeLabel(value: string) {
  return syncScopeLabels[value] ?? value;
}

function syncScopeOptions(values: string[]): SyncScopeOption[] {
  return values.map((value) => ({
    value,
    label: syncScopeLabel(value),
    description: syncScopeDescriptions[value] ?? '同步这个项目。'
  }));
}

function toDateTimeInput(value: string) {
  if (!value) return '';
  return value.slice(0, 16);
}

function serializeStoreRows(rows: StoreLocaleContent[]): StoreLocaleContentInput[] {
  return rows.map((item) => ({
    locale: item.locale,
    promotionalText: item.promotionalText,
    description: item.description,
    releaseNotes: item.releaseNotes,
    storeImages: item.storeImages
  }));
}

function marketingPayloadFromRows(
  detail: MarketingPageDetailState,
  pageName: string,
  deepLinkUrl: string,
  rows: MarketingPageLocaleContent[]
): MarketingPagePayload {
  return {
    pageId: detail.page.pageId,
    pageName,
    pageType: detail.page.pageType,
    deepLinkUrl,
    locale: detail.locale || detail.sourceLocale,
    locales: serializeMarketingRows(rows)
  };
}

function serializeMarketingRows(rows: MarketingPageLocaleContent[]): MarketingPageLocaleInput[] {
  return rows.map((item) => ({
    locale: item.locale,
    promotionalText: item.promotionalText,
    storeImages: item.storeImages
  }));
}

function storeImageFormData(payload: StoreImageUploadRequest): FormData {
  const formData = new FormData();
  payload.files.forEach((file) => {
    formData.append(`storeImageFiles__${payload.slotKey}__${payload.locale}`, file);
  });
  return formData;
}

const STORE_IMAGE_SLOT_CONFIG = [
  { key: 'phone_screenshots', label: '手机截图' },
  { key: 'tablet_screenshots', label: '平板截图' }
] as const;

type StoreImageAssetPreview = {
  id: string;
  slotKey: string;
  storageKey: string;
  downloadUrl: string;
  fileName: string;
  canDelete: boolean;
  inherited: boolean;
  sourceLocale: string;
};

function imageAssetsBySlot(
  storeImages: Record<string, unknown>
): Record<(typeof STORE_IMAGE_SLOT_CONFIG)[number]['key'], StoreImageAssetPreview[]> {
  return {
    phone_screenshots: imageAssets(storeImages).filter(
      (asset) => asset.slotKey === 'phone_screenshots'
    ),
    tablet_screenshots: imageAssets(storeImages).filter(
      (asset) => asset.slotKey === 'tablet_screenshots'
    )
  };
}

function imageAssets(storeImages: Record<string, unknown>): StoreImageAssetPreview[] {
  const assets: StoreImageAssetPreview[] = [];
  Object.entries(storeImages).forEach(([slotKey, value]) => {
    if (!value || typeof value !== 'object') return;
    const slot = value as { assets?: unknown; preview_items?: unknown; previewItems?: unknown };
    const rawPreviewItems = Array.isArray(slot.preview_items)
      ? slot.preview_items
      : Array.isArray(slot.previewItems)
        ? slot.previewItems
        : null;
    const rawAssets = rawPreviewItems ?? (Array.isArray(slot.assets) ? slot.assets : []);
    rawAssets.forEach((item, index) => {
      if (!item || typeof item !== 'object') return;
      const asset = item as {
        url?: unknown;
        storageKey?: unknown;
        storage_key?: unknown;
        downloadUrl?: unknown;
        download_url?: unknown;
        fileName?: unknown;
        file_name?: unknown;
        canDelete?: unknown;
        inherited?: unknown;
        sourceLocale?: unknown;
        source_locale?: unknown;
      };
      const downloadUrl = stringValue(asset.url) || stringValue(asset.downloadUrl) || stringValue(asset.download_url);
      const storageKey = stringValue(asset.storageKey) || stringValue(asset.storage_key);
      const fileName = stringValue(asset.fileName) || stringValue(asset.file_name);
      const inherited = Boolean(asset.inherited);
      const canDelete =
        typeof asset.canDelete === 'boolean' ? asset.canDelete : Boolean(storageKey && !inherited);
      if (!storageKey && !downloadUrl) return;
      assets.push({
        id: storageKey || `${slotKey}-${index}-${downloadUrl}`,
        slotKey,
        storageKey,
        downloadUrl,
        fileName,
        canDelete,
        inherited,
        sourceLocale: stringValue(asset.sourceLocale) || stringValue(asset.source_locale)
      });
    });
  });
  return assets;
}

function stringValue(value: unknown): string {
  return typeof value === 'string' ? value : '';
}

function errorMessage(error: unknown): string {
  if (error instanceof AdminApiError) return error.message;
  if (error instanceof Error) return error.message;
  return '请求失败，请稍后重试';
}

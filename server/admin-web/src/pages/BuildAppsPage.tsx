import { useEffect, useMemo, useState } from 'react';
import {
  AdminApiError,
  createAgentBuild,
  loadAppDetailState,
  loadBuildAppsState,
  saveAppBuildSetting,
  type AppDetailState,
  type BuildAppSummary,
  type BuildAppItem,
  type BuildAppsState,
  type BuildItem,
  type BuildSettingItem
} from '../app/apiClient';

type BuildEnvironment = 'development' | 'production';
type SettingDraft = {
  gitUrl: string;
  runnerLabels: string;
  credentialRefs: string;
  artifactType: string;
};

export function BuildAppsPage() {
  const [state, setState] = useState<BuildAppsState | null>(null);
  const [selectedAppId, setSelectedAppId] = useState('');
  const [environment, setEnvironment] = useState<BuildEnvironment>('development');
  const [gitRef, setGitRef] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [createdBuild, setCreatedBuild] = useState<BuildItem | null>(null);
  const [showAppPicker, setShowAppPicker] = useState(false);
  const [settingsApp, setSettingsApp] = useState<BuildAppSummary | null>(null);
  const [message, setMessage] = useState('');
  const [error, setError] = useState('');

  useEffect(() => {
    let cancelled = false;
    loadBuildAppsState()
      .then((payload) => {
        if (!cancelled) setState(payload);
      })
      .catch((requestError) => {
        if (!cancelled) setError(errorMessage(requestError));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  async function refreshApps(preferredAppId?: string) {
    const payload = await loadBuildAppsState();
    setState(payload);
    const preferredApp = payload.apps.find((item) => item.app.id === preferredAppId);
    if (preferredApp) {
      const keepGitRef = selectedAppId === preferredApp.app.id;
      setSelectedAppId(preferredApp.app.id);
      setGitRef(keepGitRef && gitRef.trim() ? gitRef : defaultGitRef(preferredApp.setting));
      setCreatedBuild(null);
    }
  }

  const selectedApp = useMemo(
    () => state?.apps.find((item) => item.app.id === selectedAppId) ?? null,
    [selectedAppId, state]
  );
  function selectApp(item: BuildAppItem) {
    setSelectedAppId(item.app.id);
    setEnvironment('development');
    setGitRef(defaultGitRef(item.setting));
    setCreatedBuild(null);
    setMessage('');
    setError('');
  }

  function selectEnvironment(value: string) {
    setEnvironment(value === 'production' ? 'production' : 'development');
    setCreatedBuild(null);
    setMessage('');
    setError('');
  }

  async function submitBuild() {
    if (!selectedApp || !gitRef.trim()) return;
    setSubmitting(true);
    setMessage('');
    setError('');
    try {
      const response = await createAgentBuild(selectedApp.app.id, {
        environment,
        gitRef: gitRef.trim()
      });
      setCreatedBuild(response.build);
      setMessage(response.message);
    } catch (requestError) {
      setError(errorMessage(requestError));
    } finally {
      setSubmitting(false);
    }
  }

  function openAppPicker() {
    setShowAppPicker(true);
    setMessage('');
    setError('');
  }

  function openBuildHistory() {
    history.pushState({ adminRoute: 'builds' }, '', '/admin/builds/history');
  }

  function openBuildSettings() {
    if (!selectedApp) return;
    setSettingsApp(selectedApp.app);
  }

  if (!state && !error) {
    return <div className="empty-state">正在加载应用构建...</div>;
  }

  return (
    <div className="build-apps-layout" data-build-apps-page>
      <section className="panel build-app-list-panel">
        <div className="panel-head compact">
          <div>
            <strong>已接入应用</strong>
            <span>{state?.total ?? 0} 个应用</span>
          </div>
          <button
            className="button slim"
            type="button"
            disabled={(state?.availableApps.length ?? 0) === 0}
            onClick={openAppPicker}
          >
            接入应用
          </button>
        </div>
        <div className="build-app-list">
          {(state?.apps ?? []).map((item) => (
            <button
              key={item.app.id}
              className={item.app.id === selectedAppId ? 'build-app-option active' : 'build-app-option'}
              type="button"
              onClick={() => selectApp(item)}
            >
              <span className="app-avatar" style={{ backgroundColor: item.app.iconColor }}>
                {item.app.iconText}
              </span>
              <span className="build-app-option-copy">
                <strong>{item.app.name}</strong>
                <small>{item.app.bundleIdentifier}</small>
                <small>{item.app.platform.toUpperCase()}</small>
                <small>{repositorySummary(item)}</small>
                <span className="build-app-option-meta">
                  <small>Runner: {runnerLabels(item).join(', ') || '无标签要求'}</small>
                  <small>{matchingRunnerSummary(item)}</small>
                </span>
                <small>{latestBuildSummary(item.latestBuild, true)}</small>
              </span>
            </button>
          ))}
        </div>
      </section>

      <section className="panel build-form-panel">
        {error ? <div className="notice error">{error}</div> : null}
        {!selectedApp ? (
          <div className="workspace-selection-empty">
            <h2>{state?.apps.length ? '选择应用开始构建' : '还没有接入构建的应用'}</h2>
            <p>
              {state?.apps.length
                ? '只展示已经完成构建配置的应用。'
                : '从已有应用中选择一个，并配置一份共享的源码构建设置。'}
            </p>
            {!state?.apps.length ? (
              <button
                className="button primary"
                type="button"
                disabled={(state?.availableApps.length ?? 0) === 0}
                onClick={openAppPicker}
              >
                接入已有应用
              </button>
            ) : null}
            {!state?.apps.length && state?.availableApps.length === 0 ? (
              <small>当前没有可接入的应用。</small>
            ) : null}
          </div>
        ) : (
          <>
            <div className="panel-head compact">
              <div>
                <strong>{selectedApp.app.name}</strong>
                <span>{selectedApp.app.bundleIdentifier}</span>
              </div>
              <div className="panel-actions">
                <span className="tag">{selectedApp.app.platform.toUpperCase()}</span>
                <button className="button slim" type="button" onClick={openBuildSettings}>
                  编辑构建配置
                </button>
              </div>
            </div>
            <div className="form-grid two build-create-form">
              <label>
                <span>构建环境</span>
                <select value={environment} onChange={(event) => selectEnvironment(event.target.value)}>
                  <option value="development">开发环境</option>
                  <option value="production">线上环境</option>
                </select>
              </label>
              <label>
                <span>Git ref</span>
                <input value={gitRef} onChange={(event) => setGitRef(event.target.value)} placeholder="main" />
              </label>
            </div>
            <dl className="build-setting-summary">
              <div>
                <dt>Git 仓库</dt>
                <dd>{selectedApp.setting.gitUrl}</dd>
              </div>
              <div>
                <dt>Runner 标签</dt>
                <dd>{selectedApp.setting.runnerLabels.join(', ') || '无标签要求'}</dd>
              </div>
              <div>
                <dt>产物类型</dt>
                <dd>{selectedApp.setting.artifactType}</dd>
              </div>
              <div>
                <dt>匹配节点</dt>
                <dd>{selectedApp.matchingRunnerCount} 个在线节点</dd>
              </div>
              <div>
                <dt>凭据引用</dt>
                <dd>{credentialSummary(selectedApp.setting.credentialRefs)}</dd>
              </div>
              <div>
                <dt>最近构建</dt>
                <dd>{latestBuildSummary(selectedApp.latestBuild)}</dd>
              </div>
            </dl>
            {!selectedApp.hasOnlineRunner ? (
              <div className="notice warning compact-notice">当前无匹配在线节点</div>
            ) : null}
            {message ? (
              <div className="notice ok build-created-notice">
                <div>
                  <strong>{message}</strong>
                  {createdBuild ? (
                    <span>
                      {createdBuild.id} · {createdBuild.lifecycleStatusLabel || createdBuild.status}
                    </span>
                  ) : null}
                </div>
                <button className="button slim" type="button" onClick={openBuildHistory}>
                  查看构建记录
                </button>
              </div>
            ) : null}
            <div className="form-actions">
              <button
                className="button primary"
                type="button"
                disabled={submitting || !gitRef.trim()}
                onClick={submitBuild}
              >
                {submitting ? '正在创建...' : '立即构建'}
              </button>
            </div>
          </>
        )}
      </section>
      {showAppPicker && state ? (
        <BuildAppPickerDialog
          apps={state.availableApps}
          onClose={() => setShowAppPicker(false)}
          onSelect={(app) => {
            setShowAppPicker(false);
            setSettingsApp(app);
          }}
        />
      ) : null}
      {settingsApp ? (
        <BuildSettingsDialog
          app={settingsApp}
          onClose={() => setSettingsApp(null)}
          onSaved={async (app) => {
            await refreshApps(app.id);
            setMessage(`${app.name} 的构建配置已保存`);
          }}
        />
      ) : null}
    </div>
  );
}

function BuildAppPickerDialog({
  apps,
  onClose,
  onSelect
}: {
  apps: BuildAppSummary[];
  onClose: () => void;
  onSelect: (app: BuildAppSummary) => void;
}) {
  return (
    <div className="modal-backdrop" role="presentation">
      <section
        aria-label="接入已有应用"
        aria-modal="true"
        className="build-app-picker-dialog"
        role="dialog"
      >
        <header>
          <div>
            <h3>接入已有应用</h3>
            <p>选择应用后，在当前页面配置源码构建。</p>
          </div>
          <button className="button text" type="button" onClick={onClose}>
            关闭
          </button>
        </header>
        <div className="build-app-picker-list">
          {apps.map((app) => (
            <button key={app.id} className="build-app-picker-item" type="button" onClick={() => onSelect(app)}>
              <span className="app-avatar" style={{ backgroundColor: app.iconColor }}>
                {app.iconText}
              </span>
              <span>
                <strong>{app.name}</strong>
                <small>{app.bundleIdentifier}</small>
              </span>
              <span className="tag">{app.platform.toUpperCase()}</span>
            </button>
          ))}
        </div>
      </section>
    </div>
  );
}

function BuildSettingsDialog({
  app,
  onClose,
  onSaved
}: {
  app: BuildAppSummary;
  onClose: () => void;
  onSaved: (app: BuildAppSummary) => Promise<void>;
}) {
  const [state, setState] = useState<AppDetailState | null>(null);
  const [draft, setDraft] = useState<SettingDraft>(() => settingDraft(null, app.platform));
  const [saving, setSaving] = useState(false);
  const [message, setMessage] = useState('');
  const [error, setError] = useState('');

  useEffect(() => {
    let cancelled = false;
    loadAppDetailState(app.id)
      .then((payload) => {
        if (cancelled) return;
        setState(payload);
        setDraft(settingDraft(payload.buildSetting, app.platform));
      })
      .catch((requestError) => {
        if (!cancelled) setError(errorMessage(requestError));
      });
    return () => {
      cancelled = true;
    };
  }, [app.id, app.platform]);

  function updateDraft(patch: Partial<SettingDraft>) {
    setDraft((current) => ({ ...current, ...patch }));
  }

  async function save() {
    if (!draft.gitUrl.trim() || !draft.artifactType.trim()) return;
    setSaving(true);
    setMessage('');
    setError('');
    try {
      const response = await saveAppBuildSetting(app.id, {
        gitUrl: draft.gitUrl.trim(),
        runnerLabels: splitList(draft.runnerLabels),
        credentialRefs: parseCredentialRefs(draft.credentialRefs),
        artifactType: draft.artifactType.trim(),
        optionalDefaults: state?.buildSetting?.optionalDefaults ?? {}
      });
      setState(response.state);
      setDraft(settingDraft(response.state.buildSetting, app.platform));
      setMessage('构建配置已保存');
      try {
        await onSaved(response.state.app);
      } catch (refreshError) {
        setError(`配置已保存，但应用列表刷新失败：${errorMessage(refreshError)}`);
      }
    } catch (requestError) {
      setError(errorMessage(requestError));
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="modal-backdrop" role="presentation">
      <section
        aria-label={`${app.name} 构建配置`}
        aria-modal="true"
        className="build-settings-dialog"
        role="dialog"
      >
        <header>
          <div className="build-settings-app-summary">
            <span className="app-avatar" style={{ backgroundColor: app.iconColor }}>
              {app.iconText}
            </span>
            <span>
              <h3>{app.name}</h3>
              <p>{app.bundleIdentifier} · {app.platform.toUpperCase()}</p>
            </span>
          </div>
          <button className="button text" type="button" disabled={saving} onClick={onClose}>
            关闭
          </button>
        </header>
        {!state && !error ? <div className="empty-state inline">正在加载构建配置...</div> : null}
        {error ? <div className="notice error compact">{error}</div> : null}
        {message ? <div className="notice success compact">{message}</div> : null}
        {state ? (
          <div className="form-grid two build-settings-form">
            <label className="form-wide">
              Git 仓库
              <input
                required
                value={draft.gitUrl}
                onChange={(event) => updateDraft({ gitUrl: event.target.value })}
                placeholder="git@github.com:organization/project.git"
              />
            </label>
            <label>
              产物类型
              <select
                value={draft.artifactType}
                onChange={(event) => updateDraft({ artifactType: event.target.value })}
              >
                {artifactOptions(app.platform).map((item) => (
                  <option key={item} value={item}>{item.toUpperCase()}</option>
                ))}
              </select>
            </label>
            <label>
              节点标签
              <input
                value={draft.runnerLabels}
                onChange={(event) => updateDraft({ runnerLabels: event.target.value })}
                placeholder="可选，多个标签用逗号分隔"
              />
            </label>
            <label>
              凭据引用
              <input
                value={draft.credentialRefs}
                onChange={(event) => updateDraft({ credentialRefs: event.target.value })}
                placeholder="git: git-main, signing: ios-release"
              />
            </label>
          </div>
        ) : null}
        <footer>
          <span className="muted">
            此配置同时用于开发环境和线上环境；具体环境在发起构建时选择。
          </span>
          <button
            className="button primary"
            type="button"
            disabled={!state || saving || !draft.gitUrl.trim() || !draft.artifactType.trim()}
            onClick={() => void save()}
          >
            {saving ? '保存中...' : '保存构建配置'}
          </button>
        </footer>
      </section>
    </div>
  );
}

function defaultGitRef(setting: BuildSettingItem | null | undefined): string {
  const value = setting?.optionalDefaults.gitRef;
  return typeof value === 'string' && value.trim() ? value.trim() : 'main';
}

function repositorySummary(item: BuildAppItem): string {
  return item.setting.gitUrl;
}

function runnerLabels(item: BuildAppItem): string[] {
  return item.setting.runnerLabels;
}

function matchingRunnerSummary(item: BuildAppItem): string {
  return item.matchingRunnerCount > 0
    ? `${item.matchingRunnerCount} 个在线节点`
    : '无匹配在线节点';
}

function latestBuildSummary(build: BuildItem | null, prefix = false): string {
  if (!build) return prefix ? '最近：暂无构建记录' : '暂无构建记录';
  const value = `${build.lifecycleStatusLabel || build.status} · ${build.uploadedAtLabel}`;
  return prefix ? `最近：${value}` : value;
}

function credentialSummary(credentials: Record<string, unknown>): string {
  const values = Object.entries(credentials)
    .filter(([, value]) => String(value).trim())
    .map(([key, value]) => `${key}: ${String(value)}`);
  return values.join(', ') || '无';
}

function settingDraft(setting: BuildSettingItem | null, platform: string): SettingDraft {
  return {
    gitUrl: setting?.gitUrl ?? '',
    runnerLabels: setting?.runnerLabels.join(', ') ?? '',
    credentialRefs: setting ? credentialRefsInput(setting.credentialRefs) : '',
    artifactType: setting?.artifactType ?? artifactOptions(platform)[0]
  };
}

function artifactOptions(platform: string): string[] {
  return platform.toLowerCase() === 'android' ? ['apk', 'aab'] : ['ipa'];
}

function credentialRefsInput(credentials: Record<string, string>): string {
  return Object.entries(credentials)
    .map(([key, value]) => `${key}: ${value}`)
    .join(', ');
}

function splitList(value: string): string[] {
  return value
    .split(',')
    .map((item) => item.trim())
    .filter(Boolean);
}

function parseCredentialRefs(value: string): Record<string, string> {
  const refs: Record<string, string> = {};
  for (const item of value.split(',')) {
    const normalized = item.trim();
    if (!normalized) continue;
    const separatorIndex = normalized.indexOf(':');
    if (separatorIndex < 0) {
      refs[normalized] = normalized;
      continue;
    }
    const key = normalized.slice(0, separatorIndex).trim();
    const refValue = normalized.slice(separatorIndex + 1).trim();
    if (key && refValue) refs[key] = refValue;
  }
  return refs;
}

function errorMessage(error: unknown): string {
  if (error instanceof AdminApiError) return error.message;
  if (error instanceof Error) return error.message;
  return '请求失败，请稍后重试';
}

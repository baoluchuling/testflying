import { useEffect, useState } from 'react';
import {
  AdminApiError,
  createAgentBuild,
  loadAppDetailState,
  saveAppBuildSetting,
  type AppDetailState,
  type BuildItem,
  type BuildArtifact,
  type BuildSettingItem
} from '../app/apiClient';

type BuildEnvironment = 'development' | 'production';
type SettingDraft = {
  gitUrl: string;
  runnerLabels: string;
  credentialRefs: string;
  artifactType: string;
};

export function AppDetailPage({ appId }: { appId: string }) {
  const [state, setState] = useState<AppDetailState | null>(null);
  const [environment, setEnvironment] = useState<BuildEnvironment>('development');
  const [gitRef, setGitRef] = useState('main');
  const [draft, setDraft] = useState<SettingDraft>(() => emptyDraft());
  const [message, setMessage] = useState('');
  const [error, setError] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setState(null);
    setMessage('');
    setError('');
    loadAppDetailState(appId)
      .then((payload) => {
        if (cancelled) return;
        setState(payload);
        setEnvironment('development');
        setGitRef(payload.builds[0]?.gitRef || 'main');
        setDraft(settingToDraft(payload.buildSetting));
      })
      .catch((requestError) => {
        if (!cancelled) setError(errorMessage(requestError));
      });
    return () => {
      cancelled = true;
    };
  }, [appId]);

  function updateDraft(patch: Partial<SettingDraft>) {
    setDraft((current) => ({ ...current, ...patch }));
  }

  async function submitBuild() {
    if (!state || !state.buildSetting) {
      setError('请先保存应用构建配置。');
      return;
    }
    setSubmitting(true);
    setError('');
    setMessage('');
    try {
      const response = await createAgentBuild(state.app.id, {
        environment,
        gitRef: gitRef.trim() || 'main'
      });
      setState(response.state);
      setMessage(response.message);
    } catch (requestError) {
      setError(errorMessage(requestError));
    } finally {
      setSubmitting(false);
    }
  }

  async function saveSettings() {
    if (!state) return;
    setSaving(true);
    setError('');
    setMessage('');
    try {
      const response = await saveAppBuildSetting(state.app.id, {
        gitUrl: draft.gitUrl.trim(),
        runnerLabels: splitList(draft.runnerLabels),
        credentialRefs: parseCredentialRefs(draft.credentialRefs),
        artifactType: draft.artifactType.trim(),
        optionalDefaults: state.buildSetting?.optionalDefaults ?? {}
      });
      setState(response.state);
      setDraft(settingToDraft(response.state.buildSetting));
      setMessage(response.message);
    } catch (requestError) {
      setError(errorMessage(requestError));
    } finally {
      setSaving(false);
    }
  }

  if (error && !state) return <section className="notice error">{error}</section>;
  if (!state) return <section className="empty-state">正在加载应用...</section>;

  return (
    <div className="app-detail-page" data-app-detail-page>
      <section className="panel app-detail-overview">
        <div className="app-detail-header">
          <span className="app-avatar" style={{ backgroundColor: state.app.iconColor }}>
            {state.app.iconText}
          </span>
          <div className="app-detail-copy">
            <h2>{state.app.name}</h2>
            <p>{state.app.bundleIdentifier}</p>
            <div className="meta-line">
              <span>{state.app.platform.toUpperCase()}</span>
              <span>{state.builds.length} 条构建</span>
            </div>
          </div>
        </div>
      </section>

      {error ? <div className="notice error">{error}</div> : null}
      {message ? <div className="notice ok">{message}</div> : null}

      <div className="app-detail-grid">
        <section className="panel quick-build-panel">
          <div className="panel-head">
            <strong>快速构建</strong>
            <span>{state.buildSetting ? state.buildSetting.updatedAtLabel : '未配置'}</span>
          </div>
          <div className="form-grid two">
            <label>
              环境
              <select
                value={environment}
                onChange={(event) => setEnvironment(event.target.value as BuildEnvironment)}
              >
                <option value="development">开发环境</option>
                <option value="production">线上环境</option>
              </select>
            </label>
            <label>
              Git ref
              <input
                aria-label="Git ref"
                value={gitRef}
                onChange={(event) => setGitRef(event.target.value)}
                placeholder="main"
              />
            </label>
          </div>
          {state.buildSetting ? (
            <dl className="detail-list compact">
              <div>
                <dt>Git URL</dt>
                <dd>{state.buildSetting.gitUrl}</dd>
              </div>
              <div>
                <dt>Runner Labels</dt>
                <dd>{state.buildSetting.runnerLabels.join(', ') || '-'}</dd>
              </div>
              <div>
                <dt>产物类型</dt>
                <dd>{state.buildSetting.artifactType}</dd>
              </div>
            </dl>
          ) : (
            <div className="empty-state inline">尚未配置构建设置。</div>
          )}
          <div className="form-actions align-start">
            <button
              type="button"
              className="button primary"
              disabled={!state.buildSetting || submitting}
              onClick={() => void submitBuild()}
            >
              {submitting ? '提交中' : '立即构建'}
            </button>
          </div>
        </section>

        <section className="panel app-settings-panel">
          <div className="panel-head">
            <strong>构建设置</strong>
            <span>开发环境和线上环境共用，原始密钥不会展示</span>
          </div>
          <div className="settings-grid">
            <BuildSettingCard
              setting={state.buildSetting}
              draft={draft}
              saving={saving}
              onChange={updateDraft}
              onSave={() => void saveSettings()}
            />
          </div>
        </section>
      </div>

      <section className="panel build-history-panel">
        <div className="panel-head">
          <strong>构建历史</strong>
          <span>{state.builds.length} 条</span>
        </div>
        <div className="build-history-list">
          {state.builds.map((build) => (
            <BuildHistoryRow key={build.id} build={build} />
          ))}
        </div>
        {state.builds.length === 0 ? <div className="empty-state inline">暂无构建。</div> : null}
      </section>
    </div>
  );
}

function BuildSettingCard({
  setting,
  draft,
  saving,
  onChange,
  onSave
}: {
  setting: BuildSettingItem | null;
  draft: SettingDraft;
  saving: boolean;
  onChange: (patch: Partial<SettingDraft>) => void;
  onSave: () => void;
}) {
  return (
    <section className="setting-card">
      <div className="setting-card-head">
        <strong>应用构建配置</strong>
        <span>{setting?.updatedAtLabel || '未配置'}</span>
      </div>
      <div className="form-grid two">
        <label>
          Git URL
          <input value={draft.gitUrl} onChange={(event) => onChange({ gitUrl: event.target.value })} />
        </label>
        <label>
          Artifact Type
          <input
            value={draft.artifactType}
            onChange={(event) => onChange({ artifactType: event.target.value })}
          />
        </label>
        <label>
          Runner Labels
          <input
            value={draft.runnerLabels}
            onChange={(event) => onChange({ runnerLabels: event.target.value })}
            placeholder="ios-release, mac-sign"
          />
        </label>
        <label className="setting-card-full">
          Credential Refs
          <input
            value={draft.credentialRefs}
            onChange={(event) => onChange({ credentialRefs: event.target.value })}
            placeholder="git: git-main, signing: ios-signing"
          />
        </label>
      </div>
      <div className="setting-card-meta">
        <small>当前 refs：{setting ? credentialRefsLabel(setting.credentialRefs) : '-'}</small>
      </div>
      <div className="form-actions align-start">
        <button type="button" className="button" disabled={saving} onClick={onSave}>
          {saving ? '保存中' : '保存构建配置'}
        </button>
      </div>
    </section>
  );
}

function BuildHistoryRow({ build }: { build: BuildItem }) {
  const artifacts = buildArtifacts(build);
  const diagnostic = build.failureSummary || build.humanAction || build.recentEvents[0]?.message || '';

  return (
    <div className="build-history-row">
      <div>
        <strong>{build.version || '待解析'}</strong>
        <small>build {build.buildNumber || '-'}</small>
      </div>
      <div>
        <span>{build.environmentLabel}</span>
        <small>{build.sourceLabel || build.source || '-'}</small>
      </div>
      <div>
        <span>{build.lifecycleStatusLabel || build.lifecycleStatus || build.status}</span>
        <small>{build.uploadedAtLabel}</small>
      </div>
      <div>
        <span>{build.gitRef || '-'}</span>
        <small>{artifacts.length ? `${artifacts.length} 个产物` : '无产物'}</small>
      </div>
      <div className="build-history-artifacts">
        {artifacts.length === 0 ? <small>无可用链接</small> : null}
        {artifacts.map((artifact, index) => (
          <ArtifactLinks key={`${artifact.fileName}-${index}`} artifact={artifact} />
        ))}
      </div>
      {diagnostic ? (
        <div className="build-diagnostic">
          <span>{build.failureClassification || 'diagnostic'}</span>
          <small>{diagnostic}</small>
        </div>
      ) : null}
    </div>
  );
}

function ArtifactLinks({ artifact }: { artifact: BuildArtifact }) {
  const actions = artifactActions(artifact);
  return (
    <div className="artifact-action-group compact">
      <small>
        {artifact.artifactTypeLabel || artifact.artifactType || 'Artifact'} · {artifact.fileName}
      </small>
      <span className="inline-actions">
        {actions.map((action) => (
          <a key={action.label} className="button" href={action.href}>
            {action.label}
          </a>
        ))}
      </span>
    </div>
  );
}

function artifactActions(artifact: BuildArtifact): { label: string; href: string }[] {
  const actions: { label: string; href: string }[] = [];
  const normalizedType = artifact.artifactType || '';
  if (artifact.installUrl) actions.push({ label: '安装', href: artifact.installUrl });
  if (artifact.downloadUrl) {
    actions.push({
      label: normalizedType === 'report' ? '报告' : normalizedType === 'log' ? '日志' : '下载',
      href: artifact.downloadUrl
    });
  }
  if (artifact.manifestUrl) actions.push({ label: 'Manifest', href: artifact.manifestUrl });
  return actions;
}

function buildArtifacts(build: BuildItem): BuildArtifact[] {
  if (build.artifacts?.length) return build.artifacts;
  return build.artifact ? [build.artifact] : [];
}

function credentialRefsLabel(credentialRefs: Record<string, string>): string {
  const entries = Object.entries(credentialRefs);
  if (entries.length === 0) return '-';
  return entries.map(([key, value]) => `${key}: ${value}`).join(', ');
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

function settingToDraft(setting: BuildSettingItem | null): SettingDraft {
  if (!setting) return emptyDraft();
  return {
    gitUrl: setting.gitUrl,
    runnerLabels: setting.runnerLabels.join(', '),
    credentialRefs: credentialRefsLabel(setting.credentialRefs) === '-' ? '' : credentialRefsLabel(setting.credentialRefs),
    artifactType: setting.artifactType
  };
}

function emptyDraft(): SettingDraft {
  return {
    gitUrl: '',
    runnerLabels: '',
    credentialRefs: '',
    artifactType: ''
  };
}

function errorMessage(error: unknown): string {
  if (error instanceof AdminApiError) return error.message;
  if (error instanceof Error) return error.message;
  return '请求失败，请稍后重试';
}

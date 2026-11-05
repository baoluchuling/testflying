import { useEffect, useState } from 'react';
import {
  AdminApiError,
  loadBuildsState,
  type BuildItem,
  type BuildsState
} from '../app/apiClient';

export function BuildHistoryPage() {
  const [state, setState] = useState<BuildsState | null>(null);
  const [error, setError] = useState('');
  const [copied, setCopied] = useState('');

  useEffect(() => {
    let cancelled = false;
    loadBuildsState()
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

  async function copy(label: string, value: string) {
    if (!value) return;
    await navigator.clipboard?.writeText(value);
    setCopied(label);
  }

  function navigateToApp(appId: string) {
    history.pushState({ adminRoute: 'apps' }, '', `/admin/apps/${encodeURIComponent(appId)}`);
  }

  return (
    <section className="panel table-panel" data-builds-page>
      <div className="panel-head">
        <strong>构建列表</strong>
        <span>{state?.total ?? 0} 个构建</span>
      </div>
      {error ? <div className="notice error">{error}</div> : null}
      {copied ? <div className="notice ok">已复制 {copied}</div> : null}
      <div className="data-table builds-table" role="table" aria-label="构建列表">
        <div className="data-table-row header" role="row">
          <span>应用</span>
          <span>版本</span>
          <span>平台</span>
          <span>来源 / 状态</span>
          <span>产物</span>
          <span>操作</span>
        </div>
        {(state?.builds ?? []).map((build) => (
          <BuildRow key={build.id} build={build} onCopy={copy} onOpenApp={navigateToApp} />
        ))}
      </div>
      {!state && !error ? <div className="empty-state">正在加载构建...</div> : null}
      {state && state.builds.length === 0 ? <div className="empty-state">暂无构建。</div> : null}
    </section>
  );
}

function BuildRow({
  build,
  onCopy,
  onOpenApp
}: {
  build: BuildItem;
  onCopy: (label: string, value: string) => Promise<void>;
  onOpenApp: (appId: string) => void;
}) {
  const artifacts = build.artifacts?.length ? build.artifacts : build.artifact ? [build.artifact] : [];

  return (
    <div className="data-table-row build-row" role="row">
      <button type="button" className="link-button app-cell build-app-link" onClick={() => onOpenApp(build.app.id)}>
        <span className="app-avatar" style={{ backgroundColor: build.app.iconColor }}>
          {build.app.iconText}
        </span>
        <span>
          <strong>{build.app.name}</strong>
          <small>{build.app.bundleIdentifier}</small>
        </span>
      </button>
      <span>
        <strong>{build.version}</strong>
        <small>build {build.buildNumber}</small>
      </span>
      <span>{build.platformLabel}</span>
      <span>
        <strong>{build.sourceLabel || build.environmentLabel}</strong>
        <small>{build.failureSummary || build.lifecycleStatusLabel || build.lifecycleStatus || build.status}</small>
      </span>
      <span>
        <strong>{artifacts[0]?.sizeLabel ?? '-'}</strong>
        <small>{artifacts.length ? `${artifacts.length} 个产物` : build.environmentLabel}</small>
      </span>
      <span className="artifact-actions">
        {artifacts.length === 0 ? <small>无可用链接</small> : null}
        {artifacts.map((artifact, index) => (
          <div key={`${artifact.fileName}-${index}`} className="artifact-action-group">
            <small>{artifact.fileName}</small>
            <span className="inline-actions">
              <button
                type="button"
                className="button"
                disabled={!artifact.installUrl}
                onClick={() => onCopy(`${artifact.fileName}: installUrl`, artifact.installUrl)}
              >
                复制安装
              </button>
              <button
                type="button"
                className="button"
                disabled={!artifact.downloadUrl}
                onClick={() => onCopy(`${artifact.fileName}: downloadUrl`, artifact.downloadUrl)}
              >
                下载地址
              </button>
            </span>
          </div>
        ))}
      </span>
    </div>
  );
}

function errorMessage(error: unknown): string {
  if (error instanceof AdminApiError) return error.message;
  if (error instanceof Error) return error.message;
  return '请求失败，请稍后重试';
}

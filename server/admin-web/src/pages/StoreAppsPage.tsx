import { useEffect, useState, type MouseEvent } from 'react';
import {
  AdminApiError,
  loadStoreApps,
  type StoreAppItem,
  type StoreAppsState
} from '../app/apiClient';

const filters = [
  { label: '全部', value: 'all' },
  { label: 'iOS', value: 'ios' },
  { label: 'Android', value: 'android' },
  { label: '需处理', value: 'needs' },
  { label: '可同步', value: 'ok' }
];

type LoadingState = 'idle' | 'load';

export function StoreAppsPage() {
  const [state, setState] = useState<StoreAppsState | null>(null);
  const [loading, setLoading] = useState<LoadingState>('load');
  const [error, setError] = useState('');

  useEffect(() => {
    void loadFromLocation();
    const onPopState = () => void loadFromLocation();
    window.addEventListener('popstate', onPopState);
    return () => window.removeEventListener('popstate', onPopState);
  }, []);

  async function loadFromLocation() {
    setLoading('load');
    setError('');
    try {
      setState(await loadStoreApps(location.search || ''));
    } catch (requestError) {
      setError(errorMessage(requestError));
    } finally {
      setLoading('idle');
    }
  }

  async function selectFilter(value: string) {
    const nextPath = storeAppsPath({ filter: value, appId: '' });
    history.pushState({ adminRoute: 'apps', filter: value }, '', nextPath);
    await loadFromLocation();
  }

  async function selectApp(app: StoreAppItem) {
    const nextPath = storeAppsPath({ filter: state?.filter ?? 'all', appId: app.id });
    history.pushState({ adminRoute: 'apps', appId: app.id }, '', nextPath);
    await loadFromLocation();
  }

  function openReviews(app: StoreAppItem) {
    if (!app.reviewsPath) return;
    history.pushState({ adminRoute: 'store-reviews' }, '', app.reviewsPath);
    window.dispatchEvent(new PopStateEvent('popstate'));
  }

  function openInternalPath(event: MouseEvent<HTMLAnchorElement>, path: string) {
    event.preventDefault();
    event.stopPropagation();
    if (!path) return;
    history.pushState({ adminRoute: 'accounts' }, '', path);
    window.dispatchEvent(new PopStateEvent('popstate'));
  }

  return (
    <div className="compact-page compact-store-apps-page store-apps-page" data-store-apps-page>
      {error ? <div className="notice error">{error}</div> : null}

      <div className="compact-context">
        <div className="compact-title">
          <strong>Store Operations</strong>
          <h2>商店管理</h2>
          <span>
            从应用直接进入商店内容 · 账号配置只是辅助入口 · {state?.stats.total ?? 0} 个应用 · {state?.stats.ready ?? 0} 个可同步 · {state?.stats.needs ?? 0} 个需处理
          </span>
        </div>
        <div className="compact-actions">
          <button className="button" type="button" onClick={() => void loadFromLocation()} disabled={loading !== 'idle'}>
            {loading === 'load' ? '刷新中' : '刷新状态'}
          </button>
          <button
            className="button primary"
            type="button"
            onClick={() => {
              history.pushState({ adminRoute: 'uploads' }, '', '/admin/uploads');
              window.dispatchEvent(new PopStateEvent('popstate'));
            }}
          >
            上传新包
          </button>
        </div>
      </div>

      <div className="compact-body">
        <div className="compact-grid">
          <section className="compact-column">
            <div className="compact-column-head">
              <strong>商店应用</strong>
              <span>点击应用直接进入商店管理</span>
            </div>
            <div className="compact-filter-line">
              <div className="segmented" aria-label="商店应用筛选">
                {filters.map((filter) => (
                  <button
                    key={filter.value}
                    className={state?.filter === filter.value ? 'active' : ''}
                    type="button"
                    onClick={() => void selectFilter(filter.value)}
                  >
                    {filter.label}
                  </button>
                ))}
              </div>
              <div className="meta">
                <span>{state?.stats.total ?? 0} 个应用</span>
                <span>{state?.stats.ios ?? 0} 个 iOS</span>
                <span>{state?.stats.android ?? 0} 个 Android</span>
              </div>
            </div>
            <div className="compact-scroll compact-table">
              <div className="store-app-table table-list" role="table" aria-label="商店应用">
                <div className="store-app-table-row table-row header" role="row">
                  <span>应用</span>
                  <span>平台</span>
                  <span>开发者账号</span>
                  <span>商店状态</span>
                  <span>操作</span>
                </div>
                {state?.apps.map((app) => (
                  <div
                    key={app.id}
                    className={app.selected ? 'store-app-table-row table-row active' : 'store-app-table-row table-row'}
                    role="row"
                    tabIndex={0}
                    onClick={() => void selectApp(app)}
                  >
                    <span className="app-cell">
                      <span className="app-logo" style={{ backgroundColor: app.iconColor }}>
                        {app.iconText}
                      </span>
                      <span>
                        <strong>{app.name}</strong>
                        <span>{app.bundleIdentifier}</span>
                        <small>{buildLabel(app)}</small>
                      </span>
                    </span>
                    <span>{platformLabel(app.platform)}</span>
                    <span>{app.developerAccountName || '未绑定'}</span>
                    <span className={`tag ${app.status === 'ready' ? 'ok' : 'warn'}`}>
                      {app.statusLabel}
                    </span>
                    <span className="store-app-row-actions row-actions">
                      {app.storeManagementPath ? (
                        <a
                          className="button primary"
                          href={app.storeManagementPath}
                          onClick={(event) => openInternalPath(event, app.storeManagementPath)}
                        >
                          商店管理
                        </a>
                      ) : (
                        <a
                          className="button"
                          href="/admin/accounts"
                          onClick={(event) => openInternalPath(event, '/admin/accounts')}
                        >
                          绑定账号
                        </a>
                      )}
                      <a
                        className="button"
                        href={app.reviewsPath || '/admin/store-reviews'}
                        aria-disabled={!app.reviewsPath}
                        onClick={(event) => {
                          event.preventDefault();
                          event.stopPropagation();
                          openReviews(app);
                        }}
                      >
                        评论分析
                      </a>
                    </span>
                  </div>
                ))}
              </div>
              {loading === 'load' ? <div className="empty-state">正在加载商店应用...</div> : null}
              {state && state.apps.length === 0 && loading === 'idle' ? (
                <div className="empty-state">当前筛选下没有应用。</div>
              ) : null}
            </div>
          </section>

          <aside className="compact-column">
            <div className="compact-column-head">
              <strong>账号与连接</strong>
              <a
                className="button text"
                href="/admin/accounts"
                onClick={(event) => openInternalPath(event, '/admin/accounts')}
              >
                账号列表
              </a>
            </div>
            <div className="compact-scroll compact-side-list">
              <CompactSideCard
                title="开发者账号"
                description={`${state?.accountSummary.totalAccounts ?? 0} 个账号，账号配置、Connector 和 App 绑定都从这里进入。`}
              />
              <CompactSideCard
                title="Connector 状态"
                description={`${state?.accountSummary.connectorOk ?? 0} 个正常，${state?.accountSummary.connectorNeeds ?? 0} 个需要检查。`}
                summary={state?.accountSummary.connectorNeeds ? 'Connector 需处理' : 'Connector 正常'}
                tag={state?.accountSummary.connectorNeeds ? '需处理' : '正常'}
                tone={state?.accountSummary.connectorNeeds ? 'warn' : 'ok'}
              />
              <CompactSideCard
                title="账号续费提醒"
                description={`${state?.accountSummary.renewalReminders ?? 0} 个提醒。`}
                tag={state?.accountSummary.renewalReminders ? '提醒' : '正常'}
                tone={state?.accountSummary.renewalReminders ? 'warn' : 'ok'}
              />
              <CompactSideCard
                title="已绑定应用"
                description={`${state?.accountSummary.boundApps ?? 0} 个应用已经绑定开发者账号。`}
              />
            </div>
          </aside>
        </div>
      </div>
    </div>
  );
}

function CompactSideCard({
  title,
  description,
  summary,
  tag,
  tone = ''
}: {
  title: string;
  description: string;
  summary?: string;
  tag?: string;
  tone?: string;
}) {
  return (
    <div className="compact-side-card">
      <strong>{title}</strong>
      {summary ? <span className="compact-side-card-summary">{summary}</span> : null}
      <span>{description}</span>
      {tag ? <span className={`tag ${tone}`}>{tag}</span> : null}
    </div>
  );
}

function storeAppsPath({ filter, appId }: { filter: string; appId: string }) {
  const params = new URLSearchParams();
  if (filter && filter !== 'all') params.set('filter', filter);
  if (appId) params.set('appId', appId);
  const query = params.toString();
  return query ? `/admin/apps?${query}` : '/admin/apps';
}

function platformLabel(platform: string) {
  if (platform === 'ios') return 'iOS';
  if (platform === 'android') return 'Android';
  return platform || '未知';
}

function buildLabel(app: StoreAppItem) {
  if (!app.latestBuild) return '暂无构建';
  return `${app.latestBuild.version} (${app.latestBuild.buildNumber})`;
}

function errorMessage(error: unknown): string {
  if (error instanceof AdminApiError) return error.message;
  if (error instanceof Error) return error.message;
  return '请求失败，请稍后重试';
}

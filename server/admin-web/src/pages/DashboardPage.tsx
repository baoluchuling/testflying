import { useEffect, useState } from 'react';
import {
  AdminApiError,
  loadDashboardState,
  type BuildItem,
  type DashboardState,
  type NotificationItem
} from '../app/apiClient';

export function DashboardPage() {
  const [state, setState] = useState<DashboardState | null>(null);
  const [error, setError] = useState('');

  useEffect(() => {
    let cancelled = false;
    loadDashboardState()
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

  return (
    <div className="dashboard-page" data-dashboard-page>
      {error ? <div className="notice error">{error}</div> : null}

      <section className="stat-grid" aria-label="后台指标">
        {(state?.stats ?? []).map((stat) => (
          <article key={stat.label} className={`stat-card ${stat.tone}`}>
            <span>{stat.label}</span>
            <strong>{stat.value}</strong>
          </article>
        ))}
        {!state && !error ? <div className="empty-state">正在加载总览...</div> : null}
      </section>

      <section className="dashboard-actions">
        <button type="button" className="quick-action" onClick={() => navigate('/admin/uploads')}>
          <strong>上传构建</strong>
          <span>上传 IPA / APK 并解析包信息</span>
        </button>
        <button type="button" className="quick-action" onClick={() => navigate('/admin/apps')}>
          <strong>商店管理</strong>
          <span>进入应用商店内容和账号连接</span>
        </button>
        <button type="button" className="quick-action" onClick={() => navigate('/admin/app-logs')}>
          <strong>App 日志</strong>
          <span>查看手机端实时日志流</span>
        </button>
      </section>

      <div className="dashboard-grid">
        <section className="panel table-panel">
          <div className="panel-head compact">
            <strong>最近构建</strong>
            <button type="button" className="button" onClick={() => navigate('/admin/builds')}>
              全部构建
            </button>
          </div>
          <div className="data-table dashboard-builds" role="table" aria-label="最近构建">
            <div className="data-table-row header" role="row">
              <span>应用</span>
              <span>版本</span>
              <span>平台</span>
              <span>环境</span>
              <span>上传时间</span>
            </div>
            {(state?.recentBuilds ?? []).map((build) => (
              <BuildRow key={build.id} build={build} />
            ))}
          </div>
          {state && state.recentBuilds.length === 0 ? (
            <div className="empty-state">还没有上传构建。</div>
          ) : null}
        </section>

        <section className="panel">
          <div className="panel-head compact">
            <strong>最近通知</strong>
            <button
              type="button"
              className="button"
              onClick={() => navigate('/admin/notifications')}
            >
              全部通知
            </button>
          </div>
          <div className="simple-list">
            {(state?.recentNotifications ?? []).map((notification) => (
              <NotificationRow key={notification.id} notification={notification} />
            ))}
          </div>
          {state && state.recentNotifications.length === 0 ? (
            <div className="empty-state">暂无通知。</div>
          ) : null}
        </section>
      </div>
    </div>
  );
}

function BuildRow({ build }: { build: BuildItem }) {
  return (
    <div className="data-table-row" role="row">
      <span className="app-cell">
        <span className="app-avatar" style={{ backgroundColor: build.app.iconColor }}>
          {build.app.iconText}
        </span>
        <span>
          <strong>{build.app.name}</strong>
          <small>{build.app.bundleIdentifier}</small>
        </span>
      </span>
      <span>{build.version} ({build.buildNumber})</span>
      <span>{build.platformLabel}</span>
      <span>{build.environmentLabel}</span>
      <span>{build.uploadedAtLabel}</span>
    </div>
  );
}

function NotificationRow({ notification }: { notification: NotificationItem }) {
  return (
    <article className="simple-list-row">
      <div>
        <strong>{notification.title}</strong>
        <span>{notification.subtitle}</span>
      </div>
      <em>{notification.createdAtLabel}</em>
    </article>
  );
}

function navigate(path: string) {
  history.pushState({ adminRoute: path }, '', path);
  window.dispatchEvent(new PopStateEvent('popstate'));
}

function errorMessage(error: unknown): string {
  if (error instanceof AdminApiError) return error.message;
  if (error instanceof Error) return error.message;
  return '请求失败，请稍后重试';
}

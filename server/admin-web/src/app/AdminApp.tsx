import { useEffect, useMemo, useState } from 'react';
import { bootstrapAdmin, type BootstrapResponse } from './apiClient';
import {
  buildViewFromPath,
  navKeyFromPath,
  routeKeyFromPath,
  routeTitles,
  settingsViewFromPath,
  type AdminRouteKey
} from './routes';
import { StoreAppsPage } from '../pages/StoreAppsPage';
import { StoreReviewsPage } from '../pages/StoreReviewsPage';
import { UploadPage } from '../pages/UploadPage';
import { AppLogsPage } from '../pages/AppLogsPage';
import { DashboardPage } from '../pages/DashboardPage';
import { BuildWorkspacePage } from '../pages/BuildWorkspacePage';
import { AppDetailPage } from '../pages/AppDetailPage';
import { DevicesPage } from '../pages/DevicesPage';
import { NotificationsPage } from '../pages/NotificationsPage';
import { ApiDocsPage } from '../pages/ApiDocsPage';
import { DeveloperAccountsPage } from '../pages/DeveloperAccountsPage';
import { NotFoundPage } from '../pages/NotFoundPage';
import { SettingsPage } from '../pages/SettingsPage';

const fallbackNav = [
  { key: 'dashboard', label: '总览', path: '/admin' },
  { key: 'uploads', label: '上传', path: '/admin/uploads' },
  { key: 'apps', label: '商店管理', path: '/admin/apps' },
  { key: 'store-reviews', label: '商店评论', path: '/admin/store-reviews' },
  { key: 'api-docs', label: '接口文档', path: '/admin/api-docs' },
  { key: 'builds', label: '构建', path: '/admin/builds/apps' },
  { key: 'devices', label: '设备', path: '/admin/devices' },
  { key: 'app-logs', label: 'App 日志', path: '/admin/app-logs' },
  { key: 'notifications', label: '通知', path: '/admin/notifications' },
  { key: 'settings', label: '设置', path: '/admin/settings/general' }
] as const;

type HealthState = BootstrapResponse['health'] & { checking?: boolean };

export function AdminApp() {
  const [activeRoute, setActiveRoute] = useState<AdminRouteKey>(() => routeKeyFromPath(location.pathname));
  const [activeNavRoute, setActiveNavRoute] = useState<AdminRouteKey>(() => navKeyFromPath(location.pathname));
  const [activePath, setActivePath] = useState(() => location.pathname);
  const [bootstrap, setBootstrap] = useState<BootstrapResponse | null>(null);
  const [health, setHealth] = useState<HealthState>({ state: 'idle', label: '未检查' });
  const [error, setError] = useState('');

  useEffect(() => {
    bootstrapAdmin()
      .then((payload) => {
        setBootstrap(payload);
        setHealth(payload.health);
      })
      .catch((requestError: Error) => setError(requestError.message));
  }, []);

  useEffect(() => {
    const originalPushState = history.pushState;
    let currentPath = location.pathname;
    let currentState = history.state;

    const navigationAllowed = (nextPath: string) =>
      window.dispatchEvent(
        new CustomEvent('admin:before-navigation', {
          cancelable: true,
          detail: { from: currentPath, to: nextPath }
        })
      );

    history.pushState = function pushStateWithAdminNavigation(data, unused, url) {
      const nextPath = url === undefined ? currentPath : new URL(String(url), location.href).pathname;
      if (nextPath !== currentPath && !navigationAllowed(nextPath)) return;
      originalPushState.call(this, data, unused, url);
      currentPath = location.pathname;
      currentState = history.state;
      window.dispatchEvent(new Event('admin:navigation'));
    };

    const syncRoute = () => {
      setActivePath(location.pathname);
      setActiveRoute(routeKeyFromPath(location.pathname));
      setActiveNavRoute(navKeyFromPath(location.pathname));
    };
    const handlePopState = () => {
      const nextPath = location.pathname;
      if (nextPath !== currentPath && !navigationAllowed(nextPath)) {
        originalPushState.call(history, currentState, '', currentPath);
        return;
      }
      currentPath = nextPath;
      currentState = history.state;
      syncRoute();
    };
    const handleAdminNavigation = () => {
      currentPath = location.pathname;
      currentState = history.state;
      syncRoute();
    };
    window.addEventListener('popstate', handlePopState);
    window.addEventListener('admin:navigation', handleAdminNavigation);
    return () => {
      history.pushState = originalPushState;
      window.removeEventListener('popstate', handlePopState);
      window.removeEventListener('admin:navigation', handleAdminNavigation);
    };
  }, []);

  const navItems = bootstrap?.navItems ?? fallbackNav;
  const title = useMemo(
    () => routeTitles[activeRoute === 'not-found' ? 'not-found' : activeNavRoute],
    [activeNavRoute, activeRoute]
  );
  const appDetailMatch = location.pathname.match(/^\/admin\/apps\/([^/]+)$/);
  const appDetailId = appDetailMatch ? decodeURIComponent(appDetailMatch[1]) : null;

  function navigate(path: string, key: AdminRouteKey) {
    if (location.pathname !== path) {
      history.pushState({ adminRoute: key }, '', path);
    }
  }

  async function checkHealth() {
    setHealth((current) => ({ ...current, checking: true, label: '检查中' }));
    try {
      const response = await fetch('/health', { headers: { Accept: 'application/json' }, cache: 'no-store' });
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      const payload = await response.json();
      if (payload.status !== 'ok') throw new Error('health_not_ok');
      setHealth({ state: 'ok', label: '正常' });
    } catch {
      setHealth({ state: 'error', label: '异常' });
    }
  }

  return (
    <div className="admin-shell" data-admin-app-shell data-route={activeNavRoute}>
      <header className="topbar">
        <div className="brand">
          <span className="brand-mark">TF</span>
          <strong>testflying</strong>
        </div>
        <nav className="global-nav" aria-label="主导航">
          {navItems.map((item) => {
            const key = item.key as AdminRouteKey;
            return (
              <button
                key={item.key}
                className={key === activeNavRoute ? 'nav-link active' : 'nav-link'}
                type="button"
                onClick={() => navigate(item.path, key)}
              >
                {item.label}
              </button>
            );
          })}
        </nav>
        <div className="topbar-actions">
          <button className="button" type="button" onClick={checkHealth} disabled={health.checking}>
            服务健康
          </button>
          <span className={`status-pill ${health.state}`}>{health.label}</span>
        </div>
      </header>

      <main className="admin-main">
        <section className="page-title-row">
          <div>
            <p className="eyebrow">{title.eyebrow}</p>
            <h1>{title.title}</h1>
            <p>{title.summary}</p>
          </div>
        </section>

        {error ? (
          <section className="notice error">
            <strong>新后台初始化失败</strong>
            <span>{error}</span>
          </section>
        ) : null}

        {activeRoute === 'dashboard' ? <DashboardPage /> : null}
        {activeRoute === 'uploads' ? <UploadPage /> : null}
        {activeRoute === 'apps' && appDetailId ? <AppDetailPage appId={appDetailId} /> : null}
        {activeRoute === 'apps' && !appDetailId ? <StoreAppsPage /> : null}
        {activeRoute === 'accounts' ? <DeveloperAccountsPage /> : null}
        {activeRoute === 'store-reviews' ? <StoreReviewsPage /> : null}
        {activeRoute === 'api-docs' ? <ApiDocsPage /> : null}
        {activeRoute === 'builds' ? <BuildWorkspacePage view={buildViewFromPath(activePath)} /> : null}
        {activeRoute === 'devices' ? <DevicesPage /> : null}
        {activeRoute === 'app-logs' ? <AppLogsPage /> : null}
        {activeRoute === 'notifications' ? <NotificationsPage /> : null}
        {activeRoute === 'settings' ? <SettingsPage view={settingsViewFromPath(activePath)} /> : null}
        {activeRoute === 'not-found' ? <NotFoundPage /> : null}
      </main>
    </div>
  );
}

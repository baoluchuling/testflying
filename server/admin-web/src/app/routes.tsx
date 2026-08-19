export type AdminRouteKey =
  | 'dashboard'
  | 'uploads'
  | 'apps'
  | 'accounts'
  | 'store-reviews'
  | 'api-docs'
  | 'builds'
  | 'devices'
  | 'app-logs'
  | 'notifications'
  | 'settings'
  | 'not-found';

export type BuildView = 'apps' | 'history' | 'runners';
export type SettingsView = 'general' | 'notifications' | 'llm' | 'runtime';

const routeKeys = new Set<AdminRouteKey>([
  'dashboard',
  'uploads',
  'apps',
  'accounts',
  'store-reviews',
  'api-docs',
  'builds',
  'devices',
  'app-logs',
  'notifications',
  'settings'
]);

export function routeKeyFromPath(pathname: string): AdminRouteKey {
  const relative = pathname.replace(/^\/admin\/?/, '').replace(/^\/+/, '');
  const parts = relative.split('/').filter(Boolean);
  const first = parts[0] || 'dashboard';
  if (first === 'builds') {
    return parts.length === 1 ||
      (parts.length === 2 && ['apps', 'history', 'runners'].includes(parts[1]))
      ? 'builds'
      : 'not-found';
  }
  if (first === 'settings') {
    return parts.length === 1 ||
      (parts.length === 2 && ['general', 'notifications', 'llm', 'runtime'].includes(parts[1]))
      ? 'settings'
      : 'not-found';
  }
  return routeKeys.has(first as AdminRouteKey) ? (first as AdminRouteKey) : 'not-found';
}

export function navKeyFromPath(pathname: string): AdminRouteKey {
  if (/^\/admin\/accounts\/[^/]+\/apps\/[^/]+\//.test(pathname)) {
    return 'apps';
  }
  return routeKeyFromPath(pathname);
}

export function buildViewFromPath(pathname: string): BuildView {
  const view = pathname.match(/^\/admin\/builds\/([^/?#]+)/)?.[1];
  if (view === 'history' || view === 'runners') return view;
  return 'apps';
}

export function settingsViewFromPath(pathname: string): SettingsView {
  const view = pathname.match(/^\/admin\/settings\/([^/?#]+)/)?.[1];
  if (view === 'notifications' || view === 'llm' || view === 'runtime') return view;
  return 'general';
}

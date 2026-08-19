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

export const routeTitles: Record<AdminRouteKey, { eyebrow: string; title: string; summary: string }> = {
  dashboard: {
    eyebrow: 'Internal Distribution',
    title: '总览',
    summary: '查看应用、构建、设备和账号提醒的当前状态。'
  },
  uploads: {
    eyebrow: 'Package Intake',
    title: '上传',
    summary: '自动解析包信息，支持 iOS / Android，不会提交到商店。'
  },
  apps: {
    eyebrow: 'Applications',
    title: '应用',
    summary: '查看应用详情、构建配置、构建历史和商店管理入口。'
  },
  accounts: {
    eyebrow: 'Store Accounts',
    title: '开发者账号',
    summary: '维护账号配置、Connector 连接和 App 绑定。'
  },
  'store-reviews': {
    eyebrow: 'Store Reviews',
    title: '商店评论',
    summary: '增量拉取最近评论，并通过 LLM 归纳需要关注的问题。'
  },
  'api-docs': {
    eyebrow: 'Developer API',
    title: '接口文档',
    summary: '查看商店管理对外接口、参数和调用示例。'
  },
  builds: {
    eyebrow: 'Build Automation',
    title: '构建',
    summary: '选择已接入应用发起构建，并查看历史与节点状态。'
  },
  devices: {
    eyebrow: 'Internal Distribution',
    title: '设备',
    summary: '查看已登记设备、平台、系统和签名状态。'
  },
  'app-logs': {
    eyebrow: 'App Log Console',
    title: 'App 日志',
    summary: 'App 日志会迁移为保持连接状态的前端页面。'
  },
  notifications: {
    eyebrow: 'Internal Distribution',
    title: '通知',
    summary: '按类型筛选构建、账号和设备相关通知。'
  },
  settings: {
    eyebrow: 'System Configuration',
    title: '设置',
    summary: '维护通用配置、通知渠道、LLM 和运行环境。'
  },
  'not-found': {
    eyebrow: 'Not Found',
    title: '页面不存在',
    summary: '当前地址没有对应的后台页面。'
  }
};

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

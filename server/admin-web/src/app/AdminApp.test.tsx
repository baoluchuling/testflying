import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AdminApp } from './AdminApp';
import type {
  AppDetailState,
  BuildRunnersState,
  BuildsState,
  DashboardState,
  StoreReviewsState
} from './apiClient';

const bootstrapPayload = {
  appName: 'testflying',
  navItems: [
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
  ],
  health: { state: 'idle', label: '未检查' }
};

describe('AdminApp', () => {
  beforeEach(() => {
    history.replaceState(null, '', '/admin');
    vi.spyOn(globalThis, 'fetch').mockImplementation((input) => {
      const url = String(input);
      if (url === '/admin/api/bootstrap') {
        return jsonResponse(bootstrapPayload);
      }
      if (url === '/admin/api/dashboard') {
        return jsonResponse(dashboardState);
      }
      if (url.startsWith('/admin/api/store-reviews')) {
        return jsonResponse(emptyReviewsState);
      }
      if (url === '/admin/api/builds') {
        return jsonResponse(buildsState);
      }
      if (url === '/admin/api/builds/apps') {
        return jsonResponse({ apps: [], availableApps: [], total: 0 });
      }
      if (url === '/admin/api/build-runners') {
        return jsonResponse(buildRunnersState);
      }
      if (url === '/admin/api/apps/app-1') {
        return jsonResponse(appDetailState);
      }
      return Promise.reject(new Error(`unexpected fetch ${url}`));
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('switches top-level tabs without assigning a new document location', async () => {
    const pushState = vi.spyOn(history, 'pushState');
    const user = userEvent.setup();

    render(<AdminApp />);
    await screen.findByText('服务健康');
    await user.click(screen.getByRole('button', { name: '商店评论' }));

    expect(pushState).toHaveBeenCalledWith(
      { adminRoute: 'store-reviews' },
      '',
      '/admin/store-reviews'
    );
    expect(location.pathname).toBe('/admin/store-reviews');
    expect(screen.getByRole('heading', { level: 1, name: '商店评论' })).toBeTruthy();
  });

  it('lets an in-flight one-time operation block push and pop navigation', async () => {
    const user = userEvent.setup();
    const blockNavigation = (event: Event) => event.preventDefault();

    render(<AdminApp />);
    await screen.findByText('服务健康');
    window.addEventListener('admin:before-navigation', blockNavigation);

    await user.click(screen.getByRole('button', { name: '上传' }));
    expect(location.pathname).toBe('/admin');
    expect(screen.getByRole('button', { name: '总览' }).className).toContain('active');

    history.replaceState(null, '', '/admin/uploads');
    window.dispatchEvent(new PopStateEvent('popstate'));
    expect(location.pathname).toBe('/admin');
    expect(screen.getByRole('button', { name: '总览' }).className).toContain('active');

    window.removeEventListener('admin:before-navigation', blockNavigation);
  });

  it('renders ordinary pages inside the React shell', async () => {
    const user = userEvent.setup();

    render(<AdminApp />);
    await screen.findByText('最近构建');
    await user.click(screen.getByRole('button', { name: '构建' }));

    expect(location.pathname).toBe('/admin/builds/apps');
    expect(await screen.findByRole('heading', { level: 2, name: '还没有接入构建的应用' })).toBeTruthy();
    expect(screen.queryByText('新后台重构中')).toBeNull();
  });

  it('switches build subviews inside the shell', async () => {
    const user = userEvent.setup();

    render(<AdminApp />);
    await screen.findByText('服务健康');
    await user.click(screen.getByRole('button', { name: '构建' }));
    await user.click(screen.getByRole('button', { name: '构建节点' }));

    expect(location.pathname).toBe('/admin/builds/runners');
    expect(await screen.findByText('Mac mini 1')).toBeTruthy();
    expect(screen.getByRole('heading', { level: 1, name: '构建' })).toBeTruthy();
  });

  it('moves LLM configuration under settings', async () => {
    const user = userEvent.setup();

    render(<AdminApp />);
    await screen.findByText('服务健康');
    await user.click(screen.getByRole('button', { name: '设置' }));
    await user.click(screen.getByRole('button', { name: 'LLM 配置' }));

    expect(location.pathname).toBe('/admin/settings/llm');
    expect(screen.getByRole('heading', { level: 1, name: '设置' })).toBeTruthy();
  });

  it('renders removed top-level routes as not found', async () => {
    history.replaceState(null, '', '/admin/llm-config');

    render(<AdminApp />);

    expect(await screen.findByRole('heading', { level: 2, name: '页面不存在' })).toBeTruthy();
  });

  it('renders unknown workspace child routes as not found', async () => {
    history.replaceState(null, '', '/admin/settings/unknown');

    render(<AdminApp />);

    expect(await screen.findByRole('heading', { level: 2, name: '页面不存在' })).toBeTruthy();
  });

  it('renders app detail inside the shell while keeping the apps nav active', async () => {
    history.replaceState(null, '', '/admin/apps/app-1');

    render(<AdminApp />);

    expect(await screen.findByRole('heading', { level: 2, name: 'lookrva' })).toBeTruthy();
    expect(screen.getByRole('button', { name: '商店管理' }).className).toContain('active');
    expect(screen.getByText('构建历史')).toBeTruthy();
  });
});

const dashboardState: DashboardState = {
  stats: [
    { label: '应用', value: '1', tone: 'neutral' },
    { label: '构建', value: '1', tone: 'neutral' }
  ],
  recentBuilds: [],
  recentNotifications: []
};

const emptyReviewsState: StoreReviewsState = {
  apps: [],
  selectedAccountId: null,
  selectedAppId: null,
  rating: null,
  stats: { total: 0, low: 0, ios: 0, android: 0 },
  reviews: [],
  latestFetch: null,
  latestAnalysis: null,
  analysisIssues: [],
  analysisBoundaries: []
};

const buildsState: BuildsState = {
  total: 1,
  builds: [
    {
      id: 'build-1',
      app: {
        id: 'app-1',
        name: 'lookrva',
        bundleIdentifier: 'com.example.lookrva',
        platform: 'ios',
        iconColor: '#171717',
        iconText: 'LO'
      },
      version: '1.0',
      buildNumber: '1',
      platform: 'ios',
      platformLabel: 'iOS',
      environment: 'development',
      environmentLabel: '开发环境',
      source: 'upload',
      sourceLabel: '上传',
      lifecycleStatus: 'succeeded',
      lifecycleStatusLabel: '成功',
      status: 'available',
      note: '',
      minOsVersion: 'iOS 16.0',
      gitRef: 'main',
      uploadedAt: '2026-06-29T00:00:00',
      uploadedAtLabel: '2026-06-29 00:00',
      expiresAt: null,
      expiresAtLabel: '-',
      artifact: null,
      artifacts: [],
      failureClassification: '',
      failureSummary: '',
      humanAction: '',
      recentEvents: []
    }
  ]
};

const appDetailState: AppDetailState = {
  app: {
    id: 'app-1',
    name: 'lookrva',
    bundleIdentifier: 'com.example.lookrva',
    platform: 'ios',
    iconColor: '#171717',
    iconText: 'LO'
  },
  buildSetting: {
    gitUrl: 'git@example.com:lookrva/ios.git',
    runnerLabels: ['ios-release'],
    credentialRefs: { git: 'git-main' },
    artifactType: 'ipa',
    optionalDefaults: {},
    updatedAtLabel: '2026-07-09 10:00'
  },
  builds: buildsState.builds
};

const buildRunnersState: BuildRunnersState = {
  total: 1,
  runners: [
    {
      id: 'runner-mac-1',
      name: 'Mac mini 1',
      status: 'online',
      labels: ['ios-release'],
      version: '0.1.0',
      packageAgentVersion: '0.1.0',
      lastSeenAtLabel: '2026-07-09 10:00',
      currentBuildId: 'build-1',
      capabilities: {
        platforms: ['ios'],
        llmAdapters: ['codex']
      },
      latestVersion: '0.1.0',
      updateStatus: 'current',
      updateStatusLabel: '已是最新版本'
    }
  ]
};

function jsonResponse(payload: unknown): Promise<Response> {
  return Promise.resolve(
    new Response(JSON.stringify(payload), {
      status: 200,
      headers: { 'Content-Type': 'application/json' }
    })
  );
}

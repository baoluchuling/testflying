import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { StoreAppsPage } from './StoreAppsPage';
import type { StoreAppsState } from '../app/apiClient';

const baseState: StoreAppsState = {
  apps: [
    {
      id: 'app-android',
      name: 'Readink',
      bundleIdentifier: 'com.example.readink',
      platform: 'android',
      developerAccountId: null,
      developerAccountName: '',
      iconColor: '#b45309',
      iconText: 'RE',
      storeIdentifier: 'com.example.readink',
      status: 'needs_account',
      statusLabel: '未绑定账号',
      latestBuild: { version: '2.0', buildNumber: '8', environment: 'development', uploadedAt: '2026-06-30T10:00:00' },
      selected: true,
      storeManagementPath: '',
      reviewsPath: ''
    },
    {
      id: 'app-ios',
      name: 'lookrva',
      bundleIdentifier: 'com.example.lookrva',
      platform: 'ios',
      developerAccountId: 'account-ios',
      developerAccountName: '测试账号',
      iconColor: '#18181b',
      iconText: 'LO',
      storeIdentifier: '1234567890',
      status: 'ready',
      statusLabel: '可同步',
      latestBuild: { version: '1.0', buildNumber: '1', environment: 'production', uploadedAt: '2026-06-30T11:00:00' },
      selected: false,
      storeManagementPath: '/admin/accounts/account-ios/apps/app-ios/store',
      reviewsPath: '/admin/store-reviews?accountId=account-ios&appId=app-ios'
    }
  ],
  selectedApp: null,
  filter: 'all',
  stats: { total: 2, ios: 1, android: 1, ready: 1, needs: 1 },
  accountSummary: {
    totalAccounts: 1,
    boundApps: 1,
    connectorOk: 1,
    connectorNeeds: 0,
    renewalReminders: 0
  }
};

describe('StoreAppsPage', () => {
  beforeEach(() => {
    history.replaceState(null, '', '/admin/apps');
    vi.spyOn(globalThis, 'fetch').mockImplementation(mockFetch);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('loads the store app list and summary', async () => {
    render(<StoreAppsPage />);

    expect(await screen.findByText('商店应用')).toBeTruthy();
    expect(screen.getAllByText('Readink').length).toBeGreaterThan(0);
    expect(screen.getByText('lookrva')).toBeTruthy();
    expect(screen.getByText('Connector 正常')).toBeTruthy();
  });

  it('filters apps through pushState and json reload', async () => {
    const user = userEvent.setup();
    const pushState = vi.spyOn(history, 'pushState');

    render(<StoreAppsPage />);
    await screen.findAllByText('Readink');
    await user.click(screen.getByRole('button', { name: 'iOS' }));

    expect(pushState).toHaveBeenCalledWith(
      { adminRoute: 'apps', filter: 'ios' },
      '',
      '/admin/apps?filter=ios'
    );
    expect((await screen.findAllByText('lookrva')).length).toBeGreaterThan(0);
    expect(fetch).toHaveBeenCalledWith(
      '/admin/api/store-apps?filter=ios',
      expect.objectContaining({ cache: 'no-store' })
    );
  });

  it('selects an app without a document reload', async () => {
    const user = userEvent.setup();
    const pushState = vi.spyOn(history, 'pushState');

    render(<StoreAppsPage />);
    await screen.findAllByText('Readink');
    await user.click(screen.getByRole('row', { name: /lookrva/ }));

    expect(pushState).toHaveBeenCalledWith(
      { adminRoute: 'apps', appId: 'app-ios' },
      '',
      '/admin/apps?appId=app-ios'
    );
    expect((await screen.findAllByText('测试账号')).length).toBeGreaterThan(0);
  });

  it('opens review analysis through the react router path', async () => {
    const user = userEvent.setup();

    render(<StoreAppsPage />);
    await screen.findAllByText('Readink');
    await user.click(screen.getByRole('row', { name: /lookrva/ }));
    const reviewLinks = await screen.findAllByRole('link', { name: '评论分析' });
    await user.click(reviewLinks[1]);

    expect(location.pathname).toBe('/admin/store-reviews');
    expect(location.search).toBe('?accountId=account-ios&appId=app-ios');
  });

  it('does not expose legacy admin page links for store or account actions', async () => {
    const user = userEvent.setup();

    render(<StoreAppsPage />);
    await screen.findAllByText('Readink');
    await user.click(screen.getByRole('row', { name: /lookrva/ }));

    const storeLink = await screen.findByRole('link', { name: '商店管理' });
    const accountLinks = screen.getAllByRole('link', { name: /账号|绑定/ });

    expect(storeLink.getAttribute('href')).toBe('/admin/accounts/account-ios/apps/app-ios/store');
    for (const link of accountLinks) {
      expect(link.getAttribute('href') ?? '').not.toContain('/admin/developer-accounts');
    }
  });
});

function mockFetch(input: RequestInfo | URL): Promise<Response> {
  const url = String(input);
  if (url.startsWith('/admin/api/store-apps')) {
    return jsonResponse(stateForUrl(url));
  }
  return Promise.reject(new Error(`unexpected fetch ${url}`));
}

function stateForUrl(url: string): StoreAppsState {
  const parsed = new URL(url, 'http://localhost');
  const filter = parsed.searchParams.get('filter') || 'all';
  const appId = parsed.searchParams.get('appId') || '';
  const apps = baseState.apps
    .filter((app) => {
      if (filter === 'ios') return app.platform === 'ios';
      if (filter === 'android') return app.platform === 'android';
      if (filter === 'needs') return app.status !== 'ready';
      if (filter === 'ok') return app.status === 'ready';
      return true;
    })
    .map((app) => ({ ...app, selected: app.id === appId || (!appId && app.id === 'app-android') }));
  const selectedApp = apps.find((app) => app.selected) ?? apps[0] ?? null;
  return { ...baseState, apps, selectedApp, filter };
}

function jsonResponse(payload: unknown): Promise<Response> {
  return Promise.resolve(
    new Response(JSON.stringify(payload), {
      status: 200,
      headers: { 'Content-Type': 'application/json' }
    })
  );
}

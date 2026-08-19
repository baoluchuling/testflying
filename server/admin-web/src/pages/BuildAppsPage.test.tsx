import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { BuildAppsPage } from './BuildAppsPage';

describe('BuildAppsPage', () => {
  beforeEach(() => {
    history.replaceState(null, '', '/admin/builds/apps');
    vi.spyOn(globalThis, 'fetch').mockImplementation((input, init) => {
      const url = String(input);
      if (url === '/admin/api/builds/apps') {
        return jsonResponse(buildAppsState);
      }
      if (url === '/admin/api/apps/app-lookrva/builds' && init?.method === 'POST') {
        return jsonResponse(buildCreatedResponse);
      }
      if (url === '/admin/api/apps/app-lookrva') {
        return jsonResponse({
          app: appSummary,
          builds: [],
          buildSetting: buildAppsState.apps[0].setting
        });
      }
      return Promise.reject(new Error(`unexpected fetch ${url}`));
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('selects a configured app and creates a build without leaving the page', async () => {
    const user = userEvent.setup();
    render(<BuildAppsPage />);

    await user.click(await screen.findByRole('button', { name: /lookrva/ }));
    await user.selectOptions(screen.getByLabelText('构建环境'), 'production');
    await user.clear(screen.getByLabelText('Git ref'));
    await user.type(screen.getByLabelText('Git ref'), 'release/1.2.0');
    await user.click(screen.getByRole('button', { name: '立即构建' }));

    expect(await screen.findByText('构建任务已创建')).toBeTruthy();
    expect(screen.getByText(/build-agent-123 · 排队中/)).toBeTruthy();
    expect(location.pathname).toBe('/admin/builds/apps');

    const request = vi.mocked(globalThis.fetch).mock.calls.find(
      ([input]) => String(input) === '/admin/api/apps/app-lookrva/builds'
    );
    expect(JSON.parse(String(request?.[1]?.body))).toEqual({
      environment: 'production',
      gitRef: 'release/1.2.0'
    });

    await user.click(screen.getByRole('button', { name: '查看构建记录' }));
    expect(location.pathname).toBe('/admin/builds/history');
  });

  it('shows build readiness and edits settings without leaving the build workspace', async () => {
    const user = userEvent.setup();
    render(<BuildAppsPage />);

    expect(await screen.findByText('git@example.com:lookrva/ios.git')).toBeTruthy();
    expect(screen.getByText('Runner: mobile-release')).toBeTruthy();
    expect(screen.getByText('1 个在线节点')).toBeTruthy();
    expect(screen.getByText('最近：成功 · 2026-07-12 09:30')).toBeTruthy();

    await user.click(screen.getByRole('button', { name: /lookrva/ }));
    expect(screen.getByText('git: git-main')).toBeTruthy();
    expect(screen.getByText('成功 · 2026-07-12 09:30')).toBeTruthy();

    await user.click(screen.getByRole('button', { name: '编辑构建配置' }));
    expect(await screen.findByRole('dialog', { name: 'lookrva 构建配置' })).toBeTruthy();
    expect(screen.getByText('com.example.lookrva · IOS')).toBeTruthy();
    expect(screen.queryByRole('navigation', { name: '构建环境' })).toBeNull();
    expect(screen.getAllByRole('button', { name: '保存构建配置' })).toHaveLength(1);
    expect(screen.queryByText('仓库子目录')).toBeNull();
    expect(location.pathname).toBe('/admin/builds/apps');
  });

  it('connects an existing app and saves its first build setting in place', async () => {
    vi.restoreAllMocks();
    let connected = false;
    let savedBody: Record<string, unknown> | null = null;
    vi.spyOn(globalThis, 'fetch').mockImplementation((input, init) => {
      const url = String(input);
      if (url === '/admin/api/builds/apps') {
        return jsonResponse(connected ? connectedNovelGoState : emptyBuildAppsState);
      }
      if (url === '/admin/api/apps/app-novelgo') {
        return jsonResponse({
          app: novelGoSummary,
          builds: [],
          buildSetting: null
        });
      }
      if (url === '/admin/api/apps/app-novelgo/build-setting' && init?.method === 'PUT') {
        connected = true;
        savedBody = JSON.parse(String(init.body)) as Record<string, unknown>;
        return jsonResponse({
          message: '构建配置已保存',
          build: null,
          state: {
            app: novelGoSummary,
            builds: [],
            buildSetting: connectedNovelGoState.apps[0].setting
          }
        });
      }
      return Promise.reject(new Error(`unexpected fetch ${url}`));
    });

    const user = userEvent.setup();
    render(<BuildAppsPage />);

    await user.click(await screen.findByRole('button', { name: '接入已有应用' }));
    await user.click(screen.getByRole('button', { name: /NovelGo/ }));
    expect(await screen.findByRole('dialog', { name: 'NovelGo 构建配置' })).toBeTruthy();

    await user.type(screen.getByLabelText('Git 仓库'), 'git@example.com:novelgo/app.git');
    await user.type(screen.getByLabelText('节点标签'), 'mobile, release');
    await user.click(screen.getByRole('button', { name: '保存构建配置' }));

    expect(await screen.findByText('构建配置已保存')).toBeTruthy();
    expect(savedBody).toMatchObject({
      gitUrl: 'git@example.com:novelgo/app.git',
      artifactType: 'apk',
      runnerLabels: ['mobile', 'release']
    });
    expect(savedBody).not.toHaveProperty('repoSubpath');
    expect(screen.getByText('NovelGo 的构建配置已保存')).toBeTruthy();
    expect(screen.getAllByText('git@example.com:novelgo/app.git')).toHaveLength(2);
    expect(location.pathname).toBe('/admin/builds/apps');
  });

  it('reports a refresh failure without treating a saved setting as failed', async () => {
    vi.restoreAllMocks();
    let loadCount = 0;
    vi.spyOn(globalThis, 'fetch').mockImplementation((input, init) => {
      const url = String(input);
      if (url === '/admin/api/builds/apps') {
        loadCount += 1;
        return loadCount === 1
          ? jsonResponse(emptyBuildAppsState)
          : Promise.reject(new Error('refresh unavailable'));
      }
      if (url === '/admin/api/apps/app-novelgo') {
        return jsonResponse({
          app: novelGoSummary,
          builds: [],
          buildSetting: null
        });
      }
      if (url === '/admin/api/apps/app-novelgo/build-setting' && init?.method === 'PUT') {
        return jsonResponse({
          message: '构建配置已保存',
          build: null,
          state: {
            app: novelGoSummary,
            builds: [],
            buildSetting: connectedNovelGoState.apps[0].setting
          }
        });
      }
      return Promise.reject(new Error(`unexpected fetch ${url}`));
    });

    const user = userEvent.setup();
    render(<BuildAppsPage />);

    await user.click(await screen.findByRole('button', { name: '接入已有应用' }));
    await user.click(screen.getByRole('button', { name: /NovelGo/ }));
    await user.type(await screen.findByLabelText('Git 仓库'), 'git@example.com:novelgo/app.git');
    await user.click(screen.getByRole('button', { name: '保存构建配置' }));

    expect(await screen.findByText('构建配置已保存')).toBeTruthy();
    expect(screen.getByText(/配置已保存，但应用列表刷新失败/)).toBeTruthy();
  });
});

const appSummary = {
  id: 'app-lookrva',
  name: 'lookrva',
  bundleIdentifier: 'com.example.lookrva',
  platform: 'ios',
  iconColor: '#171717',
  iconText: 'LO'
};

const novelGoSummary = {
  id: 'app-novelgo',
  name: 'NovelGo',
  bundleIdentifier: 'com.example.novelgo',
  platform: 'android',
  iconColor: '#171717',
  iconText: 'NO'
};

const emptyBuildAppsState = {
  total: 0,
  apps: [],
  availableApps: [novelGoSummary]
};

const connectedNovelGoState = {
  total: 1,
  availableApps: [],
  apps: [
    {
      app: novelGoSummary,
      latestBuild: null,
      setting: {
        gitUrl: 'git@example.com:novelgo/app.git',
        runnerLabels: ['mobile', 'release'],
        credentialRefs: {},
        artifactType: 'apk',
        optionalDefaults: {},
        updatedAtLabel: '2026-07-13 21:30'
      },
      matchingRunnerCount: 0,
      hasOnlineRunner: false
    }
  ]
};

const buildAppsState = {
  total: 1,
  availableApps: [
    novelGoSummary
  ],
  apps: [
    {
      app: appSummary,
      setting: {
        gitUrl: 'git@example.com:lookrva/ios.git',
        runnerLabels: ['mobile-release'],
        credentialRefs: { git: 'git-main' },
        artifactType: 'ipa',
        optionalDefaults: { gitRef: 'main' },
        updatedAtLabel: '2026-07-13 10:00'
      },
      matchingRunnerCount: 1,
      hasOnlineRunner: true,
      latestBuild: {
        id: 'build-latest-1',
        app: appSummary,
        version: '1.1.0',
        buildNumber: '42',
        platform: 'ios',
        platformLabel: 'iOS',
        environment: 'development',
        environmentLabel: '开发环境',
        status: 'available',
        lifecycleStatus: 'succeeded',
        lifecycleStatusLabel: '成功',
        note: '',
        minOsVersion: 'iOS 16.0',
        uploadedAt: '2026-07-12T09:30:00Z',
        uploadedAtLabel: '2026-07-12 09:30',
        expiresAt: null,
        expiresAtLabel: '-',
        artifact: null,
        artifacts: [],
        failureClassification: '',
        failureSummary: '',
        humanAction: '',
        recentEvents: []
      }
    }
  ]
};

const buildCreatedResponse = {
  message: '构建任务已创建',
  build: {
    id: 'build-agent-123',
    app: appSummary,
    version: '',
    buildNumber: '',
    platform: 'ios',
    platformLabel: 'iOS',
    environment: 'production',
    environmentLabel: '线上环境',
    status: 'queued',
    lifecycleStatus: 'queued',
    lifecycleStatusLabel: '排队中',
    note: '',
    minOsVersion: '',
    uploadedAt: '2026-07-13T10:00:00Z',
    uploadedAtLabel: '2026-07-13 10:00',
    expiresAt: null,
    expiresAtLabel: '-',
    artifact: null,
    artifacts: [],
    failureClassification: '',
    failureSummary: '',
    humanAction: '',
    recentEvents: []
  },
  state: {
    app: appSummary,
    builds: [],
    buildSetting: buildAppsState.apps[0].setting
  }
};

function jsonResponse(payload: unknown): Promise<Response> {
  return Promise.resolve(
    new Response(JSON.stringify(payload), {
      status: 200,
      headers: { 'Content-Type': 'application/json' }
    })
  );
}

import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AppDetailPage } from './AppDetailPage';

describe('AppDetailPage', () => {
  let savedSettingsBody: Record<string, unknown> | null;
  let createdBuildBody: Record<string, unknown> | null;

  beforeEach(() => {
    history.replaceState(null, '', '/admin/apps/app-ios-demo');
    savedSettingsBody = null;
    createdBuildBody = null;
    vi.spyOn(globalThis, 'fetch').mockImplementation((input, init) => {
      const url = String(input);
      if (url === '/admin/api/apps/app-ios-demo') {
        return jsonResponse(appDetailState);
      }
      if (url === '/admin/api/apps/app-ios-demo/build-setting' && init?.method === 'PUT') {
        savedSettingsBody = JSON.parse(String(init.body)) as Record<string, unknown>;
        return jsonResponse({
          message: '构建设置已保存',
          build: null,
          state: {
            ...appDetailState,
            buildSetting: {
              ...appDetailState.buildSetting,
              gitUrl: String(savedSettingsBody.gitUrl)
            }
          }
        });
      }
      if (url === '/admin/api/apps/app-ios-demo/builds') {
        createdBuildBody = JSON.parse(String(init?.body)) as Record<string, unknown>;
        return jsonResponse({
          message: '构建任务已创建',
          build: {
            ...appDetailState.builds[0],
            id: 'build-agent-1',
            lifecycleStatus: 'queued',
            lifecycleStatusLabel: '排队中',
            source: 'agent',
            sourceLabel: 'Agent'
          },
          state: appDetailState
        });
      }
      return Promise.reject(new Error(`unexpected fetch ${url}`));
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('renders app summary, quick build controls, settings, and build history', async () => {
    render(<AppDetailPage appId="app-ios-demo" />);

    expect(await screen.findByRole('heading', { name: 'AnyStories' })).toBeTruthy();
    expect(screen.getByText('com.example.any')).toBeTruthy();
    expect(screen.getByRole('button', { name: '立即构建' })).toBeTruthy();
    expect(screen.getByText('应用构建配置')).toBeTruthy();
    expect(screen.queryByText('Repo Subpath')).toBeNull();
    expect(screen.queryByText('仓库子目录')).toBeNull();
    expect(screen.queryByText('测试环境设置')).toBeNull();
    expect(screen.queryByText('线上环境设置')).toBeNull();
    expect(screen.getByText('构建历史')).toBeTruthy();
    expect(screen.getByText('build 45')).toBeTruthy();
    expect(screen.getByText('Package · AnyStories.ipa')).toBeTruthy();
    expect(screen.getByRole('link', { name: '安装' }).getAttribute('href')).toBe(
      'itms-services://?action=download-manifest&url=https://dist.example.test/manifest.plist'
    );
    expect(screen.getByRole('link', { name: '报告' }).getAttribute('href')).toBe(
      'https://dist.example.test/report.json'
    );
    expect(screen.getByRole('link', { name: '日志' }).getAttribute('href')).toBe(
      'https://dist.example.test/build.log'
    );
    expect(screen.getByText('missing_artifacts')).toBeTruthy();
    expect(screen.getByText('Automatic success requires package, symbols, and logs.')).toBeTruthy();
  });

  it('submits a quick build request', async () => {
    const user = userEvent.setup();
    render(<AppDetailPage appId="app-ios-demo" />);

    await screen.findByRole('heading', { name: 'AnyStories' });
    await user.selectOptions(screen.getByLabelText('环境'), 'production');
    await user.clear(screen.getByLabelText('Git ref'));
    await user.type(screen.getByLabelText('Git ref'), 'release/1.2.0');
    await user.click(screen.getByRole('button', { name: '立即构建' }));

    expect(await screen.findByText('构建任务已创建')).toBeTruthy();
    expect(createdBuildBody).toEqual({
      environment: 'production',
      gitRef: 'release/1.2.0'
    });
  });

  it('preserves optional defaults when saving build settings', async () => {
    const user = userEvent.setup();
    render(<AppDetailPage appId="app-ios-demo" />);

    await screen.findByRole('heading', { name: 'AnyStories' });
    const gitUrlInput = screen.getByDisplayValue('git@example.com:any/ios.git');
    await user.clear(gitUrlInput);
    await user.type(gitUrlInput, 'git@example.com:any/new-ios.git');
    await user.click(screen.getByRole('button', { name: '保存构建配置' }));

    expect(await screen.findByText('构建设置已保存')).toBeTruthy();
    expect(savedSettingsBody).toMatchObject({
      gitUrl: 'git@example.com:any/new-ios.git',
      optionalDefaults: {
        releaseChannel: 'internal',
        notifyGroups: ['qa', 'ios']
      }
    });
    expect(savedSettingsBody).not.toHaveProperty('repoSubpath');
  });

  it('keeps both build environments available with one shared setting', async () => {
    render(<AppDetailPage appId="app-ios-demo" />);

    expect(await screen.findByRole('heading', { name: 'AnyStories' })).toBeTruthy();
    const environment = screen.getByLabelText('环境') as HTMLSelectElement;
    expect([...environment.options].map((option) => [option.value, option.disabled])).toEqual([
      ['development', false],
      ['production', false]
    ]);
  });

  it('disables quick build when the shared setting is missing', async () => {
    vi.restoreAllMocks();
    vi.spyOn(globalThis, 'fetch').mockImplementation((input) => {
      const url = String(input);
      if (url === '/admin/api/apps/app-ios-demo') {
        return jsonResponse({
          ...appDetailState,
          buildSetting: null
        });
      }
      return Promise.reject(new Error(`unexpected fetch ${url}`));
    });

    render(<AppDetailPage appId="app-ios-demo" />);

    expect(await screen.findByRole('heading', { name: 'AnyStories' })).toBeTruthy();
    expect((screen.getByLabelText('环境') as HTMLSelectElement).value).toBe('development');
    expect(screen.getByText('尚未配置构建设置。')).toBeTruthy();
    expect((screen.getByRole('button', { name: '立即构建' }) as HTMLButtonElement).disabled).toBe(true);
  });
});

const appDetailState = {
  app: {
    id: 'app-ios-demo',
    name: 'AnyStories',
    bundleIdentifier: 'com.example.any',
    platform: 'ios',
    iconColor: '#2478FF',
    iconText: 'AN'
  },
  buildSetting: {
    gitUrl: 'git@example.com:any/ios.git',
    runnerLabels: ['ios-release'],
    credentialRefs: { git: 'git-main' },
    artifactType: 'ipa',
    optionalDefaults: {
      releaseChannel: 'internal',
      notifyGroups: ['qa', 'ios']
    },
    updatedAtLabel: '2026-07-09 10:00'
  },
  builds: [
    {
      id: 'build-1',
      app: {
        id: 'app-ios-demo',
        name: 'AnyStories',
        bundleIdentifier: 'com.example.any',
        platform: 'ios',
        iconColor: '#2478FF',
        iconText: 'AN'
      },
      version: '1.0',
      buildNumber: '45',
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
      minOsVersion: '',
      gitRef: 'main',
      uploadedAt: '2026-07-09T10:00:00Z',
      uploadedAtLabel: '2026-07-09 10:00',
      expiresAt: null,
      expiresAtLabel: '-',
      artifact: null,
      artifacts: [
        {
          artifactType: 'package',
          artifactTypeLabel: 'Package',
          fileName: 'AnyStories.ipa',
          sizeLabel: '12 MB',
          installUrl:
            'itms-services://?action=download-manifest&url=https://dist.example.test/manifest.plist',
          downloadUrl: 'https://dist.example.test/AnyStories.ipa',
          manifestUrl: 'https://dist.example.test/manifest.plist'
        },
        {
          artifactType: 'symbols',
          artifactTypeLabel: 'Symbols',
          fileName: 'AnyStories.dSYM.zip',
          sizeLabel: '3 MB',
          installUrl: '',
          downloadUrl: 'https://dist.example.test/AnyStories.dSYM.zip',
          manifestUrl: null
        },
        {
          artifactType: 'report',
          artifactTypeLabel: 'Report',
          fileName: 'report.json',
          sizeLabel: '2 KB',
          installUrl: '',
          downloadUrl: 'https://dist.example.test/report.json',
          manifestUrl: null
        },
        {
          artifactType: 'log',
          artifactTypeLabel: 'Log',
          fileName: 'build.log',
          sizeLabel: '4 KB',
          installUrl: '',
          downloadUrl: 'https://dist.example.test/build.log',
          manifestUrl: null
        }
      ],
      failureClassification: 'missing_artifacts',
      failureSummary: 'Automatic success requires package, symbols, and logs.',
      humanAction: 'Upload missing artifacts.',
      recentEvents: [
        {
          type: 'runner.build.needs_human',
          message: 'Upload missing artifacts.',
          createdAtLabel: '2026-07-09 10:01'
        }
      ]
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

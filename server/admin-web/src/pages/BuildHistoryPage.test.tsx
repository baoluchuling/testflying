import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { BuildHistoryPage } from './BuildHistoryPage';

describe('BuildHistoryPage', () => {
  beforeEach(() => {
    history.replaceState(null, '', '/admin/builds');
    vi.spyOn(globalThis, 'fetch').mockImplementation((input) => {
      const url = String(input);
      if (url === '/admin/api/builds') {
        return jsonResponse(buildsState);
      }
      return Promise.reject(new Error(`unexpected fetch ${url}`));
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('renders row source and lifecycle metadata, navigates to app detail, and copies artifact links', async () => {
    const user = userEvent.setup();
    const dispatchEventSpy = vi.spyOn(window, 'dispatchEvent');
    const pushStateSpy = vi.spyOn(history, 'pushState');

    render(<BuildHistoryPage />);

    expect(await screen.findByText('MultiArtifactApp')).toBeTruthy();
    expect(screen.getByText('Agent')).toBeTruthy();
    expect(screen.getByText('已分发')).toBeTruthy();
    expect(screen.getByText('2 个产物')).toBeTruthy();
    expect(screen.getByText('ios-build.ipa')).toBeTruthy();
    expect(screen.getByText('ios-build.plist')).toBeTruthy();

    await user.click(screen.getAllByRole('button', { name: '复制安装' })[0]);
    expect(await screen.findByText('已复制 ios-build.ipa: installUrl')).toBeTruthy();

    await user.click(screen.getAllByRole('button', { name: '下载地址' })[1]);
    expect(await screen.findByText('已复制 ios-build.plist: downloadUrl')).toBeTruthy();

    await user.click(screen.getByRole('button', { name: /MultiArtifactApp/ }));

    expect(pushStateSpy).toHaveBeenCalledWith(
      { adminRoute: 'apps' },
      '',
      '/admin/apps/app-multi'
    );
    expect(location.pathname).toBe('/admin/apps/app-multi');
    expect(
      dispatchEventSpy.mock.calls.some(
        ([event]) => event instanceof Event && event.type === 'admin:navigation'
      )
    ).toBe(false);
  });
});

const buildsState = {
  total: 1,
  builds: [
    {
      id: 'build-multi',
      app: {
        id: 'app-multi',
        name: 'MultiArtifactApp',
        bundleIdentifier: 'com.example.multi',
        platform: 'ios',
        iconColor: '#2478FF',
        iconText: 'MA'
      },
      version: '2.1.0',
      buildNumber: '108',
      platform: 'ios',
      platformLabel: 'iOS',
      environment: 'development',
      environmentLabel: '开发环境',
      source: 'agent',
      sourceLabel: 'Agent',
      lifecycleStatus: 'distributed',
      lifecycleStatusLabel: '已分发',
      status: 'available',
      note: '',
      minOsVersion: 'iOS 16.0',
      gitRef: 'release/2.1.0',
      uploadedAt: '2026-07-09T10:00:00Z',
      uploadedAtLabel: '2026-07-09 10:00',
      expiresAt: null,
      expiresAtLabel: '-',
      artifact: null,
      failureClassification: '',
      failureSummary: '',
      humanAction: '',
      recentEvents: [],
      artifacts: [
        {
          artifactType: 'ipa',
          artifactTypeLabel: 'IPA',
          fileName: 'ios-build.ipa',
          sizeLabel: '42 MB',
          installUrl: 'https://cdn.example.com/app/install',
          downloadUrl: 'https://cdn.example.com/app/download',
          manifestUrl: 'https://cdn.example.com/app/manifest'
        },
        {
          artifactType: 'plist',
          artifactTypeLabel: 'Manifest',
          fileName: 'ios-build.plist',
          sizeLabel: '8 KB',
          installUrl: 'https://cdn.example.com/app/plist/install',
          downloadUrl: 'https://cdn.example.com/app/manifest.plist',
          manifestUrl: null
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

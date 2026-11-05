import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { SettingsPage } from './SettingsPage';

describe('SettingsPage', () => {
  beforeEach(() => {
    vi.spyOn(globalThis, 'fetch').mockImplementation(mockFetch);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('saves notification settings and checks DingTalk without exposing secrets', async () => {
    const user = userEvent.setup();
    history.replaceState(null, '', '/admin/settings/notifications');
    render(<SettingsPage />);

    expect(await screen.findByRole('heading', { name: '通知设置' })).toBeTruthy();
    expect(screen.getAllByText('密钥已配置').length).toBeGreaterThan(0);
    expect(screen.queryByText('SEC-never-return')).toBeNull();

    await user.clear(screen.getByLabelText('请求超时'));
    await user.type(screen.getByLabelText('请求超时'), '8');
    await user.click(screen.getByRole('button', { name: '保存配置' }));
    expect(await screen.findByText('通知配置已保存')).toBeTruthy();

    await user.click(screen.getByRole('button', { name: '检查配置' }));
    expect(await screen.findByText('钉钉配置检查消息已发送')).toBeTruthy();
  });

  it('renders infrastructure values as read-only masked state', async () => {
    history.replaceState(null, '', '/admin/settings/runtime');
    render(<SettingsPage />);

    expect(await screen.findByText('TESTFLYING_DATABASE_URL')).toBeTruthy();
    expect(screen.getAllByText('已配置').length).toBeGreaterThan(0);
    expect(screen.queryByText('postgres-secret')).toBeNull();
    expect(screen.queryByRole('textbox')).toBeNull();
  });
});

const settingsState = {
  general: {
    connectorBaseUrlTemplate: 'https://connector.example.test/{accountId}',
    source: 'database'
  },
  notifications: {
    enabled: true,
    configured: true,
    webhookConfigured: true,
    secretConfigured: true,
    timeoutSeconds: 5,
    dispatchIntervalSeconds: 10,
    pendingDeliveryCount: 2,
    deadDeliveryCount: 1,
    source: 'database'
  },
  runtime: [
    {
      key: 'TESTFLYING_DATABASE_URL',
      label: '数据库连接',
      group: 'Database',
      source: 'environment',
      valueLabel: '已配置',
      configured: true,
      sensitive: true,
      restartRequired: true
    },
    {
      key: 'TESTFLYING_RUNNER_RELEASE_ROOT',
      label: 'Runner 发布目录',
      group: 'Runner',
      source: 'default',
      valueLabel: '/app/data/runner-releases',
      configured: true,
      sensitive: false,
      restartRequired: true
    }
  ]
};

function mockFetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const url = String(input);
  if (url === '/admin/api/settings' && !init?.method) return jsonResponse(settingsState);
  if (url === '/admin/api/settings/notifications' && init?.method === 'PUT') {
    return jsonResponse({ message: '通知配置已保存', state: settingsState });
  }
  if (url === '/admin/api/settings/notifications/check' && init?.method === 'POST') {
    return jsonResponse({ message: '钉钉配置检查消息已发送', state: settingsState });
  }
  return Promise.reject(new Error(`unexpected fetch ${url}`));
}

function jsonResponse(payload: unknown): Promise<Response> {
  return Promise.resolve(
    new Response(JSON.stringify(payload), {
      status: 200,
      headers: { 'Content-Type': 'application/json' }
    })
  );
}

import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { AppLogsPage } from './AppLogsPage';
import type { AppLogsState } from '../app/apiClient';

vi.mock('../app/apiClient', async () => {
  const actual = await vi.importActual<typeof import('../app/apiClient')>('../app/apiClient');
  return {
    ...actual,
    loadAppLogs: vi.fn(() => Promise.resolve(appLogsState)),
    loadAppLogEvents: vi.fn(() => Promise.resolve({ ...appLogsState, logs: [], errors: [] }))
  };
});

const appLogsState: AppLogsState = {
  connect: {
    host: '192.168.1.23',
    port: '18080',
    name: 'Mac',
    appScheme: 'anystories',
    appName: 'AnyStories',
    connectUrl: 'http://192.168.1.23:18080/app-logs/connect',
    connectPageUrl: 'http://192.168.1.23:18080/app-logs/connect',
    schemeUrl: 'anystories:///connect?host=192.168.1.23&port=18080&name=Mac',
    websocketUrl: 'ws://192.168.1.23:18080/push?token=<设备ID>'
  },
  cursor: 12,
  devices: [
    {
      token: 'device-admin-ios',
      deviceId: 'device-admin-ios',
      device: 'Admin iPhone',
      platform: 'ios',
      connected: true,
      knownToken: true,
      connectedAt: '2026-06-18T12:00:00Z',
      lastSeenAt: '2026-06-18T12:00:02Z',
      connectionCount: 1,
      errorCount: 0,
      logCount: 2
    }
  ],
  logs: [
    {
      sequence: 12,
      token: 'device-admin-ios',
      deviceId: 'device-admin-ios',
      device: 'Admin iPhone',
      platform: 'ios',
      receivedAt: '2026-06-18T12:00:02Z',
      sentAt: '2026-06-18T12:00:02Z',
      history: false,
      raw: '2026-06-18 12:00:02.000 级别=警告 消息=接口响应较慢',
      timestamp: '2026-06-18 12:00:02.000',
      level: '警告',
      tag: '网络',
      event: '重试',
      message: '接口响应较慢',
      fields: []
    },
    {
      sequence: 11,
      token: 'device-admin-ios',
      deviceId: 'device-admin-ios',
      device: 'Admin iPhone',
      platform: 'ios',
      receivedAt: '2026-06-18T12:00:01Z',
      sentAt: '2026-06-18T12:00:01Z',
      history: false,
      raw: '2026-06-18 12:00:01.000 级别=信息 消息=开始播放 章节序号=3',
      timestamp: '2026-06-18 12:00:01.000',
      level: '信息',
      tag: '播放器',
      event: '播放',
      message: '开始播放',
      fields: [{ key: '章节序号', value: '3' }]
    }
  ],
  errors: [],
  levels: ['跟踪', '调试', '信息', '警告', '错误', '致命']
};

describe('AppLogsPage', () => {
  afterEach(() => {
    vi.clearAllMocks();
  });

  function hasText(value: string) {
    return (_: string, node: Element | null) => node?.textContent?.includes(value) ?? false;
  }

  async function expectTextVisible(value: string) {
    await waitFor(() => expect(screen.queryAllByText(hasText(value)).length).toBeGreaterThan(0));
  }

  function expectTextHidden(value: string) {
    expect(screen.queryAllByText(hasText(value))).toHaveLength(0);
  }

  it('renders connected compact connection area and log stream', async () => {
    render(<AppLogsPage />);

    expect(await screen.findByText('已连接')).toBeTruthy();
    expect(screen.getByText('Admin iPhone')).toBeTruthy();
    await expectTextVisible('开始播放');
    await expectTextVisible('接口响应较慢');
  });

  it('filters logs by level without page navigation', async () => {
    const user = userEvent.setup();
    render(<AppLogsPage />);

    await expectTextVisible('开始播放');
    const levelFilter = screen.getByLabelText('级别筛选');
    await user.click(within(levelFilter).getByLabelText('信息'));

    expectTextHidden('开始播放');
    await expectTextVisible('接口响应较慢');
  });

  it('clears the current view locally', async () => {
    const user = userEvent.setup();
    render(<AppLogsPage />);

    await expectTextVisible('开始播放');
    await user.click(screen.getByRole('button', { name: '清空当前视图' }));

    expectTextHidden('开始播放');
    expect(screen.getByText('暂无日志，扫码连接 App 后会自动显示。')).toBeTruthy();
  });
});

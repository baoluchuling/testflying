import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiDocsPage } from './ApiDocsPage';
import { NotificationsPage } from './NotificationsPage';

describe('ordinary admin pages', () => {
  beforeEach(() => {
    history.replaceState(null, '', '/admin/notifications');
    Element.prototype.scrollIntoView = vi.fn();
    vi.spyOn(globalThis, 'fetch').mockImplementation((input) => {
      const url = String(input);
      if (url === '/admin/api/notifications') {
        return jsonResponse(notificationsState('all'));
      }
      if (url === '/admin/api/notifications?type=device') {
        return jsonResponse(notificationsState('device'));
      }
      if (url === '/admin/api/api-docs') {
        return jsonResponse(apiDocsState);
      }
      return Promise.reject(new Error(`unexpected fetch ${url}`));
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('filters notifications without reloading the document', async () => {
    const user = userEvent.setup();

    render(<NotificationsPage />);
    await screen.findByText('构建完成');
    await user.click(screen.getByRole('button', { name: /设备/ }));

    expect(location.pathname).toBe('/admin/notifications');
    expect(location.search).toBe('?type=device');
    expect(await screen.findByText('设备登记')).toBeTruthy();
    expect(screen.queryByText('构建完成')).toBeNull();
  });

  it('links notification channel management to settings without duplicating the form', async () => {
    const user = userEvent.setup();
    render(<NotificationsPage />);

    const link = await screen.findByRole('button', { name: '管理通知渠道' });
    expect(screen.queryByText('钉钉机器人配置')).toBeNull();
    expect(screen.queryByRole('textbox')).toBeNull();
    await user.click(link);
    expect(location.pathname).toBe('/admin/settings/notifications');
  });

  it('jumps to API docs sections from the index', async () => {
    const user = userEvent.setup();

    render(<ApiDocsPage />);
    expect(await screen.findAllByText('读取商店支持语言')).toHaveLength(2);
    await user.click(screen.getByRole('button', { name: /同步默认商店页/ }));

    expect(Element.prototype.scrollIntoView).toHaveBeenCalled();
    expect(screen.getByText('POST /v1/store-management/apps/app-1/sync-runs')).toBeTruthy();
  });
});

function notificationsState(activeType: string) {
  return {
    activeType,
    total: activeType === 'device' ? 1 : 2,
    typeCounts: [
      { type: 'all', label: '全部', count: 2 },
      { type: 'build', label: '构建', count: 1 },
      { type: 'device', label: '设备', count: 1 }
    ],
    dingtalk: {
      configured: true,
      webhookConfigured: true,
      secretConfigured: true,
      triggers: ['failed', 'needs_human'],
      pendingDeliveryCount: 2,
      deadDeliveryCount: 1
    },
    notifications:
      activeType === 'device'
        ? [
            {
              id: 'notice-device',
              type: 'device',
              section: '设备',
              iconKey: 'phone',
              title: '设备登记',
              subtitle: 'iPhone 已登记。',
              tag: '设备',
              tagColor: '#166534',
              createdAt: '2026-06-29T00:00:00',
              createdAtLabel: '2026-06-29 00:00'
            }
          ]
        : [
            {
              id: 'notice-build',
              type: 'build',
              section: '构建',
              iconKey: 'rocket',
              title: '构建完成',
              subtitle: '新构建可以安装。',
              tag: '构建',
              tagColor: '#171717',
              createdAt: '2026-06-29T00:00:00',
              createdAtLabel: '2026-06-29 00:00'
            },
            {
              id: 'notice-device',
              type: 'device',
              section: '设备',
              iconKey: 'phone',
              title: '设备登记',
              subtitle: 'iPhone 已登记。',
              tag: '设备',
              tagColor: '#166534',
              createdAt: '2026-06-29T00:00:00',
              createdAtLabel: '2026-06-29 00:00'
            }
          ]
  };
}

const apiDocsState = {
  downloadUrl: '/admin/api-docs/store-management.md',
  endpoints: [
    {
      anchor: 'endpoint-1',
      title: '读取商店支持语言',
      method: 'GET',
      path: '/v1/store-management/apps/app-1/languages',
      summary: '读取语言。',
      params: [],
      curl: 'curl https://example.test',
      response: '{"languages":[]}'
    },
    {
      anchor: 'endpoint-2',
      title: '同步默认商店页',
      method: 'POST',
      path: '/v1/store-management/apps/app-1/sync-runs',
      summary: '同步默认商店页。',
      params: [],
      curl: 'curl -X POST https://example.test',
      response: '{"status":"ok"}'
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

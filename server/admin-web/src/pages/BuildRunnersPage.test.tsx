import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { BuildRunnersPage } from './BuildRunnersPage';

let pendingProvision: ReturnType<typeof deferred<Response>> | null;

describe('BuildRunnersPage', () => {
  beforeEach(() => {
    pendingProvision = null;
    vi.spyOn(globalThis, 'fetch').mockImplementation((input, init) => {
      if (String(input) === '/admin/api/build-runners') {
        return Promise.resolve(
          new Response(
            JSON.stringify({
              runners: [
                {
                  id: 'runner-mac-1',
                  name: 'Mac mini 1',
                  status: 'online',
                  labels: ['ios-release'],
                  version: '0.1.0',
                  packageAgentVersion: '0.1.0',
                  lastSeenAtLabel: '刚刚',
                  currentBuildId: null,
                  capabilities: { platforms: ['ios'], llmAdapters: ['codex'] },
                  latestVersion: '0.2.0',
                  updateStatus: 'outdated',
                  updateStatusLabel: '可更新至 0.2.0'
                }
              ],
              total: 1
            }),
            { headers: { 'Content-Type': 'application/json' } }
          )
        );
      }
      if (String(input) === '/admin/api/build-runners/provision' && init?.method === 'POST') {
        return pendingProvision?.promise ?? Promise.resolve(provisionResponse());
      }
      return Promise.reject(new Error(`unexpected fetch ${String(input)}`));
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('renders runner status and capabilities', async () => {
    render(<BuildRunnersPage />);

    expect(await screen.findByText('Mac mini 1')).toBeTruthy();
    expect(screen.getByText('online')).toBeTruthy();
    expect(screen.getByText('ios-release')).toBeTruthy();
    expect(screen.getByText('iOS / Android')).toBeTruthy();
    expect(screen.getByText('LLM: codex')).toBeTruthy();
    expect(screen.getByText('刚刚')).toBeTruthy();
    expect(screen.getByText('可更新至 0.2.0')).toBeTruthy();
  });

  it('provisions a runner and only shows the token in the one-time result', async () => {
    render(<BuildRunnersPage />);

    fireEvent.click(await screen.findByRole('button', { name: '新增节点' }));
    expect(screen.queryByLabelText('构建平台')).toBeNull();
    const runnerIdInput = screen.getByLabelText('节点 ID') as HTMLInputElement;
    expect(runnerIdInput.getAttribute('pattern')).toBe(String.raw`[A-Za-z0-9][A-Za-z0-9._\-]{0,63}`);
    expect(runnerIdInput.getAttribute('maxlength')).toBe('64');
    fireEvent.change(runnerIdInput, { target: { value: '../runner' } });
    expect(runnerIdInput.checkValidity()).toBe(false);
    fireEvent.change(runnerIdInput, { target: { value: 'runner-mac-2' } });
    expect(runnerIdInput.checkValidity()).toBe(true);
    fireEvent.change(screen.getByLabelText('节点名称'), { target: { value: 'Mac mini 2' } });
    fireEvent.change(screen.getByLabelText('节点标签'), { target: { value: 'ios-release' } });
    fireEvent.change(screen.getByLabelText('LLM 适配器'), { target: { value: 'codex' } });
    fireEvent.click(screen.getByRole('button', { name: '生成接入配置' }));

    expect(await screen.findByText('请立即保存，关闭后无法再次查看')).toBeTruthy();
    expect(screen.getByText(/同一节点 ID 重新生成配置/)).toBeTruthy();
    expect(screen.getByText('runner-secret-once')).toBeTruthy();
    expect(screen.getByRole('button', { name: '复制配置 JSON' })).toBeTruthy();
    expect(document.querySelector('.config-preview pre')?.textContent).not.toContain('platforms');
    expect(
      screen.getByText((_, element) =>
        Boolean(
          element?.tagName === 'PRE' &&
            element.textContent?.includes('"rootDir": "/Users/Shared/TestFlyingRunner/runner-mac-2"')
        )
      )
    ).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: '关闭一次性配置' }));
    await waitFor(() => expect(screen.queryByText('runner-secret-once')).toBeNull());
  });

  it('prevents closing the dialog while a one-time token is being issued', async () => {
    pendingProvision = deferred<Response>();
    render(<BuildRunnersPage />);

    fireEvent.click(await screen.findByRole('button', { name: '新增节点' }));
    fireEvent.change(screen.getByLabelText('节点 ID'), { target: { value: 'runner-mac-2' } });
    fireEvent.change(screen.getByLabelText('节点名称'), { target: { value: 'Mac mini 2' } });
    fireEvent.click(screen.getByRole('button', { name: '生成接入配置' }));

    expect(screen.getByRole('button', { name: '关闭一次性配置' }).hasAttribute('disabled')).toBe(true);
    expect(screen.getByRole('button', { name: '取消' }).hasAttribute('disabled')).toBe(true);
    expect(screen.getByText('正在签发一次性配置，请勿关闭或刷新页面。')).toBeTruthy();
    expect(window.dispatchEvent(new CustomEvent('admin:before-navigation', { cancelable: true }))).toBe(
      false
    );

    pendingProvision.resolve(provisionResponse());
    expect(await screen.findByText('runner-secret-once')).toBeTruthy();
    expect(screen.getByRole('button', { name: '关闭一次性配置' }).hasAttribute('disabled')).toBe(false);
    await waitFor(() =>
      expect(window.dispatchEvent(new CustomEvent('admin:before-navigation', { cancelable: true }))).toBe(
        true
      )
    );
  });
});

function provisionResponse(): Response {
  return new Response(
    JSON.stringify({
      runner: {
        id: 'runner-mac-2',
        name: 'Mac mini 2',
        status: 'offline',
        labels: ['ios-release'],
        version: '',
        packageAgentVersion: '',
        lastSeenAtLabel: '-',
        currentBuildId: null,
        capabilities: {
          platforms: ['ios', 'android'],
          llmAdapters: ['codex'],
          capacity: 1,
          hostPlatform: 'darwin',
          arch: 'arm64'
        },
        latestVersion: '0.2.0',
        updateStatus: 'outdated',
        updateStatusLabel: '可更新至 0.2.0'
      },
      token: 'runner-secret-once'
    }),
    { headers: { 'Content-Type': 'application/json' } }
  );
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

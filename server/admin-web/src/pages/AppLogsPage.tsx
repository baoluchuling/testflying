import { useEffect, useMemo, useState } from 'react';
import {
  AdminApiError,
  loadAppLogEvents,
  loadAppLogs,
  type AppLogDevice,
  type AppLogEntry,
  type AppLogsState
} from '../app/apiClient';

const defaultLevels = ['跟踪', '调试', '信息', '警告', '错误', '致命'];

export function AppLogsPage() {
  const [state, setState] = useState<AppLogsState | null>(null);
  const [error, setError] = useState('');
  const [selectedToken, setSelectedToken] = useState('all');
  const [selectedLevels, setSelectedLevels] = useState<string[]>(defaultLevels);
  const [search, setSearch] = useState('');

  useEffect(() => {
    let cancelled = false;
    let cursor = 0;

    loadAppLogs()
      .then((payload) => {
        if (cancelled) return;
        cursor = payload.cursor;
        setState(payload);
        if (payload.levels.length > 0) setSelectedLevels(payload.levels);
      })
      .catch((requestError) => {
        if (!cancelled) setError(errorMessage(requestError));
      });

    const timer = window.setInterval(() => {
      loadAppLogEvents(cursor)
        .then((payload) => {
          if (cancelled) return;
          cursor = payload.cursor;
          setState((current) => (current ? mergeAppLogs(current, payload) : payload));
        })
        .catch((requestError) => {
          if (!cancelled) setError(errorMessage(requestError));
        });
    }, 1800);

    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, []);

  const devices = state?.devices ?? [];
  const logs = state?.logs ?? [];
  const levels = state?.levels.length ? state.levels : defaultLevels;
  const hasConnectedDevice = devices.some((device) => device.connected);
  const filteredLogs = useMemo(
    () =>
      logs.filter(
        (log) =>
          (selectedToken === 'all' || log.token === selectedToken) &&
          selectedLevels.includes(log.level) &&
          matchesSearch(log, search)
      ),
    [logs, selectedLevels, selectedToken, search]
  );

  function clearCurrentView() {
    setState((current) => (current ? { ...current, logs: [], errors: [] } : current));
  }

  function toggleLevel(level: string) {
    setSelectedLevels((current) =>
      current.includes(level) ? current.filter((item) => item !== level) : [...current, level]
    );
  }

  return (
    <div className="app-logs-page" data-app-logs-page>
      {error ? <div className="notice error">{error}</div> : null}

      <section className={hasConnectedDevice ? 'app-log-top-grid connected' : 'app-log-top-grid'}>
        <ConnectPanel state={state} connected={hasConnectedDevice} onClear={clearCurrentView} />
        <DevicePanel
          devices={devices}
          selectedToken={selectedToken}
          onSelectToken={setSelectedToken}
        />
      </section>

      <section className="panel app-log-stream">
        <div className="panel-head compact">
          <strong>日志流</strong>
          <div className="app-log-toolbar">
            <input
              aria-label="搜索日志"
              value={search}
              onChange={(event) => setSearch(event.currentTarget.value)}
              placeholder="搜索消息和字段值"
            />
            <div className="app-log-level-filter" aria-label="级别筛选">
              {levels.map((level) => (
                <label key={level} className={selectedLevels.includes(level) ? 'active' : ''}>
                  <input
                    type="checkbox"
                    checked={selectedLevels.includes(level)}
                    onChange={() => toggleLevel(level)}
                  />
                  {level}
                </label>
              ))}
            </div>
          </div>
        </div>

        {state?.errors.length ? (
          <div className="app-log-errors">
            {state.errors.map((item) => (
              <div key={item.sequence} className="notice error compact">
                {item.device || '未知设备'}：{item.message}
              </div>
            ))}
          </div>
        ) : null}

        <div className="log-console">
          {filteredLogs.length === 0 ? (
            <div className="log-empty">暂无日志，扫码连接 App 后会自动显示。</div>
          ) : (
            filteredLogs.map((log) => <LogRow key={log.sequence} log={log} search={search} />)
          )}
        </div>
      </section>
    </div>
  );
}

function ConnectPanel({
  state,
  connected,
  onClear
}: {
  state: AppLogsState | null;
  connected: boolean;
  onClear: () => void;
}) {
  const connect = state?.connect;
  return (
    <section className={connected ? 'panel app-log-connect connected' : 'panel app-log-connect'}>
      <div className="panel-head compact">
        <strong>连接 App</strong>
        <span className={connected ? 'status-pill ok' : 'status-pill'}>{connected ? '已连接' : '等待连接'}</span>
      </div>
      <div className="app-log-connect-content">
        {!connected && connect ? (
          <img
            className="app-log-qr"
            src={`/admin/app-logs/qr.svg?host=${connect.host}&port=${connect.port}&name=${connect.name}`}
            alt="App 日志连接二维码"
          />
        ) : null}
        <div className="app-log-connect-copy">
          <strong>{connect?.appName ?? 'AnyStories'}</strong>
          <code>{connect?.schemeUrl ?? '加载连接信息中'}</code>
          {!connected ? <code>{connect?.websocketUrl ?? ''}</code> : null}
          <div className="inline-actions">
            <button
              className="button"
              type="button"
              onClick={() => void navigator.clipboard?.writeText(connect?.connectPageUrl ?? '')}
            >
              复制扫码页面
            </button>
            <button className="button" type="button" onClick={onClear}>
              清空当前视图
            </button>
          </div>
        </div>
      </div>
    </section>
  );
}

function DevicePanel({
  devices,
  selectedToken,
  onSelectToken
}: {
  devices: AppLogDevice[];
  selectedToken: string;
  onSelectToken: (token: string) => void;
}) {
  return (
    <section className="panel app-log-devices">
      <div className="panel-head compact">
        <strong>设备</strong>
        <span>{devices.length}</span>
      </div>
      <div className="device-list">
        <button
          className={selectedToken === 'all' ? 'device-row active' : 'device-row'}
          type="button"
          onClick={() => onSelectToken('all')}
        >
          <span>全部设备</span>
          <small>显示所有日志</small>
        </button>
        {devices.map((device) => (
          <button
            key={device.token}
            className={selectedToken === device.token ? 'device-row active' : 'device-row'}
            type="button"
            onClick={() => onSelectToken(device.token)}
          >
            <span>{device.device || '未知设备'}</span>
            <small>token={device.token}</small>
            <em>{device.connected ? '在线' : '离线'}</em>
          </button>
        ))}
      </div>
    </section>
  );
}

function LogRow({ log, search }: { log: AppLogEntry; search: string }) {
  return (
    <article className={`log-row level-${log.level || 'unknown'}`}>
      <span className="log-time">{log.timestamp || timeOnly(log.receivedAt)}</span>
      <span className="log-level">{log.level || '信息'}</span>
      <div className="log-body">
        {log.tag ? <span className="log-meta">标签：{log.tag}</span> : null}
        {log.event ? <span className="log-meta">事件：{log.event}</span> : null}
        <strong>
          消息：<Highlight value={log.message || log.raw} search={search} />
        </strong>
        {log.fields.length ? (
          <div className="log-fields">
            {log.fields.map((field) => (
              <span key={`${log.sequence}-${field.key}`}>
                {field.key}：<Highlight value={field.value} search={search} />
              </span>
            ))}
          </div>
        ) : null}
      </div>
      {log.history ? <span className="log-history">历史</span> : null}
    </article>
  );
}

function Highlight({ value, search }: { value: string; search: string }) {
  const normalizedSearch = search.trim().toLowerCase();
  if (!normalizedSearch) return <>{value}</>;
  const index = value.toLowerCase().indexOf(normalizedSearch);
  if (index === -1) return <>{value}</>;
  return (
    <>
      {value.slice(0, index)}
      <mark>{value.slice(index, index + search.length)}</mark>
      {value.slice(index + search.length)}
    </>
  );
}

function mergeAppLogs(current: AppLogsState, next: AppLogsState): AppLogsState {
  return {
    ...next,
    logs: mergeBySequence(next.logs, current.logs).slice(0, 500),
    errors: mergeBySequence(next.errors, current.errors).slice(0, 100)
  };
}

function mergeBySequence<T extends { sequence: number }>(incoming: T[], existing: T[]): T[] {
  const merged = new Map<number, T>();
  for (const item of [...incoming, ...existing]) {
    merged.set(item.sequence, item);
  }
  return [...merged.values()].sort((left, right) => right.sequence - left.sequence);
}

function matchesSearch(log: AppLogEntry, search: string) {
  const value = search.trim().toLowerCase();
  if (!value) return true;
  return (
    log.message.toLowerCase().includes(value) ||
    log.fields.some((field) => field.value.toLowerCase().includes(value))
  );
}

function timeOnly(value: string) {
  return value.slice(11, 19) || value;
}

function errorMessage(error: unknown): string {
  if (error instanceof AdminApiError) return error.message;
  if (error instanceof Error) return error.message;
  return 'App 日志加载失败，请稍后重试';
}

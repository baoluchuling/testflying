import { FormEvent, useEffect, useMemo, useState } from 'react';
import {
  AdminApiError,
  loadBuildRunnersState,
  provisionBuildRunner,
  type BuildRunnerItem,
  type BuildRunnersState,
  type RunnerProvisionResponse
} from '../app/apiClient';

type RunnerForm = {
  runnerId: string;
  name: string;
  labels: string;
  arch: 'arm64' | 'amd64';
  llmAdapters: string;
};

const EMPTY_FORM: RunnerForm = {
  runnerId: '',
  name: '',
  labels: '',
  arch: 'arm64',
  llmAdapters: ''
};

const RUNNER_ID_PATTERN = String.raw`[A-Za-z0-9][A-Za-z0-9._\-]{0,63}`;

export function BuildRunnersPage() {
  const [state, setState] = useState<BuildRunnersState | null>(null);
  const [error, setError] = useState('');
  const [provisioning, setProvisioning] = useState(false);

  useEffect(() => {
    let cancelled = false;
    loadBuildRunnersState()
      .then((payload) => {
        if (!cancelled) setState(payload);
      })
      .catch((requestError) => {
        if (!cancelled) setError(errorMessage(requestError));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <>
      <section className="panel table-panel" data-build-runners-page>
        <div className="panel-head">
          <div>
            <strong>构建节点</strong>
            <p className="muted">维护节点能力、接入凭据和运行版本。</p>
          </div>
          <div className="panel-actions">
            <span>{state?.total ?? 0} 个节点</span>
            <button className="button primary" type="button" onClick={() => setProvisioning(true)}>
              新增节点
            </button>
          </div>
        </div>
        {error ? <div className="notice error">{error}</div> : null}
        <div className="data-table builds-runners-table" role="table" aria-label="构建节点列表">
          <div className="data-table-row header" role="row">
            <span>节点</span>
            <span>标签</span>
            <span>版本</span>
            <span>能力</span>
            <span>最近心跳</span>
            <span>当前构建</span>
          </div>
          {(state?.runners ?? []).map((runner) => (
            <BuildRunnerRow key={runner.id} runner={runner} />
          ))}
        </div>
        {!state && !error ? <div className="empty-state">正在加载构建节点...</div> : null}
        {state && state.runners.length === 0 ? <div className="empty-state">暂无构建节点。</div> : null}
      </section>
      {provisioning ? (
        <RunnerProvisionDialog
          onClose={() => setProvisioning(false)}
          onProvisioned={(runner) => {
            setState((current) => {
              const runners = [...(current?.runners ?? []).filter((item) => item.id !== runner.id), runner];
              return { runners, total: runners.length };
            });
          }}
        />
      ) : null}
    </>
  );
}

function BuildRunnerRow({ runner }: { runner: BuildRunnerItem }) {
  const llmAdapters = valueList(runner.capabilities.llmAdapters);
  const statusTone = runner.status === 'online' ? 'ok' : runner.status === 'busy' ? 'warn' : 'danger';
  const updateTone = runner.updateStatus === 'current' ? 'ok' : runner.updateStatus === 'outdated' ? 'warn' : '';

  return (
    <div className="data-table-row build-runner-row" role="row">
      <span>
        <strong>{runner.name}</strong>
        <small>{runner.id}</small>
        <span className={`tag ${statusTone}`}>{runner.status}</span>
      </span>
      <span>{runner.labels.join(', ') || '-'}</span>
      <span>
        <strong>{runner.version || '-'}</strong>
        <small>package-agent {runner.packageAgentVersion || '-'}</small>
        <span className={`tag ${updateTone}`}>{runner.updateStatusLabel}</span>
      </span>
      <span>
        <strong>iOS / Android</strong>
        <small>LLM: {llmAdapters.join(', ') || '-'}</small>
      </span>
      <span>{runner.lastSeenAtLabel}</span>
      <span>{runner.currentBuildId || '-'}</span>
    </div>
  );
}

function RunnerProvisionDialog({
  onClose,
  onProvisioned
}: {
  onClose: () => void;
  onProvisioned: (runner: BuildRunnerItem) => void;
}) {
  const [form, setForm] = useState<RunnerForm>(EMPTY_FORM);
  const [result, setResult] = useState<RunnerProvisionResponse | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const configJson = useMemo(() => (result ? runnerConfigJson(result) : ''), [result]);

  useEffect(() => {
    if (!saving) return;
    const preventUnload = (event: BeforeUnloadEvent) => {
      event.preventDefault();
      event.returnValue = '';
    };
    const preventNavigation = (event: Event) => event.preventDefault();
    window.addEventListener('beforeunload', preventUnload);
    window.addEventListener('admin:before-navigation', preventNavigation);
    return () => {
      window.removeEventListener('beforeunload', preventUnload);
      window.removeEventListener('admin:before-navigation', preventNavigation);
    };
  }, [saving]);

  function close() {
    if (saving) return;
    setResult(null);
    setForm(EMPTY_FORM);
    setError('');
    onClose();
  }

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setSaving(true);
    setError('');
    try {
      const response = await provisionBuildRunner({
        runnerId: form.runnerId.trim(),
        name: form.name.trim(),
        labels: commaList(form.labels),
        version: '',
        packageAgentVersion: '',
        capabilities: {
          llmAdapters: commaList(form.llmAdapters),
          capacity: 1,
          hostPlatform: 'darwin',
          arch: form.arch
        }
      });
      setResult(response);
      onProvisioned(response.runner);
    } catch (requestError) {
      setError(errorMessage(requestError));
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="modal-backdrop" role="presentation">
      <section
        aria-label={result ? '一次性节点配置' : '新增构建节点'}
        aria-modal="true"
        className="runner-provision-dialog"
        role="dialog"
      >
        <header>
          <div>
            <p className="eyebrow">Build Runner</p>
            <h3>{result ? '一次性接入配置' : '新增构建节点'}</h3>
          </div>
          <button
            className="button text"
            type="button"
            onClick={close}
            aria-label="关闭一次性配置"
            disabled={saving}
          >
            关闭
          </button>
        </header>
        {result ? (
          <RunnerProvisionResult result={result} configJson={configJson} />
        ) : (
          <form className="form-grid two" onSubmit={submit}>
            <label>
              <span>节点 ID</span>
              <input
                aria-label="节点 ID"
                maxLength={64}
                pattern={RUNNER_ID_PATTERN}
                required
                title="仅支持字母、数字、点、下划线和短横线，且必须以字母或数字开头"
                value={form.runnerId}
                onChange={(event) => setForm({ ...form, runnerId: event.target.value })}
                placeholder="runner-mac-01"
              />
            </label>
            <label>
              <span>节点名称</span>
              <input
                aria-label="节点名称"
                required
                value={form.name}
                onChange={(event) => setForm({ ...form, name: event.target.value })}
                placeholder="Mac mini 01"
              />
            </label>
            <label>
              <span>节点标签</span>
              <input
                aria-label="节点标签"
                value={form.labels}
                onChange={(event) => setForm({ ...form, labels: event.target.value })}
                placeholder="ios-release, flutter"
              />
            </label>
            <label>
              <span>节点架构</span>
              <select
                aria-label="节点架构"
                value={form.arch}
                onChange={(event) => setForm({ ...form, arch: event.target.value as RunnerForm['arch'] })}
              >
                <option value="arm64">arm64</option>
                <option value="amd64">amd64</option>
              </select>
            </label>
            <label>
              <span>LLM 适配器</span>
              <input
                aria-label="LLM 适配器"
                value={form.llmAdapters}
                onChange={(event) => setForm({ ...form, llmAdapters: event.target.value })}
                placeholder="codex, claude"
              />
            </label>
            {error ? <div className="notice error compact form-wide">{error}</div> : null}
            {saving ? (
              <div className="notice warning compact form-wide">正在签发一次性配置，请勿关闭或刷新页面。</div>
            ) : null}
            <div className="form-actions">
              <button className="button" type="button" onClick={close} disabled={saving}>
                取消
              </button>
              <button className="button primary" type="submit" disabled={saving}>
                {saving ? '生成中...' : '生成接入配置'}
              </button>
            </div>
          </form>
        )}
      </section>
    </div>
  );
}

function RunnerProvisionResult({
  result,
  configJson
}: {
  result: RunnerProvisionResponse;
  configJson: string;
}) {
  return (
    <div className="runner-provision-result">
      <div className="notice warn compact">
        <strong>请立即保存，关闭后无法再次查看</strong>
        <span>
          中心后台不会保留原始 token；若配置丢失，可使用同一节点 ID 重新生成配置，旧 token 会立即失效。
        </span>
      </div>
      <div className="one-time-token">
        <span>Runner token</span>
        <code>{result.token}</code>
        <button className="button slim" type="button" onClick={() => copyText(result.token)}>
          复制 Token
        </button>
      </div>
      <div className="config-preview">
        <div>
          <strong>runner-config.json</strong>
          <button className="button slim" type="button" onClick={() => copyText(configJson)}>
            复制配置 JSON
          </button>
        </div>
        <pre>{configJson}</pre>
      </div>
    </div>
  );
}

function runnerConfigJson(result: RunnerProvisionResponse): string {
  const capabilities = result.runner.capabilities;
  return JSON.stringify(
    {
      runnerId: result.runner.id,
      name: result.runner.name,
      token: result.token,
      serverUrl: window.location.origin,
      rootDir: `/Users/Shared/TestFlyingRunner/${result.runner.id}`,
      labels: result.runner.labels,
      llmAdapters: valueList(capabilities.llmAdapters),
      capacity: 1
    },
    null,
    2
  );
}

function copyText(value: string) {
  void navigator.clipboard?.writeText(value);
}

function commaList(value: string): string[] {
  return value
    .split(',')
    .map((item) => item.trim())
    .filter(Boolean);
}

function valueList(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value.map((item) => String(item).trim()).filter(Boolean);
}

function errorMessage(error: unknown): string {
  if (error instanceof AdminApiError) return error.message;
  if (error instanceof Error) return error.message;
  return '请求失败，请稍后重试';
}

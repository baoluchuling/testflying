import { FormEvent, useEffect, useState } from 'react';
import type {
  NotificationSettingsPayload,
  NotificationSettingsState
} from '../app/apiClient';

type NotificationForm = {
  enabled: boolean;
  webhookUrl: string;
  secret: string;
  timeoutSeconds: string;
  dispatchIntervalSeconds: string;
};

export function NotificationSettingsPage({
  state,
  onSave,
  onCheck
}: {
  state: NotificationSettingsState;
  onSave: (payload: NotificationSettingsPayload) => Promise<void>;
  onCheck: () => Promise<void>;
}) {
  const [form, setForm] = useState(() => formFromState(state));
  const [saving, setSaving] = useState(false);
  const [checking, setChecking] = useState(false);

  useEffect(() => setForm(formFromState(state)), [state]);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setSaving(true);
    try {
      await onSave({
        enabled: form.enabled,
        webhookUrl: form.webhookUrl.trim() || null,
        secret: form.secret.trim() || null,
        timeoutSeconds: Number(form.timeoutSeconds),
        dispatchIntervalSeconds: Number(form.dispatchIntervalSeconds)
      });
    } finally {
      setSaving(false);
    }
  }

  async function check() {
    setChecking(true);
    try {
      await onCheck();
    } finally {
      setChecking(false);
    }
  }

  return (
    <section className="panel settings-panel">
      <div className="settings-section-head">
        <div>
          <p className="eyebrow">Notifications</p>
          <h2>通知设置</h2>
          <p>构建失败或需要人工处理时，通过钉钉自定义机器人发送通知。</p>
        </div>
        <span className={`tag ${state.configured ? 'ok' : 'warn'}`}>
          {state.configured ? '已配置' : '未配置'}
        </span>
      </div>
      <div className="settings-metrics" aria-label="通知投递状态">
        <div>
          <span>待发送</span>
          <strong>{state.pendingDeliveryCount}</strong>
        </div>
        <div>
          <span>失败</span>
          <strong>{state.deadDeliveryCount}</strong>
        </div>
        <div>
          <span>Webhook</span>
          <strong>{state.webhookConfigured ? '已配置' : '未配置'}</strong>
        </div>
        <div>
          <span>加签密钥</span>
          <strong>{state.secretConfigured ? '密钥已配置' : '未配置'}</strong>
        </div>
      </div>
      <form className="settings-form settings-form-grid" onSubmit={submit}>
        <label className="toggle-field">
          <input
            type="checkbox"
            checked={form.enabled}
            onChange={(event) => setForm({ ...form, enabled: event.target.checked })}
          />
          <span>
            <strong>启用钉钉通知</strong>
            <small>关闭后保留配置，但不再投递新通知。</small>
          </span>
        </label>
        <label className="form-wide">
          <span>Webhook URL</span>
          <input
            aria-label="Webhook URL"
            type="password"
            value={form.webhookUrl}
            onChange={(event) => setForm({ ...form, webhookUrl: event.target.value })}
            placeholder={state.webhookConfigured ? 'Webhook 已配置，留空不修改' : '请输入钉钉 Webhook URL'}
          />
        </label>
        <label className="form-wide">
          <span>加签密钥</span>
          <input
            aria-label="加签密钥"
            type="password"
            value={form.secret}
            onChange={(event) => setForm({ ...form, secret: event.target.value })}
            placeholder={state.secretConfigured ? '密钥已配置，留空不修改' : '请输入 SEC 开头的密钥'}
          />
          {state.secretConfigured ? <small>密钥已配置</small> : null}
        </label>
        <label>
          <span>请求超时</span>
          <div className="input-with-unit">
            <input
              aria-label="请求超时"
              min="1"
              step="1"
              type="number"
              value={form.timeoutSeconds}
              onChange={(event) => setForm({ ...form, timeoutSeconds: event.target.value })}
            />
            <span>秒</span>
          </div>
        </label>
        <label>
          <span>投递间隔</span>
          <div className="input-with-unit">
            <input
              aria-label="投递间隔"
              min="1"
              step="1"
              type="number"
              value={form.dispatchIntervalSeconds}
              onChange={(event) =>
                setForm({ ...form, dispatchIntervalSeconds: event.target.value })
              }
            />
            <span>秒</span>
          </div>
        </label>
        <div className="form-actions">
          <button className="button" type="button" disabled={checking} onClick={() => void check()}>
            {checking ? '检查中...' : '检查配置'}
          </button>
          <button className="button primary" type="submit" disabled={saving}>
            {saving ? '保存中...' : '保存配置'}
          </button>
        </div>
      </form>
    </section>
  );
}

function formFromState(state: NotificationSettingsState): NotificationForm {
  return {
    enabled: state.enabled,
    webhookUrl: '',
    secret: '',
    timeoutSeconds: String(state.timeoutSeconds),
    dispatchIntervalSeconds: String(state.dispatchIntervalSeconds)
  };
}

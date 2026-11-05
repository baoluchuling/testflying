import { FormEvent, useEffect, useState } from 'react';
import type { GeneralSettingsPayload, GeneralSettingsState } from '../app/apiClient';

export function GeneralSettingsPage({
  state,
  onSave
}: {
  state: GeneralSettingsState;
  onSave: (payload: GeneralSettingsPayload) => Promise<void>;
}) {
  const [template, setTemplate] = useState(state.connectorBaseUrlTemplate);
  const [saving, setSaving] = useState(false);

  useEffect(() => setTemplate(state.connectorBaseUrlTemplate), [state.connectorBaseUrlTemplate]);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setSaving(true);
    try {
      await onSave({ connectorBaseUrlTemplate: template.trim() || null });
    } finally {
      setSaving(false);
    }
  }

  return (
    <section className="panel settings-panel">
      <div className="settings-section-head">
        <div>
          <p className="eyebrow">General</p>
          <h2>通用设置</h2>
          <p>维护中心后台自动生成 Connector 地址时使用的模板。</p>
        </div>
        <span className="tag">来源：{sourceLabel(state.source)}</span>
      </div>
      <form className="settings-form" onSubmit={submit}>
        <label>
          <span>Connector 地址模板</span>
          <input
            aria-label="Connector 地址模板"
            value={template}
            onChange={(event) => setTemplate(event.target.value)}
            placeholder="https://connector.example.com/{account_id}"
          />
          <small>可使用 {'{account_id}'} 占位符；留空表示不自动生成地址。</small>
        </label>
        <div className="form-actions">
          <button className="button primary" type="submit" disabled={saving}>
            {saving ? '保存中...' : '保存配置'}
          </button>
        </div>
      </form>
    </section>
  );
}

function sourceLabel(source: string) {
  if (source === 'database') return '后台配置';
  if (source === 'environment') return '环境变量';
  return '默认值';
}

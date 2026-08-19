import { useEffect, useMemo, useState } from 'react';
import {
  AdminApiError,
  createLlmProfile,
  loadLlmConfig,
  updateLlmFeatureBinding,
  updateLlmProfile,
  type LlmConfigState,
  type LlmPresetItem,
  type LlmProfileItem,
  type LlmProfilePayload
} from '../app/apiClient';

const emptyForm: LlmProfilePayload = {
  name: '',
  protocol: 'openai_compatible',
  baseUrl: 'https://api.openai.com/v1',
  model: 'gpt-4o-mini',
  apiKey: '',
  authHeader: 'authorization_bearer'
};

export function LlmConfigPage() {
  const [state, setState] = useState<LlmConfigState | null>(null);
  const [selectedProfileId, setSelectedProfileId] = useState('');
  const [form, setForm] = useState<LlmProfilePayload>(emptyForm);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [message, setMessage] = useState('');
  const [error, setError] = useState('');

  const selectedProfile = useMemo(
    () => state?.profiles.find((profile) => profile.id === selectedProfileId) ?? null,
    [selectedProfileId, state?.profiles]
  );

  useEffect(() => {
    void refresh();
  }, []);

  async function refresh() {
    setLoading(true);
    setError('');
    try {
      const payload = await loadLlmConfig();
      setState(payload);
      if (!selectedProfileId && payload.profiles[0]) {
        selectProfile(payload.profiles[0]);
      }
    } catch (requestError) {
      setError(errorMessage(requestError));
    } finally {
      setLoading(false);
    }
  }

  function selectProfile(profile: LlmProfileItem) {
    setSelectedProfileId(profile.id);
    setForm({
      name: profile.name,
      protocol: profile.protocol,
      baseUrl: profile.baseUrl,
      model: profile.model,
      apiKey: '',
      authHeader: profile.authHeader
    });
  }

  function newProfile(preset?: LlmPresetItem) {
    setSelectedProfileId('');
    setForm({
      name: preset?.label ?? '',
      protocol: preset?.protocol ?? 'openai_compatible',
      baseUrl: preset?.baseUrl ?? 'https://api.openai.com/v1',
      model: preset?.model ?? 'gpt-4o-mini',
      apiKey: '',
      authHeader: preset?.authHeader ?? 'authorization_bearer'
    });
  }

  function patchForm(patch: Partial<LlmProfilePayload>) {
    setForm((current) => ({ ...current, ...patch }));
  }

  async function saveProfile() {
    setSaving(true);
    setError('');
    setMessage('');
    try {
      const payload = selectedProfileId
        ? await updateLlmProfile(selectedProfileId, form)
        : await createLlmProfile(form);
      setState(payload.state);
      setSelectedProfileId(payload.profile.id);
      setForm({
        name: payload.profile.name,
        protocol: payload.profile.protocol,
        baseUrl: payload.profile.baseUrl,
        model: payload.profile.model,
        apiKey: '',
        authHeader: payload.profile.authHeader
      });
      setMessage(payload.message);
    } catch (requestError) {
      setError(errorMessage(requestError));
    } finally {
      setSaving(false);
    }
  }

  async function bindFeature(featureKey: string, primaryProfileId: string) {
    setError('');
    setMessage('');
    try {
      const payload = await updateLlmFeatureBinding(featureKey, {
        primaryProfileId: primaryProfileId || null
      });
      setState(payload.state);
      setMessage(payload.message);
    } catch (requestError) {
      setError(errorMessage(requestError));
    }
  }

  return (
    <div className="llm-config-page">
      {error ? <div className="notice error">{error}</div> : null}
      {message ? <div className="notice success">{message}</div> : null}

      <section className="panel llm-config-panel">
        <div className="panel-head compact">
          <div>
            <strong>模型配置</strong>
            <span>只配置协议、地址、模型和密钥；功能使用关系在下方绑定。</span>
          </div>
          <button className="button" type="button" onClick={() => newProfile()}>
            新建模型
          </button>
        </div>

        <div className="llm-config-grid">
          <aside className="llm-profile-list" aria-label="模型列表">
            <div className="llm-preset-row">
              {(state?.presets ?? []).map((preset) => (
                <button
                  key={preset.key}
                  className="button subtle"
                  type="button"
                  onClick={() => newProfile(preset)}
                >
                  {preset.label}
                </button>
              ))}
            </div>
            {loading ? <div className="empty-state">正在加载 LLM 配置...</div> : null}
            {state?.profiles.map((profile) => (
              <button
                key={profile.id}
                className={
                  profile.id === selectedProfileId ? 'llm-profile-card active' : 'llm-profile-card'
                }
                type="button"
                onClick={() => selectProfile(profile)}
              >
                <strong>{profile.name}</strong>
                <span>{profile.protocolLabel}</span>
                <small>
                  {profile.model} · {profile.statusLabel}
                </small>
              </button>
            ))}
            {state && state.profiles.length === 0 ? (
              <div className="empty-state">还没有模型。可以先点小米 MiMo 预设。</div>
            ) : null}
          </aside>

          <div className="llm-profile-editor">
            <div className="form-grid two">
              <label>
                <span>模型名称</span>
                <input
                  value={form.name}
                  onChange={(event) => patchForm({ name: event.target.value })}
                  placeholder="例如：小米 MiMo"
                />
              </label>
              <label>
                <span>接口协议</span>
                <select
                  value={form.protocol}
                  onChange={(event) => {
                    const protocol = state?.protocols.find(
                      (item) => item.key === event.target.value
                    );
                    patchForm({
                      protocol: event.target.value,
                      baseUrl: protocol?.defaultBaseUrl ?? form.baseUrl,
                      model: protocol?.defaultModel ?? form.model,
                      authHeader: protocol?.defaultAuthHeader ?? form.authHeader
                    });
                  }}
                >
                  {(state?.protocols ?? []).map((protocol) => (
                    <option key={protocol.key} value={protocol.key}>
                      {protocol.label}
                    </option>
                  ))}
                </select>
              </label>
              <label>
                <span>Base URL</span>
                <input
                  value={form.baseUrl}
                  onChange={(event) => patchForm({ baseUrl: event.target.value })}
                  placeholder="https://api.example.com/v1"
                />
              </label>
              <label>
                <span>模型 ID</span>
                <input
                  value={form.model}
                  onChange={(event) => patchForm({ model: event.target.value })}
                  placeholder="mimo-v2.5-pro"
                />
              </label>
              <label>
                <span>鉴权头</span>
                <select
                  value={form.authHeader}
                  onChange={(event) => patchForm({ authHeader: event.target.value })}
                >
                  <option value="authorization_bearer">Authorization: Bearer</option>
                  <option value="api-key">api-key</option>
                  <option value="x-api-key">x-api-key</option>
                </select>
              </label>
              <label>
                <span>API Key</span>
                <input
                  type="password"
                  value={form.apiKey ?? ''}
                  onChange={(event) => patchForm({ apiKey: event.target.value })}
                  placeholder={
                    selectedProfile?.apiKeySet
                      ? `已保存 ${selectedProfile.apiKeyPreview}，留空不修改`
                      : '请输入 API Key'
                  }
                />
              </label>
            </div>
            <div className="form-actions">
              <button className="button primary" type="button" onClick={saveProfile} disabled={saving}>
                {saving ? '保存中' : selectedProfileId ? '更新模型' : '保存模型'}
              </button>
            </div>
          </div>
        </div>
      </section>

      <section className="panel llm-binding-panel">
        <div className="panel-head compact">
          <div>
            <strong>功能绑定</strong>
            <span>一个功能同一时间只绑定一个主模型。</span>
          </div>
        </div>
        <div className="llm-binding-list">
          {(state?.featureBindings ?? []).map((binding) => (
            <div key={binding.featureKey} className="llm-binding-row">
              <div>
                <strong>{binding.featureLabel}</strong>
                <span>{binding.description}</span>
              </div>
              <select
                value={binding.primaryProfileId ?? ''}
                onChange={(event) => void bindFeature(binding.featureKey, event.target.value)}
              >
                <option value="">不启用</option>
                {(state?.profiles ?? []).map((profile) => (
                  <option key={profile.id} value={profile.id}>
                    {profile.name}
                  </option>
                ))}
              </select>
              <span className={`tag ${binding.status === 'ready' ? 'ok' : 'warn'}`}>
                {binding.statusLabel}
              </span>
            </div>
          ))}
        </div>
      </section>
    </div>
  );
}

function errorMessage(error: unknown) {
  if (error instanceof AdminApiError) return error.message;
  if (error instanceof Error) return error.message;
  return '请求失败';
}

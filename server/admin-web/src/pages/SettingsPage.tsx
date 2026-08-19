import { useEffect, useState } from 'react';
import {
  AdminApiError,
  checkNotificationSettings,
  loadSettingsState,
  saveGeneralSettings,
  saveNotificationSettings,
  type GeneralSettingsPayload,
  type NotificationSettingsPayload,
  type SettingsActionResponse,
  type SettingsState
} from '../app/apiClient';
import { settingsViewFromPath, type SettingsView } from '../app/routes';
import { GeneralSettingsPage } from './GeneralSettingsPage';
import { LlmConfigPage } from './LlmConfigPage';
import { NotificationSettingsPage } from './NotificationSettingsPage';
import { RuntimeSettingsPage } from './RuntimeSettingsPage';

const views: Array<{ key: SettingsView; label: string; path: string }> = [
  { key: 'general', label: '通用设置', path: '/admin/settings/general' },
  { key: 'notifications', label: '通知设置', path: '/admin/settings/notifications' },
  { key: 'llm', label: 'LLM 配置', path: '/admin/settings/llm' },
  { key: 'runtime', label: '运行环境', path: '/admin/settings/runtime' }
];

export function SettingsPage({ view }: { view?: SettingsView }) {
  const [activeView, setActiveView] = useState<SettingsView>(
    view ?? settingsViewFromPath(location.pathname)
  );
  const [state, setState] = useState<SettingsState | null>(null);
  const [message, setMessage] = useState('');
  const [error, setError] = useState('');

  useEffect(() => {
    if (view) setActiveView(view);
  }, [view]);

  useEffect(() => {
    let cancelled = false;
    loadSettingsState()
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

  function navigate(item: (typeof views)[number]) {
    setActiveView(item.key);
    history.pushState({ adminRoute: 'settings' }, '', item.path);
  }

  async function runAction(request: () => Promise<SettingsActionResponse>) {
    setError('');
    setMessage('');
    try {
      const response = await request();
      setState(response.state);
      setMessage(response.message);
    } catch (requestError) {
      setError(errorMessage(requestError));
    }
  }

  return (
    <div className="section-workspace settings-workspace" data-settings-workspace>
      <nav className="secondary-nav" aria-label="设置导航">
        {views.map((item) => (
          <button
            key={item.key}
            className={activeView === item.key ? 'secondary-nav-item active' : 'secondary-nav-item'}
            type="button"
            onClick={() => navigate(item)}
          >
            {item.label}
          </button>
        ))}
      </nav>
      {error ? <div className="notice error compact">{error}</div> : null}
      {message ? <div className="notice success compact">{message}</div> : null}
      {!state && activeView !== 'llm' && !error ? (
        <section className="panel empty-state">正在加载设置...</section>
      ) : null}
      {state && activeView === 'general' ? (
        <GeneralSettingsPage
          state={state.general}
          onSave={(payload: GeneralSettingsPayload) =>
            runAction(() => saveGeneralSettings(payload))
          }
        />
      ) : null}
      {state && activeView === 'notifications' ? (
        <NotificationSettingsPage
          state={state.notifications}
          onSave={(payload: NotificationSettingsPayload) =>
            runAction(() => saveNotificationSettings(payload))
          }
          onCheck={() => runAction(checkNotificationSettings)}
        />
      ) : null}
      {activeView === 'llm' ? <LlmConfigPage /> : null}
      {state && activeView === 'runtime' ? <RuntimeSettingsPage items={state.runtime} /> : null}
    </div>
  );
}

function errorMessage(error: unknown) {
  if (error instanceof AdminApiError) return error.message;
  if (error instanceof Error) return error.message;
  return '请求失败，请稍后重试';
}

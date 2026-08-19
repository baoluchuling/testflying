import { useEffect, useState } from 'react';
import {
  AdminApiError,
  loadNotificationsState,
  type NotificationItem,
  type NotificationsState
} from '../app/apiClient';

export function NotificationsPage() {
  const [state, setState] = useState<NotificationsState | null>(null);
  const [error, setError] = useState('');

  useEffect(() => {
    void loadFromLocation();
    const onPopState = () => void loadFromLocation();
    window.addEventListener('popstate', onPopState);
    return () => window.removeEventListener('popstate', onPopState);
  }, []);

  async function loadFromLocation() {
    setError('');
    try {
      setState(await loadNotificationsState(location.search || ''));
    } catch (requestError) {
      setError(errorMessage(requestError));
    }
  }

  async function selectType(type: string) {
    const params = new URLSearchParams();
    if (type !== 'all') params.set('type', type);
    const query = params.toString();
    history.pushState(
      { adminRoute: 'notifications', type },
      '',
      query ? `/admin/notifications?${query}` : '/admin/notifications'
    );
    await loadFromLocation();
  }

  return (
    <div className="notifications-page" data-notifications-page>
      <section className="panel">
        <div className="panel-head">
          <div>
            <strong>通知</strong>
            <p className="muted">{state?.total ?? 0} 条记录</p>
          </div>
          <button
            className="button subtle"
            type="button"
            onClick={() =>
              history.pushState(
                { adminRoute: 'settings' },
                '',
                '/admin/settings/notifications'
              )
            }
          >
            管理通知渠道
          </button>
        </div>
        <div className="filter-tabs notification-filter-tabs" aria-label="通知类型筛选">
          {(state?.typeCounts ?? [{ type: 'all', label: '全部', count: 0 }]).map((item) => (
            <button
              key={item.type}
              type="button"
              className={state?.activeType === item.type ? 'filter-tab active' : 'filter-tab'}
              onClick={() => void selectType(item.type)}
            >
              {item.label}
              <span>{item.count}</span>
            </button>
          ))}
        </div>
        {error ? <div className="notice error">{error}</div> : null}
        <div className="notification-list">
          {(state?.notifications ?? []).map((notification) => (
            <NotificationRow key={notification.id} notification={notification} />
          ))}
        </div>
        {!state && !error ? <div className="empty-state">正在加载通知...</div> : null}
        {state && state.notifications.length === 0 ? (
          <div className="empty-state">当前筛选下没有通知。</div>
        ) : null}
      </section>
    </div>
  );
}

function NotificationRow({ notification }: { notification: NotificationItem }) {
  return (
    <article className="notification-row">
      <span className="notification-dot" style={{ backgroundColor: notification.tagColor }} />
      <div>
        <strong>{notification.title}</strong>
        <span>{notification.subtitle}</span>
        <small>{notification.section} · {notification.createdAtLabel}</small>
      </div>
      <em>{notification.tag}</em>
    </article>
  );
}

function errorMessage(error: unknown): string {
  if (error instanceof AdminApiError) return error.message;
  if (error instanceof Error) return error.message;
  return '请求失败，请稍后重试';
}

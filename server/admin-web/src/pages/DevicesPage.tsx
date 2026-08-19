import { useCallback, useEffect, useState } from 'react';
import {
  AdminApiError,
  loadDevicesState,
  type DeviceItem,
  type DevicesState
} from '../app/apiClient';

export function DevicesPage() {
  const [state, setState] = useState<DevicesState | null>(null);
  const [error, setError] = useState('');
  const [refreshing, setRefreshing] = useState(false);

  const load = useCallback(async () => {
    setRefreshing(true);
    try {
      setState(await loadDevicesState());
      setError('');
    } catch (requestError) {
      setError(errorMessage(requestError));
    } finally {
      setRefreshing(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const devices = state?.devices ?? [];
  const iosCount = devices.filter((device) => device.platformLabel === 'iOS').length;

  return (
    <div className="compact-page compact-devices-page devices-page" data-devices-page>
      <div className="compact-context">
        <div className="compact-title">
          <strong>Device Registry</strong>
          <h2>设备</h2>
          <span>
            {state?.total ?? 0} 台设备 · {iosCount} 台 iOS · {devices.length - iosCount} 台 Android
          </span>
        </div>
        <div className="compact-actions">
          <button className="button" type="button" onClick={() => void load()} disabled={refreshing}>
            {refreshing ? '刷新中' : '刷新设备'}
          </button>
        </div>
      </div>

      <div className="compact-body">
        <section className="compact-column">
          <div className="compact-column-head">
            <strong>设备列表</strong>
            <span>设备登记事实 · 审批在后续管理能力中提供</span>
          </div>
          {error ? <div className="notice error compact">{error}</div> : null}
          <div className="compact-scroll">
            <div className="data-table devices-table" role="table" aria-label="设备列表">
              <div className="data-table-row header" role="row">
                <span>设备</span>
                <span>平台</span>
                <span>负责人</span>
                <span>系统</span>
                <span>状态</span>
                <span>登记时间</span>
              </div>
              {devices.map((device) => (
                <DeviceRow key={device.id} device={device} />
              ))}
            </div>
            {!state && !error ? <div className="empty-state">正在加载设备...</div> : null}
            {state && devices.length === 0 ? <div className="empty-state">暂无设备。</div> : null}
          </div>
        </section>
      </div>
    </div>
  );
}

function DeviceRow({ device }: { device: DeviceItem }) {
  return (
    <div className="data-table-row device-table-row" role="row">
      <span>
        <strong>{device.name}</strong>
        <small>{device.udid}</small>
      </span>
      <span>{device.platformLabel}</span>
      <span>{device.owner || '-'}</span>
      <span>{device.osVersion || '-'}</span>
      <span>
        <span className="tag ok">{device.status}</span>
        <small>{device.certificateStatus}</small>
      </span>
      <span>{device.registeredAtLabel}</span>
    </div>
  );
}

function errorMessage(error: unknown): string {
  if (error instanceof AdminApiError) return error.message;
  if (error instanceof Error) return error.message;
  return '请求失败，请稍后重试';
}

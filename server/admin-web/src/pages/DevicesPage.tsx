import { useEffect, useState } from 'react';
import {
  AdminApiError,
  loadDevicesState,
  type DeviceItem,
  type DevicesState
} from '../app/apiClient';

export function DevicesPage() {
  const [state, setState] = useState<DevicesState | null>(null);
  const [error, setError] = useState('');

  useEffect(() => {
    let cancelled = false;
    loadDevicesState()
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
    <section className="panel table-panel" data-devices-page>
      <div className="panel-head">
        <strong>设备列表</strong>
        <span>{state?.total ?? 0} 台设备</span>
      </div>
      {error ? <div className="notice error">{error}</div> : null}
      <div className="data-table devices-table" role="table" aria-label="设备列表">
        <div className="data-table-row header" role="row">
          <span>设备</span>
          <span>平台</span>
          <span>负责人</span>
          <span>系统</span>
          <span>状态</span>
          <span>登记时间</span>
        </div>
        {(state?.devices ?? []).map((device) => (
          <DeviceRow key={device.id} device={device} />
        ))}
      </div>
      {!state && !error ? <div className="empty-state">正在加载设备...</div> : null}
      {state && state.devices.length === 0 ? <div className="empty-state">暂无设备。</div> : null}
    </section>
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

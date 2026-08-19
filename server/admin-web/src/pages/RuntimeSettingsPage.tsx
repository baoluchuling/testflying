import { useMemo } from 'react';
import type { RuntimeEnvironmentItem } from '../app/apiClient';

export function RuntimeSettingsPage({ items }: { items: RuntimeEnvironmentItem[] }) {
  const groups = useMemo(() => groupItems(items), [items]);

  return (
    <section className="panel settings-panel runtime-settings-panel">
      <div className="settings-section-head">
        <div>
          <p className="eyebrow">Runtime</p>
          <h2>运行环境</h2>
          <p>只读展示部署环境状态。修改环境变量后需要重启服务。</p>
        </div>
        <span className="tag">只读</span>
      </div>
      <div className="runtime-groups">
        {groups.map(([group, groupItems]) => (
          <section key={group} className="runtime-group">
            <h3>{group}</h3>
            <div className="runtime-list">
              {groupItems.map((item) => (
                <div key={item.key} className="runtime-row">
                  <div>
                    <strong>{item.key}</strong>
                    <span>{item.label}</span>
                  </div>
                  <div>
                    <span className={`tag ${item.configured ? 'ok' : 'warn'}`}>
                      {item.valueLabel}
                    </span>
                    <small>{sourceLabel(item.source)}</small>
                    {item.restartRequired ? <small>需重启</small> : null}
                  </div>
                </div>
              ))}
            </div>
          </section>
        ))}
      </div>
    </section>
  );
}

function groupItems(items: RuntimeEnvironmentItem[]) {
  const groups = new Map<string, RuntimeEnvironmentItem[]>();
  items.forEach((item) => groups.set(item.group, [...(groups.get(item.group) ?? []), item]));
  return [...groups.entries()];
}

function sourceLabel(source: string) {
  return source === 'environment' ? '环境变量' : '默认值';
}

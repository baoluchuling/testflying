import { useEffect, useState } from 'react';
import {
  AdminApiError,
  loadApiDocsState,
  type ApiDocEndpoint,
  type ApiDocsState
} from '../app/apiClient';

export function ApiDocsPage() {
  const [state, setState] = useState<ApiDocsState | null>(null);
  const [activeAnchor, setActiveAnchor] = useState('');
  const [error, setError] = useState('');

  useEffect(() => {
    let cancelled = false;
    loadApiDocsState()
      .then((payload) => {
        if (cancelled) return;
        setState(payload);
        setActiveAnchor(payload.endpoints[0]?.anchor ?? '');
      })
      .catch((requestError) => {
        if (!cancelled) setError(errorMessage(requestError));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  function jumpTo(endpoint: ApiDocEndpoint) {
    setActiveAnchor(endpoint.anchor);
    document.getElementById(endpoint.anchor)?.scrollIntoView({ block: 'start' });
  }

  return (
    <div className="api-docs-page" data-api-docs-page>
      {error ? <div className="notice error">{error}</div> : null}
      <div className="api-docs-layout">
        <aside className="panel api-docs-index">
          <div className="panel-head compact">
            <strong>目录</strong>
            {state?.downloadUrl ? (
              <a className="button" href={state.downloadUrl}>
                下载 MD
              </a>
            ) : null}
          </div>
          <div className="api-docs-index-list">
            {(state?.endpoints ?? []).map((endpoint) => (
              <button
                key={endpoint.anchor}
                type="button"
                className={endpoint.anchor === activeAnchor ? 'active' : ''}
                onClick={() => jumpTo(endpoint)}
              >
                <span>{endpoint.method}</span>
                <strong>{endpoint.title}</strong>
              </button>
            ))}
          </div>
        </aside>

        <main className="api-docs-content">
          {(state?.endpoints ?? []).map((endpoint) => (
            <EndpointCard key={endpoint.anchor} endpoint={endpoint} />
          ))}
          {!state && !error ? <div className="empty-state">正在加载接口文档...</div> : null}
        </main>
      </div>
    </div>
  );
}

function EndpointCard({ endpoint }: { endpoint: ApiDocEndpoint }) {
  return (
    <section className="panel api-endpoint-card" id={endpoint.anchor}>
      <div className="panel-head compact">
        <div>
          <strong>{endpoint.title}</strong>
          <span>{endpoint.summary}</span>
        </div>
        <code>{endpoint.method} {endpoint.path}</code>
      </div>

      {endpoint.params.length > 0 ? (
        <div className="api-param-table">
          <div className="api-param-row header">
            <span>参数</span>
            <span>位置</span>
            <span>必填</span>
            <span>说明</span>
          </div>
          {endpoint.params.map((param) => (
            <div key={`${endpoint.anchor}-${param.name}-${param.location}`} className="api-param-row">
              <span><code>{param.name}</code></span>
              <span>{param.location}</span>
              <span>{param.required}</span>
              <span>{param.description}</span>
            </div>
          ))}
        </div>
      ) : null}

      <div className="api-code-grid">
        <div>
          <strong>请求示例</strong>
          <pre>{endpoint.curl || '暂无示例'}</pre>
        </div>
        <div>
          <strong>响应示例</strong>
          <pre>{endpoint.response || '暂无示例'}</pre>
        </div>
      </div>
    </section>
  );
}

function errorMessage(error: unknown): string {
  if (error instanceof AdminApiError) return error.message;
  if (error instanceof Error) return error.message;
  return '请求失败，请稍后重试';
}

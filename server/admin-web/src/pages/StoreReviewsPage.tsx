import { useEffect, useMemo, useState } from 'react';
import {
  AdminApiError,
  analyzeStoreReviews,
  fetchStoreReviews,
  loadStoreReviews,
  type ReviewAppItem,
  type StoreReviewsState
} from '../app/apiClient';

const ratingFilters = [
  { label: '全部', value: null },
  { label: '1 星', value: 1 },
  { label: '2 星', value: 2 },
  { label: '3 星', value: 3 }
];

type LoadingAction = 'idle' | 'load' | 'fetch' | 'analyze';

export function StoreReviewsPage() {
  const [state, setState] = useState<StoreReviewsState | null>(null);
  const [loading, setLoading] = useState<LoadingAction>('load');
  const [notice, setNotice] = useState('');
  const [error, setError] = useState('');

  const selectedApp = useMemo(
    () => state?.apps.find((app) => app.selected) ?? null,
    [state]
  );

  useEffect(() => {
    void loadFromLocation();
    const onPopState = () => void loadFromLocation();
    window.addEventListener('popstate', onPopState);
    return () => window.removeEventListener('popstate', onPopState);
  }, []);

  async function loadFromLocation() {
    setLoading('load');
    setError('');
    try {
      const query = location.search || '';
      setState(await loadStoreReviews(query));
    } catch (requestError) {
      setError(errorMessage(requestError));
    } finally {
      setLoading('idle');
    }
  }

  async function selectApp(app: ReviewAppItem) {
    const nextPath = reviewPath({ accountId: app.accountId, appId: app.appId, rating: null });
    history.pushState({ adminRoute: 'store-reviews', accountId: app.accountId, appId: app.appId }, '', nextPath);
    await loadFromLocation();
  }

  async function selectRating(rating: number | null) {
    if (!state?.selectedAccountId || !state.selectedAppId) return;
    const nextPath = reviewPath({
      accountId: state.selectedAccountId,
      appId: state.selectedAppId,
      rating
    });
    history.pushState({ adminRoute: 'store-reviews', rating }, '', nextPath);
    await loadFromLocation();
  }

  async function runFetch() {
    if (!state?.selectedAccountId || !state.selectedAppId) return;
    setLoading('fetch');
    setError('');
    setNotice('');
    try {
      const response = await fetchStoreReviews(state.selectedAccountId, state.selectedAppId);
      setState(response.state);
      setNotice(`${response.message}：新增 ${response.state.latestFetch?.insertedCount ?? 0} 条`);
    } catch (requestError) {
      setError(errorMessage(requestError));
    } finally {
      setLoading('idle');
    }
  }

  async function runAnalyze() {
    if (!state?.selectedAccountId || !state.selectedAppId) return;
    setLoading('analyze');
    setError('');
    setNotice('');
    try {
      const response = await analyzeStoreReviews(state.selectedAccountId, state.selectedAppId);
      setState(response.state);
      if (response.error) {
        setError(response.error.message);
      } else {
        setNotice(response.message);
      }
    } catch (requestError) {
      setError(errorMessage(requestError));
    } finally {
      setLoading('idle');
    }
  }

  return (
    <div className="compact-page compact-reviews-page reviews-workspace" data-store-reviews-page>
      <div className="compact-context">
        <div className="compact-title">
          <strong>Store Review Intelligence</strong>
          <h2>商店评论</h2>
          <span>
            {selectedApp ? selectedApp.appName : '未选择应用'} · {selectedApp?.platform ?? '全部平台'} · 本地 {state?.stats.total ?? 0} 条 · 低分 {state?.stats.low ?? 0} 条 · 只做分析不回复
          </span>
        </div>
        <div className="compact-actions">
          <button className="button" type="button" onClick={() => void runFetch()} disabled={!selectedApp || loading !== 'idle'}>
            {loading === 'fetch' ? '拉取中' : '拉取最新评论'}
          </button>
          <button className="button primary" type="button" onClick={() => void runAnalyze()} disabled={!selectedApp || loading !== 'idle'}>
            {loading === 'analyze' ? '分析中' : 'LLM 分析'}
          </button>
        </div>
      </div>

      <div className="compact-body">
        <div className="compact-review-grid">
          <aside className="compact-column">
            <div className="compact-column-head">
              <strong>商店应用</strong>
              <span>{state?.apps.length ?? 0} 个</span>
            </div>
            <div className="compact-scroll">
              <div className="compact-filter-line">
                <div className="segmented" aria-label="评论应用平台筛选">
                  <button className="active" type="button">全部</button>
                  <button type="button">iOS</button>
                  <button type="button">Android</button>
                </div>
              </div>
              {state?.apps.map((app) => (
                <button
                  key={`${app.accountId}:${app.appId}`}
                  className={app.selected ? 'compact-review-app active review-app-row active' : 'compact-review-app review-app-row'}
                  type="button"
                  onClick={() => void selectApp(app)}
                >
                  <span className="app-logo" style={{ backgroundColor: app.iconColor }}>
                    {app.appName.slice(0, 2).toUpperCase()}
                  </span>
                  <span className="review-app-copy">
                    <strong>{app.appName}</strong>
                    <small>{app.platform} · {app.accountName}</small>
                  </span>
                  <span className={app.reviewCount ? 'tag warn' : 'tag'}>{app.reviewCount}</span>
                </button>
              ))}
              {state && state.apps.length === 0 ? (
                <div className="empty-state">还没有绑定开发者账号的应用。</div>
              ) : null}
            </div>
          </aside>

          <section className="compact-column">
            <div className="compact-review-toolbar">
              <div>
                <strong>{selectedApp ? `${selectedApp.appName} 评论` : '评论'}</strong>
                <div className="meta">
                  <span>空库只拉 20 条</span>
                  <span>遇到已存在同创建日评论停止</span>
                </div>
              </div>
              <div className="segmented" aria-label="评论评分筛选">
                {ratingFilters.map((filter) => (
                  <button
                    key={filter.label}
                    className={state?.rating === filter.value ? 'active' : ''}
                    type="button"
                    onClick={() => void selectRating(filter.value)}
                  >
                    {filter.label}
                  </button>
                ))}
              </div>
            </div>

            {notice ? <div className="notice ok compact">{notice}</div> : null}
            {error ? <div className="notice error compact">{error}</div> : null}

            <div className="compact-scroll compact-review-list">
              {loading === 'load' ? <div className="empty-state">正在加载评论...</div> : null}
              {state?.reviews.map((review, index) => (
                <article className={index === 0 ? 'compact-review-card review-card selected' : 'compact-review-card review-card'} key={review.id}>
                  <header>
                    <strong>{review.title || '无标题评论'}</strong>
                    <span className={review.rating && review.rating <= 2 ? 'tag danger' : 'tag'}>
                      {review.rating ?? '-'} 星
                    </span>
                  </header>
                  <p>{review.body || '暂无评论内容'}</p>
                  <footer>
                    <span>{review.locale || '未知语言'} · {review.appVersion || '未知版本'} · {formatDate(review.createdAt)}</span>
                    <span>{review.authorName || '匿名用户'}</span>
                  </footer>
                </article>
              ))}
              {state && state.reviews.length === 0 && loading !== 'load' ? (
                <div className="empty-state">还没有匹配的本地评论。</div>
              ) : null}
            </div>
          </section>

          <aside className="compact-column">
            <div className="compact-column-head">
              <strong>分析结果</strong>
              <span className={state?.latestAnalysis?.status === 'succeeded' ? 'tag ok' : 'tag'}>
                {state?.latestAnalysis?.status === 'succeeded' ? '已完成' : '未完成'}
              </span>
            </div>
            <div className="compact-scroll">
              {state?.latestAnalysis?.status === 'succeeded' ? (
                <>
                  <section className="compact-analysis-summary analysis-summary-card">
                    <strong>分析摘要</strong>
                    <p>{state.latestAnalysis.summary || '暂无摘要'}</p>
                    <span className="meta">
                      <span>{state.latestAnalysis.reviewCount} 条评论</span>
                      <span>{state.latestAnalysis.issueCount} 个关注点</span>
                      <span>按优先级排序</span>
                    </span>
                  </section>
                  <div className="compact-analysis-list issue-list">
                    {state.analysisIssues.map((issue) => (
                      <article className="compact-issue-card issue-card" key={`${issue.title}:${issue.severity}`}>
                        <header>
                          <strong>{issue.title}</strong>
                          <span className={`tag ${severityClass(issue.severity) === 'high' ? 'danger' : severityClass(issue.severity) === 'low' ? '' : 'warn'}`}>
                            {severityLabel(issue.severity)}
                          </span>
                        </header>
                        <dl className="compact-issue-details">
                          <div>
                            <dt>关注点</dt>
                            <dd>{issue.focus || '需要人工确认影响范围。'}</dd>
                          </div>
                          {issue.evidence.length ? (
                            <div>
                              <dt>证据</dt>
                              <dd>{issue.evidence.join('；')}</dd>
                            </div>
                          ) : null}
                          <div>
                            <dt>建议</dt>
                            <dd>{issue.suggestion || '先确认影响范围，再决定是否进入修复排期。'}</dd>
                          </div>
                        </dl>
                        {issue.representativeReviewIds.length ? (
                          <span className="meta"><span>代表评论：{issue.representativeReviewIds.join('、')}</span></span>
                        ) : null}
                      </article>
                    ))}
                  </div>
                </>
              ) : (
                <div className="empty-state">拉取评论后运行 LLM 分析。</div>
              )}
              <ul className="boundary-list">
                {state?.analysisBoundaries.map((item) => <li key={item}>{item}</li>)}
              </ul>
            </div>
          </aside>
        </div>
      </div>
    </div>
  );
}

function reviewPath({
  accountId,
  appId,
  rating
}: {
  accountId: string;
  appId: string;
  rating: number | null;
}) {
  const params = new URLSearchParams({ accountId, appId });
  if (rating) params.set('rating', String(rating));
  return `/admin/store-reviews?${params.toString()}`;
}

function errorMessage(error: unknown): string {
  if (error instanceof AdminApiError) return error.message;
  if (error instanceof Error) return error.message;
  return '请求失败，请稍后重试';
}

function formatDate(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString('zh-CN', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit'
  });
}

function severityLabel(value: string): string {
  if (value === 'high') return '高优先级';
  if (value === 'medium') return '中优先级';
  if (value === 'low') return '低优先级';
  return '待判断';
}

function severityClass(value: string): string {
  if (value === 'high') return 'high';
  if (value === 'low') return 'low';
  return 'medium';
}

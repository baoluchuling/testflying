import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { StoreReviewsPage } from './StoreReviewsPage';
import type { StoreReviewActionResponse, StoreReviewsState } from '../app/apiClient';

const baseState: StoreReviewsState = {
  apps: [
    {
      accountId: 'account-ios',
      appId: 'app-ios',
      appName: 'lookrva',
      bundleIdentifier: 'com.example.lookrva',
      platform: 'iOS',
      accountName: '测试账号',
      iconColor: '#18181b',
      reviewCount: 2,
      selected: true
    },
    {
      accountId: 'account-google',
      appId: 'app-android',
      appName: 'Readink',
      bundleIdentifier: 'com.example.readink',
      platform: 'Android',
      accountName: 'RDK/NG',
      iconColor: '#2563eb',
      reviewCount: 1,
      selected: false
    }
  ],
  selectedAccountId: 'account-ios',
  selectedAppId: 'app-ios',
  rating: null,
  stats: { total: 3, low: 1, ios: 2, android: 1 },
  reviews: [
    {
      id: 'review-1',
      storeReviewId: 'store-review-1',
      rating: 5,
      title: '很好用',
      body: '阅读体验很好',
      authorName: 'tester',
      locale: 'zh-Hans',
      territory: 'CN',
      appVersion: '1.0',
      createdAt: '2026-06-30T10:00:00'
    }
  ],
  latestFetch: null,
  latestAnalysis: null,
  analysisIssues: [],
  analysisBoundaries: ['只分析本地已拉取评论']
};

describe('StoreReviewsPage', () => {
  beforeEach(() => {
    history.replaceState(null, '', '/admin/store-reviews');
    vi.spyOn(globalThis, 'fetch').mockImplementation(mockFetch);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('loads review state from the admin api', async () => {
    render(<StoreReviewsPage />);

    expect(await screen.findByText('lookrva 评论')).toBeTruthy();
    expect(screen.getByText('阅读体验很好')).toBeTruthy();
  });

  it('switches selected app by pushing state and reloading json only', async () => {
    const user = userEvent.setup();
    const pushState = vi.spyOn(history, 'pushState');

    render(<StoreReviewsPage />);
    await screen.findByText('lookrva 评论');
    await user.click(screen.getByRole('button', { name: /Readink/ }));

    expect(pushState).toHaveBeenCalledWith(
      { adminRoute: 'store-reviews', accountId: 'account-google', appId: 'app-android' },
      '',
      '/admin/store-reviews?accountId=account-google&appId=app-android'
    );
    expect(await screen.findByText('Readink 评论')).toBeTruthy();
    expect(fetch).toHaveBeenCalledWith(
      '/admin/api/store-reviews?accountId=account-google&appId=app-android',
      expect.objectContaining({ cache: 'no-store' })
    );
  });

  it('filters by rating without a document reload', async () => {
    const user = userEvent.setup();
    const pushState = vi.spyOn(history, 'pushState');

    render(<StoreReviewsPage />);
    await screen.findByText('lookrva 评论');
    await user.click(screen.getByRole('button', { name: '3 星' }));

    expect(pushState).toHaveBeenCalledWith(
      { adminRoute: 'store-reviews', rating: 3 },
      '',
      '/admin/store-reviews?accountId=account-ios&appId=app-ios&rating=3'
    );
    expect(await screen.findByText('3 星卡顿')).toBeTruthy();
  });

  it('fetches newest reviews and updates the current state', async () => {
    const user = userEvent.setup();

    render(<StoreReviewsPage />);
    await screen.findByText('lookrva 评论');
    await user.click(screen.getByRole('button', { name: '拉取最新评论' }));

    await waitFor(() => {
      expect(screen.getByText('最新评论已拉取：新增 2 条')).toBeTruthy();
    });
    expect(fetch).toHaveBeenCalledWith(
      '/admin/api/store-reviews/fetch',
      expect.objectContaining({
        method: 'POST',
        body: JSON.stringify({ accountId: 'account-ios', appId: 'app-ios' })
      })
    );
  });

  it('renders llm analysis as structured Chinese sections', async () => {
    const user = userEvent.setup();

    render(<StoreReviewsPage />);
    await screen.findByText('lookrva 评论');
    await user.click(screen.getByRole('button', { name: 'LLM 分析' }));

    expect(await screen.findByText('分析摘要')).toBeTruthy();
    expect(screen.getByText('评论集中反馈登录失败，需要优先排查。')).toBeTruthy();
    expect(screen.getByText('高优先级')).toBeTruthy();
    expect(screen.getByText('关注点')).toBeTruthy();
    expect(screen.getByText('证据')).toBeTruthy();
    expect(screen.getByText('建议')).toBeTruthy();
    expect(screen.getByText('登录后一直转圈')).toBeTruthy();
    expect(screen.getByText('优先复测最新版本登录链路。')).toBeTruthy();
    expect(screen.getByText('代表评论：store-review-1')).toBeTruthy();
  });
});

function mockFetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const url = String(input);
  if (url.startsWith('/admin/api/store-reviews/fetch')) {
    const action: StoreReviewActionResponse = {
      message: '最新评论已拉取',
      result: {
        id: 'fetch-run-1',
        status: 'succeeded',
        pageCount: 1,
        fetchedCount: 2,
        insertedCount: 2,
        duplicateCount: 0,
        stoppedReason: 'no_more_pages',
        finishedAt: '2026-06-30T10:01:00',
        errorSummary: ''
      },
      state: {
        ...baseState,
        latestFetch: {
          id: 'fetch-run-1',
          status: 'succeeded',
          pageCount: 1,
          fetchedCount: 2,
          insertedCount: 2,
          duplicateCount: 0,
          stoppedReason: 'no_more_pages',
          finishedAt: '2026-06-30T10:01:00',
          errorSummary: ''
        }
      }
    };
    return jsonResponse(action);
  }

  if (url.startsWith('/admin/api/store-reviews/analyze')) {
    const action: StoreReviewActionResponse = {
      message: '评论分析已完成',
      result: {
        id: 'analysis-run-1',
        status: 'succeeded',
        reviewCount: 2,
        lowRatingCount: 1,
        issueCount: 1,
        summary: '评论集中反馈登录失败，需要优先排查。',
        finishedAt: '2026-06-30T10:02:00',
        errorSummary: ''
      },
      state: {
        ...baseState,
        latestAnalysis: {
          id: 'analysis-run-1',
          status: 'succeeded',
          reviewCount: 2,
          lowRatingCount: 1,
          issueCount: 1,
          summary: '评论集中反馈登录失败，需要优先排查。',
          finishedAt: '2026-06-30T10:02:00',
          errorSummary: ''
        },
        analysisIssues: [
          {
            title: '登录失败影响进入',
            severity: 'high',
            count: 1,
            focus: '需要确认登录服务是否有回归。',
            evidence: ['登录后一直转圈'],
            suggestion: '优先复测最新版本登录链路。',
            representativeReviewIds: ['store-review-1']
          }
        ]
      }
    };
    return jsonResponse(action);
  }

  if (url.startsWith('/admin/api/store-reviews')) {
    const state = stateForUrl(url);
    return jsonResponse(state);
  }

  return Promise.reject(new Error(`unexpected fetch ${url} ${init?.method ?? 'GET'}`));
}

function stateForUrl(url: string): StoreReviewsState {
  const parsed = new URL(url, 'http://localhost');
  if (parsed.searchParams.get('appId') === 'app-android') {
    return {
      ...baseState,
      apps: baseState.apps.map((app) => ({
        ...app,
        selected: app.appId === 'app-android'
      })),
      selectedAccountId: 'account-google',
      selectedAppId: 'app-android',
      reviews: [
        {
          id: 'review-android',
          storeReviewId: 'store-review-android',
          rating: 2,
          title: '闪退',
          body: '打开后闪退',
          authorName: 'android-user',
          locale: 'zh-Hans',
          territory: 'CN',
          appVersion: '2.0',
          createdAt: '2026-06-30T11:00:00'
        }
      ]
    };
  }
  if (parsed.searchParams.get('rating') === '3') {
    return {
      ...baseState,
      rating: 3,
      reviews: [
        {
          id: 'review-rating',
          storeReviewId: 'store-review-rating',
          rating: 3,
          title: '3 星卡顿',
          body: '列表滚动有卡顿',
          authorName: 'ios-user',
          locale: 'zh-Hans',
          territory: 'CN',
          appVersion: '1.0',
          createdAt: '2026-06-30T12:00:00'
        }
      ]
    };
  }
  return baseState;
}

function jsonResponse(payload: unknown): Promise<Response> {
  return Promise.resolve(
    new Response(JSON.stringify(payload), {
      status: 200,
      headers: { 'Content-Type': 'application/json' }
    })
  );
}

import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { LlmConfigPage } from './LlmConfigPage';
import type { LlmConfigState, LlmProfileItem } from '../app/apiClient';

const baseState: LlmConfigState = {
  protocols: [
    {
      key: 'openai_compatible',
      label: 'OpenAI 兼容',
      defaultBaseUrl: 'https://api.openai.com/v1',
      defaultModel: 'gpt-4o-mini',
      defaultAuthHeader: 'authorization_bearer'
    },
    {
      key: 'claude_compatible',
      label: 'Claude 兼容',
      defaultBaseUrl: 'https://api.anthropic.com',
      defaultModel: 'claude-3-5-haiku-latest',
      defaultAuthHeader: 'x-api-key'
    }
  ],
  presets: [
    {
      key: 'minimax_openai',
      label: 'MiniMax（OpenAI 兼容）',
      protocol: 'openai_compatible',
      baseUrl: 'https://api.minimax.chat/v1',
      model: 'MiniMax-M2',
      authHeader: 'api-key'
    }
  ],
  profiles: [],
  featureBindings: [
    {
      featureKey: 'review_analysis',
      featureLabel: '评论分析',
      description: '商店评论增量拉取后的问题归纳。',
      primaryProfileId: null,
      fallbackProfileId: null,
      effectiveProfileLabel: '未选择模型',
      status: 'unbound',
      statusLabel: '未绑定'
    }
  ]
};

describe('LlmConfigPage', () => {
  beforeEach(() => {
    vi.spyOn(globalThis, 'fetch').mockImplementation(mockFetch);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('saves MiniMax as an OpenAI-compatible profile preset', async () => {
    const user = userEvent.setup();

    render(<LlmConfigPage />);
    await user.click(await screen.findByRole('button', { name: 'MiniMax（OpenAI 兼容）' }));
    await user.type(screen.getByLabelText('API Key'), 'secret-key');
    await user.click(screen.getByRole('button', { name: '保存模型' }));

    expect(fetch).toHaveBeenCalledWith(
      '/admin/api/llm-config/profiles',
      expect.objectContaining({
        method: 'POST',
        body: JSON.stringify({
          name: 'MiniMax（OpenAI 兼容）',
          protocol: 'openai_compatible',
          baseUrl: 'https://api.minimax.chat/v1',
          model: 'MiniMax-M2',
          apiKey: 'secret-key',
          authHeader: 'api-key'
        })
      })
    );
    expect(await screen.findByText('LLM 模型已保存')).toBeTruthy();
  });
});

function mockFetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const url = String(input);
  if (url === '/admin/api/llm-config' && !init?.method) {
    return jsonResponse(baseState);
  }
  if (url === '/admin/api/llm-config/profiles' && init?.method === 'POST') {
    const profile: LlmProfileItem = {
      id: 'llm-minimax',
      name: 'MiniMax（OpenAI 兼容）',
      protocol: 'openai_compatible',
      protocolLabel: 'OpenAI 兼容',
      baseUrl: 'https://api.minimax.chat/v1',
      model: 'MiniMax-M2',
      authHeader: 'api-key',
      authHeaderLabel: 'api-key',
      apiKeySet: true,
      apiKeyPreview: 'secr...-key',
      status: 'configured',
      statusLabel: '已配置',
      updatedAtLabel: '刚刚'
    };
    return jsonResponse({
      message: 'LLM 模型已保存',
      profile,
      state: { ...baseState, profiles: [profile] }
    });
  }
  return Promise.reject(new Error(`unexpected fetch ${url}`));
}

function jsonResponse(payload: unknown): Promise<Response> {
  return Promise.resolve(
    new Response(JSON.stringify(payload), {
      status: 200,
      headers: { 'Content-Type': 'application/json' }
    })
  );
}

import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DeveloperAccountsPage } from './DeveloperAccountsPage';
import type {
  DeveloperAccountDetailState,
  MarketingPageActionResponse,
  MarketingPageDetailState,
  StoreWorkspaceActionResponse,
  StoreWorkspaceState
} from '../app/apiClient';

describe('DeveloperAccountsPage store workspace', () => {
  beforeEach(() => {
    vi.spyOn(globalThis, 'fetch').mockImplementation(mockFetch);
    holdSyncResponse = false;
    releaseSyncResponse = null;
    workspaceResponseState = workspaceState;
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('shows editable inputs after expanding store page languages', async () => {
    const user = userEvent.setup();
    history.replaceState(null, '', '/admin/accounts/account-ios/apps/app-ios/store');

    render(<DeveloperAccountsPage />);

    await screen.findByText(/lookrva · com\.example\.lookrva/);
    await screen.findAllByText(/当前商店最新版本（含未发布）：1\.0/);
    await user.click(screen.getByRole('button', { name: '展开描述多语言' }));
    const hantDescription = await screen.findByRole('textbox', { name: 'zh-Hant 描述' });
    await user.clear(hantDescription);
    await user.type(hantDescription, '繁體描述');
    await user.click(screen.getByRole('button', { name: '保存草稿' }));

    await waitFor(() => {
      expect(lastJsonPayload('/workspace/metadata')?.locales).toEqual(
        expect.arrayContaining([
          expect.objectContaining({ locale: 'zh-Hant', description: '繁體描述' })
        ])
      );
    });
  });

  it('shows editable inputs after expanding marketing page languages', async () => {
    const user = userEvent.setup();
    history.replaceState(
      null,
      '',
      '/admin/accounts/account-ios/apps/app-ios/marketing-pages/page-1'
    );

    render(<DeveloperAccountsPage />);

    await screen.findByDisplayValue('测试自定义产品页');
    await user.click(screen.getByRole('button', { name: '展开宣传文本多语言' }));
    const frenchText = await screen.findByRole('textbox', { name: 'fr-FR 宣传文本' });
    await user.clear(frenchText);
    await user.type(frenchText, 'Texte promotionnel');
    await user.click(screen.getByRole('button', { name: '保存草稿' }));

    await waitFor(() => {
      expect(lastJsonPayload('/marketing-pages/page-1')?.locales).toEqual(
        expect.arrayContaining([
          expect.objectContaining({ locale: 'fr-FR', promotionalText: 'Texte promotionnel' })
        ])
      );
    });
  });

  it('translates a store page field from source text into other locale inputs', async () => {
    const user = userEvent.setup();
    history.replaceState(null, '', '/admin/accounts/account-ios/apps/app-ios/store');

    render(<DeveloperAccountsPage />);

    await screen.findByText(/lookrva · com\.example\.lookrva/);
    await screen.findAllByText(/当前商店最新版本（含未发布）：1\.0/);
    await user.click(await screen.findByRole('button', { name: '翻译描述到其他语言' }));
    await user.click(screen.getByRole('button', { name: '展开描述多语言' }));

    const hantDescription = (await screen.findByRole('textbox', {
      name: 'zh-Hant 描述'
    })) as HTMLTextAreaElement;
    const frenchDescription = (await screen.findByRole('textbox', {
      name: 'fr-FR 描述'
    })) as HTMLTextAreaElement;
    await waitFor(() => {
      expect(hantDescription.value).toBe('繁體描述');
      expect(frenchDescription.value).toBe('Description française');
    });

    await user.click(screen.getByRole('button', { name: '保存草稿' }));

    await waitFor(() => {
      expect(lastJsonPayload('/workspace/metadata')?.locales).toEqual(
        expect.arrayContaining([
          expect.objectContaining({ locale: 'zh-Hant', description: '繁體描述' }),
          expect.objectContaining({ locale: 'fr-FR', description: 'Description française' })
        ])
      );
    });
  });

  it('translates marketing page text from source text into other locale inputs', async () => {
    const user = userEvent.setup();
    history.replaceState(
      null,
      '',
      '/admin/accounts/account-ios/apps/app-ios/marketing-pages/page-1'
    );

    render(<DeveloperAccountsPage />);

    await screen.findByDisplayValue('测试自定义产品页');
    await user.click(screen.getByRole('button', { name: '翻译宣传文本到其他语言' }));
    await user.click(screen.getByRole('button', { name: '展开宣传文本多语言' }));

    const frenchText = (await screen.findByRole('textbox', {
      name: 'fr-FR 宣传文本'
    })) as HTMLTextAreaElement;
    await waitFor(() => {
      expect(frenchText.value).toBe('Texte promotionnel');
    });
  });

  it('shows a friendly confirmation dialog and loading state before syncing the default store page', async () => {
    const user = userEvent.setup();
    holdSyncResponse = true;
    history.replaceState(null, '', '/admin/accounts/account-ios/apps/app-ios/store');

    render(<DeveloperAccountsPage />);

    await screen.findAllByText(/当前商店最新版本（含未发布）：1\.0/);
    await user.click(await screen.findByRole('button', { name: '同步到商店' }));

    const dialog = screen.getByRole('dialog', { name: '同步商店页' });
    expect(dialog).not.toBeNull();
    const releaseNotesCheckbox = within(dialog).getByRole('checkbox', {
      name: /版本说明/
    }) as HTMLInputElement;
    const storeImagesCheckbox = within(dialog).getByRole('checkbox', {
      name: /商店图/
    }) as HTMLInputElement;
    expect(releaseNotesCheckbox.checked).toBe(true);
    expect(storeImagesCheckbox.checked).toBe(true);
    expect(within(dialog).getByText('en-US、zh-Hant、fr-FR')).not.toBeNull();

    await user.click(storeImagesCheckbox);
    await user.click(screen.getByRole('button', { name: '确认同步' }));

    expect(lastJsonPayload('/workspace/metadata/sync')?.syncScopes).toEqual([
      'release_notes',
      'metadata'
    ]);
    expect(screen.getAllByRole('button', { name: '同步中...' }).length).toBeGreaterThan(0);

    releaseSyncResponse?.();
    await waitFor(() => {
      expect(screen.queryByRole('dialog', { name: '同步商店页' })).toBeNull();
    });
  });

  it('previews uploaded store images in separate phone and tablet groups', async () => {
    const user = userEvent.setup();
    history.replaceState(null, '', '/admin/accounts/account-ios/apps/app-ios/store');

    render(<DeveloperAccountsPage />);

    await screen.findByText(/lookrva · com\.example\.lookrva/);
    await user.upload(
      await screen.findByLabelText('en-US 手机截图上传'),
      new File(['phone'], 'phone.png', { type: 'image/png' })
    );

    await waitFor(() => {
      expect(Array.from(lastFormData('/workspace/metadata/store-images')?.keys() ?? [])).toContain(
        'storeImageFiles__phone_screenshots__en-US'
      );
    });

    const phoneGroup = await screen.findByRole('group', { name: 'en-US 手机截图' });
    const tabletGroup = await screen.findByRole('group', { name: 'en-US 平板截图' });
    expect(
      within(phoneGroup).getByAltText('en-US 手机截图').getAttribute('src')
    ).toBe('/admin/artifacts/store-images/phone.png');
    expect(within(phoneGroup).queryByAltText('en-US 平板截图')).toBeNull();
    expect(
      within(tabletGroup).getByAltText('en-US 平板截图').getAttribute('src')
    ).toBe('/admin/artifacts/store-images/tablet.png');
    expect(within(tabletGroup).queryByAltText('en-US 手机截图')).toBeNull();
  });

  it('does not show delete action for inherited store images', async () => {
    const user = userEvent.setup();
    workspaceResponseState = inheritedImageWorkspaceState;
    history.replaceState(null, '', '/admin/accounts/account-ios/apps/app-ios/store');

    render(<DeveloperAccountsPage />);

    await screen.findByText(/lookrva · com\.example\.lookrva/);
    await user.click(await screen.findByRole('button', { name: '展开商店图多语言' }));

    const sourceGroups = await screen.findAllByRole('group', { name: 'en-US 手机截图' });
    const inheritedGroup = await screen.findByRole('group', { name: 'zh-Hant 手机截图' });
    expect(
      sourceGroups.some((group) => within(group).queryByRole('button', { name: '删除' }))
    ).toBe(true);
    expect(within(inheritedGroup).getByAltText('zh-Hant 手机截图').getAttribute('src')).toBe(
      '/admin/artifacts/store-images/phone.png'
    );
    expect(within(inheritedGroup).queryByRole('button', { name: '删除' })).toBeNull();
  });
});

let holdSyncResponse = false;
let releaseSyncResponse: (() => void) | null = null;
let workspaceResponseState: StoreWorkspaceState;

const accountDetail: DeveloperAccountDetailState = {
  account: {
    id: 'account-ios',
    teamName: '测试账号',
    status: 'ok',
    statusLabel: '正常',
    expiresAt: '',
    expiresAtLabel: '未设置',
    remainingDays: 0,
    appNames: ['lookrva'],
    connectorName: 'Mac Connector',
    connectorStatus: 'ok',
    connectorStatusLabel: '正常',
    latestSyncStatus: 'ok',
    latestSyncAtLabel: '刚刚',
    detailPath: '/admin/accounts/account-ios'
  },
  connector: {
    name: 'Mac Connector',
    baseUrl: 'http://127.0.0.1:18080',
    authToken: '',
    status: 'ok',
    statusLabel: '正常',
    checkedAtLabel: '刚刚'
  },
  accountStorePlatform: 'ios',
  apps: [
    {
      id: 'app-ios',
      name: 'lookrva',
      bundleIdentifier: 'com.example.lookrva',
      platform: 'ios',
      platformLabel: 'iOS',
      iconColor: '#18181b',
      iconText: 'LO',
      storeAppId: '1234567890',
      storePackageName: '',
      latestVersionLabel: '1.0',
      storePath: '/admin/accounts/account-ios/apps/app-ios/store',
      marketingPath: '/admin/accounts/account-ios/apps/app-ios/marketing',
      releaseNotesPath: '/admin/accounts/account-ios/apps/app-ios/release-notes',
      connectionPath: '/admin/accounts/account-ios/apps/app-ios/connection'
    }
  ],
  unassignedApps: [],
  syncRuns: []
};

const workspaceState: StoreWorkspaceState = {
  account: accountDetail.account,
  app: accountDetail.apps[0],
  section: 'store',
  version: '1.0',
  locale: 'en-US',
  sourceLocale: 'en-US',
  supportedLocales: ['en-US', 'zh-Hant', 'fr-FR'],
  localizedMetadata: [
    {
      locale: 'en-US',
      isSource: true,
      keywords: '',
      promotionalText: 'Source promo',
      description: 'Source description',
      releaseNotes: 'Fix bugs',
      storeImages: {}
    },
    {
      locale: 'zh-Hant',
      isSource: false,
      keywords: '',
      promotionalText: '',
      description: '',
      releaseNotes: '',
      storeImages: {}
    },
    {
      locale: 'fr-FR',
      isSource: false,
      keywords: '',
      promotionalText: '',
      description: '',
      releaseNotes: '',
      storeImages: {}
    }
  ],
  connector: accountDetail.connector,
  preflightStatus: 'idle',
  preflightLabel: '等待检查',
  syncRuns: [],
  marketingPages: [
    {
      id: 'page-1',
      pageId: 'page-1',
      pageName: '测试自定义产品页',
      pageType: 'custom_product_page',
      typeLabel: '自定义产品页',
      status: 'draft',
      statusLabel: '草稿',
      applePageIdLabel: '暂无',
      deepLinkUrl: '',
      languageCount: 3,
      filledTextCount: 1,
      assetCount: 0,
      detailPath: '/admin/accounts/account-ios/apps/app-ios/marketing-pages/page-1'
    }
  ]
};

const uploadedImageWorkspaceState: StoreWorkspaceState = {
  ...workspaceState,
  localizedMetadata: workspaceState.localizedMetadata.map((locale) =>
    locale.locale === 'en-US'
      ? {
          ...locale,
          storeImages: {
            phone_screenshots: {
              preview_items: [
                {
                  url: '/admin/artifacts/store-images/phone.png',
                  fileName: 'phone.png',
                  storageKey: 'store-images/phone.png'
                }
              ]
            },
            tablet_screenshots: {
              preview_items: [
                {
                  url: '/admin/artifacts/store-images/tablet.png',
                  fileName: 'tablet.png',
                  storageKey: 'store-images/tablet.png'
                }
              ]
            }
          }
        }
      : locale
  )
};

const inheritedImageWorkspaceState: StoreWorkspaceState = {
  ...workspaceState,
  localizedMetadata: workspaceState.localizedMetadata.map((locale) => {
    if (locale.locale === 'en-US') {
      return {
        ...locale,
        storeImages: {
          phone_screenshots: {
            preview_items: [
              {
                url: '/admin/artifacts/store-images/phone.png',
                fileName: 'phone.png',
                storageKey: 'store-images/phone.png',
                inherited: false,
                canDelete: true
              }
            ]
          }
        }
      };
    }
    if (locale.locale === 'zh-Hant') {
      return {
        ...locale,
        storeImages: {
          phone_screenshots: {
            assets: [],
            preview_items: [
              {
                url: '/admin/artifacts/store-images/phone.png',
                fileName: 'phone.png',
                storageKey: 'store-images/phone.png',
                inherited: true,
                sourceLocale: 'en-US',
                canDelete: false
              }
            ]
          }
        }
      };
    }
    return locale;
  })
};

const marketingPageState: MarketingPageDetailState = {
  account: accountDetail.account,
  app: accountDetail.apps[0],
  page: workspaceState.marketingPages[0],
  section: 'marketing',
  locale: 'en-US',
  sourceLocale: 'en-US',
  supportedLocales: ['en-US', 'zh-Hant', 'fr-FR'],
  localizedPage: [
    { locale: 'en-US', isSource: true, promotionalText: 'Source promo', storeImages: {} },
    { locale: 'zh-Hant', isSource: false, promotionalText: '', storeImages: {} },
    { locale: 'fr-FR', isSource: false, promotionalText: '', storeImages: {} }
  ],
  connector: accountDetail.connector,
  preflightStatus: 'idle',
  preflightLabel: '等待检查',
  syncRuns: []
};

function mockFetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const url = String(input);
  if (url === '/admin/api/developer-accounts/account-ios') {
    return jsonResponse(accountDetail);
  }
  if (
    url === '/admin/api/developer-accounts/account-ios/apps/app-ios/workspace' ||
    url.startsWith('/admin/api/developer-accounts/account-ios/apps/app-ios/workspace?')
  ) {
    const section = new URL(url, 'http://localhost').searchParams.get('section') || 'store';
    return jsonResponse({ ...workspaceResponseState, section });
  }
  if (url.endsWith('/workspace/metadata')) {
    const response: StoreWorkspaceActionResponse = {
      message: '已保存',
      state: workspaceState,
      syncRuns: []
    };
    return jsonResponse(response);
  }
  if (url.endsWith('/workspace/metadata/store-images') && init?.method === 'POST') {
    const response: StoreWorkspaceActionResponse = {
      message: '已上传',
      state: uploadedImageWorkspaceState,
      syncRuns: []
    };
    return jsonResponse(response);
  }
  if (url.endsWith('/workspace/release-notes')) {
    const response: StoreWorkspaceActionResponse = {
      message: '已保存',
      state: workspaceState,
      syncRuns: []
    };
    return jsonResponse(response);
  }
  if (url.endsWith('/workspace/metadata/sync')) {
    const response: StoreWorkspaceActionResponse = {
      message: '已同步',
      state: workspaceState,
      syncRuns: []
    };
    if (holdSyncResponse) {
      return new Promise((resolve) => {
        releaseSyncResponse = () => resolve(makeJsonResponse(response));
      });
    }
    return jsonResponse(response);
  }
  if (url === '/admin/api/store-translation') {
    const payload = init?.body ? JSON.parse(String(init.body)) : {};
    if (payload.field === 'description') {
      return jsonResponse({
        translations: {
          'zh-Hant': '繁體描述',
          'fr-FR': 'Description française'
        }
      });
    }
    if (payload.field === 'promotionalText') {
      return jsonResponse({
        translations: {
          'zh-Hant': '繁體宣傳文本',
          'fr-FR': 'Texte promotionnel'
        }
      });
    }
    return jsonResponse({ translations: {} });
  }
  if (url.endsWith('/workspace/marketing-pages/page-1') && init?.method === 'PUT') {
    const response: MarketingPageActionResponse = {
      message: '已保存',
      state: marketingPageState,
      workspace: null,
      syncRuns: []
    };
    return jsonResponse(response);
  }
  if (url.endsWith('/workspace/marketing-pages/page-1')) {
    return jsonResponse(marketingPageState);
  }
  return Promise.reject(new Error(`unexpected fetch ${url}`));
}

function lastJsonPayload(urlPart: string): Record<string, unknown> | null {
  const calls = vi.mocked(fetch).mock.calls;
  for (const [url, init] of calls.slice().reverse()) {
    if (!String(url).includes(urlPart) || typeof init?.body !== 'string') continue;
    return JSON.parse(init.body) as Record<string, unknown>;
  }
  return null;
}

function lastFormData(urlPart: string): FormData | null {
  const calls = vi.mocked(fetch).mock.calls;
  for (const [url, init] of calls.slice().reverse()) {
    if (!String(url).includes(urlPart) || !(init?.body instanceof FormData)) continue;
    return init.body;
  }
  return null;
}

function jsonResponse(payload: unknown): Promise<Response> {
  return Promise.resolve(makeJsonResponse(payload));
}

function makeJsonResponse(payload: unknown): Response {
  return new Response(JSON.stringify(payload), {
    status: 200,
    headers: { 'Content-Type': 'application/json' }
  });
}

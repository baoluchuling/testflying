import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { UploadPage } from './UploadPage';
import type { AdminUploadResponse, UploadState } from '../app/apiClient';

vi.mock('../app/apiClient', async () => {
  const actual = await vi.importActual<typeof import('../app/apiClient')>('../app/apiClient');
  return {
    ...actual,
    loadUploadState: vi.fn(() => Promise.resolve(uploadState)),
    uploadPackage: vi.fn((_formData: FormData, onProgress: (percent: number) => void) => {
      onProgress(42);
      onProgress(100);
      return Promise.resolve(uploadResponse);
    })
  };
});

const uploadState: UploadState = {
  accounts: [
    {
      id: 'account-ios',
      teamName: '测试账号',
      status: 'ok',
      platform: null
    }
  ]
};

const uploadResponse: AdminUploadResponse = {
  message: '上传成功，包信息已自动解析',
  state: uploadState,
  result: {
    appId: 'app-android-autoparse',
    appName: 'Auto Parsed',
    bundleIdentifier: 'com.example.autoparse',
    platform: 'android',
    environment: 'development',
    version: '4.5.6',
    buildNumber: '321',
    developerAccount: '未绑定账号',
    storeIdentifier: 'com.example.autoparse',
    installUrl: 'https://dist.example.test/artifacts/app.apk',
    manifestUrl: null,
    downloadUrl: 'https://dist.example.test/artifacts/app.apk'
  }
};

describe('UploadPage', () => {
  afterEach(() => {
    vi.clearAllMocks();
  });

  it('loads developer account options', async () => {
    render(<UploadPage />);

    expect(await screen.findByRole('option', { name: '测试账号' })).toBeTruthy();
    expect(screen.getByText('上传成功后会展示应用名称、包名、版本号、构建号和安装地址。')).toBeTruthy();
  });

  it('shows progress and parsed package result after upload', async () => {
    const user = userEvent.setup();
    render(<UploadPage />);

    await screen.findByRole('option', { name: '测试账号' });
    await user.selectOptions(screen.getByLabelText('平台'), 'android');
    await user.upload(
      screen.getByLabelText('选择安装包'),
      new File(['package'], 'app.apk', { type: 'application/vnd.android.package-archive' })
    );
    expect(screen.getByText('app.apk')).toBeTruthy();
    await user.click(screen.getByRole('button', { name: '开始上传' }));

    expect(await screen.findByText('上传成功，包信息已自动解析')).toBeTruthy();
    expect(screen.getByText('Auto Parsed')).toBeTruthy();
    expect(screen.getAllByText('com.example.autoparse')).toHaveLength(2);
    expect(screen.getByText('4.5.6 (321)')).toBeTruthy();
  });
});

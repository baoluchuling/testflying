import { FormEvent, useEffect, useState } from 'react';
import {
  AdminApiError,
  loadUploadState,
  uploadPackage,
  type UploadResult,
  type UploadState
} from '../app/apiClient';

type UploadStatus = 'idle' | 'uploading' | 'done' | 'error';

type UploadView = {
  status: UploadStatus;
  progress: number;
  fileName: string;
  notice: string;
  error: string;
  result: UploadResult | null;
};

const defaultUploadView: UploadView = {
  status: 'idle',
  progress: 0,
  fileName: '',
  notice: '',
  error: '',
  result: null
};

let uploadViewSnapshot: UploadView = defaultUploadView;
const uploadListeners = new Set<(view: UploadView) => void>();

export function UploadPage() {
  const [state, setState] = useState<UploadState | null>(null);
  const [view, setView] = useState<UploadView>(uploadViewSnapshot);
  const [platform, setPlatform] = useState('ios');

  useEffect(() => {
    const listener = (nextView: UploadView) => setView(nextView);
    uploadListeners.add(listener);
    setView(uploadViewSnapshot);
    return () => {
      uploadListeners.delete(listener);
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    loadUploadState()
      .then((payload) => {
        if (!cancelled) setState(payload);
      })
      .catch((error: Error) => {
        if (!cancelled) {
          updateUploadView({ status: 'error', error: error.message });
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  async function submitUpload(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (view.status === 'uploading') return;
    const form = event.currentTarget;
    const formData = new FormData(form);
    const fileInput = form.elements.namedItem('file');
    const selectedFile = fileInput instanceof HTMLInputElement ? fileInput.files?.[0] : null;
    const file = formData.get('file');
    const fileName = file instanceof File ? file.name : '';
    if (!fileName && selectedFile) {
      formData.set('file', selectedFile);
    }
    const selectedFileName = selectedFile?.name || fileName;
    if (!selectedFileName) {
      updateUploadView({ status: 'error', error: '请选择 IPA 或 APK 安装包', notice: '' });
      return;
    }
    updateUploadView({
      status: 'uploading',
      progress: 0,
      fileName: selectedFileName,
      notice: '正在上传',
      error: '',
      result: null
    });
    try {
      const response = await uploadPackage(formData, (progress) => {
        updateUploadView({ progress, notice: progress >= 100 ? '正在解析包信息' : '正在上传' });
      });
      setState(response.state);
      updateUploadView({
        status: 'done',
        progress: 100,
        notice: response.message,
        error: '',
        result: response.result
      });
    } catch (requestError) {
      updateUploadView({
        status: 'error',
        error: errorMessage(requestError),
        notice: '',
        progress: uploadViewSnapshot.progress
      });
    }
  }

  return (
    <div className="compact-page compact-upload-page upload-page" data-upload-page>
      <div className="compact-context">
        <div className="compact-title">
          <strong>Package Intake</strong>
          <h2>上传</h2>
          <span>自动解析包信息，支持 iOS / Android，不会提交到商店。</span>
        </div>
        <div className="compact-actions">
          <span className="tag">
            {view.status === 'uploading' ? '上传中' : view.status === 'done' ? '已完成' : '等待选择'}
          </span>
        </div>
      </div>

      <div className="compact-body">
        <div className="compact-upload-grid">
          <form className="panel upload-panel" noValidate onSubmit={(event) => void submitUpload(event)}>
            <div className="panel-head compact">
              <strong>上传 IPA / APK</strong>
              <div className="inline-actions">
                <button className="button" type="reset" disabled={view.status === 'uploading'}>
                  清空
                </button>
                <button className="button primary" type="submit" disabled={view.status === 'uploading'}>
                  {view.status === 'uploading' ? `${view.progress}%` : '开始上传'}
                </button>
              </div>
            </div>

            <label className="upload-drop-zone">
              <input
                aria-label="选择安装包"
                name="file"
                type="file"
                accept=".ipa,.apk,application/vnd.android.package-archive,application/octet-stream"
                required
                disabled={view.status === 'uploading'}
                onChange={(event) => {
                  updateUploadView({ fileName: event.currentTarget.files?.[0]?.name ?? '' });
                }}
              />
              <span>选择安装包</span>
              <strong>{view.fileName || '还没有选择文件'}</strong>
            </label>

            <details className="upload-config">
              <summary>上传配置</summary>
              <div className="form-grid two">
                <label>
                  <span>平台</span>
                  <select
                    name="platform"
                    value={platform}
                    onChange={(event) => setPlatform(event.currentTarget.value)}
                    disabled={view.status === 'uploading'}
                  >
                    <option value="ios">iOS</option>
                    <option value="android">Android</option>
                  </select>
                </label>
                <label>
                  <span>环境</span>
                  <select name="environment" defaultValue="development" disabled={view.status === 'uploading'}>
                    <option value="development">开发环境</option>
                    <option value="production">线上环境</option>
                  </select>
                </label>
              </div>

              <div className="form-grid two">
                <label>
                  <span>开发者账号</span>
                  <select name="developerAccountId" disabled={view.status === 'uploading'}>
                    <option value="">不绑定账号</option>
                    {state?.accounts.map((account) => (
                      <option key={account.id} value={account.id}>
                        {account.teamName}
                      </option>
                    ))}
                  </select>
                </label>
                {platform === 'ios' ? (
                  <label>
                    <span>App Store Connect App ID</span>
                    <input
                      name="storeAppId"
                      placeholder="可选，数字 ID"
                      disabled={view.status === 'uploading'}
                    />
                  </label>
                ) : (
                  <label>
                    <span>Google Play package name</span>
                    <input
                      name="storePackageName"
                      placeholder="默认使用 APK package"
                      disabled={view.status === 'uploading'}
                    />
                  </label>
                )}
              </div>

              <label>
                <span>更新说明</span>
                <textarea
                  name="changelog"
                  rows={4}
                  placeholder="本次构建变更、验证重点或风险说明"
                  disabled={view.status === 'uploading'}
                />
              </label>
            </details>

            {view.status === 'uploading' ? (
              <div className="upload-progress" aria-label="上传进度">
                <span style={{ width: `${view.progress}%` }} />
              </div>
            ) : null}
          </form>

          <aside className="upload-side">
            {view.error ? <div className="notice error">{view.error}</div> : null}
            {view.notice && !view.error ? <div className="notice ok">{view.notice}</div> : null}
            {view.result ? <UploadResultPanel result={view.result} /> : <UploadEmptyPanel />}
          </aside>
        </div>
      </div>
    </div>
  );
}

function UploadResultPanel({ result }: { result: UploadResult }) {
  return (
    <section className="panel upload-result-panel">
      <div className="panel-head compact">
        <strong>解析结果</strong>
        <span>{platformLabel(result.platform)}</span>
      </div>
      <dl className="detail-list">
        <Detail label="应用" value={result.appName} />
        <Detail label="包名" value={result.bundleIdentifier} />
        <Detail label="环境" value={environmentLabel(result.environment)} />
        <Detail label="版本" value={`${result.version} (${result.buildNumber})`} />
        <Detail label="开发者账号" value={result.developerAccount} />
        <Detail label="商店标识" value={result.storeIdentifier} />
      </dl>
      <div className="install-links">
        <Copyable label="installUrl" value={result.installUrl} />
        {result.manifestUrl ? <Copyable label="manifestUrl" value={result.manifestUrl} /> : null}
        {result.downloadUrl ? <Copyable label="downloadUrl" value={result.downloadUrl} /> : null}
      </div>
    </section>
  );
}

function UploadEmptyPanel() {
  return (
    <section className="panel upload-result-panel">
      <div className="panel-head compact">
        <strong>解析结果</strong>
        <span>等待上传</span>
      </div>
      <p className="muted">上传成功后会展示应用名称、包名、版本号、构建号和安装地址。</p>
    </section>
  );
}

function Detail({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <dt>{label}</dt>
      <dd>{value || '-'}</dd>
    </div>
  );
}

function Copyable({ label, value }: { label: string; value: string }) {
  return (
    <div className="copyable">
      <span>{label}</span>
      <code>{value}</code>
      <button className="button" type="button" onClick={() => void navigator.clipboard?.writeText(value)}>
        复制
      </button>
    </div>
  );
}

function updateUploadView(patch: Partial<UploadView>) {
  uploadViewSnapshot = { ...uploadViewSnapshot, ...patch };
  for (const listener of uploadListeners) {
    listener(uploadViewSnapshot);
  }
}

function platformLabel(platform: string) {
  if (platform === 'ios') return 'iOS';
  if (platform === 'android') return 'Android';
  return platform || '未知';
}

function environmentLabel(environment: string) {
  if (environment === 'production') return '线上环境';
  if (environment === 'development') return '开发环境';
  return environment || '未知环境';
}

function errorMessage(error: unknown): string {
  if (error instanceof AdminApiError) return error.message;
  if (error instanceof Error) return error.message;
  return '上传失败，请稍后重试';
}

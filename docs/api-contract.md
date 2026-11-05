# testflying-api 接口契约

本文档描述服务端第一版已经实现的 API。服务端只保存内部应用分发事实，不保存安装状态、下载进度、暂停状态、用户手动排序和通知已读状态。

## 请求上下文

常规客户端请求携带：

```http
Authorization: Bearer <token>
X-Device-ID: <registered-device-id>
X-Client-Platform: ios
```

当前版本预留 token 上下文，但还没有接入正式登录态。设备可见性由 `X-Device-ID` 和服务端设备登记事实决定。

## 健康检查

```http
GET /health
```

响应：

```json
{"status":"ok"}
```

## Workspace 快照

```http
GET /v1/test-distribution/workspace
```

响应顶层结构：

```json
{
  "apps": [],
  "builds": [],
  "devices": [],
  "developerAccounts": [],
  "notifications": [],
  "installTasks": [],
  "sortOrder": {
    "buildIds": []
  },
  "profile": {}
}
```

规则：

- `apps` 和 `builds` 只包含当前设备和平台可见的分发事实。
- `builds[].installInfo.installUrl` 是客户端点击安装时打开的地址。
- iOS 构建的 `installUrl` 使用 `itms-services://?action=download-manifest&url=<manifest>`。
- Android 构建的 `installUrl` 直接指向 APK 下载地址。
- `installTasks` 永远返回空数组。
- `sortOrder.buildIds` 永远返回空数组。
- 通知对象不包含 `isRead` 或 `readAt`。

## 上传包

```http
POST /v1/test-distribution/uploads
Content-Type: multipart/form-data
```

表单字段：

- `file`：IPA 或 APK。
- `platform`：`ios` 或 `android`。
- `environment`：`development` 或 `production`。
- `changelog`：可选更新说明。
- `appName`：可选，仅用于覆盖服务端解析出的应用名称。
- `developerAccountId`：可选，上传后把新 App 或未绑定 App 归属到指定开发者账号。
- `storeAppId`：可选，Apple App Store Connect App ID。
- `storePackageName`：可选，Google Play package 或备用商店包名。

iOS 上传会从 IPA 的 `Payload/*.app/Info.plist` 解析 bundle id、应用名、版本号和 build number。Android 上传会从 APK 的 binary manifest 和资源表解析 package name、应用名、version name 和 version code。

上传成功后服务端会：

- 根据 bundle id 或 package name 加平台 upsert 应用。
- 如果传入 `developerAccountId`，校验账号存在；如果 App 已绑定其他账号，则拒绝上传，避免静默改绑。
- 创建构建。
- 保存 IPA/APK 制品。
- iOS 额外生成 `manifest.plist`。
- 创建一条 `build` 类型通知。
- 给同平台已登记设备创建构建可见性。

`installUrl`、`manifestUrl` 和 `downloadUrl` 在上传时由当前部署的公开 URL 配置生成。S3/MinIO 部署依赖 `TESTFLYING_S3_PUBLIC_BASE_URL`，本地文件部署依赖 `TESTFLYING_PUBLIC_BASE_URL`。这些 URL 写入数据库后不会随环境变量自动变化。

上传不会创建安装任务，也不会记录某台设备是否安装过该构建。

## 导入商店版本草稿

```http
POST /v1/store-management/developer-accounts/{accountId}/apps/{appId}/store-versions/{version}/draft
Authorization: Bearer <TESTFLYING_STATIC_TOKEN>
Content-Type: application/json
```

这个接口给内部脚本、CI 或内容生成工具使用。它只把某个商店版本的文案元数据和版本说明保存到
testflying 草稿，不调用 connector，不做商店版本预检查，也不同步到 App Store Connect 或
Google Play Console。

请求体示例：

```json
{
  "sourceLocale": "en-US",
  "locales": [
    {
      "locale": "en-US",
      "keywords": "novel,reader,story",
      "promotionalText": "Read better stories every day.",
      "description": "Long store description.",
      "releaseNotes": "Fix bugs"
    },
    {
      "locale": "zh-Hant",
      "keywords": "",
      "promotionalText": "",
      "description": "",
      "releaseNotes": ""
    }
  ]
}
```

如果某个语言的 `keywords`、`promotionalText`、`description` 或 `releaseNotes` 为空，会使用
`sourceLocale` 对应内容填充。只传版本说明时可以不传文案元数据；如果传了文案元数据，
`description` 最终不能为空，否则返回 `422`。

成功响应：

```json
{
  "version": "1.0.0",
  "locales": ["en-US", "zh-Hant"],
  "savedMetadataDrafts": 2,
  "savedReleaseNoteDrafts": 2,
  "warnings": []
}
```

这个接口不会创建 `store_sync_runs` 记录。后续需要同步到商店时，可以从管理后台打开对应
App 的商店管理页面执行同步，也可以由第三方电脑调用直接同步接口：

```http
POST /v1/store-management/developer-accounts/{accountId}/apps/{appId}/sync-runs
POST /v1/store-management/developer-accounts/{accountId}/apps/{appId}/marketing-pages/{pageId}/sync-runs
GET /v1/store-management/developer-accounts/{accountId}/apps/{appId}/store-releases
GET /v1/store-management/developer-accounts/{accountId}/apps/{appId}/product-page-optimizations
POST /v1/store-management/developer-accounts/{accountId}/apps/{appId}/product-page-optimizations
```

直接同步接口只读取已经保存的草稿，中心后台会执行预检查、限流、幂等处理、同步记录创建和
Connector 调用。`store-releases` 用于通过中心后台读取 Google Play tracks / releases /
versionCodes，再把目标 `storeTrack` / `storeVersionCode` 传给同步接口。产品页面优化接口当前只支持 App Store Connect：`GET` 查询已有实验，`POST`
创建新的产品页面优化实验，并建议通过 `idempotencyKey` 避免第三方电脑重试时重复创建。

## 管理后台

管理后台是给内部管理员使用的 React 单页应用，静态资源由 FastAPI 在 `/admin` 下提供；
`/admin/<页面路径>` 由前端路由处理，不是对外稳定的 HTML 接口契约。它使用 HTTP Basic 认证：

- 用户名：`TESTFLYING_ADMIN_USERNAME`，默认 `admin`。
- 密码：复用 `TESTFLYING_STATIC_TOKEN`。

后台上传表单复用 `POST /v1/test-distribution/uploads` 的业务逻辑，上传成功后同样会创建应用、构建、制品、iOS manifest 和通知。后台页面使用浏览器上传进度事件展示上传百分比；服务端收到完整文件后再解析包信息和写入数据库。MinIO Console 只管理对象存储文件，不能替代管理后台上传，因为直接上传到 MinIO 不会写入业务数据库。

管理后台当前提供商店管理、构建、设备、App 日志、通知、系统设置和接口文档。商店管理从应用直接进入，开发者账号用于维护账号、App 绑定和 connector 配置；商店同步的字段、图片、多语言和隔离规则见 `store-sync.md`。构建任务、macOS Runner、自动更新、制品验收和钉钉通知见 `build-delivery.md`。

内部后台页面路径如下：

| 路径 | 用途 |
| --- | --- |
| `/admin/builds/apps` | 已接入应用和构建触发 |
| `/admin/builds/history` | 构建记录与制品 |
| `/admin/builds/runners` | Runner 预配和状态 |
| `/admin/settings/general` | Connector 默认模板 |
| `/admin/settings/notifications` | 钉钉配置和连接检查 |
| `/admin/settings/llm` | LLM 模型和功能绑定 |
| `/admin/settings/runtime` | 只读运行环境 |

通用设置和通知设置采用数据库优先、环境变量兜底。管理后台不会改写 `.env`；运行环境与通知读取接口
只返回脱敏状态，不返回数据库连接串、对象存储密钥、静态 token 或钉钉加签密钥原文。

`/admin/api/*` 是后台自身的内部数据接口，可能随后台交互调整；第三方系统应使用 `docs/store-management-api.md` 中列出的对外商店管理 API，而不是依赖后台页面路径或内部 API。

## 设备

```http
GET /v1/test-distribution/devices/current
GET /v1/test-distribution/devices
POST /v1/test-distribution/devices/registration-link
```

`devices/current` 在设备未登记时返回：

```json
{
  "code": "device_not_registered",
  "message": "当前设备未登记",
  "retryable": false
}
```

`registration-link` 只生成登记请求，不自动审批设备，也不授予构建可见性。

## 开发者账号续费

```http
GET /v1/test-distribution/developer-accounts
GET /v1/test-distribution/developer-accounts/renewals
```

账号响应包含：

- `expiresAt`
- `status`
- `appIds`
- `remainingDays`
- `renewalActionLabel`

账号提醒是服务端事实。客户端是否关闭提醒、如何展示横幅、滚动位置等都由客户端本地处理。

## 通知 Feed

```http
GET /v1/test-distribution/notifications
GET /v1/test-distribution/notifications?type=build
GET /v1/test-distribution/notifications?type=account
GET /v1/test-distribution/notifications?type=device
```

通知类型：

- `build`
- `account`
- `device`

服务端没有通知已读写接口，不实现：

- `PATCH /v1/test-distribution/notifications/{notificationId}`
- `POST /v1/test-distribution/notifications/mark-all-read`

## 已删除的用户态接口

服务端第一版不实现这些旧接口：

- `POST /v1/test-distribution/builds/{buildId}/install-tasks`
- `PATCH /v1/test-distribution/install-tasks/{taskId}`
- `GET /v1/test-distribution/install-tasks/{taskId}`
- `PUT /v1/test-distribution/users/me/build-sort-order`
- `PATCH /v1/test-distribution/notifications/{notificationId}`
- `POST /v1/test-distribution/notifications/mark-all-read`

这些能力属于客户端状态：安装中、暂停、下载进度、手动排序和通知已读由移动端本地保存。

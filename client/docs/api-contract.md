# testflying API 契约草案

本文档定义客户端从本地 mock service 迁移到真实后端时需要保持的接口边界。当前阶段 App 默认仍使用 `MockTestFlightService`；`RemoteTestFlightService`、DTO mapper、`ApiClient` 和真实 HTTP transport 已存在。远端模式需要通过 `dart-define` 显式开启，正式登录态、token 刷新和已校准的后端地址仍待接入。

## 设计原则

- 客户端只展示和操作测试分发数据，不在设备端手动创建应用。
- 新应用和新构建由包上传流水线创建：CI 或后台上传 IPA 后，后端生成应用、构建、通知和安装权限。
- 客户端刷新 workspace 后自动看到新应用；必要时通过通知提示新构建。
- 后端返回的数据结构应能直接映射到 `TestFlightWorkspace`。
- 排序、筛选、安装状态、暂停状态、下载进度和通知已读都由客户端本地维护。

## 认证和上下文

所有接口默认携带当前登录用户和当前设备上下文：

```http
Authorization: Bearer <access-token>
X-Device-ID: <registered-device-id>
X-Client-Platform: ios
```

客户端不在接口里传负责人或角色，权限由后端根据 token 判断。

当前 `ApiClient` 已统一补齐 `Authorization`、`X-Device-ID`、`X-Client-Platform`、`Accept` 和请求体的 `Content-Type`。`HttpApiTransport` 负责 JSON 编码、HTTP 请求和 JSON 响应解码。真实接入时只需要补 token/deviceId 来源、token 刷新和真实地址。

远端模式启动参数：

```bash
flutter run \
  --dart-define=TESTFLYING_SERVICE=remote \
  --dart-define=TESTFLYING_API_ENV=development \
  --dart-define=TESTFLYING_API_BASE_URL=https://dist.example.test \
  --dart-define=TESTFLYING_ACCESS_TOKEN=<access-token> \
  --dart-define=TESTFLYING_DEVICE_ID=<registered-device-id> \
  --dart-define=TESTFLYING_CLIENT_PLATFORM=ios
```

## Workspace 快照

首页、设备页、通知页、我的页优先从一个聚合快照启动，减少多端首屏拼装复杂度。

```http
GET /v1/test-distribution/workspace
```

响应字段映射：

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

客户端模型：

- `apps` -> `List<InternalApp>`
- `builds` -> `List<InternalBuild>`
- `devices` -> `List<TestDevice>`
- `developerAccounts` -> `List<DeveloperAccount>`
- `notifications` -> `List<AppNotification>`
- `installTasks` -> `List<InstallTask>`
- `sortOrder` -> `AppSortOrder`
- `profile` -> `UserProfile`

客户端转换路径：

```text
ApiTransport -> ApiClient -> RemoteWorkspaceDto -> TestFlightWorkspace -> Widget
```

Widget 不直接读取 HTTP DTO，也不直接依赖 JSON 字段名。

## 应用和构建

当前客户端只通过 workspace 聚合快照读取应用和构建。构建响应需要包含客户端安装入口：

```json
{
  "id": "aurora",
  "platform": "ios",
  "installUrl": "itms-services://?action=download-manifest&url=https%3A%2F%2Fdist.example.com%2Fmanifest.plist",
  "manifestUrl": "https://dist.example.com/manifest.plist",
  "downloadUrl": "https://dist.example.com/app.ipa",
  "minOsVersion": "iOS 16.0",
  "expiresAt": "2026-06-20T12:00:00Z",
  "isInstallable": true,
  "unavailableReason": null
}
```

平台规则：

- `platform=ios` 时，`installUrl` 优先使用 `itms-services://?action=download-manifest&url=<encoded manifest.plist>`。
- `platform=android` 时，`installUrl` 可直接指向 APK 下载地址或内部分发下载页。
- `manifestUrl` 只对 iOS 必填；Android 可以为空。
- `downloadUrl` 表示真实 IPA/APK 包地址，用于详情展示、审计或后端调试。
- `isInstallable=false` 时客户端禁用安装入口，并展示 `unavailableReason`。

应用和构建创建不由设备端完成。真实创建入口属于上传系统：

```http
POST /v1/test-distribution/uploads
```

上传完成后后端负责：

1. 识别 bundle id、版本号、build number、环境和签名信息。
2. 创建或更新 `InternalApp`。
3. 创建 `InternalBuild`。
4. 生成 `AppNotification`。
5. 根据权限和设备池计算可安装范围。

## 安装和手动排序

客户端点击安装时只打开 `build.installInfo.installUrl`，不创建服务端安装任务。

- iOS：`InstallLauncher` 打开 `itms-services://?action=download-manifest&url=<manifest.plist>`。
- Android：`InstallLauncher` 打开 APK 下载地址或下载页 URL。
- 安装中、暂停、继续、下载进度和安装任务列表都写入 `WorkspacePreferencesStore`。
- 手动排序只写入 `WorkspacePreferencesStore`，不向服务端提交。
- 刷新 workspace 时，客户端先读取服务端分发事实，再把本地安装状态和排序覆盖回 `TestFlightWorkspace`。

## 设备

```http
GET /v1/test-distribution/devices/current
GET /v1/test-distribution/devices
POST /v1/test-distribution/devices/registration-link
```

设备端可以请求登记链接，但不直接绕过审批加入设备池。设备状态由后端和审批系统更新。

## 开发者账号续费

```http
GET /v1/test-distribution/developer-accounts
GET /v1/test-distribution/developer-accounts/renewals
```

续费提醒不是全局横幅，由 workspace 中的 `developerAccounts` 驱动，跟随首页设备与证书区域展示。

## 通知

```http
GET /v1/test-distribution/notifications
```

通知类型必须和客户端 `NoticeType` 对齐：

- `build`
- `account`
- `device`

客户端本地保留 `all` 作为筛选项，不要求后端返回。

通知已读状态由客户端本地保存。服务端不提供单条已读或全部已读接口。

## 错误约定

通用错误响应：

```json
{
  "code": "device_not_registered",
  "message": "当前设备未登记",
  "retryable": false
}
```

建议错误码：

- `auth_expired`
- `device_not_registered`
- `build_not_found`
- `build_not_installable`
- `developer_account_expired`
- `rate_limited`

## 下一步实现边界

`RemoteTestFlightService` 已实现和 `MockTestFlightService` 相同的客户端 service 边界。页面只消费 `TestFlightWorkspace` 和服务方法，不直接读取 HTTP DTO。

剩余真实接入工作：

- 接入登录态、token 刷新和当前设备 ID 来源。
- 用 `TESTFLYING_SERVICE=remote` 将 App 入口从 `MockTestFlightService` 切换到 `RemoteTestFlightService`。
- 用真实接口样例校准 DTO 字段和错误码。

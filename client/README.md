# testflying

`testflying` 是一个 Flutter 原型项目，目标是探索“内部应用分发 / 类 TestFlight 安装列表”的体验。当前 UI 已扩展为内部测试工具工作台，覆盖首页、设备、通知、我的四个主要页面。

项目目前仍处在原型阶段。数据来源已经从页面硬编码迁移到 service 层，默认使用本地 mock service，客户端会本地保存安装状态和手动排序。安装入口已经抽成可替换 launcher，支持 iOS `itms-services://?...manifest.plist` 和 Android APK 链接。真实 HTTP transport 和远端 service 开关已经存在，但还没有接入正式登录态、token 刷新和已校准的后端地址。

## 当前状态

- 已生成 Android、iOS、Web、macOS、Linux、Windows 的 Flutter 多端工程结构。
- `lib/main.dart` 启动 Material 应用，并直接进入 `HomePage`。
- `lib/pages/home.dart` 承载底部四页导航和首页本地状态。
- `lib/pages/tab_pages.dart` 承载设备、通知、我的页以及共享 UI 组件。
- `lib/models/internal_build.dart` 定义构建渠道、构建状态、安装平台和构建条目。
- `lib/models/workspace_data.dart` 定义应用、设备、开发者账号、通知、安装任务、排序和个人资料模型。
- `lib/services/testflight_service.dart` 定义页面依赖的服务边界。
- `lib/services/install_launcher.dart` 定义 iOS/Android 外部安装入口。
- `lib/services/api_client.dart` 定义接口环境、请求上下文、真实 HTTP transport 和错误映射。
- `lib/services/testflight_service_factory.dart` 根据 `dart-define` 在 mock service 和 remote service 之间切换。
- `lib/services/remote_workspace_dto.dart` 将后端 JSON DTO 映射为 `TestFlightWorkspace`。
- `lib/services/remote_testflight_service.dart` 提供真实后端接入时的 service 骨架。
- `lib/services/mock_testflight_service.dart` 提供本地 mock 数据和安装/排序状态变更。
- `lib/services/workspace_preferences_store.dart` 通过 `shared_preferences` 保存排序、安装状态和安装任务。
- `docs/ui-baseline.md` 记录当前已通过的 UI 基线和交互验收点。
- `docs/api-contract.md` 记录后续接真实后端时的资源、接口和错误约定。
- `lib/pages/app_details.dart` 和 `lib/pages/build_details.dart` 已提供应用详情和构建详情入口。
- `pubspec.yaml` 已引入 `go_router`，但项目还没有真正接入路由。
- `test/widget_test.dart` 覆盖主要 UI 和交互，service 测试覆盖 mock 数据、本地状态、API client、remote DTO 和 remote service 请求。

## 目录结构

```text
apps/testflying/
├── lib/
│   ├── main.dart
│   ├── models/
│   │   ├── internal_build.dart
│   │   └── workspace_data.dart
│   ├── pages/
│       ├── home.dart
│       ├── tab_pages.dart
│       ├── app_details.dart
│       └── build_details.dart
│   └── services/
│       ├── testflight_service.dart
│       ├── install_launcher.dart
│       ├── api_client.dart
│       ├── testflight_service_factory.dart
│       ├── remote_workspace_dto.dart
│       ├── remote_testflight_service.dart
│       ├── mock_testflight_service.dart
│       └── workspace_preferences_store.dart
├── docs/
│   ├── api-contract.md
│   └── ui-baseline.md
├── test/
│   ├── widget_test.dart
│   ├── mock_testflight_service_test.dart
│   ├── api_client_test.dart
│   └── remote_testflight_service_test.dart
├── android/
├── ios/
├── web/
├── macos/
├── linux/
├── windows/
├── pubspec.yaml
└── analysis_options.yaml
```

## 开发命令

在 `apps/testflying` 目录下执行：

```bash
flutter pub get
flutter analyze
flutter test
flutter run
```

指定运行目标：

```bash
flutter run -d chrome
flutter run -d ios
flutter run -d macos
```

Web 远端联调示例：

```bash
fvm flutter run -d chrome \
  --dart-define=TESTFLYING_SERVICE=remote \
  --dart-define=TESTFLYING_API_ENV=development \
  --dart-define=TESTFLYING_API_BASE_URL=http://127.0.0.1:8000 \
  --dart-define=TESTFLYING_ACCESS_TOKEN=dev-token \
  --dart-define=TESTFLYING_DEVICE_ID=device-001 \
  --dart-define=TESTFLYING_CLIENT_PLATFORM=ios
```

浏览器访问真实后端时，后端需要允许 Web 站点来源的 CORS。Web 上点击安装会先尝试打开服务端返回的 `installUrl`；如果浏览器拦截 iOS `itms-services://`，客户端会降级打开 `manifestUrl` 或 `downloadUrl`。

远端接口模式默认关闭。如需接真实后端，用 `dart-define` 显式开启：

```bash
flutter run \
  --dart-define=TESTFLYING_SERVICE=remote \
  --dart-define=TESTFLYING_API_ENV=development \
  --dart-define=TESTFLYING_API_BASE_URL=https://dist.example.test \
  --dart-define=TESTFLYING_ACCESS_TOKEN=<access-token> \
  --dart-define=TESTFLYING_DEVICE_ID=<registered-device-id> \
  --dart-define=TESTFLYING_CLIENT_PLATFORM=ios
```

可用配置：

- `TESTFLYING_SERVICE`：`mock` 或 `remote`，默认 `mock`。
- `TESTFLYING_API_ENV`：`development`、`staging` 或 `production`，默认 `development`。
- `TESTFLYING_API_BASE_URL`：可选，覆盖内置环境地址。
- `TESTFLYING_ACCESS_TOKEN`：远端接口的 bearer token。
- `TESTFLYING_DEVICE_ID`：当前已登记设备 ID，默认 `local`。
- `TESTFLYING_CLIENT_PLATFORM`：`ios` 或 `android`，默认 `ios`。

## 实现说明

当前 UI 应视为可交互原型，而不是生产可用功能。继续扩展前应保持以下边界：

1. UI 交互以 `docs/ui-baseline.md` 为验收基线。
2. 新数据先进入 `MockTestFlightService` 和 `TestFlightWorkspace`，不要重新散落到 Widget 内部。
3. 安装入口只通过 `InstallLauncher` 打开，Widget 不直接拼装 `itms-services` 或 APK URL。
4. 用户可变状态只通过 service 层写入 `WorkspacePreferencesStore`，Widget 不直接操作本地存储。
5. 真实接口接入时通过 `TESTFLYING_SERVICE=remote` 切到 `RemoteTestFlightService`，不要在 Widget 里判断环境。
6. 行为变更需要补充 widget test 或 service test。
7. API 资源和状态变更以 `docs/api-contract.md` 为契约草案。

## Git 说明

项目本地目录里包含 `build/`、`.dart_tool/` 等 Flutter 生成物，但它们已经被项目 `.gitignore` 忽略。除非有明确的发布或可复现构建需求，不要把生成物提交进仓库。

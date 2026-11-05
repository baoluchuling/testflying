# 构建交付与 Runner 节点

## 适用范围

本文档描述 TestFlying 当前的自动构建交付链路：后台创建构建任务、macOS Runner 领取任务、
`package-agent` 执行构建、回传制品与结果，以及失败通知和节点自动更新。

手动上传 IPA/APK 仍然可用，且与 Runner 构建并存：

- 手动上传直接保存已产出的包并分发。
- Runner 构建从 Git 仓库和指定 ref 开始执行，完成后把制品回传到中心后台。

当前 Runner 的运行系统只支持 macOS（`darwin/arm64`、`darwin/amd64`），但节点可以在配置中
声明可处理 `ios`、`android` 或两者的构建任务。

## 角色与边界

| 组件 | 职责 | 不负责 |
| --- | --- | --- |
| 中心后台 | 保存应用构建配置、分配任务、保存事件和制品、展示状态、发送站内/钉钉通知、托管更新包 | 持有节点本机签名证书或直接执行构建命令 |
| macOS Runner | 心跳、按标签领取任务、检出仓库、调用 `package-agent`、上传制品、执行已验证更新 | 修改中心后台任务以外的配置 |
| `package-agent` | 基于构建输入与项目配置运行受策略约束的构建，产出 `report.json` | 提交或推送 Git、为了通过构建修改受保护的项目源码/构建脚本 |

每个 Runner 当前固定单并发（`capacity=1`），并固定支持 iOS 与 Android 构建。任务分配匹配应用
构建配置中的标签；标签不匹配的 Runner 不会领取任务。

## 构建流程

```text
应用构建配置 + 创建构建任务
        |
        v
queued -> Runner 按标签领取 -> assigned / building
        |
        v
检出指定 Git ref -> package-agent -> report.json + 制品
        |
        +-- 成功：上传包、符号文件、日志和报告 -> succeeded
        |
        +-- 失败或需要人工处理：保存分类、摘要、人工动作 -> failed / needs_human
```

Runner 会持续上报心跳并轮询任务。分配中的任务有租约保护；失联或执行失败后，中心后台会按重试上限
处理，最多尝试 5 次，超过上限的任务进入 `needs_human`。常见生命周期包括：
`queued`、`assigned`、`preparing`、`building`、`uploading_artifacts`、`succeeded`、`failed`、
`needs_human` 和 `cancelled`。

## 后台工作区

自动构建集中在以下页面，不再使用独立一级“构建节点”入口：

| 页面 | 用途 |
| --- | --- |
| `/admin/builds/apps` | 选择已接入应用、环境和 Git ref，触发构建 |
| `/admin/builds/history` | 查看构建记录、执行状态和制品 |
| `/admin/builds/runners` | 预配 Runner，查看节点心跳、能力、版本和更新状态 |

应用列表只展示已经配置构建来源的应用。触发前会检查是否存在匹配标签的在线 Runner；没有匹配节点时
页面会明确提示，但不会改写应用配置。

## 后台配置构建

在“应用构建”页面中从已有应用接入构建。每个应用只维护一份源码构建配置，开发环境与线上环境共用；
接入和后续编辑都在当前页面弹窗中完成，不跳转到商店管理。创建任务时需要明确：

- Git 仓库地址和 Git ref。
- 目标构建环境：`development` 或 `production`。
- 需要的 Runner 标签。
- 制品类型。
- 凭据引用名，而不是凭据内容。

Runner 会将仓库签出到任务工作区，并始终以签出后的仓库根目录作为 `package-agent` 的项目目录。
TestFlying 不支持为应用配置仓库子目录；一个应用应对应一个可直接构建的仓库根目录。

凭据引用仅作为任务中的不透明标识传给节点，例如 `apple-signing-prod`、`android-keystore-a`。
中心后台不会保存私钥、令牌、密码或完整服务账号凭据；节点自行将这些引用映射到本机安全存储。

## 创建和安装 macOS Runner

先由中心后台预配 Runner。预配接口会生成一次性可见的 Runner token；token 仅应写入目标节点的
安装配置，不能提交到 Git、上传到公开制品库或粘贴到构建日志。

生成安装包的配置至少包含。构建节点固定同时支持 iOS 和 Android，不需要声明构建平台：

```json
{
  "runnerId": "runner-mac-1",
  "name": "Mac Runner 1",
  "token": "replace-with-runner-token",
  "serverUrl": "https://testflying.example.com",
  "rootDir": "/Users/Shared/TestFlyingRunner/runner-mac-1",
  "labels": ["ios-release", "flutter"],
  "llmAdapters": ["codex", "claude"],
  "capacity": 1
}
```

`runnerId` 必须以字母或数字开头，只能包含字母、数字、点、下划线和短横线，最长 64 个字符。后台会用它生成节点工作目录，因此不接受路径分隔符或 `..`。

在发布机生成节点安装包和更新 bundle：

```bash
scripts/build_runner_installer.sh \
  /absolute/path/to/runner-config.json \
  outputs/build-runner-installer \
  0.2.0
```

发布机构建时需要 `go`、`python3.11`、`ditto`、`shasum`、`pkgbuild` 和 macOS 的标准命令行工具。
输出目录包含：

- `TestFlyingBuildRunner-<version>-darwin-<arch>.pkg`：推荐安装方式。
- `build-runner-macos/`：无法使用 `.pkg` 时的解压备用安装方式，运行其中的 `install.command`。
- `darwin/<arch>/release.json`、更新 ZIP 与 `.sha256`：供已安装节点自动更新。

`.pkg` 会将 Runner 和冻结后的 `package-agent` 安装到
`/Library/Application Support/TestFlying/build-runner`，为当前登录用户创建
`com.testflying.build-runner` LaunchAgent，并立即启动。运行日志位于：

```text
~/Library/Logs/TestFlying/build-runner/runner.log
~/Library/Logs/TestFlying/build-runner/runner.err.log
```

安装后可在管理后台 `/admin/builds/runners` 确认节点状态、标签、版本、能力、最近心跳和当前任务。

## 自动更新

中心后台从 `TESTFLYING_RUNNER_RELEASE_ROOT` 读取发布清单。Docker Compose 默认将其映射为：

```text
/app/data/runner-releases
```

将新构建输出的 `darwin/` 目录复制到该发布根目录后，已安装的 Runner 会使用自身 token 查询更新。
默认轮询任务间隔为 5 秒，默认检查更新间隔为 30 分钟，均可在安装配置中调整。

更新具有以下约束：

- 仅接受服务端认证后的相对下载路径。
- 下载后必须通过服务端发布的 SHA-256 校验。
- ZIP 必须且只能包含 `testflying-build-runner` 与 `package-agent` 两个常规文件。
- 不允许降级任一已安装组件。
- 两个二进制会先完成校验和暂存，再原子替换；替换完成后 Runner 正常退出，由 LaunchAgent 重启。

发布新版本前应同时更新 Runner 与 `package-agent` 的语义版本，且 `release.json` 中的平台、架构、
文件名和 SHA-256 必须与实际 bundle 一致。

## 构建验收和 Git 边界

`package-agent` 可以在 Runner 管理的检出目录内执行 `git fetch`、`git pull`、状态检查等本地 Git
操作，但策略明确阻止 `git commit` 和 `git push`。它也不能通过修改项目源码、构建脚本或关键原生
工程文件来让构建通过。

本地真实构建验收脚本用于在接入节点前验证这条边界：

```bash
scripts/verify_real_build.sh \
  <PROJECT_DIR> \
  <PACKAGE_AGENT_BIN> \
  <CONFIG_JSON> \
  <ios|android> \
  <development|production> \
  <artifact_type> \
  <OUTPUT_DIR>
```

脚本会在输出目录写入 `acceptance.json`、`agent.stdout.log`、`agent.stderr.log` 和 Agent 输出。成功
需要同时满足：

- `package-agent` 退出成功且 `report.json` 的 `status` 为 `success`。
- 至少存在一个包、符号文件和日志制品。
- Git HEAD 和已有工作区状态未变化。
- 新增文件只出现在允许的生成目录（如 `build/`、`.dart_tool/`、`.gradle/`、`Pods/`、
  `DerivedData/`）或验收输出目录。

如果验收失败，应读取 `acceptance.json` 的 `classification`、`summary` 和 Git 违规字段定位原因，
不要忽略 Git 边界后继续发布。

## 通知与钉钉投递

Agent 来源的构建进入 `failed` 或 `needs_human` 时，中心后台会创建站内构建通知。配置钉钉机器人后，
同一状态还会进入持久化投递队列；构建最终状态不依赖钉钉投递是否成功。

服务端环境变量：

```bash
TESTFLYING_DINGTALK_WEBHOOK_URL=https://oapi.dingtalk.com/robot/send?access_token=...
TESTFLYING_DINGTALK_SECRET=SEC...
TESTFLYING_DINGTALK_TIMEOUT_SECONDS=5
TESTFLYING_DINGTALK_DISPATCH_INTERVAL_SECONDS=10
```

Webhook URL、启用状态、超时和扫描间隔可以在 `/admin/settings/notifications` 保存；加签密钥只允许
覆盖写入，读取时仅返回“已配置”状态。没有数据库配置时继续使用服务端环境变量。投递失败
按 1 分钟、5 分钟、30 分钟、2 小时重试；第 5 次失败后标记为 `dead`。通知页面会展示钉钉是否已配置、
待发送数和最终失败数。

Connector 默认地址模板在 `/admin/settings/general` 维护，LLM 模型与功能绑定在
`/admin/settings/llm` 维护。业务设置均采用数据库优先、环境变量兜底；管理后台不会读取或改写 `.env`。
`/admin/settings/runtime` 只读展示部署配置是否存在，敏感值始终脱敏。

## Runner 接口

以下接口仅供已预配 Runner 使用，全部通过 Runner token 的 Bearer 认证：

```text
POST /admin/api/build-runners/register
POST /admin/api/build-runners/heartbeat
POST /admin/api/build-runners/poll
POST /admin/api/build-runners/{runnerId}/updates/check
GET  /admin/api/build-runners/{runnerId}/updates/{platform}/{arch}/{version}/bundle
POST /admin/api/build-runners/builds/{buildId}/events
POST /admin/api/build-runners/builds/{buildId}/artifacts
POST /admin/api/build-runners/builds/{buildId}/complete
```

管理后台的预配、应用构建设置和创建任务接口使用管理员认证；它们不应暴露给 Runner 或普通客户端。

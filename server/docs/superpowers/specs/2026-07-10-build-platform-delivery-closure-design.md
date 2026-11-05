# TestFlying 构建平台交付闭环设计

## 背景

TestFlying 已具备应用级构建配置、构建历史、Runner 调度、`package-agent`
受限执行、构建制品上传和 `needs_human` 状态。当前剩余交付缺口是：

- 构建超出 Agent 边界后没有对外发送钉钉通知。
- macOS 安装器只包含 Go Runner，没有自带独立 `package-agent`。
- Runner 没有自动更新能力。
- 尚无可重复执行的真实 Flutter/原生项目构建验收流程。

本设计补齐以上闭环，不改变 Agent 的源代码写入边界，也不允许 Agent 执行
`git commit` 或 `git push`。Agent 允许执行 `git fetch`、`git pull`、状态检查以及受管
workspace 内的本地 Git 操作，但不得借这些操作改写用户原始 Jenkins workspace。

## 目标

1. `failed` 和 `needs_human` 构建通过钉钉自定义机器人可靠通知。
2. 管理后台“通知”页面展示钉钉配置状态和完整配置教程，不泄露凭据。
3. macOS 安装包同时携带 Build Runner 和独立 `package-agent`，双击即可安装并启动。
4. Runner 能从 TestFlying 服务发现、校验并原子安装新版本。
5. 提供真实项目验收入口，并至少在现有 Flutter 工程上执行实际构建验证。

## 非目标

- 本阶段不增加应用商店提交或发布。
- 本阶段不支持钉钉以外的通知渠道。
- 本阶段不允许 Agent 为通过构建而修改项目源代码或项目构建脚本。
- 本阶段不自动生成或替换 Apple/Android 签名凭据。
- 本阶段不执行 Git 推送。

## 总体架构

构建完成接口仍以数据库中的构建状态为事实来源。状态进入 `failed` 或
`needs_human` 时，同一事务创建站内通知和钉钉投递任务。独立后台循环读取投递任务，
完成加签、发送、重试和最终失败记录。钉钉不可用不会回滚或改变构建结果。

Runner 安装产物由 macOS 发布脚本生成。产物内同时包含 Go Runner、通过 PyInstaller
生成的独立 `package-agent`、节点配置和安装脚本。Runner 定时从中心服务读取适配当前
macOS 架构的版本清单，校验 SHA-256 后替换两个二进制文件并退出，由 LaunchAgent
重新拉起。

真实构建验收脚本在构建前后检查 Git dirty 状态和项目写入边界，调用真实
`package-agent`，并验证包、符号表、报告和日志四类必需制品。

## 钉钉通知

### 配置

服务端新增以下环境变量：

- `TESTFLYING_DINGTALK_WEBHOOK_URL`：钉钉自定义机器人 Webhook URL。
- `TESTFLYING_DINGTALK_SECRET`：机器人“加签”安全设置中的密钥。
- `TESTFLYING_DINGTALK_TIMEOUT_SECONDS`：单次请求超时，默认 `5`。
- `TESTFLYING_DINGTALK_DISPATCH_INTERVAL_SECONDS`：后台扫描间隔，默认 `10`。

URL 或密钥任一缺失时，钉钉渠道状态为 `not_configured`。服务端不发送请求，但保留
站内通知。URL 和密钥只存在于进程环境中，不写数据库、不写日志、不通过管理 API
返回。

### 触发条件与去重

仅在 Agent 构建首次进入以下终态时创建钉钉投递：

- `failed`
- `needs_human`

投递使用 `build_id + lifecycle_status + channel` 作为唯一事件键。同一完成请求重放、
Runner 重试或服务重启均不得产生重复消息。成功和取消构建不发送钉钉消息。

### 消息内容

消息使用钉钉 `markdown` 格式，包含：

- 应用名称和 bundle identifier/package name
- 平台、环境、构建 ID 和 Git ref
- 终态、失败分类和脱敏后的失败摘要
- 需要人工执行的动作
- TestFlying 应用详情页链接

凭据引用、Runner token、Webhook URL、签名密钥和疑似 secret 文本必须经过现有脱敏器，
不得进入持久化 payload 或钉钉消息。

### 加签

发送端按钉钉自定义机器人加签协议生成毫秒时间戳。签名原文为
`timestamp + "\n" + secret`，使用 secret 执行 HMAC-SHA256，结果 Base64 后 URL 编码，
并以 `timestamp` 和 `sign` 查询参数调用 Webhook。

### 持久化和重试

新增 `webhook_deliveries` 表：

- `id`
- `channel`
- `event_key`，唯一
- `status`：`pending`、`delivered`、`dead`
- `payload_json`
- `attempt_count`
- `next_attempt_at`
- `last_error`
- `created_at`
- `delivered_at`

后台投递器在应用启动后运行，并按 `0 秒、1 分钟、5 分钟、30 分钟、2 小时` 退避。
达到五次仍失败时标记 `dead`。错误文本需要脱敏并限制长度。服务重启后继续处理
`pending` 任务。

## 管理后台教程

现有“通知”页面新增“钉钉机器人配置”区，不新增独立设置页面。后端只返回：

- `configured`
- `webhookConfigured`
- `secretConfigured`
- 当前触发状态列表
- `pendingDeliveryCount`
- `deadDeliveryCount`

页面显示：

1. 在钉钉群中创建自定义机器人。
2. 安全设置选择“加签”，取得 Webhook URL 和密钥。
3. 配置两个必需环境变量的 Docker Compose 示例。
4. 重启 TestFlying 服务并在页面确认状态为“已配置”。
5. 说明只在 `failed` 和 `needs_human` 时发送消息。

页面不提供 URL 或密钥输入框，也不回显部分掩码，避免浏览器和 API 接触服务端凭据。

## macOS 一键安装包

### 发布产物

`scripts/build_runner_installer.sh` 生成当前 macOS 架构的发布目录和安装包，至少包含：

- `testflying-build-runner`
- PyInstaller `--onefile` 生成的 `package-agent`
- 节点 `config.json`
- `install.command`
- `release.json`
- 用于自动更新的二进制 bundle ZIP
- SHA-256 校验文件
- 可双击安装的 `TestFlyingBuildRunner-<version>-darwin-<arch>.pkg`

安装包命名包含版本、平台和架构。构建脚本在缺少配置、PyInstaller、Go、`pkgbuild`
或二进制生成失败时直接退出，不产出半成品。每个 `.pkg` 必须使用管理后台为目标节点
签发的完整 Runner 配置生成；包含空 token、空服务地址或空 Runner ID 的通用占位包禁止
生成。

### 安装行为

用户双击 `.pkg` 后：

1. 将两个二进制文件和配置写入 TestFlying 专用目录。
2. 将 `packageAgentBin` 固定为同目录内的独立 `package-agent`，无需系统 Python。
3. 配置文件权限设为 `0600`。
4. 安装并加载 LaunchAgent，Runner 以当前登录用户身份运行，从而复用本机 Codex 或
   Claude CLI 登录状态。
5. 安装结束后检查进程和日志路径；失败时返回非零状态并显示具体日志位置。

安装器不得安装或覆盖 Codex/Claude CLI。`package-agent` 继续按
`codex -> claude -> llm-runtime` 的既定顺序自动发现本机 LLM。

## Runner 自动更新

### 服务端发布目录和接口

服务端新增 `TESTFLYING_RUNNER_RELEASE_ROOT`。发布脚本输出的版本清单和二进制 bundle
由部署流程复制到该目录。

Runner 使用现有 Runner token 调用版本接口，提交：

- 当前 Runner 版本
- 当前 `package-agent` 版本
- `darwin`
- `amd64` 或 `arm64`

服务端只返回适配该平台和架构的最新版本、下载 URL、bundle SHA-256、Runner 版本和
`package-agent` 版本。下载接口同样要求 Runner 鉴权，路径必须受发布目录约束。

### 更新流程

Runner 启动时检查一次，之后默认每 30 分钟检查：

1. 下载到 Runner 根目录内的临时目录。
2. 校验完整文件 SHA-256。
3. 安全解压，只接受清单声明的两个普通文件，拒绝绝对路径、符号链接和 `..`。
4. 校验两个二进制存在且可执行。
5. 先保留上一版本，再原子替换两个二进制。
6. 更新成功后退出，由 LaunchAgent 重启。
7. 任一步骤失败时保留当前版本，记录脱敏错误并继续构建轮询。

自动更新不得修改项目 workspace、Git 仓库或签名材料。

## 真实构建验收

新增验收脚本，输入项目目录、平台、环境和制品类型。脚本执行：

1. 记录当前 HEAD、tracked dirty 和 untracked 文件清单。
2. 创建隔离的 Agent 输出目录。
3. 调用安装产物中的真实 `package-agent`，不使用 fake CLI。
4. 验证 report 状态和包、symbols、report、log 四类制品。
5. 再次检查 Git 状态，确认项目源码和构建脚本未被改写。
6. 输出机器可读验收报告。

第一轮使用 `/Users/admin/ai_project/apps/testflying` Flutter 工程验证 Android APK/AAB
和 iOS xcarchive/IPA/dSYM。环境依赖可以自动安装或修复；如果项目签名配置或源代码必须
修改，验收必须以 `needs_human` 失败并触发钉钉通知，不得绕过边界。

原生 Android 和原生 iOS 的命令与制品路径继续由项目已有脚本或 LLM 受限计划提供，
并由单元/集成测试覆盖对应 Gradle 和 Xcode 命令形态。

## 测试策略

- 钉钉签名单元测试使用固定时间戳和密钥。
- 钉钉 HTTP 测试覆盖成功、超时、非零 `errcode`、脱敏、去重、重试和重启恢复。
- 管理 API 和 React 测试覆盖配置状态及教程内容，断言响应不包含 URL 和密钥。
- 安装器测试覆盖两个二进制、权限、LaunchAgent、缺少工具失败和配置失败。
- Go 更新器测试覆盖版本选择、鉴权、校验失败、Zip Slip、原子替换和回滚。
- 真实验收脚本覆盖 dirty 检测、必需制品门禁和 `needs_human` 边界。
- 最终运行后端、前端、`package-agent`、Go Runner、迁移往返、安装包实构建和真实项目验收。

## 验收标准

只有同时满足以下条件才可宣称完成：

- 钉钉机器人收到真实 `needs_human` 或失败构建消息。
- 通知页面显示正确配置状态和教程，且任何 API/日志均未泄露凭据。
- 在干净 macOS 用户环境中双击安装后，Runner 自动上线并发现本机 LLM。
- 发布新版本后 Runner 自动更新两个二进制并恢复在线。
- 至少一个真实 Flutter Android 工程分别产出 APK 和 AAB，并产出 symbols/report/log。
- 至少一个真实 Flutter iOS 构建产出 xcarchive/IPA/dSYM/report/log；若只能因项目签名或源码
  修改受阻，则必须留下 `needs_human` 证据和钉钉通知，不能记为成功。
- 全量自动化测试和数据库迁移往返通过。
- Git 工作区仅包含本任务预期改动，且没有执行 Git 推送。

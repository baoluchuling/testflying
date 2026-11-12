# macOS 截图 Connector

外部系统按平台、包名绑定截图节点，提交平台、包名、截图要求和 target 数组。中心创建任务并返回 jobId，之后保存、节点执行和结果通知都使用该 ID，调用方无需传入。Mac 上的 Connector 为每个任务开启独立的 Codex Goal 会话。Codex 自行发现项目、准备模拟器和操作 App，并在同一会话中调用 Connector 提供的校验、截图和回传工具。节点将 PNG 直接上传 OSS，中心保存目标与图片地址的对应关系；任务结束后中心按 Studio capture-v1 协议回传完整结果；通知独立于截图状态。

## 创建节点

管理员在 HTTPS 配置页解锁，打开「截图 Connector 部署」，可新建或选择截图节点并下载该节点的 macOS Apple Silicon（arm64）部署 ZIP。也可通过现有接口创建，省略 `kind` 时仍创建商店 Connector。

```http
POST /v1/connectors
Authorization: Bearer ADMIN_TOKEN
Content-Type: application/json

{"name":"截图 Mac","kind":"screenshot"}
```

管理员也可通过 HTTPS 调用 `GET /v1/connectors/{connectorId}/screenshot-installer` 下载部署 ZIP。ZIP 内包含 worker、启动脚本、公开 CA、说明和带当前节点 Token 的 `connector.json`，请私下交给对应的 Mac。

已有和新发放的接入 Token 均可调用全部启用的截图节点。接入系统用同一枚 Token 调用商店和截图接口，并用发起任务的 Token 查询任务状态。通用商店 Token 无法派截图任务。 [接入凭据管理](connector-setup.md)

## 外部对接文档

截图节点列表、按包名绑定、触发和查询任务的唯一对接说明已合并到 [中心统一接口文档](store-api.md#截图节点与截图任务https)。触发只传 platform、bundleIdentifier、prompt 和 target 数组，响应返回中心生成的 jobId；查询仅作排障备用。本页仅保留实体 Mac 部署运维说明。中心保留 prompt 原文，通过 get_capture_task 提供 jobId、应用身份与目标；本轮协议 3 需安装最新截图节点程序。

## 在实体 Mac 上运行

将部署 ZIP 解压到 Mac 的私密目录并限制目录访问，将 `connector.json` 设为 `0600`。在这台 Mac 上提前准备专用账号、项目、Codex 登录和可用的模拟器。Codex 根据任务在本机查找项目，访问范围由其本机权限配置决定。Android 的 ADB 服务使用默认本地端口，由部署方启动。Connector 优先使用可选的 `adbPath`，其次查找 Android SDK 和 `PATH` 中的 `adb`。

```json
{
  "kind": "screenshot",
  "connectorId": "11111111-1111-4111-8111-111111111111",
  "token": "NODE_TOKEN_FROM_INSTALLATION",
  "messageKey": "MESSAGE_KEY_FROM_INSTALLATION",
  "serverUrl": "https://store-api.example.com/store-api"
}
```

使用私有 CA 时，通过 `caFile` 指向该 CA 文件。Rust Connector 从安装账号的 `PATH` 找到 Codex CLI，沿用该账号的登录和用户配置，并以内置 `workspace-write` 沙盒和 `never` 审批模式启动。特殊安装可通过 `codexPath` 或 `codexHome` 指定绝对路径。Codex 按本机权限查找项目，任务的工作区设为 Connector 创建的私有临时目录，用于存放任务产物，完成、失败或正常停止时清理。[OpenAI Docs](https://learn.chatgpt.com/docs/sandboxing)

需要支持 Goal 和 App Server 动态工具的 Codex CLI，已验证 0.139.0 的初始化协议。Connector 为该进程启用 Goal。Codex 自行决定复用、创建或启动模拟器，以及是否构建、安装和启动 App。会话先调用 `get_capture_task` 读取 jobId、应用与 target；`validate_simulator` 检查所选设备，`capture_screenshot` 按 targetIndex 截取该设备并返回 screenshotId，`upload_screenshot` 用相同 targetIndex 上传这张图。可恢复的工具失败会返回给当前 Goal，Codex 可以修复后重试。每张图确认后继续下一个目标，全部 target 均确认上传后 Goal 才算完成。[OpenAI Docs](https://learn.chatgpt.com/docs/app-server)

将解压目录移到截图账号的 `~/Library/Application Support/` 下，填好配置并设好文件权限后，在该目录执行安装。安装脚本会请求管理员授权，将节点注册为系统 LaunchDaemon，并以截图账号立即启动。开机后服务自动运行，无需保持终端打开。

```sh
chmod 700 .
chmod 600 connector.json
sh ./install.sh
```

启动后，截图节点和商店节点一样主动建立加密 WSS 长连接。中心入队后实时发出任务通知，断线重连后节点会重新检查队列。后台「截图 Connector 部署」的节点下拉框显示连接状态，可点「刷新连接状态」。实际截图仍要求本机对应模拟器可用。

升级已有节点时，把新版 ZIP 中的 `screenshot-connector`、`run.sh`、`install.sh` 和 `testflying-prod-ca.crt` 覆盖到原安装目录，保留原来的 `connector.json`，再运行 `sh ./install.sh` 重启服务。

在安装目录执行 `tail -F connector.log` 即可查看后台日志。按 jobId 查找 `job_claimed`、`codex_goal_started`、`simulator_unavailable`、`image_ready` 或 `result_rejected`，可定位执行阶段。完整提示词用 JSON 字符串保存在领单记录中。日志只保存在 Mac 上，文件权限为 `0600`。服务启动时和运行期间每分钟清理超过 24 小时的记录。原生 Goal 会话历史由安装账号的 Codex 管理。

节点收到通知后通过 `POST /v1/connectors/{id}/screenshot-jobs/claim` 并带 X-Screenshot-Protocol: 3 领取任务，空队列返回 204。执行失败通过 `POST /v1/connectors/{id}/screenshot-jobs/{jobId}/failure` 报告。截图后通过 POST /v1/connectors/{id}/screenshot-jobs/{jobId}/upload-ticket 获取短时上传地址，直接 PUT PNG 到 OSS，再 POST 同一路径下的 complete 确认结果。两次请求均传 targetIndex（从 0 开始）；jobId 与触发响应中的 UUID 相同。图片最大 16 MiB。中心接口要求节点 Bearer Token 和 X-Screenshot-Lease；OSS 请求只携带短时签名及绑定的图片元数据。任务超时后需要由外部显式决定是否创建新任务。

## OSS 与外部通知配置

在中心后台「截图上传与结果通知」保存 OSS 和外部通知配置。OSS 凭据加密保存在中心，节点部署包中不包含 AccessKey。截图节点通过中心获取绑定任务、租约与图片摘要的短时 PUT 地址，将原始 PNG 直接上传至 OSS，再通知中心读取对象元数据验证并记录结果。中心不接收该链路图片字节，也不落盘保存图片。OSS 配置缺失不拦截截图执行，在实际上传阶段返回 screenshot_oss_not_configured。

填写 Studio 完整 HTTP 或 HTTPS `/api/hooks/captures` 地址和独立 Bearer Token；中心在任务结束后 POST 完整结果，必要时 GET 同路径后追加 jobId 对账。缺少地址或 Token 为 not_configured，不阻止截图执行；通知状态通过 result.notificationStatus 查看，失败只重试回传、不重新执行采集。图片使用公开 HTTPS PNG 直链，路径为 store-studio/materials/图片ID前两位/32位图片ID.png，不含 jobId。没有主题参数，一单只使用一种设备；多页面、多语言仍支持。完整回传格式与重试规则见统一文档。

本轮需在截图 Mac 重新安装最新 arm64 部署包以升级程序，沿用原节点身份和 Codex 环境。旧程序（包括此前协议 2）不能领取协议 3 的新任务，返回 screenshot_connector_upgrade_required 且任务保留在队列；旧 PNG 回传入口在 OSS 启用后也返回该错误。完整配置 API、公开直链和外部接口见[中心统一文档](store-api.md#截图节点与截图任务https)。

# macOS 截图 Connector

外部系统提交节点 ID 和截图要求。Mac 上的 Connector 为每个任务开启独立的 Codex Goal 会话。Codex 自行发现项目、准备模拟器和操作 App，并在同一会话中调用 Connector 提供的校验、截图和回传工具。中心将 PNG 转发给配置的外部服务。

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

## 外部系统

```http
POST /v1/screenshot-jobs
Authorization: Bearer INTEGRATION_TOKEN
Content-Type: application/json

{
  "connectorId":"11111111-1111-4111-8111-111111111111",
  "prompt":"截 Amberia iOS 的 iPhone 首页，用于 App Store 商店页。竖屏，保留完整画面和原始分辨率。"
}
```

成功返回 `202` 和 `{"jobId":"…","status":"queued"}`。`prompt` 只需描述应用、页面、设备偏好和画面要求。Connector 将其原样作为首轮用户消息，工具和完成条件由 Connector 单独设置。提示词不能为空，上限为 16 KiB。`profile` 是可选的任务提示，允许 1–64 个 ASCII 字母、数字、下划线或短横线。查询省略该字段的任务时返回 `"profile":null`。发起任务的系统用同一枚 Token 请求 `GET /v1/screenshot-jobs/{jobId}`。状态为 `queued`、`running`、`uploading`、`succeeded` 或 `failed`。成功时 `result` 可包含外部服务返回的 HTTPS `imageUrl`，失败时 `error` 为中心错误代码。每个 Mac 同时处理一个任务，最多排队 20 个，任务记录保留约七天。

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

需要支持 Goal 和 App Server 动态工具的 Codex CLI，已验证 0.139.0 的初始化协议。Connector 为该进程启用 Goal。Codex 自行决定复用、创建或启动模拟器，以及是否构建、安装和启动 App。会话内的 `validate_simulator` 检查所选设备，`capture_screenshot` 截取该设备并返回本任务的 `screenshotId`，`upload_screenshot` 通过中心交付这张图。可恢复的工具失败会返回给当前 Goal，Codex 可以修复后重试。中心确认上传后 Goal 才算完成。[OpenAI Docs](https://learn.chatgpt.com/docs/app-server)

将解压目录移到截图账号的 `~/Library/Application Support/` 下，填好配置并设好文件权限后，在该目录执行安装。安装脚本会请求管理员授权，将节点注册为系统 LaunchDaemon，并以截图账号立即启动。开机后服务自动运行，无需保持终端打开。

```sh
chmod 700 .
chmod 600 connector.json
sh ./install.sh
```

启动后，截图节点和商店节点一样主动建立加密 WSS 长连接。中心入队后实时发出任务通知，断线重连后节点会重新检查队列。后台「截图 Connector 部署」的节点下拉框显示连接状态，可点「刷新连接状态」。实际截图仍要求本机对应模拟器可用。

升级已有节点时，把新版 ZIP 中的 `screenshot-connector`、`run.sh`、`install.sh` 和 `testflying-prod-ca.crt` 覆盖到原安装目录，保留原来的 `connector.json`，再运行 `sh ./install.sh` 重启服务。

在安装目录执行 `tail -F connector.log` 即可查看后台日志。按任务 ID 查找 `job_claimed`、`codex_goal_started`、`simulator_unavailable`、`image_ready` 或 `result_rejected`，可定位执行阶段。完整提示词用 JSON 字符串保存在领单记录中。日志只保存在 Mac 上，文件权限为 `0600`。服务启动时和运行期间每分钟清理超过 24 小时的记录。原生 Goal 会话历史由安装账号的 Codex 管理。

节点收到通知后通过 `POST /v1/connectors/{id}/screenshot-jobs/claim` 领取任务，空队列返回 204。执行失败通过 `POST /v1/connectors/{id}/screenshot-jobs/{jobId}/failure` 报告。成功后通过 `PUT /v1/connectors/{id}/screenshot-jobs/{jobId}/image` 上传 PNG 原始字节，最大 16 MiB。两条任务结果接口都要求节点 Bearer Token 和领单返回的 `X-Screenshot-Lease`。任务超时后需要重新提交。

## 外部上传占位协议

在中心设置 `STORE_API_SCREENSHOT_UPLOAD_URL`。真实服务可选设置 `STORE_API_SCREENSHOT_UPLOAD_TOKEN` 用于下游 Bearer 鉴权，节点或外部系统的 Token 不随转发发送。URL 必须是固定 HTTPS 地址，本地联调可用回环 HTTP。任务可以在地址为空或转发配置无效时提交、领取和执行。Mac 交回图片时，如果中心仍无有效的接收配置，回传接口返回 `503 screenshot_upload_unconfigured`，任务状态为 `failed`，并记录此回传错误、释放任务租约。

中心在内存中接收并验证 PNG，随后向该地址 POST `multipart/form-data`。图片字段名为 `file`，文件名为 `screenshot.png`，并发送值为 jobId 的 `Idempotency-Key`。中心只持久化任务状态和可选的 HTTPS `imageUrl`。外部正式协议明确后可单独更换这一层。中心拒绝跳转和重复交图，上传超时记录为 `delivery_unknown`，避免重试导致重复上传。

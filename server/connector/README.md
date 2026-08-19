# testflying-connector

`testflying-connector` 是按开发者账号隔离部署的商店 API 中转器。它使用 Go 实现，运行时是单二进制服务，不依赖数据库、MinIO 或 Python Web 框架。

中心后台 `testflying-server` 只保存 connector 地址和调用 token，不保存 Apple `.p8`、Google service account 凭据或商店访问 token。每个 connector 只绑定一个开发者账号，只接受该账号的同步请求。

## 模式

- `mock`：默认模式，用于本地开发和无商店凭据测试，返回可预测的示例结果。
- `live`：生产模式，读取 Apple / Google 凭据并调用真实商店 API。

## 连接方式

- HTTP 模式：connector 监听本机端口，中心后台直接访问 `http://<connector-host>:8100`。适合有固定内网/VPN/公网入口的机器。
- Active 模式：connector 不监听公网端口，而是主动请求中心后台 `/connector-agent/v1/poll` 领取任务，再把结果回传 `/connector-agent/v1/results`。Windows 一次性安装包默认使用这种方式，不需要 SSH 反向隧道，也不需要在 Windows 上开放入站端口。

生产环境必须显式设置：

```bash
TESTFLYING_CONNECTOR_STORE_MODE=live
```

## 本地启动

```bash
cd apps/testflying-server/connector
go test ./...
TESTFLYING_CONNECTOR_DEVELOPER_ACCOUNT_ID=account-apple-enterprise \
TESTFLYING_CONNECTOR_TOKEN=dev-connector-token \
go run ./cmd/testflying-connector
```

默认监听 `:8100`，可用 `TESTFLYING_CONNECTOR_LISTEN_ADDR` 覆盖。

## Windows 一次性安装包

推荐在中心后台的开发者账号详情页生成 Windows 安装包：

1. 打开 `开发者账号`，进入目标账号。
2. 在 `Connector 配置` 中找到 `生成 Windows 一次性安装包`。
3. 上传当前账号需要的凭据：
   - iOS / App Store Connect：`Issuer ID`、`Key ID`、`.p8` 文件。
   - Android / Google Play：`client_email`、`private_key` 拆分字段，或完整 service account JSON。
4. 下载 zip 后复制到 Windows，解压。
5. 用管理员 PowerShell 执行：

```powershell
powershell.exe -ExecutionPolicy Bypass -File .\install.ps1
```

安装包会把文件复制到：

```text
C:\ProgramData\TestFlying\connectors\<account_id>
```

并注册计划任务：

```text
testflying-connector-<account_id>
```

手动重启：

```powershell
schtasks /Run /TN testflying-connector-<account_id>
```

查看日志：

```text
C:\ProgramData\TestFlying\connectors\<account_id>\logs\connector.log
```

后续更新不需要重新复制 zip 或重新填写配置。首次安装完成后，直接在 Windows 上运行安装目录里的更新脚本：

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "C:\ProgramData\TestFlying\connectors\<account_id>\update.ps1"
```

`update.ps1` 会从 GitHub Release 自动下载最新 Windows connector，停止计划任务，备份旧的 `testflying-connector.exe`，只替换新的 exe，再重启计划任务。它不会覆盖 `config.json`、`secrets`、账号信息或商店凭据。

如果是已经装过的旧机器，安装目录里还没有 `update.ps1`，不用重新运行 `install.ps1`。下载一次新的 Windows 安装包，解压后直接运行解压目录里的 `.\update.ps1`；脚本会自动把自己复制到 `C:\ProgramData\TestFlying\connectors\<account_id>\update.ps1`，以后就只运行安装目录里的脚本。

安装包内的 `config.json` 会配置 active 模式：

```json
{
  "accountId": "account-a",
  "connectorToken": "<generated-token>",
  "storeMode": "live",
  "centerUrl": "http://47.90.163.122:8000",
  "apple": {
    "issuerId": "<issuer-id>",
    "keyId": "<key-id>",
    "privateKeyPath": "C:\\ProgramData\\TestFlying\\connectors\\account-a\\secrets\\apple\\AuthKey_XXXX.p8"
  }
}
```

中心后台会同步把该账号的 connector 地址改为 `active://<account_id>`。Apple `.p8` 和 Google service account 凭据只写入下载包和 Windows 本机目录，后台不会长期保存这些文件。

## Windows 单机部署

CI 会在 GitHub Release 里生成 Windows 单二进制压缩包：

```text
testflying-connector-windows-amd64-<SHA>.zip
```

下载后解压得到 `testflying-connector-windows-amd64-<SHA>.exe`。推荐固定放到 `C:\testflying-connector`，再用 Windows 任务计划程序开机自启：

```powershell
$Sha = "<commit sha>"
$Root = "C:\testflying-connector"
$TaskName = "testflying-connector-account-a"

New-Item -ItemType Directory -Force $Root, "$Root\secrets\apple", "$Root\secrets\google" | Out-Null
Expand-Archive -Force "$Root\testflying-connector-windows-amd64-$Sha.zip" $Root
Move-Item -Force "$Root\testflying-connector-windows-amd64-$Sha.exe" "$Root\testflying-connector.exe"

@"
`$env:TESTFLYING_CONNECTOR_LISTEN_ADDR = ":8100"
`$env:TESTFLYING_CONNECTOR_STORE_MODE = "live"
`$env:TESTFLYING_CONNECTOR_DEVELOPER_ACCOUNT_ID = "account-a"
`$env:TESTFLYING_CONNECTOR_TOKEN = "<random-token>"

# Apple
`$env:TESTFLYING_CONNECTOR_APPLE_ISSUER_ID = "<issuer-id>"
`$env:TESTFLYING_CONNECTOR_APPLE_KEY_ID = "<key-id>"
`$env:TESTFLYING_CONNECTOR_APPLE_PRIVATE_KEY_PATH = "$Root\secrets\apple\AuthKey_XXXX.p8"

# Google
# `$env:TESTFLYING_CONNECTOR_GOOGLE_SERVICE_ACCOUNT_JSON_PATH = "$Root\secrets\google\service-account.json"
# 或者使用拆分字段:
# `$env:TESTFLYING_CONNECTOR_GOOGLE_CLIENT_EMAIL = "service-account@project.iam.gserviceaccount.com"
# `$env:TESTFLYING_CONNECTOR_GOOGLE_PRIVATE_KEY = "-----BEGIN PRIVATE KEY-----`n...`n-----END PRIVATE KEY-----"

`$LogPath = "$Root\logs\connector.log"
`$CommandLine = "`"$Root\testflying-connector.exe`" >> `"`$LogPath`" 2>&1"
& `$env:ComSpec /d /s /c `$CommandLine
exit `$LASTEXITCODE
"@ | Set-Content -Encoding UTF8 "$Root\run-connector.ps1"

schtasks /Create /TN $TaskName /SC ONSTART /RL HIGHEST /RU SYSTEM /TR "powershell.exe -ExecutionPolicy Bypass -File $Root\run-connector.ps1" /F
schtasks /Run /TN $TaskName
Invoke-RestMethod http://127.0.0.1:8100/health
```

中心后台里的 Connector 地址要填这台 Windows 机器对中心后台可访问的地址，例如 `http://192.168.1.20:8100`；调用 Token 要和 `TESTFLYING_CONNECTOR_TOKEN` 一致。

如果使用 active 模式，则不需要填写 Windows 机器地址；中心后台 connector 地址使用 `active://<account_id>`。

## Docker

```bash
cd apps/testflying-server/connector
docker build -t testflying-connector:local .
docker run --rm -p 8100:8100 --memory=64m \
  -e TESTFLYING_CONNECTOR_DEVELOPER_ACCOUNT_ID=account-apple-enterprise \
  -e TESTFLYING_CONNECTOR_TOKEN=dev-connector-token \
  testflying-connector:local
```

## Apple 凭据

Apple 使用 App Store Connect API Key。准备步骤：

1. 登录 App Store Connect。
2. 进入 Users and Access / Integrations / App Store Connect API。
3. 创建 Team API Key。
4. 记录 `Issuer ID` 和 `Key ID`。
5. 下载 `.p8` 私钥文件。
6. 角色建议先用 `App Manager`，因为需要读取和修改 App Store 信息、版本本地化和商店文案。

部署示例：

```bash
docker run -d --name testflying-connector-apple-a \
  -p 8100:8100 --memory=64m \
  -v /opt/testflying/secrets/apple-a:/run/secrets/apple:ro \
  -e TESTFLYING_CONNECTOR_STORE_MODE=live \
  -e TESTFLYING_CONNECTOR_DEVELOPER_ACCOUNT_ID=apple-account-a \
  -e TESTFLYING_CONNECTOR_TOKEN='<random-token>' \
  -e TESTFLYING_CONNECTOR_APPLE_ISSUER_ID='<issuer-id>' \
  -e TESTFLYING_CONNECTOR_APPLE_KEY_ID='<key-id>' \
  -e TESTFLYING_CONNECTOR_APPLE_PRIVATE_KEY_PATH=/run/secrets/apple/AuthKey_XXXX.p8 \
  testflying-connector:local
```

## Google 凭据

Google 使用 service account。准备步骤：

1. 创建或选择 Google Cloud Project。
2. 启用 Google Play Developer API。
3. 创建 service account。
4. 在 Google Play Console 的 Users and permissions 中邀请 service account 邮箱。
5. 给目标 App 授权，至少需要能查看和管理商店信息。
6. 下载 service account JSON，或记录 JSON 里的 `client_email` 和 `private_key` 两个字段。

部署示例：

```bash
docker run -d --name testflying-connector-google-a \
  -p 8100:8100 --memory=64m \
  -v /opt/testflying/secrets/google-a:/run/secrets/google:ro \
  -e TESTFLYING_CONNECTOR_STORE_MODE=live \
  -e TESTFLYING_CONNECTOR_DEVELOPER_ACCOUNT_ID=google-account-a \
  -e TESTFLYING_CONNECTOR_TOKEN='<random-token>' \
  -e TESTFLYING_CONNECTOR_GOOGLE_SERVICE_ACCOUNT_JSON_PATH=/run/secrets/google/service-account.json \
  testflying-connector:local
```

如果只有拆分字段，也可以不挂载 JSON 文件：

```bash
docker run -d --name testflying-connector-google-a \
  -p 8100:8100 --memory=64m \
  -e TESTFLYING_CONNECTOR_STORE_MODE=live \
  -e TESTFLYING_CONNECTOR_DEVELOPER_ACCOUNT_ID=google-account-a \
  -e TESTFLYING_CONNECTOR_TOKEN='<random-token>' \
  -e TESTFLYING_CONNECTOR_GOOGLE_CLIENT_EMAIL='service-account@project.iam.gserviceaccount.com' \
  -e TESTFLYING_CONNECTOR_GOOGLE_PRIVATE_KEY='-----BEGIN PRIVATE KEY-----\n...\n-----END PRIVATE KEY-----' \
  testflying-connector:local
```

## 接口

- `GET /health`
- `POST /v1/preflight`
- `GET /v1/apps/{app_id}/supported-locales`
- `GET /v1/apps/{app_id}/store-listings`
- `GET /v1/apps/{app_id}/store-images`
- `GET /v1/apps/{app_id}/store-releases`
- `POST /v1/sync-runs`

除 `/health` 外，所有接口都需要：

```http
Authorization: Bearer <connector-token>
```

`live` 模式当前能力：

- Apple / iOS：支持版本预检查、版本支持语言、版本说明同步、文字类商店元数据同步、商店图同步、自定义产品页面和产品页面优化接口。
- Google / Android：支持连接预检查、商店 listing 支持语言、商店 release 查询、文字类商店元数据同步、商店图同步、版本说明同步。
- Google / Android 版本说明同步可以显式传 `storeRelease.track` / `storeRelease.versionCode`；未传时会从 Google Play tracks 中自动选择最高 `versionCode` 的 release。

## 限流

`GET /health` 不限流。其它商店接口按平台限流：

- Google / Android 默认 `200` 次 / `60` 秒。
- Apple / iOS 根据 Apple 返回的 `X-Rate-Limit` 中 `user-hour-lim` 下调 20% 后执行；未拿到响应头前使用 fallback `2880` 次 / 小时。

可用环境变量覆盖：

- `TESTFLYING_CONNECTOR_GOOGLE_RATE_LIMIT_MAX_REQUESTS`
- `TESTFLYING_CONNECTOR_GOOGLE_RATE_LIMIT_WINDOW_SECONDS`
- `TESTFLYING_CONNECTOR_APPLE_RATE_LIMIT_FALLBACK_MAX_REQUESTS`
- `TESTFLYING_CONNECTOR_APPLE_RATE_LIMIT_WINDOW_SECONDS`
- `TESTFLYING_CONNECTOR_APPLE_RATE_LIMIT_SAFETY_RATIO`

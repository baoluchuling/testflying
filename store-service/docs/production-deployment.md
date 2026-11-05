# 独立 Store API 生产部署

本次部署使用 Linux amd64 静态程序与 systemd 单实例，减少服务器上的构建开销。仓库中的 Docker/Compose 部署仍可独立使用。

文档中的 `store-api.example.com` 为占位域名，真实部署地址从私密运维配置获取，不提交到 GitHub。HTTP 和 HTTPS 入口并存；HTTP 调用无需证书，HTTPS/WSS 继续验证证书。

## 服务与文件

| 项目 | 值 |
|---|---|
| HTTP 基地址示例 | `http://store-api.example.com/store-api` |
| HTTPS 基地址 | `https://store-api.example.com/store-api` |
| 健康检查 | `/health`，无需认证 |
| 本机监听 | `127.0.0.1:8200` |
| systemd 单元 | `testflying-store-api.service`，开机自启 |
| 发布目录 | `/opt/testflying-store-api/versions/{部署提交}` |
| 当前版本 | `/opt/testflying-store-api/current` |
| 注册表 | `/var/lib/testflying-store-api/registry.sqlite3` |
| 私密配置 | `/etc/testflying-store-api/service.env`，root 可读 |
| 代理配置 | `/etc/bugsmith/zentao-tls/nginx.conf` 中 `/store-api/` 路径 |
| 原代理配置备份 | `/opt/testflying-store-api/nginx.before-store-api.conf` |

管理员 Token、应用 API Token、节点签名密钥分别生成。签名密钥与注册表必须同时保留；更换签名密钥会使所有已发节点凭据失效。商店凭据只由账号 Connector 本机提供，不放在此服务器配置中。

服务器使用现有私有 CA 签发的 IP 证书。公开 CA 证书在部署完成后保存为 `deploy/testflying-prod-ca.crt`；它不包含私钥。客户端应信任该 CA，curl 用 `--cacert`，Connector 可用 `SSL_CERT_FILE` 指定证书路径。

## HTTP 调用

```sh
BASE_URL=http://store-api.example.com/store-api
curl --fail "$BASE_URL/health"
curl --fail "$BASE_URL/v1/store-management/apps/ios/com.example.app/store-listings?version=1.2.0" \
  -H "Authorization: Bearer $STORE_API_TOKEN"
```

HTTP 不需要 `--cacert`，也不需要关闭 HTTPS 校验；Token 和正文在此入口明文传输。图片上传仍按申请、PUT 文件、sync 的顺序，HTTP 申请返回 HTTP 的 uploadUrl。新节点安装配置保留 HTTPS serverUrl，由 Connector 建立 WSS。

代理的 HTTP server 配置见 [http-store-api.conf](../deploy/http-store-api.conf)，只开放 `/store-api/`，其他 HTTP 路径返回 404。此配置使用 [Nginx sub_filter](https://nginx.org/en/docs/http/ngx_http_sub_module.html) 仅适配生成的 uploadUrl 字段；修改前必须确认模块可用并通过 nginx -t。现有受管运维脚本 [enable_http.py](../scripts/enable_http.py) 用于首次启用，保留配置备份、失败回滚，并通过模拟节点验证完整 HTTP 上传流程；已启用时拒绝覆盖，更新需单独审阅。

## 无后台页面的节点安装

从 root 可读的 `service.env` 在可信操作环境获取 `STORE_API_ADMIN_TOKEN`，设置为调用端 `ADMIN_TOKEN`。不要把配置或完整创建响应写入公共日志。

```sh
BASE_URL=https://store-api.example.com/store-api
CA_FILE=/absolute/path/to/testflying-prod-ca.crt

curl --fail --cacert "$CA_FILE" "$BASE_URL/health"

curl --fail --cacert "$CA_FILE" -X POST "$BASE_URL/v1/connectors" \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"name":"Apple account A"}'
```

把响应中的 `connector.connectorId` 设为 `CONNECTOR_ID`，下载对应平台的 ZIP：

```sh
curl --fail --cacert "$CA_FILE" \
  "$BASE_URL/v1/connectors/$CONNECTOR_ID/installer/darwin-arm64" \
  -H "Authorization: Bearer $ADMIN_TOKEN" -o connector.zip
```

包内已有 `connectorId`、节点 Token 和正确的 `serverUrl`。解压后在 `connector.json` 的 `provider` 中配置 Apple/Google 凭据路径。

节点身份是运行时配置，不编译进程序。每个平台共用一份已发布二进制；新增节点只生成配置并重新打 ZIP。更换身份或重置 Token 后替换 `connector.json` 并重启即可，无需编译。

加密/上传版本的配置还包含独立 messageKey、uploadKey，原节点升级时需要更新程序及配置；ID、Token 和应用映射保留。Connector 同时提供 HTTP 和 WSS，可选 baseUrl 在节点本地配置并随加密认证上报，创建节点不传 mode 或 baseUrl。

运行示例：

```sh
SSL_CERT_FILE="$CA_FILE" ./start.sh
```

首次上线发布目标为 `linux-amd64` 和 `darwin-arm64`，Windows x64 的后续发布记录见 `windows-release-verification.json`。Intel Mac、Linux arm64 需要另行构建并发布对应二进制，未发布时下载接口返回 503。一个账号创建一个节点，再通过管理接口将多个应用映射到它，见 [API 参考](api-reference.md)。

Windows 下载接口使用相同节点身份与管理员认证，只把目标设为 `windows-amd64`：

```sh
curl --fail --cacert "$CA_FILE" \
  "$BASE_URL/v1/connectors/$CONNECTOR_ID/installer/windows-amd64" \
  -H "Authorization: Bearer $ADMIN_TOKEN" -o connector-windows.zip
```

接口 ZIP 含 `store-connector.exe`、`connector.json` 和 `start.cmd`。使用私有 CA 时，可在 PowerShell 中先设置 `$env:SSL_CERT_FILE` 为 CA 文件的绝对路径，再执行 `./start.cmd`。本次额外交付的 `releases/windows-amd64/connector-windows.zip` 已附公开 CA、自动设置证书路径的启动脚本和说明，内含一个独立启用、待配置账号的节点身份。这个私密 ZIP 不进入 Git，也不要公开分享。Windows 发布验证覆盖 PE x64 格式、系统 DLL、线上 ZIP 和身份配置，尚未覆盖 Windows 实机运行。

## 运维与验收

```sh
ssh testflying-prod 'systemctl status testflying-store-api --no-pager'
ssh testflying-prod 'journalctl -u testflying-store-api -n 50 --no-pager'
```

源码验证见 `verification.json`，发布程序来源与 SHA-256 见 `deployment-build.json`。本次线上实际验收结果见 `deployment-verification.json`，只有完整验收成功才写入该文件。验收节点在结束时禁用、断开并解除应用映射，不配置真实账号、不执行商店写入。

`scripts/build_release.py` 在受信 artifact-run 内构建 Linux amd64 与 macOS arm64 发布程序，传 `windows` 只构建 Windows x64；工具链、Cargo、Zig 和临时配置均位于托管根。`scripts/deploy_prod.py` 通过已验证 SSH 上传正式发布文件，执行 `deploy/install-prod.sh` 首次安装并做外部 HTTPS、安装包和真实 WSS 连接验收。首次安装脚本拒绝覆盖已有安装；后续升级应先检查当前版本、注册表备份和代理状态，再做版本切换。`scripts/publish_windows.py` 额外发布 Windows 程序并验证下载，不重启 API。

首次安装前需在调用环境设置真实 `STORE_API_PUBLIC_URL`，部署脚本会传入远程安装过程；源码中的占位地址不能直接用于部署。现有安装的真实配置与节点 JSON 保留在各自私密位置，不因公开文档脱敏而更改。

部署只新增独立 API。现有 Testflying Python 后台和 Go Connector 继续独立运行，尚未迁移调用新服务。真实 Apple/Google 账号能力仍需接入账号节点后联调。

## 上传反向代理配置

本地图片经中心转发时，Store API 前的 Nginx 必须设置 `client_max_body_size 64m`、`proxy_request_buffering off` 和 HTTP/1.1，防止旧的 2m 代理限制提前拒绝文件或默认缓冲把图片写入中心磁盘；`proxy_read_timeout 1860s` 覆盖最大 30 分钟上传窗口，并预留 60 秒响应余量。业务 JSON 仍由 Rust 的 2 MiB 限制校验。只修改 Store API 路径，保留其他应用的配置。上传 API 见 [图片上传文档](image-upload-api.md)。

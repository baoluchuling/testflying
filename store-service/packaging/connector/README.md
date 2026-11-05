# Store Connector v0.1.0

通用 Rust Connector。Linux 包适用于 x86_64；macOS 包适用于 Apple Silicon arm64；Windows 包适用于 x64。

## 首次使用

1. 在中心创建节点，下载该节点的安装配置，保存为程序同目录的 `connector.json`。
   管理员接口：`POST /v1/connectors` 创建，`GET /v1/connectors/{id}/installation` 获取配置。
2. 在本机配置 Apple / Google 商店账号凭据，使用本机可访问的凭据文件路径。
3. Windows 双击 `start.cmd`；Linux/macOS 执行 `./start.sh`。

包里的 `connector.example.json` 和 `connector-google.example.json` 仅为占位模板，不能直接连接中心。真实 connectorId、token、messageKey、uploadKey 和 serverUrl 由中心生成，无需为每个节点重新编译。

当前生产中心使用的公开 CA 为 `testflying-prod-ca.crt`。未显式设置 SSL_CERT_FILE 时，启动脚本会使用此文件。连接其他中心时，可自行指定对应的 SSL_CERT_FILE。包内没有任何节点身份或商店账号凭据。

## 升级已有节点

停止旧程序，替换 `store-connector` / `store-connector.exe`，保留现有 `connector.json` 与账号凭据，再启动。不要用占位模板覆盖真实配置。已有 v2 配置格式无需修改。

## 当前能力

- 同时支持 HTTP 和主动 WSS；WSS 业务消息另有 Noise 会话加密。
- 本地图片通过中心流式转发，单图最多 64 MiB。
- 上传凭据有效期 30 分钟，已认证上传有独立的 30 分钟传输窗口。
- 完成图片保留 15 分钟供 sync 使用，消费或过期后释放。

## 验证范围

已通过 42 项 Rust 测试和 26 项生产验收检查，包含 HTTPS、实际 macOS Connector 的加密 WSS、中心图片转发、完整性校验和资产消费。Linux 程序为静态 x86_64 ELF；Windows 程序已验证 PE x64 和系统 DLL 依赖，尚未在 Windows 实机执行。macOS 程序未进行 Apple 代码签名或公证。不代表已验证真实商店账号的所有操作。

发布页的 SHA256SUMS 文件可校验压缩包完整性。

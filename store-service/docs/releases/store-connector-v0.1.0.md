# Store Connector v0.1.0（Rust）

独立商店服务的通用 Connector，支持中心自动定位节点、HTTP/WSS 同时运行、WSS 业务消息加密及中心流式转发图片。上传凭据与单次传输窗口均为 **30 分钟**。

| 文件 | 适用系统 |
|---|---|
| `store-connector-v0.1.0-linux-amd64.tar.gz` | Linux x86_64，静态程序 |
| `store-connector-v0.1.0-darwin-arm64.tar.gz` | macOS Apple Silicon（arm64） |
| `store-connector-v0.1.0-windows-amd64.zip` | Windows x64 |
| `store-connector-v0.1.0-SHA256SUMS.txt` | 三个平台压缩包的 SHA-256 |

每个包包含通用程序、配置模板、启动脚本、说明和公开 CA，**不含真实节点 Token、密钥或账号凭据**。首次使用需从中心获取节点的 `connector.json`；已有 v2 节点升级时仅替换程序，保留原配置。新增节点无需重新编译。

已通过 42 项 Rust 测试、26 项线上验收检查，并实际验证 3 MiB 图片经过 HTTPS 中心和加密 WSS 到节点、校验完整性并在同步后消费。Windows 尚未实机执行，macOS 程序未进行 Apple 签名或公证；未宣称全部真实商店操作均已实测。

本版本为独立 Rust 商店 Connector，不替换旧 Testflying 后台的默认 Latest Release。

# Go 版 testflying-connector 设计

## 背景

`testflying-connector` 只是开发者账号旁边的商店 API 中转器，不应该承担中心后台、数据库或文件存储职责。当前 Python/FastAPI 版本已经跑通了中心后台协议，但运行时依赖和镜像体积偏重，不适合一台资源较弱的跳板机上部署多个账号 connector。

## 目标

- 用 Go 重写 `connector/` 子项目，生成单二进制镜像。
- 保持中心后台现有 connector 协议兼容。
- 支持 `mock` 和 `live` 两种模式：本地默认 `mock`，生产显式开启 `live`。
- 一个 connector 只绑定一个 `developer_account_id`。
- 商店凭据只存在 connector 部署环境，中心后台只保存 connector URL 和调用 token。
- 保留平台限流：Google 默认 200 次/分钟，Apple 使用 fallback 或接口返回限额的 80%。

## 非目标

- 不在 connector 中保存同步任务状态。
- 不引入 PostgreSQL、MinIO 或本地数据库。
- 不在中心后台保存 Apple `.p8`、Google service account JSON 或商店访问 token。
- 不做截图上传、自动提审、跨账号批量同步。

## 架构

Go connector 使用 `net/http` 提供 HTTP 服务，内部拆分为配置读取、鉴权、请求模型、限流、商店客户端和路由处理。`mock` 模式保留当前可运行示例逻辑，用于本地和无凭据测试；`live` 模式会读取 Apple / Google 凭据，生成 Apple JWT 或 Google OAuth access token，并作为后续真实商店调用的基础。

中心后台仍调用以下端点：

- `GET /health`
- `POST /v1/preflight`
- `GET /v1/apps/{app_id}/supported-locales`
- `POST /v1/sync-runs`

## 凭据配置

### Apple

Apple 使用 App Store Connect API Key。部署 connector 前需要在 App Store Connect 的 Users and Access / Integrations 中创建 API key，并记录：

- Issuer ID
- Key ID
- 下载的 `.p8` 私钥文件

建议 key 角色先使用 `App Manager`，因为第一版需要读取和修改 App Store 信息、版本本地化和商店文案。

生产配置示例：

```bash
TESTFLYING_CONNECTOR_STORE_MODE=live
TESTFLYING_CONNECTOR_DEVELOPER_ACCOUNT_ID=apple-account-a
TESTFLYING_CONNECTOR_TOKEN=<random-token>
TESTFLYING_CONNECTOR_APPLE_ISSUER_ID=<issuer-id>
TESTFLYING_CONNECTOR_APPLE_KEY_ID=<key-id>
TESTFLYING_CONNECTOR_APPLE_PRIVATE_KEY_PATH=/run/secrets/apple/AuthKey_XXXX.p8
```

### Google

Google 使用 service account。部署 connector 前需要：

- 创建或选择 Google Cloud Project。
- 启用 Google Play Developer API。
- 创建 service account。
- 在 Google Play Console 的用户与权限中邀请该 service account，并给目标 App 授权。
- 下载 service account JSON。

生产配置示例：

```bash
TESTFLYING_CONNECTOR_STORE_MODE=live
TESTFLYING_CONNECTOR_DEVELOPER_ACCOUNT_ID=google-account-a
TESTFLYING_CONNECTOR_TOKEN=<random-token>
TESTFLYING_CONNECTOR_GOOGLE_SERVICE_ACCOUNT_JSON_PATH=/run/secrets/google/service-account.json
TESTFLYING_CONNECTOR_GOOGLE_DEVELOPER_ID=<optional-developer-id>
```

## 错误处理

- token 不匹配返回 `401`。
- 请求账号和 connector 绑定账号不一致返回 `403`。
- 限流返回 `429`，带 `Retry-After`。
- 凭据缺失或格式错误时，`health` 返回可读的凭据状态；商店接口在 `live` 模式下返回友好的失败摘要。
- connector 不输出完整密钥、Authorization header 或 service account JSON。

## 验证

- Go 单元测试覆盖鉴权、账号校验、预检查、同步响应、限流和凭据读取。
- CI 同时运行 Python 后端测试和 Go connector 测试。
- Docker 构建产物应为 Go 二进制镜像，运行端口保持 `8100`。

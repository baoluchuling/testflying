# Independent Store API

独立的 Rust 商店 API 与账号级 Connector。Testflying 和其他后台可以按 `platform + Bundle ID/包名` 调用，同一个账号的多个应用映射到一个 Connector。

本目录可独立构建、部署，不依赖 Testflying Python 数据库。现有 `server/` 后台、页面、旧接口与 Go Connector 本次保持原样，尚未切换到此服务；新旧服务不要同时对同一个账号执行写操作。

服务已独立部署，当前发布与验收记录见 [生产部署说明](docs/production-deployment.md)和[上传部署验收](docs/upload-deployment-verification.json)。新服务与旧服务使用独立凭据，详见下方配置说明。

业务 API 已支持 HTTP 与 HTTPS。HTTP 无需证书参数，上传凭据申请也返回 HTTP 上传地址；Connector 仍使用原有 HTTPS/WSS 配置。公开文档统一使用占位域名 `store-api.example.com`，真实地址由部署方私下提供。HTTP 会明文传输 Token 和业务内容。

新 Connector 源码在 [`crates/store-connector`](crates/store-connector)，它只依赖本目录的共享协议和商店适配。旧 Go Connector 在 [`../server/connector`](../server/connector)，有自己的 Go 模块与镜像。迁移时可整体复制 `store-service/`；GitHub Actions 入口留在仓库根，运行目录仍是 `store-service/`。

通用 Connector 安装包：[Store Connector v0.1.0](https://github.com/baoluchuling/testflying/releases/tag/store-connector-v0.1.0)，提供 Linux x64、macOS Apple Silicon arm64 和 Windows x64。包中只含配置模板，节点专属配置通过中心单独获取。已有 v2 节点升级时保留原 `connector.json`，替换程序即可。包内说明见 [packaging/connector/README.md](packaging/connector/README.md)。

```text
Testflying / 其他管理后台
          │ platform + Bundle ID/包名
          ▼
      Store API ── 应用映射 ── Connector 身份
          │                       │
          ├── HTTP 直连集群节点 ────┤
          └── 本地节点主动 WSS ────┘
                                  │ 账号凭据 / 本机出口
                                  ▼
                             Apple / Google
```

## 边界

- Store API 只保存 Connector 注册表、Token 签发 nonce 和应用映射。不保存文案、页面、截图、发布草稿或任务队列。
- 一个 Connector 代表一个商店账号。商店私钥和 Google service account JSON 仅配置在该节点。
- 外部操作不需要中心账号 ID、内部应用 ID 或 Apple 数字 App ID；iOS 节点按 Bundle ID 查询真实 App ID。
- API 端和 Connector 都按账号限制同一时刻一条执行请求，忙碌时返回 409。
- 断线、错误和超时不会自动重试或重放。用户重新触发是一次新请求。超时不能证明商店未修改，调用方应先查询商店状态。
- 已经开始的节点操作会完成必要清理，即使调用方已断开；期间保持账号忙碌，结果丢弃。进程突然退出仍可能留下待到期的 Google edit 或 Apple 部分修改。
- 单个 Store API 实例拥有所有 WSS 连接。当前部署使用单实例；不能把多副本随机负载均衡当成支持跨进程路由。

## 工程

| Crate | 职责 |
|---|---|
| store-protocol | 应用标识、操作、请求关联和统一错误 |
| store-provider | Apple/Google 身份认证、商店读写、限流和图片处理 |
| store-api | 注册表、应用映射、鉴权、HTTP/WSS 路由与节点安装包 |
| store-connector | 账号本地执行，支持 HTTP 服务与主动 WSS |

业务接入先看 [商店业务与图片上传接口](docs/store-api.md)，包含 9 个商店操作和 2 个上传接口。含节点管理与通信协议的完整中文文档见 [api-reference.md](docs/api-reference.md)，机器可读定义见 [openapi.yaml](docs/openapi.yaml)。能力与测试对应见 [provider-capabilities.md](docs/provider-capabilities.md)，验证和清理记录见 [verification.md](docs/verification.md)。Apple 版本说明和商店文案/截图、Google listings/版本说明/截图、评论、Apple CPP/PPO 均由本地 Rust Provider 执行，不需要 Ruby、fastlane 或旧 Go 进程。

## API 服务配置

| 环境变量 | 说明 |
|---|---|
| STORE_API_ADMIN_TOKEN | 管理注册表与获取安装包的 Token，至少 16 字节 |
| STORE_API_TOKEN | 两个后台调用商店操作的 Token，至少 16 字节 |
| STORE_API_NODE_SIGNING_KEY | 至少 32 字节的持久随机密钥，签发节点 Token |
| STORE_API_PUBLIC_URL | 节点能访问的 API 基础地址，例如 https://store.example.com |
| STORE_API_LISTEN | 默认 0.0.0.0:8200 |
| STORE_API_DATABASE | 默认 store-registry.sqlite3，Compose 使用 /app/data/registry.sqlite3 |
| STORE_API_REQUEST_TIMEOUT_SECONDS | 默认 60，允许 1–300 秒 |
| STORE_API_CONNECTOR_RELEASE_ROOT | 已编译节点二进制根目录，默认 releases |

用运维密钥系统提供上述三个独立随机密钥。备份注册表时同时备份节点签发密钥；丢失或更改签发密钥会使现有节点 Token 失效。

生产首次安装会分别随机生成这些密钥，不复用旧服务的 `TESTFLYING_STATIC_TOKEN` 或 `TESTFLYING_CONNECTOR_TOKEN`，新服务也不读取这两个变量。`STORE_API_TOKEN` 用于业务调用，`STORE_API_ADMIN_TOKEN` 用于节点与映射管理，两者必须不同；每个 Connector 的节点 Token 由新服务单独签发，只用于节点认证。停用旧 Token 不影响新服务；撤销旧凭据或下线旧服务需要在旧服务侧单独处理。

Docker Compose 从本目录启动，读取已设置的环境变量：

```sh
docker compose -f compose.yml up -d --build
```

Compose 只构建并运行 `store-api` 镜像。Connector 镜像可在部署环境单独构建：

```sh
docker build --target store-connector -t testflying-store-connector:local .
```

Connector 镜像默认读取 `/app/connector.json`，每个账号挂载自己的只读配置及凭据文件，并确保容器运行用户有读取权限。API 镜像不再内置 Connector 二进制；下载节点 ZIP 仍需将对应平台的发布二进制放入 `releases/`。

本地受本工作区 AGENTS.md 约束的构建和测试必须通过 artifact-run，见下方“验证”。上面的 Compose 命令用于容器部署；当前生产采用下方文档中的 systemd 部署。

## 创建节点和应用映射

服务器原生 systemd 部署、HTTPS 地址、凭据位置和无后台下载安装步骤见 [生产部署说明](docs/production-deployment.md)。

以下 `BASE_URL`、`ADMIN_TOKEN`、`API_TOKEN` 为调用端已配置环境变量；示例不包含真实密钥。

创建本地 WSS 节点：

```sh
curl -X POST "$BASE_URL/v1/connectors" \
  -H "Authorization: Bearer $ADMIN_TOKEN" -H 'Content-Type: application/json' \
  -d '{"name":"Apple account A"}'
```

创建节点只传名称，不再选择连接模式或填写节点地址。需要中心 HTTP 回退时，在节点本机 connector.json 的 baseUrl 设置中心可达的节点地址，它与认证信息一起上报：

```json
{"baseUrl":"https://connector-b.example.com"}
```

创建响应包含 `connector` 和 `installation`。前者是脱敏节点信息，后者包含运行时 `connectorId`、`token`、`serverUrl`、独立的 `messageKey`/`uploadKey` 及初始 Provider 配置。普通节点列表不返回秘密。新增节点只生成配置并打 ZIP，不重新编译程序。

映射应用，多个应用可以传同一个 `CONNECTOR_ID`：

```sh
curl -X PUT "$BASE_URL/v1/apps/ios/com.example.app/connector" \
  -H "Authorization: Bearer $ADMIN_TOKEN" -H 'Content-Type: application/json' \
  -d "{\"connectorId\":\"$CONNECTOR_ID\"}"
```

相同平台和标识只保留一个映射；再次 PUT 会明确重新绑定。大小写精确匹配，iOS 和 Android 的同名标识互不影响。DELETE 同一路径解除映射，GET 读取映射。

节点管理：

| 方法/路径 | 行为 |
|---|---|
| GET /v1/connectors | 脱敏列表；connected 仅表示当前 WSS 连接，不是 HTTP 健康探测 |
| GET /v1/connectors/{id} | 脱敏详情 |
| PATCH /v1/connectors/{id} | `{"enabled":false}` 禁用并断开 WSS；设 true 恢复资格 |
| POST /v1/connectors/{id}/rotate-token | 重新签发 Token，断开 WSS；节点须更换安装配置 |
| GET /v1/connectors/{id}/installation | 获取私密安装 JSON，Cache-Control: no-store |
| GET /v1/connectors/{id}/installer/{target} | 下载二进制、connector.json、启动脚本组成的 ZIP |

## 安装包与节点运行

安装包复用相同平台的发布二进制，不为每个账号重新编译。将 CI 的节点产物放到：

```text
releases/linux-amd64/store-connector
releases/linux-arm64/store-connector
releases/darwin-amd64/store-connector
releases/darwin-arm64/store-connector
releases/windows-amd64/store-connector.exe
```

API 只读取上述允许目标下的二进制。目标未发布返回 503；它不会触发编译、下载第三方程序或返回缺失的假安装包。当前已发布 Linux amd64、macOS arm64 和 Windows amd64。CI 另配置了 macOS amd64 构建任务，但不代表该目标已经发布；Linux arm64 需要额外构建。

解压 ZIP 后，在 `connector.json` 的 `provider` 中配置本机商店凭据路径。Apple 示例见 [examples/connector.json](examples/connector.json)，Google 示例见 [examples/connector-google.json](examples/connector-google.json)。安装包中固化的是节点身份，商店凭据由账号节点本地提供，不经过 Store API。

运行 `start.sh` / `start.cmd`，或直接执行：

```sh
./store-connector connector.json
```

Connector 同时监听 listenAddr 并使用 serverUrl 主动建立 WSS。中心先验证 Bearer Token，再完成带节点独立密钥的 Noise 会话认证；baseUrl 与认证资料一起提交，仅在密钥确认成功后登记。消息以二进制分块加密，拒绝明文和旧版协议。中心优先在线 WSS，没有 WSS 时使用已登记 HTTP 地址；失败不换通道重试。nodeId 即独立 connectorId，与本机 IP 无关，同一身份第二条活动 WSS 被拒绝。HTTP 回退的传输加密依赖 baseUrl 使用 HTTPS。

本版本增加 messageKey/uploadKey 和加密协议，已有节点需重新下载配置及对应平台程序；ID、Token 和应用映射保留。安装配置的分发仍依赖可信管理员 HTTPS 渠道。Noise 使用 snow 实现，其项目未声明经过正式安全审计，不将本次功能测试等同于密码学审计。

## 本地图片经中心上传

外部调用始终访问中心，上传 URL 不指向节点，不要求先上传对象存储。准备上传取得短时签名凭据，再 PUT 本地文件字节到中心；中心按块转发给 Connector，随后 sync 引用 assetId。上传凭据不入库，只有验签密钥长期保存在节点配置；中心不落盘，节点使用匿名临时文件，消费结束或超时清理。单张最大 64 MiB，2 MiB 仅是业务 JSON 上限。

完整外部参数与 curl 示例见 [图片上传接口](docs/image-upload-api.md)。

住宅 VPN 只负责商店出口时，应让公司内网/服务器网段走直接路由。控制面不使用环境 HTTP 代理；Provider 支持系统代理环境变量或可选 `storeProxyUrl`，商店流量也可直接由系统 VPN 路由。WSS 反向代理需转发 Upgrade，空闲超时应大于 35 秒；两端每 10 秒发送心跳。

## 实时商店接口

统一前缀：`/v1/store-management/apps/{platform}/{bundleIdentifier}`。下列操作使用 `Authorization: Bearer $API_TOKEN`。

| 方法/后缀 | 内容 |
|---|---|
| GET supported-locales | 语言 |
| GET store-listings | 商店文案 |
| GET store-images | 截图/图片引用 |
| GET store-releases | Google tracks/releases；iOS 与旧实现一致返回空数组 |
| GET store-reviews | 评论、上游游标及页内筛选 |
| GET product-page-optimizations | Apple PPO 实验 |
| POST product-page-optimizations | 创建 Apple PPO 实验与 treatments |
| POST preflight | 查询当前可写条件 |
| POST sync | 直接同步请求中的内容，不读取/保存草稿 |

iOS 查询版本文案/截图/版本语言时传 `version`；不传版本只返回应用级语言/名称等信息。评论支持 pageSize/pageToken/startIndex/translationLanguage/sort，以及 rating/locale/territory/date/startDate/endDate/timezone；筛选作用于当前上游页，保留下一页游标。日期默认 Asia/Shanghai。

直接更新指定字段，缺失字段保持原值，显式空字符串用于清空：

```sh
curl -X POST "$BASE_URL/v1/store-management/apps/ios/com.example.app/sync" \
  -H "Authorization: Bearer $API_TOKEN" -H 'Content-Type: application/json' \
  -d '{"operation":"update_app_metadata","version":"1.2.0","locale":"en-US","syncScopes":["description","promotional_text"],"metadata":{"description":"New description","promotionalText":""}}'
```

Google 版本说明：

```json
{
  "operation":"update_release_notes",
  "locale":"en-US",
  "releaseNotes":"Fixes and improvements",
  "storeRelease":{"track":"production","versionCode":"123"}
}
```

未指定目标时选择现有最高 versionCode 的 release。提交 edit 始终带 `changesNotSentForReview=true` 与 `changesInReviewBehavior=ERROR_IF_IN_REVIEW`，不会主动送审或发布。

截图通过 `metadata.storeImages` 或 CPP 的 `marketingPage.storeImages` 提供 URL/asset；支持 `phone_screenshots`、`tablet_screenshots`、`feature_graphic_url`，槽值格式为 `{"urls":["https://assets.example.com/shot.png"]}` 或 `{"assets":[{"downloadUrl":"...","fileName":"shot.png"}]}`。`syncScopes:["store_images"]` 仅更新显式指定的槽。节点按大小/格式/像素边界验证；商店的具体设备尺寸仍由上游校验。

Apple CPP 使用 `operation:"update_marketing_page"`，携带 `marketingPage:{pageName,applePageId,promotionalText,storeImages}`；不传 applePageId 时按当前应用最近版本建立 CPP。已有 CPP 必须属于定位到的应用。PPO 创建内容为 `{name,trafficProportion,locales,treatments:[{name,appIconName,locales}]}`。以上写入均直接作用于商店，Apple 多步骤操作不能保证整体回滚。

旧草稿导入路径 `metadata-content-sets`、`store-versions/.../draft` 和 `marketing-pages` 在新服务不存在。新服务也不提供测试包、构建、设备或后台页面。

## 验证

从 `store-service/` 目录使用通过门禁的托管执行：

```sh
"$HOME/Library/Application Support/CodexArtifactLifecycle/bin/artifact-run" \
  --adapter generic --project "$PWD" \
  -- python3 -B scripts/verify_store_refactor.py rust
```

脚本先证明托管根和只读 ownership marker，再把 Cargo 下载、target、测试临时文件放入该 run 的临时目录。执行 fmt、全 workspace tests、clippy -D warnings，并分别构建 API 与 Connector 的本机 release 二进制；成功即清理，失败只保留有期限的紧凑证据。Cargo.lock 是保留的正式依赖锁文件，[verification.json](docs/verification.json) 保存最近一次成功检查的命令、测试摘要、源码哈希和 run ID。

fake upstream 测试使用临时生成的 JWT 私钥与进程内 loopback 服务，不访问真实 Apple/Google。已有验证记录覆盖 42 项 Rust 测试、Linux/macOS/Windows 发布构建和 26 项线上部署检查；线上上传验收使用模拟商店节点。Docker 构建、完整跨平台 CI 与真实 Apple/Google 账号联调仍需在对应环境验证，详见 [verification.md](docs/verification.md)。

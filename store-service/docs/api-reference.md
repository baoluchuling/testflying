# 商店 API 接口文档

版本：0.1.0。本文对应 `store-service/` 当前 Rust 实现；线上版本见 [生产部署记录](production-deployment.md)，现有 Testflying 后台尚未接入。示例均为接口格式示意，不是线上账号数据。

外部后台统一使用 [中心统一接口文档](store-api.md)，包括 10 个商店、2 个商店节点、6 个截图节点／任务接口，以及 locale/device、iOS 版本管理和图片上传。

中心同时提供带接入 Token 或业务 Token 的 `GET /v1/store-management/api-docs`：默认返回 JSON 功能目录与完整文档，`?format=markdown` 返回完整 Markdown，`?format=openapi` 返回 OpenAPI YAML。无需应用绑定或在线节点；接入 Token 仍要求 HTTPS。此目录说明中心契约，不实时探测节点的本地平台模式或商店权限。

外部商店、节点和截图接口统一以 [中心统一接口文档](store-api.md) 和在线 api-docs 为准，本页保留底层通信参考。截图节点列表、按包名绑定、触发及状态查询均在统一文档中；[macOS 截图 Connector](screenshot-connector.md) 仅作部署运维说明。

本地文件上传新增两个中心接口：POST `APP_PATH/uploads` 申请短时凭据，PUT `APP_PATH/uploads/{assetId}` 发送文件字节。中心流式转发到 Connector，sync 用 assetId 引用，不需要先上对象存储。参数、限额和完整步骤见 [图片上传接口](image-upload-api.md)。

## 1. 地址、鉴权和应用标识

Store API 默认端口为 **8200**，下文使用：

```text
BASE_URL = http://127.0.0.1:8200
APP_PATH = /v1/store-management/apps/{platform}/{bundleIdentifier}
```

| 参数 | 说明 |
|---|---|
| platform | `ios` 或 `android`，小写 |
| bundleIdentifier | iOS Bundle ID 或 Android 包名，例如 `com.example.reader`；大小写精确匹配 |
| connectorId | 独立节点 UUID，只用于管理和节点连接，普通商店调用不需要传 |

应用标识长度为 1–180 字节，允许字母、数字、点、下划线和短横线，不允许首尾点或连续两个点。同名 iOS/Android 应用分别定位。

一个 Connector 可同时配置一套 Apple 和一套 Google 凭据，两边的多个应用都可以绑定到该节点。应用清单只在中心维护，不写入节点配置。Store API 根据应用标识查映射，再选择 HTTP 或 WSS 执行；不需要调用方提供 `accountId`、内部 `appId` 或 Apple 数字 App ID。

生成包含凭据与相对路径的专属安装包可使用 `/connector-setup/` [下载配置页](connector-setup.md)。页面通过 HTTPS 使用管理员 Token 查询节点、创建节点和获取安装包，账号文件在浏览器本地合并。

所有需认证的请求使用：

```http
Authorization: Bearer TOKEN
Content-Type: application/json
```

| Token | 用途 | 服务配置 |
|---|---|---|
| 应用 API Token | 查询和更新商店 | `STORE_API_TOKEN` |
| 管理 Token | 创建节点、维护映射、下载安装包 | `STORE_API_ADMIN_TOKEN` |
| 接入 Token | 访问全部节点、保存应用绑定及读写商店 | 管理员在 HTTPS 下载页按系统名发放 |
| 节点 Token | Connector 连接或执行协议 | 创建节点时签发，每节点独立 |

应用和管理 Token 必须不同。JSON 字段使用 camelCase；请求体最大 **2 MiB**。普通业务成功响应不额外包装 `data`，示例中的根字段就是实际返回结构。

## 2. 接口总览

### 2.1 商店操作：接入 Token 或应用 API Token

`GET /v1/store-management/connectors` 返回全部启用的商店节点。已有与新发放的接入 Token 均自动涵盖后续新增节点。

以下路径均在 `APP_PATH` 后追加：

| 方法 | 后缀 | 用途 | iOS | Android |
|---|---|---|---|---|
| GET | `/supported-locales` | 查询现有商店语言 | 支持 | 支持 |
| GET | `/store-listings` | 查询商店文案 | 支持 | 支持 |
| GET | `/store-images` | 查询图片和截图引用 | 支持 | 支持 |
| GET | `/store-releases` | 查询 tracks/releases | 当前返回空数组 | 支持 |
| GET | `/store-reviews` | 查询评论、分页及筛选 | 支持 | 支持 |
| GET | `/product-page-optimizations` | 查询 PPO 实验 | 支持 | 当前返回空数组 |
| POST | `/product-page-optimizations` | 创建 PPO 实验 | 支持 | 不支持 |
| POST | `/preflight` | 查询当前写入条件 | 支持 | 支持 |
| POST | `/sync` | 直接更新文案、版本说明、截图或 CPP | 支持 | 文案、版本说明、截图 |

### 2.2 管理操作：管理 Token

| 方法 | 路径 | 用途 |
|---|---|---|
| GET | `/v1/connectors` | 列出节点 |
| POST | `/v1/connectors` | 创建账号节点并签发身份 |
| GET | `/v1/connectors/{id}` | 查询节点详情 |
| PATCH | `/v1/connectors/{id}` | 启用／禁用节点 |
| POST | `/v1/connectors/{id}/rotate-token` | 重置节点 Token |
| GET | `/v1/connectors/{id}/installation` | 获取节点安装配置 |
| GET | `/v1/connectors/{id}/installer/{target}` | 下载带身份配置的 ZIP 安装包 |
| GET | `/v1/connectors/{id}/screenshot-installer` | 通过 HTTPS 下载截图节点的 macOS 部署 ZIP |
| GET | `/v1/apps/{platform}/{bundleIdentifier}/connector` | 查询应用对应节点 |
| PUT | `/v1/apps/{platform}/{bundleIdentifier}/connector` | 设置或更换应用映射，接入 Token 也可通过 HTTPS 调用 |
| DELETE | `/v1/apps/{platform}/{bundleIdentifier}/connector` | 解除映射 |

### 2.3 健康检查和节点协议

| 目标 | 方法 | 路径 | 鉴权 |
|---|---|---|---|
| Store API | GET、POST | `/connector-setup/api/connectors` | HTTPS + 管理员 Token |
| Store API | GET | `/connector-setup/api/connectors/{id}/installer/{target}` | HTTPS + 管理员 Token |
| Store API | GET | `/health` | 无 |
| Store API | GET → WebSocket | `/v1/connectors/{id}/ws` | 节点 Token |
| HTTP Connector | GET | `/health` | 无 |
| HTTP Connector | POST | `/v1/execute` | 节点 Token |

两个 `/health` 都返回 `{"status":"ok"}`，只证明服务存活，不代表真实商店凭据有效。

## 3. 查询商店数据

### 3.1 查询语言

```http
GET /v1/store-management/apps/ios/com.example.reader/supported-locales?version=1.2.0
```

| Query 参数 | 必填 | 说明 |
|---|---|---|
| version | 否 | iOS 版本号；Android 不使用 |

```json
{"locales":["en-US","zh-Hans"]}
```

iOS 指定且找到版本时读取版本语言；不指定或未找到版本时读取应用级语言。Android 返回当前 listings 中已有的语言。这里不是完整的“平台所有可支持语言”字典。

### 3.2 查询文案

```http
GET /v1/store-management/apps/ios/com.example.reader/store-listings?version=1.2.0
```

Query 参数为可选 `version`。iOS 不指定版本时仅有应用级名称、副标题等内容；指定版本后合并该版本描述、关键词、宣传文本和版本说明。

```json
{
  "listings":[
    {
      "locale":"en-US",
      "title":"Example Reader",
      "subtitle":"Read every day",
      "description":"App description",
      "fullDescription":"App description",
      "keywords":"reader,books",
      "promotionalText":"New features available",
      "releaseNotes":"Fixes and improvements"
    }
  ]
}
```

Android 的主要字段为 `locale`、`title`、`shortDescription`、`fullDescription`、`description`、`videoUrl`。平台不提供的字段不保证出现，不能把缺失字段当作主动清空。

### 3.3 查询图片

```http
GET /v1/store-management/apps/ios/com.example.reader/store-images?version=1.2.0
```

iOS 查询版本截图需要 `version`；不指定或版本不存在时 `locales` 为空。Android 不使用版本参数。

```json
{
  "locales":[
    {
      "locale":"en-US",
      "images":{
        "phone_screenshots":[
          {
            "id":"image-id",
            "fileName":"phone.png",
            "url":"https://assets.example.com/phone.png",
            "width":1284,
            "height":2778
          }
        ],
        "tablet_screenshots":[]
      }
    }
  ]
}
```

Android 读取槽包括 `feature_graphic_url`、`phone_screenshots`、`seven_inch_screenshots`、`ten_inch_screenshots`、`tv_screenshots`、`wear_screenshots`，图片字段沿用上游引用，可能包含 `sha1`、`sha256`。iOS 上游有时提供含 `{w}`、`{h}`、`{f}` 等占位符的图片模板 URL，调用方须结合尺寸处理；不能保证所有返回 URL 都是最终渲染地址。

### 3.4 查询 release

```http
GET /v1/store-management/apps/android/com.example.reader/store-releases
```

```json
{
  "releases":[
    {
      "track":"production",
      "name":"1.2.0",
      "status":"completed",
      "versionCodes":["123"],
      "releaseNotes":[{"language":"en-US","text":"Fixes and improvements"}]
    }
  ]
}
```

Android 按各 release 的最高数值 versionCode 倒序返回；同值再按 track 排序。iOS 当前固定返回 `{"releases":[]}`，此接口尚未实现 Apple 版本列表查询。

### 3.5 查询评论

```http
GET /v1/store-management/apps/ios/com.example.reader/store-reviews?pageSize=50&date=2026-10-30&timezone=Asia%2FShanghai&rating=5
```

| Query 参数 | 类型 | 默认／说明 |
|---|---|---|
| pageSize | 正整数 | 默认 50；iOS 最多 200，Android 最多 100；超过上限会截到上限 |
| pageToken | string | 上一次返回的 `nextPageToken` |
| startIndex | string | Android 上游分页起点 |
| translationLanguage | string | Android 上游评论翻译语言 |
| sort | string | iOS 默认 `-createdDate`，直接传上游；Android 当前不使用 |
| rating | integer | 可选，1–5 |
| locale | string | 语言筛选，忽略大小写 |
| territory | string | 地区筛选，忽略大小写；缺少地区的评论无法匹配指定地区 |
| date | YYYY-MM-DD | 指定一天；不能同时传 startDate/endDate |
| startDate | YYYY-MM-DD | 起始日，包含当天 |
| endDate | YYYY-MM-DD | 结束日，包含当天 |
| timezone | IANA 时区 | 默认 `Asia/Shanghai` |

```json
{
  "reviews":[
    {
      "id":"review-id",
      "platform":"ios",
      "rating":5,
      "title":"Useful",
      "body":"Easy to use",
      "authorName":"Reader",
      "reviewerNickname":"Reader",
      "locale":"en-US",
      "territory":"US",
      "appVersion":"1.2.0",
      "createdAt":"2026-10-30T08:00:00Z",
      "updatedAt":"2026-10-30T08:00:00Z"
    }
  ],
  "nextPageToken":"next-cursor",
  "rawPageInfo":{},
  "filters":{
    "startAt":"2026-10-29T16:00:00+00:00",
    "endAt":"2026-10-30T15:59:59.999999+00:00",
    "timezone":"Asia/Shanghai",
    "locale":"",
    "territory":"",
    "rating":5
  }
}
```

筛选只作用于本次从上游取得的这一页，不会额外扫描分页补足条数。因此可能 `reviews` 为空但仍有 `nextPageToken`。游标原样继续传回；为空表示当前没有下一页游标。Android 响应可包含 `appVersionCode`、`originalText`，iOS 可包含 `raw`、`rawPageInfo`。

### 3.6 查询 PPO

```http
GET /v1/store-management/apps/ios/com.example.reader/product-page-optimizations
```

```json
{
  "experiments":[
    {
      "id":"experiment-id",
      "name":"Screenshot experiment",
      "platform":"IOS",
      "state":"PREPARE_FOR_SUBMISSION",
      "trafficProportion":50,
      "reviewRequired":true,
      "startDate":"",
      "endDate":"",
      "treatments":[{"id":"treatment-id","name":"Variant A","appIconName":""}]
    }
  ]
}
```

Android 当前固定返回 `{"experiments":[]}`。当前 PPO 列表不扩展查询每个 treatment 的语言或图片内容。

## 4. 写入前预检查

```http
POST /v1/store-management/apps/ios/com.example.reader/preflight
```

```json
{"operation":"update_app_metadata","version":"1.2.0","locale":"en-US"}
```

| 字段 | 必填 | 说明 |
|---|---|---|
| operation | 是 | update_app_metadata / update_release_notes / update_marketing_page |
| version | 条件必填 | iOS 非 CPP 操作必填；Android 不使用 |
| locale | 否 | 可以传，但当前预检查不强制要求 |

```json
{
  "canSync":true,
  "reasonCode":null,
  "message":"Store content is editable",
  "storeState":{"versionExists":true,"editable":true,"currentStatus":"PREPARE_FOR_SUBMISSION"}
}
```

iOS 根据版本存在性及状态判断；CPP 使用单独的可写检查分支。Android 通过创建并清理临时 edit 检查访问能力。预检查成功不保证随后所有字段、图片或真实商店写入一定成功，也不提交审核或发布。

`canSync:false` 仍可能是 HTTP 200，例如 `reasonCode:"store_version_not_editable"`。调用方必须读取该字段。

## 5. 直接同步内容

```http
POST /v1/store-management/apps/{platform}/{bundleIdentifier}/sync
```

每次请求处理**一个语言、一种操作**。内容从请求直接发送给商店，不读取或保存草稿，也不返回可稍后查询的持久任务 ID。

### 5.1 更新默认商店文案

```json
{
  "operation":"update_app_metadata",
  "version":"1.2.0",
  "locale":"en-US",
  "syncScopes":["description","promotional_text"],
  "metadata":{"description":"Updated description","promotionalText":""}
}
```

| 字段 | 必填 | 说明 |
|---|---|---|
| operation | 是 | 固定 update_app_metadata |
| locale | 是 | 商店语言 |
| version | iOS 必填 | 已存在的商店版本；不自动创建版本 |
| metadata | 是 | 实际更新内容，字段必须为字符串，图片另见 5.3 |
| syncScopes | 否 | 省略时更新请求中提供的受支持文字字段；图片必须显式选 store_images |

| metadata 字段 | 对应 scope | iOS | Android |
|---|---|---|---|
| title | title | 应用名称 | listing 标题 |
| subtitle | subtitle | 副标题 | 不写入 |
| description / fullDescription | description | 优先 description | 优先 fullDescription |
| keywords | keywords | 关键词 | 不写入 |
| promotionalText | promotional_text | 宣传文本 | 可作为 shortDescription 的备用输入 |
| shortDescription | short_description | 不写入 | 简短描述，优先于 promotionalText |
| videoUrl | video | 不写入 | listing 视频链接 |
| storeImages | store_images | 截图 | 图片和截图 |

`metadata` 这个 scope 表示所有受支持文字字段。允许的 scope 还包括 CPP 的 `marketing_text`；应按实际平台和操作选择，不能将“参数被识别”理解为所有平台都会写入该字段。`release_notes` 不是合法 scope，版本说明使用单独 operation。

- 缺失的字段保持原值，选中的字段显式传 `""` 表示清空。
- `syncScopes` 一旦提供，必须是非空的已知字符串数组；不能传字符串、null 或空数组。
- iOS 要求目标版本当前可编辑；新语言会确保应用级与版本级语言配对。创建应用级语言需要名称，缺少本次 title 时尝试使用已有主语言名称；无法取得合法名称会报错。

### 5.2 更新版本说明

iOS：

```json
{"operation":"update_release_notes","version":"1.2.0","locale":"en-US","releaseNotes":"Fixes and improvements"}
```

Android：

```json
{
  "operation":"update_release_notes",
  "locale":"en-US",
  "releaseNotes":"Fixes and improvements",
  "storeRelease":{"track":"production","versionCode":"123","versionName":"1.2.0"}
}
```

`locale`、非空 `releaseNotes` 必填；iOS 还需 `version`。Android 的 `storeRelease` 可省略，规则如下：

| 字段 | 类型 | 用途 |
|---|---|---|
| track | 非空 string | 限定 track，例如 production/internal |
| versionCode | 非空 string | 限定 release 中的 versionCode，必须是字符串，不能传数字 |
| versionName | 非空 string | 限定 release.name |

多个条件同时提供时必须同时满足。省略整个 storeRelease 时选择现有最高数值 versionCode 对应的 release；提供部分条件时在匹配范围内选择。不能传空对象、null 或未知字段。更新时保留其他语言说明及其他 release。

Android edit 提交带 `changesNotSentForReview=true` 和 `changesInReviewBehavior=ERROR_IF_IN_REVIEW`；当前接口不负责送审或发布。

### 5.3 更新截图／图片

使用 update_app_metadata，显式选择 `store_images`：

```json
{
  "operation":"update_app_metadata",
  "version":"1.2.0",
  "locale":"en-US",
  "syncScopes":["store_images"],
  "metadata":{
    "storeImages":{
      "phone_screenshots":{"urls":["https://assets.example.com/phone-1.png"]},
      "tablet_screenshots":{"assets":[{"downloadUrl":"https://assets.example.com/tablet-1.png","fileName":"tablet-1.png"}]}
    }
  }
}
```

| 写入槽 | iOS | Android |
|---|---|---|
| phone_screenshots | 当前手机截图组 | phoneScreenshots |
| tablet_screenshots | 当前平板截图组 | 同时更新 sevenInchScreenshots 和 tenInchScreenshots |
| feature_graphic_url | 不支持 | featureGraphic |

读取槽比写入槽多；当前不能直接写 `tv_screenshots`、`wear_screenshots` 等读取槽。

每槽可以提供 `urls`、`assets` 或两者；两者合计最多 10 张。资产对象允许 `downloadUrl`、`fileName`、`contentType`、`sizeBytes`、`storageKey`；优先使用 downloadUrl，缺失时 storageKey 也必须是可下载的 HTTP(S) URL，不是本地文件或对象存储相对键。

- 更新会替换本次明确给出的槽，没给出的槽保持不变。
- `{"urls":[]}` 或 `{"assets":[]}` 明确清空该槽；`{}`、错误类型或未知槽会被拒绝。
- 当前只接受可完整解码的 PNG/JPEG，单图最大 64 MiB，有解码内存和像素边界；具体商店设备尺寸仍由上游校验。
- 当前图片通过 URL 引用，不提供 multipart 本地上传、素材库或图片草稿接口。

### 5.4 创建／更新 Apple 自定义产品页 CPP

```json
{
  "operation":"update_marketing_page",
  "locale":"en-US",
  "syncScopes":["marketing_text"],
  "marketingPage":{"pageName":"Summer campaign","promotionalText":"Summer reading"}
}
```

| marketingPage 字段 | 必填 | 说明 |
|---|---|---|
| pageName | 是 | 页面名称；创建和更新请求均需传 |
| applePageId | 否 | 不传则新建；传入则更新已有页面，并验证归属当前应用 |
| locale | 否 | 可覆盖顶层 locale；两处至少一处非空 |
| promotionalText | 否 | 宣传文本；可显式空字符串清空 |
| storeImages | 否 | 同 5.3；需显式选择 store_images |

`syncScopes` 省略时处理提供的宣传文本；同步图片需包含 `store_images`。新建基于当前应用最近的已有商店版本。当前不提供 CPP 列表、读取内容、删除、送审或发布接口，也不写入关键词和深链字段。

```json
{
  "status":"succeeded",
  "message":"Store content synchronized",
  "errorCode":null,
  "errorSummary":null,
  "storeState":{"applePageId":"page-id","created":true}
}
```

调用方保存 applePageId，后续通过同一个 sync 接口继续更新。Android 营销页写入不支持，返回业务 failed。

### 5.5 同步响应

成功：

```json
{"status":"succeeded","message":"Store content synchronized","errorCode":null,"errorSummary":null,"storeState":{}}
```

业务拒绝也可能为 HTTP 200：

```json
{"status":"failed","message":"Target store version is not editable","errorCode":"store_version_not_editable","errorSummary":"Target store version is not editable"}
```

其他业务 errorCode 包括 `store_version_missing`、`empty_sync_payload`、`google_release_target_missing`、`google_marketing_page_unsupported`。Android 版本说明成功时 storeState 可包含 track、versionCode、versionName；CPP 成功时包含 applePageId、created。

## 6. 创建 Apple PPO 实验

```http
POST /v1/store-management/apps/ios/com.example.reader/product-page-optimizations
```

```json
{
  "name":"Screenshot experiment",
  "trafficProportion":50,
  "locales":["en-US","zh-Hans"],
  "treatments":[
    {"name":"Variant A","locales":["en-US"]},
    {"name":"Variant B","appIconName":"AlternateIcon"}
  ]
}
```

| 字段 | 必填 | 说明 |
|---|---|---|
| name | 是 | 非空实验名称 |
| trafficProportion | 是 | 整数 1–100 |
| locales | 否 | 非空语言字符串组成的数组，供 treatment 回退 |
| treatments | 否 | treatment 数组；提供时每项 name 非空 |
| treatments[].appIconName | 否 | 图标名称 |
| treatments[].locales | 否 | 省略或空数组时使用顶层 locales |

返回 `{"experiment":{...}}`，字段同 3.6，创建响应中的 treatments 可带 locales。省略 treatments 时当前不会自动创建默认 treatment。此接口只创建配置，不包含实验启动、终止或流量发布操作。Android 返回 422。

## 7. Connector 管理

本节全部使用管理 Token。常规节点对象不包含节点 Token、签发 nonce 或商店凭据：

```json
{
  "connectorId":"636b5d86-4e1b-4f88-aeb9-a1abdf0cdd33",
  "name":"Apple account A",
  "baseUrl":null,
  "enabled":true,
  "appCount":2,
  "connected":true
}
```

`connected` 仅代表已经完成加密认证的活动 WSS 连接，不能用它判断 HTTP 入口是否存活。`appCount` 是当前绑定应用数。节点同时支持 HTTP 和 WSS，无 mode 字段；baseUrl 是节点认证后上报的内部地址，外部业务接口不返回这个地址。

### 7.1 创建节点

```http
POST /v1/connectors
```

创建只需名称：

```json
{"name":"Apple account A"}
```

name 非空且最多 160 字节；mode/baseUrl 不再是创建参数。节点本地配置的可选 baseUrl 与认证一并提交，必须是无用户名、密码、query 和 fragment 的 HTTP(S) 地址。中心优先 WSS，离线时才按已登记地址走 HTTP；执行失败不换通道重试。

成功 HTTP **201**：

```json
{
  "connector":{
    "connectorId":"636b5d86-4e1b-4f88-aeb9-a1abdf0cdd33",
    "name":"Apple account A",
    "baseUrl":null,
    "enabled":true,
    "appCount":0,
    "connected":false
  },
  "installation":{
    "connectorId":"636b5d86-4e1b-4f88-aeb9-a1abdf0cdd33",
    "token":"节点私密Token",
    "serverUrl":"https://store.example.com",
    "messageKey":"节点私密Noise会话密钥",
    "uploadKey":"节点私密上传验签密钥",
    "baseUrl":null,
    "listenAddr":"0.0.0.0:8100",
    "requestTimeoutSeconds":60,
    "provider":{"mode":"live"}
  }
}
```

身份和独立用途密钥都在运行时 JSON 中，不编译进程序；每个平台只编译一次。商店凭据路径仍由节点本地配置，见 [Connector 示例](../examples/connector.json)。普通列表/详情不返回这些密钥。已有节点升级时需重新下载配置和程序；ID、Token、映射保留。

### 7.2 节点列表与详情

```http
GET /v1/connectors
GET /v1/connectors/{id}
```

列表返回 `{"connectors":[节点对象]}`，按名称、ID 排序；详情直接返回节点对象。当前列表没有分页参数。

### 7.3 启用／禁用

```http
PATCH /v1/connectors/{id}
```

```json
{"enabled":false}
```

返回节点对象。禁用后立即移除活动 WSS 连接，新业务请求不再路由到该节点；不撤销已发往真实商店的操作。传 true 恢复连接资格，保留原 Token。

当前 PATCH 只接受 enabled；不支持通过它修改 name、mode 或 baseUrl，也没有节点删除接口。

### 7.4 重置 Token

```http
POST /v1/connectors/{id}/rotate-token
```

无需请求体，返回新的 installation 对象，旧 Token 失效并断开当前 WSS。connectorId 保持不变；节点须使用新配置。禁用状态不会因重置 Token 自动恢复。

### 7.5 获取安装配置

```http
GET /v1/connectors/{id}/installation
```

返回 installation 对象，带 `Cache-Control: no-store`；禁用节点返回 409。普通列表不返回 Token，但管理员可以通过此接口再次取得配置。

### 7.6 下载节点安装包

```http
GET /v1/connectors/{id}/installer/{target}
```

| target | 二进制 |
|---|---|
| linux-amd64 | store-connector |
| linux-arm64 | store-connector |
| darwin-amd64 | store-connector |
| darwin-arm64 | store-connector |
| windows-amd64 | store-connector.exe |

成功返回 `Content-Type: application/zip` 和 `Cache-Control: no-store`。ZIP 包含对应平台二进制、connector.json，以及 start.sh 或 start.cmd。它是解压运行包，不是 MSI/PKG 或自动注册系统服务的安装器。

目标二进制必须预先发布到服务的 release 目录；缺失返回 503 release_unavailable，不会在请求时编译。当前 CI 不自动产出 Linux arm64，需要另行提供该目标二进制。

## 8. 应用与 Connector 映射

查询与解除映射使用管理 Token。设置或换绑也可通过 HTTPS 使用接入 Token。路径如下。

```text
/v1/apps/{platform}/{bundleIdentifier}/connector
```

### 设置或更换：PUT

```json
{"connectorId":"636b5d86-4e1b-4f88-aeb9-a1abdf0cdd33"}
```

成功返回对应节点对象。映射键为 platform + bundleIdentifier；同一个键再次 PUT 会替换绑定，不创建重复映射。多个不同应用可传同一个 connectorId。节点必须存在且启用。

### 查询：GET

返回对应节点对象；应用未映射返回 404 app_not_found，节点禁用返回 503 connector_disabled。

### 解除：DELETE

无请求体，成功返回 HTTP **204**，没有 JSON 响应体。重复解除未存在的映射也返回 204；不会删除 Connector。

当前不提供独立应用目录或应用列表接口；应用由调用后台管理，这里只维护路由关系。

## 9. 节点通信协议

普通业务后台只使用前述商店接口；本节用于 Connector 实现和排障。

### 9.1 WSS 主动连接

```http
GET /v1/connectors/{connectorId}/ws
Authorization: Bearer NODE_TOKEN
Upgrade: websocket
Sec-WebSocket-Protocol: testflying.store.v2
```

连接地址由 serverUrl 的 http/https 转为 ws/wss。节点通过 UUID + Token 完成外层认证，再用安装配置中的独立 messageKey 完成 Noise_NNpsk0_25519_ChaChaPoly_BLAKE2s 握手。握手载荷同时携带 connectorId/baseUrl，服务端收到新会话的密钥确认后才保存地址、登记连接。未认证、旧密钥、未确认或重复连接不覆盖节点地址；同一身份第二条活动连接返回 409。不依赖本机 IP。

API 下发 StoreRequest，节点沿原连接返回 StoreReply；JSON 按最多 32 KiB 分片、逐片加密并通过二进制帧发送，方向密钥和计数器由 Noise 管理。超过 2 MiB 的业务 JSON、明文、篡改和重放均拒绝。两端每 10 秒 Ping，35 秒无活动则关闭，单次发送最长等待 5 秒。重连只恢复连接，不补发旧请求。HTTP 回退依赖 HTTPS 提供传输加密，不使用 Noise；可信安装配置分发和端点密钥安全仍是前提，snow 本身未声明正式安全审计。

### 9.2 HTTP 直连

API 请求节点自身地址：

```http
POST /v1/execute
Authorization: Bearer NODE_TOKEN
```

两种连接共用以下消息。StoreRequest：

```json
{
  "requestId":"5cbcb802-f207-41d7-b72e-20d223b209a4",
  "platform":"ios",
  "bundleIdentifier":"com.example.reader",
  "operation":"store_listings",
  "payload":{"version":"1.2.0"}
}
```

| 内部 operation | 对应商店操作 |
|---|---|
| supported_locales | 查询语言 |
| store_listings | 查询文案 |
| store_images | 查询图片 |
| store_releases | 查询 release |
| store_reviews | 查询评论 |
| product_page_optimizations | 查询 PPO |
| create_product_page_optimization | 创建 PPO |
| preflight | 预检查 |
| sync | 同步；具体 update_* 操作放在 payload.operation |
| upload_start / upload_chunk / upload_finish / upload_abort | 中心内部上传控制；外部调用方只使用中心上传接口 |

StoreReply：

```json
{
  "requestId":"5cbcb802-f207-41d7-b72e-20d223b209a4",
  "status":200,
  "body":{"listings":[]}
}
```

HTTP 节点的正常协议响应本身可以是 HTTP 200，真实操作状态在 StoreReply.status 中；Store API 会展开为对外 HTTP 状态和 body。节点请求的 requestId 为 1–128 字节，响应必须原样对应。此协议中的 payload 不能覆盖外层应用标识来选择其他应用。

## 10. 错误与失败处理

受控业务／传输错误通常返回：

```json
{"error":{"code":"connector_disconnected","message":"Connector disconnected; trigger a new request when available"}}
```

| HTTP | 常见 code | 含义 |
|---|---|---|
| 401 | unauthorized | 应用／管理 Token 错误 |
| 401 | invalid_node_token | 节点身份错误或已禁用 |
| 404 | app_not_found / connector_not_found / not_found | 应用未映射、节点不存在、资源不存在 |
| 409 | connector_busy | 同一账号正在执行其他请求 |
| 409 | connector_already_connected / wrong_transport | 重复 WSS 身份或连接模式不符 |
| 409 / 503 | connector_disabled | 管理写入／安装请求拒绝，或业务路由发现节点禁用 |
| 422 | invalid_store_payload / invalid_app_identifier / unsupported_target | 参数、应用标识或安装目标错误 |
| 429 | store_rate_limited / store_upstream_error | 本地商店限流或上游限流 |
| 502 | store_upstream_error / store_edit_cleanup_failed | 商店调用失败或 Google edit 清理失败 |
| 502 | connector_rejected / invalid_connector_reply / execution_failed | 节点拒绝、响应不合法或执行停止 |
| 503 | connector_disconnected / store_credentials_unavailable | 节点断开或本地凭据不可用 |
| 503 | release_unavailable / invalid_release | 安装包二进制未发布或不合法 |
| 504 | connector_timeout / execution_timeout | 中心或节点等待超时 |

框架层的 JSON 格式错误、非法 path 类型、Content-Type 和超大请求体等，还可能返回 400/413/415/422，其响应格式不保证是上述 error 对象。同步业务失败另按 5.5 的 status/errorCode 判断。

执行约定：

1. 每个账号同时只执行一条请求，其他请求返回 409，不排入持久队列。
2. 默认等待 60 秒，可分别配置 API 和节点超时。失败、断线、进程重启不自动重放，也没有幂等键去重。
3. 超时不等于商店没有修改；已经开始的节点工作可能继续完成清理，期间保持忙碌，结果丢弃。
4. Apple 多步骤写入可能部分成功；Google 正常失败路径尝试删除 edit。调用方重新触发前应查询实际商店状态。

## 11. 推荐接入顺序与当前缺口

1. 管理端创建账号 Connector，取得安装身份。
2. 下载对应平台 ZIP，在节点本地填写该账号的商店凭据路径并启动。
3. 按每个应用的 platform + 包名／Bundle ID 设置映射；同账号应用绑定同一个 Connector。
4. 业务后台使用应用 API Token 查询语言、文案等数据，再调用 preflight/sync。
5. 遇到错误向用户返回结果，由用户重新触发；不自动重试写操作。

当前没有商店内容草稿、素材上传库、后台页面、持久任务查询、通用送审／发布、评论回复、CPP 列表／删除或 PPO 启停接口。iOS release 列表尚未实现，Android CPP/PPO 不支持。上述范围不应当成“全部 Apple/Google 官方 API 的封装”。

机器可读定义见 [openapi.yaml](openapi.yaml)，启动和配置见 [README](../README.md)，已验证能力见 [provider-capabilities.md](provider-capabilities.md)。

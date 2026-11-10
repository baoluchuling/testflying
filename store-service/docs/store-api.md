# 商店业务与节点接入接口

外部后台用同一枚接入 Token 查询可用节点、绑定应用，再调用商店查询、同步和图片上传接口。本文完整覆盖 **9 个商店业务接口、2 个外部节点查询／绑定接口**，另附图片上传配套接口与管理员查询接口。契约版本：`2026-11-10`。

## 在线获取文档和功能目录

```http
GET /v1/store-management/api-docs
Authorization: Bearer INTEGRATION_TOKEN
```

无需先绑定应用或连接节点。使用与商店查询相同的接入 Token（HTTPS）或业务 API Token（HTTP／HTTPS）；管理 Token 不用于该接口。响应带 `Cache-Control: no-store`。

| format 参数 | 响应类型 | 内容 |
|---|---|---|
| 省略或 json | application/json | schemaVersion、contractVersion、11 个核心接口的 endpoints、2 个配套 uploadEndpoints、imageQuery、sync、limitations 及完整 Markdown 文档 |
| markdown | text/markdown | 本文完整内容 |
| openapi | application/yaml | 可导入工具的完整 OpenAPI 定义 |

```sh
curl --fail "$BASE_URL/v1/store-management/api-docs" \
  -H "Authorization: Bearer $INTEGRATION_TOKEN"
curl --fail "$BASE_URL/v1/store-management/api-docs?format=markdown" \
  -H "Authorization: Bearer $INTEGRATION_TOKEN"
curl --fail "$BASE_URL/v1/store-management/api-docs?format=openapi" \
  -H "Authorization: Bearer $INTEGRATION_TOKEN"
```

`BASE_URL` 使用下文中心基础地址，私有 CA 认证方式同其他 HTTPS 调用。未知 format 返回 `422 invalid_document_format`，未认证或已撤销 Token 返回 401。JSON 中路径为相对中心基础地址的路径，`{platform}`、`{bundleIdentifier}` 等为调用时需要替换的占位符。

该接口说明的是中心公开契约，不会调用商店、修改绑定或探测节点本地平台配置。节点需运行新版商店 Connector；实际调用仍受节点 `all`／`ios`／`android` 平台配置、在线状态、商店凭据权限和目标版本状态影响。中心不保存节点的平台模式。文档随中心程序发布，三个格式来自同一部署版本。

## 通用约定

中心提供 HTTP 和 HTTPS 两个入口。使用部署方提供的实际地址，保留其路径前缀。下文接口路径均追加在基础地址后。

| 入口 | 基础地址示例 | 证书 |
|---|---|---|
| HTTP | `http://store-api.example.com/store-api` | 无需 CA 参数，Token 和正文以明文传输 |
| HTTPS | `https://store-api.example.com/store-api` | 验证服务器证书，使用私有 CA 时可传 `--cacert` |

Apple 和 Google 商店凭据由 Connector 持有。 [公开 CA 证书](../deploy/testflying-prod-ca.crt)

按应用调用的商店业务和图片上传接口使用以下路径前缀。

```text
/v1/store-management/apps/{platform}/{bundleIdentifier}
```

| 参数 | 说明 |
|---|---|
| platform | ios / android，小写 |
| bundleIdentifier | iOS Bundle ID 或 Android 包名，大小写精确匹配，例如 com.example.reader |

调用前先[为应用选择并绑定节点](#列出节点和绑定应用)。业务请求只传平台和应用标识。

JSON 字段为 camelCase，最大请求体 2 MiB。以下响应均为格式示例。`{APP}` 表示上述应用路径前缀。

实际文件上传使用申请接口返回的短时 `uploadToken`，正文为原始文件字节，单图最大 64 MiB。外部请求统一访问中心地址。

## 对接凭据

| 凭据 | 支持入口 | 用途 |
|---|---|---|
| 接入 Token | HTTPS | 查全部启用的商店节点、绑定或换绑应用、调用商店业务、申请上传 |
| 业务 API Token（`STORE_API_TOKEN`） | HTTP、HTTPS | 查启用的商店节点、调用已绑定应用的商店接口、申请上传 |
| 管理 Token | HTTP、HTTPS | 查询节点详情、读取当前绑定、绑定或换绑应用 |
| 短时 `uploadToken` | 使用返回的 `uploadUrl` | 上传图片文件字节 |

中心管理员在 HTTPS 配置页打开「接入凭据」，用管理 Token 解锁，填写系统名称并发放。每套内部系统配置一枚接入 Token，调用时使用 `Authorization: Bearer INTEGRATION_TOKEN`。 [Connector 配置页说明](connector-setup.md)

已有和新发放的接入 Token 均覆盖全部节点，包括后续新增的节点。应用管理后端持有 Token，并负责校验当前用户对应用的权限。中心管理员可在同一管理区撤销 Token。

### HTTP 商店调用示例

应用已绑定节点后，使用业务 API Token 调用 HTTP 入口。

```sh
curl --fail 'http://store-api.example.com/store-api/v1/store-management/apps/ios/com.example.reader/store-listings?version=1.2.0' \
  -H "Authorization: Bearer $STORE_API_TOKEN"
```

HTTP 入口用同一枚业务 API Token 申请图片上传凭据，申请成功后直接对返回的 HTTP `uploadUrl` 发送 PUT。接入 Token 发往 HTTP 入口会返回 `403 https_required`。

## 接口列表

| 序号 | 方法 | 路径 | 用途 |
|---|---|---|---|
| — | GET | /v1/store-management/connectors | 读取全部启用的商店节点 |
| — | PUT | /v1/apps/{platform}/{bundleIdentifier}/connector | 设置或更换应用绑定 |
| 1 | GET | {APP}/supported-locales | 查询现有商店语言 |
| 2 | GET | {APP}/store-listings | 查询商店文案 |
| 3 | GET | {APP}/store-images | 查询图片和截图 |
| 4 | GET | {APP}/store-releases | 查询版本 release |
| 5 | GET | {APP}/store-reviews | 查询评论 |
| 6 | GET | {APP}/product-page-optimizations | 查询 Apple PPO 实验 |
| 7 | POST | {APP}/product-page-optimizations | 创建 Apple PPO 实验 |
| 8 | POST | {APP}/preflight | 检查当前写入条件 |
| 9 | POST | {APP}/sync | 单项或多字段同步内容 |

图片上传配套接口（不计入上述 9 个商店业务接口）：

| 序号 | 方法 | 路径 | 用途 |
|---|---|---|---|
| 10 | POST | {APP}/uploads | 申请图片上传凭据 |
| 11 | PUT | {APP}/uploads/{assetId} | 经中心上传图片文件字节 |

功能与平台范围：

| 功能 | iOS | Android |
|---|---|---|
| 语言、文案查询 | 支持，版本号影响返回字段 | 支持 |
| 图片按语言筛选 | locale | locale |
| 图片按手机／平板筛选 | phone／tablet | phone／tablet；平板返回七寸与十寸两组 |
| 图片按折叠屏筛选 | foldable，iPhone Duo | 不支持，返回 422，不回退为手机 |
| 版本列表 | 未实现，返回空 releases | 支持 |
| 评论查询、分页和当前页筛选 | 支持 | 支持 |
| PPO 实验查询／创建 | 支持 | 查询为空，创建不支持 |
| 文案、版本说明同步 | 支持 | 支持 |
| CPP 宣传文本和截图同步 | 支持 | 不支持 |
| 图片上传和替换 | Asset Library、关联及排序；手机／平板／折叠屏 | 手机／平板／feature graphic |

当前不提供通用送审、发布、评论回复、CPP 列表／删除或 PPO 启停功能。管理员专用节点和绑定查询见下节附录，不属于外部的两个接入接口。

## 列出节点和绑定应用

### 查询可选节点

```http
GET /v1/store-management/connectors
Authorization: Bearer INTEGRATION_TOKEN
```

成功返回 `200`。接入 Token 和业务 API Token 均可查询全部启用的商店节点。无可选节点时返回 `{"connectors":[]}`。

```json
{
  "connectors": [
    {
      "connectorId": "636b5d86-4e1b-4f88-aeb9-a1abdf0cdd33",
      "name": "商店节点 A",
      "enabled": true,
      "connected": true
    }
  ]
}
```

应用管理页可以显示 `name`，保存 `connectorId`，并用 `connected` 标注 WSS 在线状态。WSS 未连接时，中心仍可尝试通过节点已登记的 HTTP 入口执行商店请求。

### 绑定或更换应用节点

应用管理后端核验当前用户对应用的操作权限后，用同一枚接入 Token 发起绑定。iOS 路径传 Bundle ID，Android 路径传包名。

```http
PUT /v1/apps/ios/com.example.reader/connector
Authorization: Bearer INTEGRATION_TOKEN
Content-Type: application/json

{"connectorId":"636b5d86-4e1b-4f88-aeb9-a1abdf0cdd33"}
```

成功返回 `200` 和绑定后的节点对象，其中包含 `connectorId`、`name`、`enabled`、`connected` 和 `appCount`。

绑定键是 `platform + bundleIdentifier`。对同一个应用再次 PUT 会替换原绑定，多个应用可绑定同一个节点。绑定或换绑的目标须为已启用的商店节点。

| HTTP | 错误码 | 触发条件 |
|---|---|---|
| 401 | `unauthorized` | Token 缺失、无效或已撤销 |
| 403 | `https_required` | 通过 HTTP 使用接入 Token |
| 404 | `connector_not_found` | 目标节点不存在 |
| 409 | `connector_disabled` | 目标节点已停用 |
| 422 | `invalid_app_identifier` | 应用标识不合法 |
| 422 | `invalid_connector_kind` | 目标节点不是商店节点 |

绑定成功后，商店业务仍使用同一枚接入 Token，中心根据平台和应用标识选择节点。

```http
GET /v1/store-management/apps/ios/com.example.reader/store-listings?version=1.2.0
Authorization: Bearer INTEGRATION_TOKEN
```

### 管理员查询节点或当前绑定

以下 GET 接口使用 `Authorization: Bearer ADMIN_TOKEN`。

| 路径 | 用途 |
|---|---|
| `/v1/connectors` | 查询中心节点列表 |
| `/v1/connectors/{connectorId}` | 查询指定节点详情 |
| `/v1/apps/{platform}/{bundleIdentifier}/connector` | 查询应用当前绑定的节点 |

查应用绑定时，未绑定返回 `404 app_not_found`，绑定的节点已停用返回 `503 connector_disabled`。使用接入 Token 的后台可根据 PUT 成功响应记录所选节点。

## 1. 查询语言

```http
GET /v1/store-management/apps/ios/com.example.reader/supported-locales?version=1.2.0
```

| Query 参数 | 必填 | 说明 |
|---|---|---|
| version | 否 | iOS 版本号；Android 不使用 |

```json
{"locales":["en-US","zh-Hans"]}
```

iOS 找到指定版本时读取版本语言，否则读取应用级语言；Android 读取当前 listings 已有语言。不是平台全部可支持语言的字典。

## 2. 查询文案

```http
GET /v1/store-management/apps/ios/com.example.reader/store-listings?version=1.2.0
```

Query 参数：可选 version。iOS 不指定版本时仅返回应用级名称、副标题等内容；指定版本后合并描述、关键词、宣传文本和版本说明。

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

Android 主要字段：locale、title、shortDescription、fullDescription、description、videoUrl。未提供的字段不保证出现。

## 3. 查询图片和截图

```http
GET /v1/store-management/apps/ios/com.example.reader/store-images?version=1.2.0
GET /v1/store-management/apps/ios/com.example.reader/store-images?version=1.2.0&locale=en-US&device=phone
GET /v1/store-management/apps/ios/com.example.reader/store-images?version=1.2.0&locale=ja&device=foldable
```

iOS 需要 version 才能查询版本截图，未指定或版本不存在时返回空 locales；Android 不使用 version。

| 参数 | 含义 |
|---|---|
| locale | 可选，按商店语言精确筛选，忽略大小写并去首尾空白。省略或空白返回全部语言，不存在时返回空 locales，不自动回退其他语言 |
| device | 可选，`phone` 手机、`tablet` 平板、`foldable` 折叠屏。省略或空白返回全部已有图片类型，指定时 images 只包含所选设备的槽位 |

两个参数可组合，由新版商店 Connector 在读取图片前筛选，中心沿用转发逻辑。已有节点必须升级程序并重启，不提供旧节点兼容筛选。设备参数非法时返回 422。Android 没有独立折叠屏图片类型，`device=foldable` 返回 422 `unsupported_image_device`。

`locale` 的候选值可先从接口 1 `supported-locales` 取得，按商店原始值传入，例如 `en-US`、`zh-Hans`、`ja`。它返回当前应用已有语言，不是全球语言字典。参数不传表示全部，`device` 不接受 `all` 或 `duo`，折叠屏统一传 `foldable`。

| 查询意图 | Query 示例 |
|---|---|
| iOS 指定版本的全部语言、全部设备 | version=1.2.0 |
| iOS 英文的全部设备 | version=1.2.0&locale=en-US |
| iOS 全部语言的平板截图 | version=1.2.0&device=tablet |
| iOS 日语折叠屏截图 | version=1.2.0&locale=ja&device=foldable |
| Android 英文手机截图 | locale=en-US&device=phone |
| Android 中文平板截图 | locale=zh-CN&device=tablet |

没有匹配语言时返回 `{"locales":[]}`；语言存在但设备暂无图片时，保留该语言并返回所选槽位空数组。`device` 只用于 `store-images`，不筛选文案、评论或同步内容；评论的 locale 是独立的当前页筛选参数。不要假定其他读取接口支持语言筛选。

```json
{
  "locales":[
    {
      "locale":"en-US",
      "images":{
        "phone_screenshots":[
          {"id":"image-id","fileName":"phone.png","url":"https://assets.example.com/phone.png","width":1284,"height":2778}
        ],
        "tablet_screenshots":[],
        "foldable_screenshots":[]
      }
    }
  ]
}
```

iOS 从 Apple Asset Library placements 读取手机、平板和 iPhone Duo 折叠屏截图，使用运行时参考数据区分设备组。折叠屏保留各内外屏组，图片项附带 `placementId`、`placementGroup`、`displayClass`，不会混入普通手机组。URL 可能是上游模板地址，需要处理尺寸／格式占位符。选中的设备没有图片时对应槽位返回空数组。查询依次读取语言、设备参考数据和关联图片，不下载图片二进制，中心不保存图片。

Apple 查询和图片同步使用现有节点 API Key，需要具备 Asset Library 访问权限，权限不足按上游错误返回。[Apple 新资源说明](https://developer.apple.com/documentation/appstoreconnectapi/understanding-the-app-asset-library)。

Android `device=phone` 只读取 phone_screenshots，`device=tablet` 只读取 seven_inch_screenshots 和 ten_inch_screenshots。未指定设备时读取槽包括：

- feature_graphic_url
- phone_screenshots
- seven_inch_screenshots
- ten_inch_screenshots
- tv_screenshots
- wear_screenshots

Android 图片引用可能带 sha1、sha256 等上游字段。

## 4. 查询 release

```http
GET /v1/store-management/apps/android/com.example.reader/store-releases
```

无需额外参数。

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

Android 按最高数值 versionCode 倒序返回。**iOS 当前未实现版本列表查询，固定返回 `{"releases":[]}`。**

## 5. 查询评论

```http
GET /v1/store-management/apps/ios/com.example.reader/store-reviews?pageSize=50&rating=5&date=2026-10-30
```

| Query 参数 | 类型 | 说明 |
|---|---|---|
| pageSize | 正整数 | 默认 50；iOS 最多 200，Android 最多 100，超出时截到上限 |
| pageToken | string | 上次的 nextPageToken |
| startIndex | string | Android 分页起点 |
| translationLanguage | string | Android 上游翻译语言 |
| sort | string | iOS 默认 -createdDate；Android 当前不使用 |
| rating | integer | 可选，1–5 |
| locale | string | 可选，语言筛选，忽略大小写 |
| territory | string | 可选，地区筛选，忽略大小写 |
| date | YYYY-MM-DD | 指定一天，不能与 startDate/endDate 同时传 |
| startDate | YYYY-MM-DD | 起始日，含当天 |
| endDate | YYYY-MM-DD | 结束日，含当天 |
| timezone | string | IANA 时区，默认 Asia/Shanghai |

```json
{
  "reviews":[
    {"id":"review-id","platform":"ios","rating":5,"title":"Useful","body":"Easy to use","authorName":"Reader","locale":"en-US","territory":"US","appVersion":"1.2.0","createdAt":"2026-10-30T08:00:00Z","updatedAt":"2026-10-30T08:00:00Z"}
  ],
  "nextPageToken":"next-cursor",
  "filters":{"timezone":"Asia/Shanghai","locale":"","territory":"","rating":5,"startAt":"2026-10-29T16:00:00+00:00","endAt":"2026-10-30T15:59:59.999999+00:00"}
}
```

筛选只处理当前上游页，不额外翻页补足数量；可能 reviews 为空但仍有 nextPageToken。游标原样传回，空字符串表示当前没有下一页游标。平台返回字段不同，Android 可带 appVersionCode、originalText，iOS 可带 reviewerNickname、raw、rawPageInfo。

## 6. 查询 Apple PPO 实验

```http
GET /v1/store-management/apps/ios/com.example.reader/product-page-optimizations
```

无需额外参数。

```json
{
  "experiments":[
    {"id":"experiment-id","name":"Screenshot experiment","platform":"IOS","state":"PREPARE_FOR_SUBMISSION","trafficProportion":50,"reviewRequired":true,"startDate":"","endDate":"","treatments":[{"id":"treatment-id","name":"Variant A","appIconName":""}]}
  ]
}
```

Android 当前返回 `{"experiments":[]}`。列表不扩展查询 treatment 的语言和图片内容。

## 7. 创建 Apple PPO 实验

```http
POST /v1/store-management/apps/ios/com.example.reader/product-page-optimizations
```

```json
{
  "name":"Screenshot experiment",
  "trafficProportion":50,
  "locales":["en-US","zh-Hans"],
  "treatments":[{"name":"Variant A","locales":["en-US"]},{"name":"Variant B","appIconName":"AlternateIcon"}]
}
```

| 字段 | 必填 | 说明 |
|---|---|---|
| name | 是 | 非空实验名称 |
| trafficProportion | 是 | 整数 1–100 |
| locales | 否 | 语言数组，供 treatment 回退 |
| treatments | 否 | treatment 数组，每项 name 必填 |
| treatments[].appIconName | 否 | 图标名称 |
| treatments[].locales | 否 | 省略或空数组时使用顶层 locales |

返回 `{"experiment":{...}}`，字段同接口 6，创建响应的 treatments 可包含 locales。不传 treatments 时不会自动创建默认 treatment。此接口不负责启动、终止或发布实验；Android 返回 422。

## 8. 写入前预检查

```http
POST /v1/store-management/apps/ios/com.example.reader/preflight
```

```json
{"operation":"update_app_metadata","version":"1.2.0","locale":"en-US"}
```

| 字段 | 必填 | 说明 |
|---|---|---|
| operation | 是 | update_app_metadata / update_release_notes / update_marketing_page |
| version | 条件必填 | iOS 非 CPP 操作必填 |
| locale | 否 | 当前预检查不强制要求 |

```json
{
  "canSync":true,
  "reasonCode":null,
  "message":"Store content is editable",
  "storeState":{"versionExists":true,"editable":true,"currentStatus":"PREPARE_FOR_SUBMISSION"}
}
```

HTTP 200 不代表允许同步，必须判断 canSync。iOS 检查版本存在性、可编辑状态；Android 通过临时 edit 检查访问能力。它不保证随后所有字段、图片或写入一定通过。

## 9. 同步内容

```http
POST /v1/store-management/apps/{platform}/{bundleIdentifier}/sync
```

**每次一个应用、一个语言、一种 operation，支持只同步一个字段。** 不从草稿读取，不保存草稿，也不创建可查询的持久任务。

### 9.1 更新文案

只修改英文描述，其他字段、语言和截图保持原样：

```json
{
  "operation":"update_app_metadata",
  "version":"1.2.0",
  "locale":"en-US",
  "syncScopes":["description"],
  "metadata":{"description":"Updated description"}
}
```

| 字段 | 必填 | 说明 |
|---|---|---|
| operation | 是 | 固定 update_app_metadata |
| locale | 是 | 商店语言 |
| version | iOS 必填 | 已存在且可编辑的商店版本 |
| metadata | 是 | 本次内容 |
| syncScopes | 否 | 省略时同步请求中提供的受支持文字字段；图片需显式选择 store_images |

| metadata 字段 | scope | iOS | Android |
|---|---|---|---|
| title | title | 名称 | 标题 |
| subtitle | subtitle | 副标题 | 不写入 |
| description / fullDescription | description | 优先 description | 优先 fullDescription |
| keywords | keywords | 关键词 | 不写入 |
| promotionalText | promotional_text | 宣传文本 | 可作为简短描述的备用输入 |
| shortDescription | short_description | 不写入 | 简短描述，优先于 promotionalText |
| videoUrl | video | 不写入 | 视频链接 |
| storeImages | store_images | 截图 | 图片／截图 |

`metadata` scope 表示全部受支持文字字段。字段不传则保留，选中的文字字段传 `""` 表示清空。syncScopes 提供时必须是非空的已知字符串数组；空数组、字符串、null、未知 scope 都会拒绝。iOS 新语言还需要合法应用名称，未提供 title 时尝试已有主语言名称。

### 9.2 更新版本说明

iOS：

```json
{"operation":"update_release_notes","version":"1.2.0","locale":"en-US","releaseNotes":"Fixes and improvements"}
```

Android：

```json
{"operation":"update_release_notes","locale":"en-US","releaseNotes":"Fixes and improvements","storeRelease":{"track":"production","versionCode":"123","versionName":"1.2.0"}}
```

locale 和非空 releaseNotes 必填，iOS 还需 version。Android storeRelease 可省略；若提供，可用非空字符串 track、versionCode、versionName 组合筛选，versionCode 必须传字符串。条件同时满足才匹配；不指定时选现有最高数值 versionCode，指定部分条件时在匹配范围内选择。保留其他语言和其他 release。

`release_notes` 不是合法 scope，版本说明通过本 operation 更新。Android 提交 edit 不自动送审或发布。

### 9.3 更新图片

```json
{
  "operation":"update_app_metadata",
  "version":"1.2.0",
  "locale":"en-US",
  "syncScopes":["store_images"],
  "metadata":{"storeImages":{"phone_screenshots":{"urls":["https://assets.example.com/phone-1.png"]}}}
}
```

写入槽：

图片读取的 `device` 不作为 sync 的 Query 或正文选择器。写入时通过顶层 `locale` 选语言，通过下列槽位选设备，并显式设置 `syncScopes:["store_images"]`。例如写入日语折叠屏截图：

```json
{
  "operation":"update_app_metadata",
  "version":"1.2.0",
  "locale":"ja",
  "syncScopes":["store_images"],
  "metadata":{"storeImages":{"foldable_screenshots":{"urls":["https://assets.example.com/duo-1.png"]}}}
}
```

| 槽 | iOS | Android |
|---|---|---|
| phone_screenshots | 手机截图组 | phoneScreenshots |
| tablet_screenshots | 平板截图组 | 同时更新七寸和十寸平板组 |
| foldable_screenshots | iPhone Duo 折叠屏截图组，保留内外屏关联 | 不支持 |
| feature_graphic_url | 不支持 | featureGraphic |

**图片按选中槽整组替换**，不是追加；未传的槽保留。`{"urls":[]}` 或 `{"assets":[]}` 清空该槽，目前不支持单张追加／删除。

每槽可以传 urls 或 assets 数组，两者合计最多 10 张。assets 示例：

```json
{"assets":[{"downloadUrl":"https://assets.example.com/phone-1.png","fileName":"phone-1.png"}]}
```

下载 URL 必须为 HTTP(S)；图片须为 PNG/JPEG，单图最多 64 MiB。本地文件先通过接口 10、11 上传，再在 assets 中引用 `{"assetId":"上传返回的资产ID"}`，无需先上传对象存储。该上传使用原始文件正文，不使用 multipart。中心不保存图片，也不提供中心图片库或草稿。

iOS 普通版本和 CPP 的图片同步均采用 Apple Asset Library。Connector 读取目标应用的素材库和最新规格，校验图片尺寸、格式、透明通道、文件大小及设备组，再将所有新图片上传到素材库。分块上传不携带节点 Apple JWT，提交时仅设置 `uploaded=true`。每张图片最多等待处理 30 秒，确认成功并验证 Apple 返回的 specId 后，才替换所选设备类别的 placements，并按传入顺序设置每组展示顺序。一个素材可以关联到多个匹配的设备组。

图片处理失败、规格不匹配或等待超时会返回错误并保留原有图片关联。明确空数组只删除目标语言和选中设备类别的关联，不删除素材库图片；其他语言、页面和未选设备保留。Apple 多步写入不是事务，关联替换阶段失败时可能已有部分关联被修改，失败上传也可能留下未关联的库素材。接口不会自动重放商店写入或自动删除库素材。设备规格及每组数量上限以实时参考数据为准。

### 9.4 创建／更新 Apple CPP

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
| pageName | 是 | 创建和更新请求均需传 |
| applePageId | 否 | 不传则创建，传入则更新并验证应用归属 |
| locale | 否 | 可覆盖顶层 locale，两处至少一处非空 |
| promotionalText | 否 | 宣传文本 |
| storeImages | 否 | 同 9.3，需显式选择 store_images |

新建基于应用最近已有版本。不传 syncScopes 时处理提供的宣传文本；`marketing_text` 选择文字、`store_images` 选择图片。返回 storeState 中的 applePageId 用于后续更新。Android 不支持 CPP，返回业务 failed。

当前没有 CPP 列表、内容读取、删除、送审、发布、关键词或深链字段写入接口。

### 9.5 同步响应

成功：

```json
{"status":"succeeded","message":"Store content synchronized","errorCode":null,"errorSummary":null,"storeState":{}}
```

业务失败可能仍为 HTTP 200：

```json
{"status":"failed","message":"Target store version is not editable","errorCode":"store_version_not_editable","errorSummary":"Target store version is not editable"}
```

常见业务失败包括 store_version_missing、store_version_not_editable、empty_sync_payload、google_release_target_missing、google_marketing_page_unsupported。调用方必须判断 status，而不只判断 HTTP 状态。

CPP 成功时 storeState 包含 applePageId、created；Android 版本说明成功时可包含 track、versionCode、versionName。

## 10. 申请图片上传

外部后台只访问中心 Store API，无需知道节点 ID、节点地址或永久节点凭据。调用方发送本地文件的实际字节；仅发送文件路径不能让服务器读取调用方磁盘。

`POST /v1/store-management/apps/{platform}/{bundleIdentifier}/uploads`

认证使用 `Authorization: Bearer INTEGRATION_TOKEN`。旧集成仍可使用配置中的 `STORE_API_TOKEN`。

```json
{
  "fileName": "screen.png",
  "contentType": "image/png",
  "sizeBytes": 3145728,
  "sha256": "本地文件的64位SHA-256十六进制值"
}
```

支持 image/png、image/jpeg，文件最多 64 MiB，sizeBytes 必须是文件实际字节数。文件名不含目录或控制字符。应用必须已映射到可达 Connector。

成功 201：

```json
{
  "assetId": "8fd33c58-36ce-4b26-9e54-52450c79a175",
  "uploadUrl": "https://store-api.example.com/store-api/v1/store-management/apps/ios/com.example.reader/uploads/8fd33c58-36ce-4b26-9e54-52450c79a175",
  "uploadToken": "短时签名凭据",
  "expiresAt": 1793769999,
  "method": "PUT"
}
```

uploadUrl 始终是中心地址：HTTP 入口申请返回 HTTP 上传地址，HTTPS 入口申请返回 HTTPS 上传地址，调用方直接使用返回值。uploadToken 绑定当前节点、平台、应用、assetId、大小、类型和 SHA-256，有效期 30 分钟，不写入数据库。凭据为签名 JWT，不是加密容器，不应公开、放入查询字符串或日志。中心与 Connector 需要保留独立验签密钥，不能把“凭据不入库”理解成无需密钥。重置节点 Token 会同时使旧上传凭据失效。

## 11. 上传图片文件字节

向接口 10 返回的 `uploadUrl` 发 PUT。`Content-Type` 应与申请时一致，JPEG 使用 `image/jpeg`。`UPLOAD_URL` 和 `UPLOAD_TOKEN` 取自申请响应。

```sh
curl --fail \
  --max-time 1860 \
  -X PUT "$UPLOAD_URL" \
  -H "Authorization: Bearer $UPLOAD_TOKEN" \
  -H 'Content-Type: image/png' \
  --upload-file ./screen.png
```

此处使用 uploadToken，不是商店 API Token。正文是原始文件，不是 JSON、Base64 或 multipart。需要在凭据过期前开始上传；单次上传从开始传输起最多允许 30 分钟，与凭据有效期分别计算。中心流式读取并逐块等待节点确认，不保存临时文件；WSS 可用时使用加密二进制帧，否则由中心走已认证登记的节点 HTTP 入口。一次文件传输固定通道，失败不自动切换、续传或重试。

成功 201 返回 assetId、sizeBytes、sha256、expiresInSeconds（900）。节点验证真实大小、哈希、PNG/JPEG 格式、尺寸和解码内存限制后才把资产标记为可用。同一 assetId 不覆盖已有上传或完成文件。

### 11.1 同步时引用已上传图片

调用接口 9（sync）时，继续使用提交上传申请时的接入 Token 或业务 API Token。下例更新指定语言的手机截图槽位。

```json
{
  "operation": "update_app_metadata",
  "version": "1.2.0",
  "locale": "en-US",
  "syncScopes": ["store_images"],
  "metadata": {
    "storeImages": {
      "phone_screenshots": {
        "assets": [{"assetId": "8fd33c58-36ce-4b26-9e54-52450c79a175"}]
      }
    }
  }
}
```

每个已上传图片引用只提供 assetId，不再附带 downloadUrl 或 storageKey。仍支持原有 URL 图片来源，也可在一个槽位里混用多个 assetId/URL。**槽位按完整列表替换**，只传一张会把该槽替换为这一张，未指定槽保留；CPP 的 marketingPage.storeImages 也使用相同资产引用方式。未指定 store_images 的操作和 preflight 不消费资产。

资产只限同一平台、应用和账号节点使用。全部引用在任何商店操作之前验证；缺失、过期、跨应用或未完成引用失败。实际 sync 执行结束（成功、失败或取消）释放独占资产，不可再次使用；需要重试时重新申请并上传。节点重启后临时资产失效。

### 11.2 限额、清理和上传错误

单图 64 MiB，节点总预留 512 MiB，最多 20 个资产。未完成上传最多保留 30 分钟，完成资产从完成时起最多保留 15 分钟，定期清理。匿名文件在 Unix 下不保留目录项，Windows 下最后句柄关闭时删除，进程退出或崩溃也由 OS 回收。中心没有图片草稿、上传表或凭据表。

签名凭据不是严格的一次性令牌：有效期内可以尝试，但节点以 assetId 检查当前会话，不覆盖完成资产。若要严格的凭据“一用即废”，需要另加服务端使用记录，本版不这样宣称。

常见错误：401 凭据无效/过期或应用映射已变；403 跨应用资产；404 资产缺失/过期；409 上传重复、偏移不符或资产在用；413 超过签名大小；422 大小/哈希/图像无效；429 节点暂存配额满；503 节点离线；507 本地临时存储不可用。

### 11.3 外部后台完整调用顺序

在 store-service 目录可直接使用仅依赖 Python 标准库的 [外部调用示例](../examples/upload-image.py)：

```sh
STORE_API_TOKEN="$INTEGRATION_TOKEN" python3 examples/upload-image.py \
  --base-url https://store-api.example.com/store-api \
  --platform ios --bundle-id com.example.reader \
  --file ./screen.png
```

该示例计算文件大小和 SHA-256、申请凭据并流式上传，只输出资产信息，不自动写入商店。拿到 assetId 后按第 11.1 节调用 sync。

使用 HTTP 时，将 `--base-url` 改为 HTTP 地址，并让 `STORE_API_TOKEN` 使用业务 API Token。

1. 本地读取文件大小并计算 SHA-256。
2. 用接入 Token POST 申请上传凭据。
3. curl --upload-file 或 SDK 文件流 PUT 到中心返回的 uploadUrl。
4. 等待 201 后，把 assetId 放进 sync。
5. 清除调用端持有的 uploadToken；失败时重新触发，不自动重放商店写入。

中心与 Connector 的 WSS 使用 TLS，正文另有 Noise 会话加密；业务 HTTP 入口不改变节点的 WSS 连接。外部后台到中心的 HTTP 请求（包括 Token、安装配置下载和图片）没有 TLS 保护。本版未在节点 HTTP JSON 执行入口上叠加 Noise，节点 HTTP 回退应配置 HTTPS。消息加密不能防止端点进程或安装配置失陷。

## 错误和重试规则

受控错误格式：

```json
{"error":{"code":"connector_disconnected","message":"Connector disconnected; trigger a new request when available"}}
```

| HTTP | 常见含义 |
|---|---|
| 401 | API Token 错误 |
| 404 | 应用未映射或资源不存在 |
| 409 | 账号正忙 |
| 422 | 请求参数错误 |
| 429 | 本地／上游商店限流 |
| 502 | 商店调用、节点响应或 Google edit 清理失败 |
| 503 | 节点断开、禁用或商店凭据不可用 |
| 504 | 执行等待超时 |

框架层参数解析、JSON 格式、Content-Type 或体积错误还可能返回 400/413/415/422，响应格式不保证统一。

每个账号同时只执行一条商店请求，商店请求默认等待 60 秒；文件传输使用第 11 节的独立 30 分钟期限。失败、断线不自动重放或重试，没有持久任务或幂等键去重。超时不代表商店未修改，已开始的工作可能继续完成清理，期间保持账号忙碌；用户重新触发前应查询真实状态。Apple 多步骤写入可能部分成功。

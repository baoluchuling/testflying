# testflying 商店连接对外 API

本文只给第三方电脑或外部系统调用使用，不包含后台 UI、Connector 内部协议和实现细节。

## 基础信息

- 基础地址：`http://47.90.163.122:8000`
- 鉴权：所有接口都需要请求头 `Authorization: Bearer <TOKEN>`
- `{accountId}`：testflying 开发者账号 ID，例如 `738W4ARM22`
- `{appId}`：testflying 内部 App ID，不是 Apple 数字 App ID，也不是 Android package name
- 所有连接真实商店的接口都支持显式覆盖商店目标：iOS 用 `iosAppId`（兼容 `storeAppId` / `appleAppId`），Android 用 `packageName`
- 传了真实 `iosAppId` 或 `packageName` 后，中心后台会直接下发给 connector；不传才使用后台绑定值或包解析值
- Google Play 同步会提交 edit，但默认带 `changesNotSentForReview=true`，不会直接送审；如果已有变更正在审核中，会按 `changesInReviewBehavior=ERROR_IF_IN_REVIEW` 返回错误
- LLM 用户反馈分类接口按 token 限流：`30 次/分钟`、`300 次/小时`

## 接口总览

| 接口 | 方法 | 是否连接商店 | 用途 |
| --- | --- | --- | --- |
| `/v1/store-management/developer-accounts/{accountId}/apps/{appId}/store-locales` | GET | 是 | 读取商店支持语言 |
| `/v1/store-management/developer-accounts/{accountId}/apps/{appId}/store-listings` | GET | 是 | 读取商店文案 |
| `/v1/store-management/developer-accounts/{accountId}/apps/{appId}/store-images` | GET | 是 | 读取商店截图 |
| `/v1/store-management/developer-accounts/{accountId}/apps/{appId}/store-releases` | GET | 是 | 读取 Google Play release |
| `/v1/store-management/developer-accounts/{accountId}/apps/{appId}/store-reviews` | GET | 是 | 读取商店评论 |
| `/v1/store-management/developer-accounts/{accountId}/apps/{appId}/metadata-content-sets` | POST | 否 | 保存默认商店页草稿 |
| `/v1/store-management/developer-accounts/{accountId}/apps/{appId}/store-versions/{version}/draft` | POST | 否 | 保存版本草稿 |
| `/v1/store-management/developer-accounts/{accountId}/apps/{appId}/marketing-pages` | POST | 否 | 创建自定义产品页面草稿 |
| `/v1/store-management/developer-accounts/{accountId}/apps/{appId}/sync-runs` | POST | 是 | 同步默认商店页 |
| `/v1/store-management/developer-accounts/{accountId}/apps/{appId}/marketing-pages/{pageId}/sync-runs` | POST | 是 | 同步自定义产品页面 |
| `/v1/store-management/developer-accounts/{accountId}/apps/{appId}/product-page-optimizations` | GET | 是 | 查询产品页面优化 |
| `/v1/store-management/developer-accounts/{accountId}/apps/{appId}/product-page-optimizations` | POST | 是 | 创建产品页面优化 |
| `/v1/llm/feedback-classifications` | POST | 否 | 用户反馈问题分类 |

## 1. 读取商店支持语言

```http
GET /v1/store-management/developer-accounts/{accountId}/apps/{appId}/store-locales
```

用于读取 App Store Connect 或 Google Play 当前 App 支持的语言。

参数：

| 参数 | 位置 | 必填 | 说明 |
| --- | --- | --- | --- |
| `accountId` | path | 是 | 开发者账号 ID |
| `appId` | path | 是 | testflying 内部 App ID |
| `version` | query | 否 | 商店版本上下文 |
| `iosAppId` / `storeAppId` / `appleAppId` | query | 否 | iOS 真实 App Store Connect 数字 App ID |
| `packageName` | query | 否 | Android 真实 Google Play package name |

```bash
curl "$BASE_URL/v1/store-management/developer-accounts/$ACCOUNT_ID/apps/$APP_ID/store-locales?iosAppId=1234567890" \
  -H "Authorization: Bearer $TOKEN"
```

```json
{
  "accountId": "738W4ARM22",
  "appId": "app-ios-com-app-qw-readink",
  "platform": "ios",
  "version": "",
  "locales": ["en-US", "zh-Hant"]
}
```

## 2. 读取商店文案

```http
GET /v1/store-management/developer-accounts/{accountId}/apps/{appId}/store-listings
```

用于读取商店侧已存在的描述、宣传文本、关键词等文案。

参数：

| 参数 | 位置 | 必填 | 说明 |
| --- | --- | --- | --- |
| `accountId` | path | 是 | 开发者账号 ID |
| `appId` | path | 是 | testflying 内部 App ID |
| `version` | query | 否 | 商店版本上下文 |
| `iosAppId` / `storeAppId` / `appleAppId` | query | 否 | iOS 真实 App Store Connect 数字 App ID |
| `packageName` | query | 否 | Android 真实 Google Play package name |

```bash
curl "$BASE_URL/v1/store-management/developer-accounts/$ACCOUNT_ID/apps/$APP_ID/store-listings?packageName=com.example.android" \
  -H "Authorization: Bearer $TOKEN"
```

```json
{
  "accountId": "738W4ARM22",
  "appId": "app-ios-com-app-qw-readink",
  "platform": "ios",
  "listings": [
    {
      "locale": "en-US",
      "promotionalText": "Read better stories every day.",
      "description": "Long store description."
    }
  ]
}
```

## 3. 读取商店截图

```http
GET /v1/store-management/developer-accounts/{accountId}/apps/{appId}/store-images
```

用于读取商店侧当前截图。iOS 返回 iPhone/iPad 截图；Android 返回 phone/tablet 截图和 feature graphic。

参数：

| 参数 | 位置 | 必填 | 说明 |
| --- | --- | --- | --- |
| `accountId` | path | 是 | 开发者账号 ID |
| `appId` | path | 是 | testflying 内部 App ID |
| `version` | query | 否 | 商店版本上下文 |
| `iosAppId` / `storeAppId` / `appleAppId` | query | 否 | iOS 真实 App Store Connect 数字 App ID |
| `packageName` | query | 否 | Android 真实 Google Play package name |

```bash
curl "$BASE_URL/v1/store-management/developer-accounts/$ACCOUNT_ID/apps/$APP_ID/store-images?iosAppId=1234567890" \
  -H "Authorization: Bearer $TOKEN"
```

```json
{
  "accountId": "738W4ARM22",
  "appId": "app-ios-com-app-qw-readink",
  "platform": "ios",
  "locales": [
    {
      "locale": "en-US",
      "images": {
        "phone_screenshots": [
          {
            "fileName": "01.png",
            "url": "https://example.test/01.png"
          }
        ]
      }
    }
  ]
}
```

## 4. 读取 Google Play Release

```http
GET /v1/store-management/developer-accounts/{accountId}/apps/{appId}/store-releases
```

用于读取 Google Play tracks、releases 和 versionCodes。同步 Android 版本说明前，可以先用这个接口确定 `storeTrack` 和 `storeVersionCode`。

参数：

| 参数 | 位置 | 必填 | 说明 |
| --- | --- | --- | --- |
| `accountId` | path | 是 | 开发者账号 ID |
| `appId` | path | 是 | testflying 内部 Android App ID |
| `version` | query | 否 | 查询上下文 |
| `packageName` | query | 否 | Android 真实 Google Play package name |

```bash
curl "$BASE_URL/v1/store-management/developer-accounts/$ACCOUNT_ID/apps/$APP_ID/store-releases?packageName=com.example.android" \
  -H "Authorization: Bearer $TOKEN"
```

```json
{
  "accountId": "738W4ARM22",
  "appId": "app-android-com-app-android-qw-readink",
  "platform": "android",
  "releases": [
    {
      "track": "production",
      "status": "completed",
      "versionCodes": ["10013404"],
      "releaseNotes": []
    }
  ]
}
```

## 5. 读取商店评论

```http
GET /v1/store-management/developer-accounts/{accountId}/apps/{appId}/store-reviews
```

用于读取 App Store Connect 或 Google Play 评论。connector 只负责调用商店评论接口并返回结果，日期、评分、语言、地区等筛选由中心后台处理。

参数：

| 参数 | 位置 | 必填 | 说明 |
| --- | --- | --- | --- |
| `accountId` | path | 是 | 开发者账号 ID |
| `appId` | path | 是 | testflying 内部 App ID |
| `iosAppId` / `storeAppId` / `appleAppId` | query | 否 | iOS 直传 App Store Connect 数字 App ID |
| `packageName` | query | 否 | Android 直传 Google Play package name |
| `date` | query | 否 | 指定日期，格式 `YYYY-MM-DD`，不能和 `startDate/endDate` 同时使用 |
| `startDate` | query | 否 | 起始日期，格式 `YYYY-MM-DD` |
| `endDate` | query | 否 | 结束日期，格式 `YYYY-MM-DD` |
| `timezone` | query | 否 | 日期过滤时区，默认 `Asia/Shanghai` |
| `locale` | query | 否 | 中心后台按评论语言过滤 |
| `territory` | query | 否 | 中心后台按评论地区过滤 |
| `rating` | query | 否 | 中心后台按评分过滤，范围 `1-5` |
| `pageSize` | query | 否 | 单页数量，默认 `50`；iOS 最大 `200`，Android 最大 `100` |
| `pageToken` | query | 否 | 商店分页 token |
| `startIndex` | query | 否 | Google Play 原生分页起始位置 |
| `translationLanguage` | query | 否 | Google Play 原生翻译语言参数 |

```bash
curl "$BASE_URL/v1/store-management/developer-accounts/$ACCOUNT_ID/apps/$APP_ID/store-reviews?date=2026-06-24&rating=5&pageSize=50" \
  -H "Authorization: Bearer $TOKEN"
```

```json
{
  "accountId": "738W4ARM22",
  "appId": "app-ios-com-app-qw-readink",
  "platform": "ios",
  "reviews": [
    {
      "id": "123456",
      "platform": "ios",
      "rating": 5,
      "body": "Works well.",
      "locale": "en-US",
      "territory": "US",
      "appVersion": "1.0.0",
      "createdAt": "2026-06-24T10:00:00Z"
    }
  ],
  "nextPageToken": ""
}
```

## 6. 保存默认商店页草稿

```http
POST /v1/store-management/developer-accounts/{accountId}/apps/{appId}/metadata-content-sets
```

用于把默认商店页文案和截图保存到 testflying，不会同步到真实商店。

参数：

| 参数 | 位置 | 必填 | 说明 |
| --- | --- | --- | --- |
| `accountId` | path | 是 | 开发者账号 ID |
| `appId` | path | 是 | testflying 内部 App ID |
| `sourceLocale` | form | 否 | 源语言，默认 `en-US` |
| `locales` | form | 是 | 语言列表，多个值逗号分隔 |
| `promotionalText__{locale}` | form | 否 | 指定语言宣传文本 |
| `description__{locale}` | form | 是 | 指定语言描述 |
| `storeImageFiles__phoneScreenshots__{locale}` | file | 否 | 手机截图 |
| `storeImageFiles__tabletScreenshots__{locale}` | file | 否 | 平板截图 |
| `storeImageFiles__featureGraphicUrl__{locale}` | file | 否 | Android feature graphic |

```bash
curl -X POST "$BASE_URL/v1/store-management/developer-accounts/$ACCOUNT_ID/apps/$APP_ID/metadata-content-sets" \
  -H "Authorization: Bearer $TOKEN" \
  -F "sourceLocale=en-US" \
  -F "locales=en-US,zh-Hant" \
  -F "description__en-US=Long store description"
```

```json
{
  "contentSetId": "default",
  "contentSetName": "默认上架内容",
  "locales": ["en-US", "zh-Hant"],
  "savedLocales": 2,
  "uploadedAssets": 0,
  "warnings": []
}
```

## 7. 保存版本草稿

```http
POST /v1/store-management/developer-accounts/{accountId}/apps/{appId}/store-versions/{version}/draft
```

用于保存指定版本的版本说明，也可以同时保存默认商店页文案草稿。不会同步到真实商店。

参数：

| 参数 | 位置 | 必填 | 说明 |
| --- | --- | --- | --- |
| `accountId` | path | 是 | 开发者账号 ID |
| `appId` | path | 是 | testflying 内部 App ID |
| `version` | path | 是 | 商店版本号 |
| `sourceLocale` | body | 否 | 源语言，默认 `en-US` |
| `locales` | body | 是 | 语言列表 |
| `metadataByLocale` | body | 否 | 各语言宣传文本和描述 |
| `releaseNotesByLocale` | body | 否 | 各语言版本说明 |

```bash
curl -X POST "$BASE_URL/v1/store-management/developer-accounts/$ACCOUNT_ID/apps/$APP_ID/store-versions/$VERSION/draft" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"locales":["en-US"],"releaseNotesByLocale":{"en-US":"Fix bugs"}}'
```

```json
{
  "version": "1.0.0",
  "locales": ["en-US"],
  "savedMetadataDrafts": 0,
  "savedReleaseNoteDrafts": 1,
  "warnings": []
}
```

## 8. 创建自定义产品页面草稿

```http
POST /v1/store-management/developer-accounts/{accountId}/apps/{appId}/marketing-pages
```

用于创建 App Store Connect 自定义产品页面草稿，不会同步到真实商店。

参数：

| 参数 | 位置 | 必填 | 说明 |
| --- | --- | --- | --- |
| `accountId` | path | 是 | 开发者账号 ID |
| `appId` | path | 是 | testflying 内部 iOS App ID |
| `pageName` | form | 是 | 页面名称 |
| `sourceLocale` | form | 否 | 源语言，默认 `en-US` |
| `locales` | form | 是 | 语言列表，多个值逗号分隔 |
| `promotionalText__{locale}` | form | 否 | 页面宣传文本 |
| `storeImageFiles__phoneScreenshots__{locale}` | file | 否 | 手机截图 |
| `storeImageFiles__tabletScreenshots__{locale}` | file | 否 | 平板截图 |

```bash
curl -X POST "$BASE_URL/v1/store-management/developer-accounts/$ACCOUNT_ID/apps/$APP_ID/marketing-pages" \
  -H "Authorization: Bearer $TOKEN" \
  -F "pageName=Campaign A" \
  -F "locales=en-US" \
  -F "promotionalText__en-US=Campaign copy"
```

```json
{
  "pageId": "page-xxxx",
  "pageName": "Campaign A",
  "status": "draft",
  "locales": ["en-US"],
  "savedLocales": 1,
  "uploadedAssets": 0,
  "warnings": []
}
```

## 9. 同步默认商店页

```http
POST /v1/store-management/developer-accounts/{accountId}/apps/{appId}/sync-runs
```

用于把默认商店页草稿同步到真实商店。可同步文案、商店图、版本说明。后台 UI 和第三方电脑直接调用都走同一条 connector 同步链路。

参数：

| 参数 | 位置 | 必填 | 说明 |
| --- | --- | --- | --- |
| `accountId` | path | 是 | 开发者账号 ID |
| `appId` | path | 是 | testflying 内部 App ID |
| `version` | body | 是 | 商店版本号 |
| `locales` | body | 是 | 需要同步的语言 |
| `scopes` | body | 否 | 默认 `["metadata","release_notes","store_images"]` |
| `iosAppId` | body | 否 | iOS 直传 App Store Connect 数字 App ID；传了就直接下发给 connector，不再使用后台绑定值 |
| `packageName` | body | 否 | Android 直传 Google Play package name；传了就直接下发给 connector，不再使用后台绑定值 |
| `storeTrack` | body | 否 | Android 版本说明目标轨道 |
| `storeVersionCode` | body | 否 | Android 版本说明目标 versionCode |
| `actor` | body | 否 | 操作来源 |
| `idempotencyKey` | body | 否 | 幂等键 |

```bash
curl -X POST "$BASE_URL/v1/store-management/developer-accounts/$ACCOUNT_ID/apps/$APP_ID/sync-runs" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"version":"1.0.0","locales":["en-US"],"scopes":["metadata","release_notes","store_images"],"iosAppId":"1234567890","packageName":"com.example.android","idempotencyKey":"sync-001"}'
```

```json
{
  "status": "succeeded",
  "accountId": "738W4ARM22",
  "appId": "app-ios-com-app-qw-readink",
  "version": "1.0.0",
  "scopes": ["metadata", "release_notes", "store_images"],
  "locales": ["en-US"],
  "runs": [],
  "idempotent": false
}
```

## 10. 同步自定义产品页面

```http
POST /v1/store-management/developer-accounts/{accountId}/apps/{appId}/marketing-pages/{pageId}/sync-runs
```

用于把指定自定义产品页面草稿同步到 App Store Connect。

参数：

| 参数 | 位置 | 必填 | 说明 |
| --- | --- | --- | --- |
| `accountId` | path | 是 | 开发者账号 ID |
| `appId` | path | 是 | testflying 内部 iOS App ID |
| `pageId` | path | 是 | 自定义产品页面 ID |
| `locales` | body | 是 | 需要同步的语言 |
| `scopes` | body | 否 | 默认 `["marketing_text","store_images"]` |
| `iosAppId` | body | 否 | 直传 App Store Connect 数字 App ID；传了就直接下发给 connector，不再使用后台绑定值 |
| `actor` | body | 否 | 操作来源 |
| `idempotencyKey` | body | 否 | 幂等键 |

```bash
curl -X POST "$BASE_URL/v1/store-management/developer-accounts/$ACCOUNT_ID/apps/$APP_ID/marketing-pages/$PAGE_ID/sync-runs" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"locales":["en-US"],"scopes":["marketing_text","store_images"],"iosAppId":"1234567890","idempotencyKey":"page-sync-001"}'
```

```json
{
  "status": "succeeded",
  "pageId": "page-xxxx",
  "scopes": ["marketing_text", "store_images"],
  "locales": ["en-US"],
  "runs": [],
  "idempotent": false
}
```

## 11. 查询产品页面优化

```http
GET /v1/store-management/developer-accounts/{accountId}/apps/{appId}/product-page-optimizations
```

用于查询 App Store Connect 产品页面优化实验。

参数：

| 参数 | 位置 | 必填 | 说明 |
| --- | --- | --- | --- |
| `accountId` | path | 是 | 开发者账号 ID |
| `appId` | path | 是 | testflying 内部 iOS App ID |
| `iosAppId` / `storeAppId` / `appleAppId` | query | 否 | iOS 真实 App Store Connect 数字 App ID |

```bash
curl "$BASE_URL/v1/store-management/developer-accounts/$ACCOUNT_ID/apps/$APP_ID/product-page-optimizations?iosAppId=1234567890" \
  -H "Authorization: Bearer $TOKEN"
```

```json
{
  "accountId": "738W4ARM22",
  "appId": "app-ios-com-app-qw-readink",
  "experiments": []
}
```

## 12. 创建产品页面优化

```http
POST /v1/store-management/developer-accounts/{accountId}/apps/{appId}/product-page-optimizations
```

用于创建 App Store Connect 产品页面优化实验。

参数：

| 参数 | 位置 | 必填 | 说明 |
| --- | --- | --- | --- |
| `accountId` | path | 是 | 开发者账号 ID |
| `appId` | path | 是 | testflying 内部 iOS App ID |
| `name` | body | 是 | 实验名称 |
| `trafficProportion` | body | 否 | 流量比例，默认 `50` |
| `locales` | body | 否 | 实验语言 |
| `treatments` | body | 是 | 实验变体 |
| `iosAppId` | body | 否 | iOS 真实 App Store Connect 数字 App ID；兼容 `storeAppId` / `appleAppId` |
| `idempotencyKey` | body | 否 | 幂等键 |

```bash
curl -X POST "$BASE_URL/v1/store-management/developer-accounts/$ACCOUNT_ID/apps/$APP_ID/product-page-optimizations" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"name":"Landing Test","trafficProportion":50,"locales":["en-US"],"treatments":[{"name":"Variant A"}],"iosAppId":"1234567890","idempotencyKey":"ppo-001"}'
```

```json
{
  "accountId": "738W4ARM22",
  "appId": "app-ios-com-app-qw-readink",
  "experiment": {
    "id": "ppo-xxxx",
    "name": "Landing Test",
    "state": "PREPARE_FOR_SUBMISSION"
  },
  "idempotent": false
}
```

## 13. 用户反馈问题分类

```http
POST /v1/llm/feedback-classifications
```

根据外部系统传入的用户反馈内容，返回问题类型、是否是 bug、是否是建议、重要性和内部处理建议。

这个接口只调用已配置的 LLM，不会连接 App Store Connect 或 Google Play，也不会保存分析记录。

参数：

| 参数 | 位置 | 必填 | 说明 |
| --- | --- | --- | --- |
| `feedbackId` | body | 否 | 外部系统自己的反馈 ID，会原样返回 |
| `content` | body | 否 | 用户反馈原文，最多 8000 字；和 `images` 至少提供一个 |
| `title` | body | 否 | 反馈标题 |
| `source` | body | 否 | 来源，例如 `app_store`、`google_play`、`in_app`、`support`、`manual` |
| `platform` | body | 否 | 平台，例如 `ios`、`android`、`web`、`unknown` |
| `app` | body | 否 | App 上下文，支持 `id`、`name`、`version` |
| `locale` | body | 否 | 反馈语言，默认 `zh-CN` |
| `context` | body | 否 | 额外上下文，例如评分、设备、系统版本、标签 |
| `images` | body | 否 | 图片数组，最多 5 张。OpenAI 兼容模型支持 `http(s)` 图片 URL 或 `data:image/...;base64,...`；Claude 兼容模型第一版只支持 `data:image/...;base64,...` |

`images[]` 字段：

| 参数 | 必填 | 说明 |
| --- | --- | --- |
| `url` | 是 | 图片地址。中心后台不下载、不保存图片，URL 需要能被 LLM 服务访问 |
| `name` | 否 | 图片名称 |
| `mimeType` | 否 | 图片 MIME 类型，例如 `image/png` |
| `detail` | 否 | `auto`、`low` 或 `high`，默认 `auto` |

```bash
curl -X POST "$BASE_URL/v1/llm/feedback-classifications" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "feedbackId": "fb-1001",
    "content": "打不开，一直闪退，更新后就这样",
    "source": "app_store",
    "platform": "ios",
    "app": {
      "id": "com.example.app",
      "name": "Example",
      "version": "1.0.0"
    },
    "context": {
      "rating": 1,
      "device": "iPhone 15",
      "osVersion": "iOS 18"
    },
    "images": [
      {
        "url": "https://cdn.example.com/feedback/fb-1001-screen.png",
        "name": "用户截图",
        "mimeType": "image/png",
        "detail": "high"
      }
    ]
  }'
```

```json
{
  "feedbackId": "fb-1001",
  "category": "crash",
  "categoryLabel": "闪退问题",
  "isBug": true,
  "isSuggestion": false,
  "severity": "high",
  "priority": "p1",
  "confidence": 0.88,
  "summary": "用户反馈新版本持续闪退。",
  "problem": "应用打开后异常退出。",
  "evidence": ["打不开", "一直闪退"],
  "suggestedAction": "优先排查新版本启动崩溃。",
  "routing": {
    "team": "client",
    "labels": ["crash", "ios"]
  },
  "needsHumanReview": false,
  "model": {
    "provider": "configured",
    "protocol": "openai_compatible",
    "model": "mimo-v2.5-pro"
  }
}
```

## 错误格式

```json
{
  "code": "invalid_static_token",
  "message": "接口 token 不正确",
  "retryable": false
}
```

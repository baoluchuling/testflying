# iOS 商店评论接口使用文档

本文档说明第三方电脑如何通过 testflying 中心后台读取 App Store Connect 中某个 iOS App 的用户评论。

## 1. 接口信息

```http
GET /v1/store-management/developer-accounts/{accountId}/apps/{appId}/store-reviews
Authorization: Bearer <TESTFLYING_STATIC_TOKEN>
```

当前中心后台地址：

```text
http://47.90.163.122:8000
```

此接口经过以下调用链读取数据：

```text
第三方电脑 -> testflying 中心后台 -> 账号 Connector -> App Store Connect
```

第三方电脑只需要中心后台的静态访问 Token。Connector Token 和 Apple 私钥由中心后台及对应 Connector 内部使用，不需要传给第三方电脑。

## 2. 鉴权 Token

请求头格式：

```http
Authorization: Bearer <TESTFLYING_STATIC_TOKEN>
```

Token 来自中心后台部署环境变量：

```text
TESTFLYING_STATIC_TOKEN
```

调用方应通过安全的配置或密钥管理渠道取得 Token，不要把 Token 写入代码仓库、日志或公开文档。

## 3. 三种 ID

| 名称 | 位置 | 示例 | 说明 |
| --- | --- | --- | --- |
| `accountId` | URL path | `738W4ARM22` | testflying 中的开发者账号 ID |
| `appId` | URL path | `app-ios-com-boluchuling-app-lookrva` | testflying 内部 App ID |
| `iosAppId` | query | `1234567890` | App Store Connect 数字 App ID |

`appId` 不是 App Store Connect 数字 App ID，也不是 Bundle ID。

如果 testflying 中的 App 已正确绑定 App Store Connect 数字 App ID，可以不传 `iosAppId`。第三方调用时建议显式传入 `iosAppId`，这样目标更明确。

## 4. 请求参数

| 参数 | 位置 | 必填 | 说明 |
| --- | --- | --- | --- |
| `accountId` | path | 是 | testflying 开发者账号 ID |
| `appId` | path | 是 | testflying 内部 App ID |
| `iosAppId` | query | 建议 | App Store Connect 数字 App ID |
| `pageSize` | query | 否 | 每页数量，默认 `50`，iOS 最大 `200` |
| `pageToken` | query | 否 | 上一页响应中的 `nextPageToken`；第一页不传 |
| `sort` | query | 否 | 排序方式，默认 `-createdDate`，即最新评论优先 |
| `date` | query | 否 | 指定日期，格式 `YYYY-MM-DD` |
| `startDate` | query | 否 | 开始日期，格式 `YYYY-MM-DD` |
| `endDate` | query | 否 | 结束日期，格式 `YYYY-MM-DD` |
| `timezone` | query | 否 | 日期过滤时区，默认 `Asia/Shanghai` |
| `locale` | query | 否 | 按评论语言精确过滤，例如 `en-US` |
| `territory` | query | 否 | 按地区精确过滤，例如 `US` |
| `rating` | query | 否 | 按评分过滤，范围 `1` 到 `5` |

注意：

- `date` 不能和 `startDate`、`endDate` 同时使用。
- 日期、语言、地区和评分筛选由中心后台对当前页结果执行。
- 要获取全部评论，必须逐页请求直到 `nextPageToken` 为空。
- 即使某一页经过筛选后 `reviews` 为空，只要 `nextPageToken` 仍有值，就必须继续请求下一页。

## 5. 获取第一页评论

先设置变量：

```bash
BASE_URL="http://47.90.163.122:8000"
TOKEN="<中心后台 TESTFLYING_STATIC_TOKEN>"
ACCOUNT_ID="<开发者账号 ID>"
APP_ID="<testflying 内部 App ID>"
IOS_APP_ID="<App Store Connect 数字 App ID>"
```

请求最新的 200 条评论：

```bash
curl --fail-with-body --silent --show-error --get \
  "$BASE_URL/v1/store-management/developer-accounts/$ACCOUNT_ID/apps/$APP_ID/store-reviews" \
  -H "Authorization: Bearer $TOKEN" \
  --data-urlencode "iosAppId=$IOS_APP_ID" \
  --data-urlencode "pageSize=200"
```

只读取指定日期、五星的评论：

```bash
curl --fail-with-body --silent --show-error --get \
  "$BASE_URL/v1/store-management/developer-accounts/$ACCOUNT_ID/apps/$APP_ID/store-reviews" \
  -H "Authorization: Bearer $TOKEN" \
  --data-urlencode "iosAppId=$IOS_APP_ID" \
  --data-urlencode "pageSize=200" \
  --data-urlencode "date=2026-08-12" \
  --data-urlencode "timezone=Asia/Shanghai" \
  --data-urlencode "rating=5"
```

## 6. 拉取全部评论

以下脚本需要安装 `jq`。它会持续请求下一页，并将所有评论保存为 JSON Lines 文件，每行一条评论。

```bash
#!/usr/bin/env bash
set -euo pipefail

BASE_URL="http://47.90.163.122:8000"
TOKEN="<中心后台 TESTFLYING_STATIC_TOKEN>"
ACCOUNT_ID="<开发者账号 ID>"
APP_ID="<testflying 内部 App ID>"
IOS_APP_ID="<App Store Connect 数字 App ID>"
OUTPUT_FILE="ios-store-reviews.jsonl"

PAGE_TOKEN=""
PAGE_NUMBER=1
TOTAL=0

: > "$OUTPUT_FILE"

while true; do
  RESPONSE_FILE="$(mktemp)"
  trap 'rm -f "$RESPONSE_FILE"' EXIT

  CURL_ARGS=(
    --fail-with-body
    --silent
    --show-error
    --get
    "$BASE_URL/v1/store-management/developer-accounts/$ACCOUNT_ID/apps/$APP_ID/store-reviews"
    -H "Authorization: Bearer $TOKEN"
    --data-urlencode "iosAppId=$IOS_APP_ID"
    --data-urlencode "pageSize=200"
  )

  if [[ -n "$PAGE_TOKEN" ]]; then
    CURL_ARGS+=(--data-urlencode "pageToken=$PAGE_TOKEN")
  fi

  curl "${CURL_ARGS[@]}" > "$RESPONSE_FILE"

  PAGE_COUNT="$(jq '.reviews | length' "$RESPONSE_FILE")"
  jq -c '.reviews[]' "$RESPONSE_FILE" >> "$OUTPUT_FILE"
  TOTAL=$((TOTAL + PAGE_COUNT))

  echo "第 $PAGE_NUMBER 页：$PAGE_COUNT 条，累计 $TOTAL 条"

  PAGE_TOKEN="$(jq -r '.nextPageToken // empty' "$RESPONSE_FILE")"
  rm -f "$RESPONSE_FILE"
  trap - EXIT

  if [[ -z "$PAGE_TOKEN" ]]; then
    break
  fi

  PAGE_NUMBER=$((PAGE_NUMBER + 1))
done

echo "完成：共 $TOTAL 条，文件：$OUTPUT_FILE"
```

运行：

```bash
chmod +x fetch-all-ios-reviews.sh
./fetch-all-ios-reviews.sh
```

输出文件 `ios-store-reviews.jsonl` 示例：

```json
{"id":"1234567890","platform":"ios","rating":5,"title":"Great","body":"Works well.","authorName":"reader","locale":"en-US","territory":"US","appVersion":"1.0.0","createdAt":"2026-08-12T10:00:00Z"}
```

## 7. 响应格式

```json
{
  "accountId": "738W4ARM22",
  "appId": "app-ios-com-boluchuling-app-lookrva",
  "platform": "ios",
  "reviews": [
    {
      "id": "1234567890",
      "platform": "ios",
      "rating": 5,
      "title": "Great",
      "body": "Works well.",
      "authorName": "reader",
      "reviewerNickname": "reader",
      "locale": "en-US",
      "territory": "US",
      "appVersion": "1.0.0",
      "createdAt": "2026-08-12T10:00:00Z",
      "updatedAt": "2026-08-12T10:00:00Z"
    }
  ],
  "nextPageToken": "下一页游标；最后一页为空字符串",
  "filters": {
    "startAt": "",
    "endAt": "",
    "timezone": "Asia/Shanghai",
    "locale": "",
    "territory": "",
    "rating": null
  }
}
```

关键字段：

| 字段 | 说明 |
| --- | --- |
| `reviews` | 当前页经过中心后台筛选后的评论数组 |
| `nextPageToken` | 下一页游标；为空表示已经到最后一页 |
| `filters` | 中心后台实际采用的筛选条件 |

## 8. 常见错误

### 401：接口 Token 不正确

```json
{
  "code": "invalid_static_token",
  "message": "接口 token 不正确"
}
```

检查请求头是否为：

```http
Authorization: Bearer <TESTFLYING_STATIC_TOKEN>
```

不要使用 Connector Token、Apple Key ID 或 Apple 私钥作为此处的 Bearer Token。

### 404：账号或 App 不存在

检查 URL 中的 `accountId` 和 `appId` 是否为 testflying 内部 ID，并确认该 App 已绑定到对应开发者账号。

### 422：筛选参数不正确

常见原因：

- `date` 和 `startDate` / `endDate` 同时传入。
- 日期不是 `YYYY-MM-DD` 格式。
- `startDate` 晚于 `endDate`。
- `timezone` 不是有效的 IANA 时区名称。
- `rating` 不在 `1` 到 `5` 范围内。

### 502：Connector 或 App Store Connect 调用失败

检查：

- 对应开发者账号的 Connector 是否在线。
- Connector 是否配置了正确的 Apple Issuer ID、Key ID 和私钥。
- `iosAppId` 是否为目标 App 的 App Store Connect 数字 ID。
- Apple API 凭据是否有读取评论的权限。

## 9. 安全建议

- 为测试和生产环境配置不同的 `TESTFLYING_STATIC_TOKEN`。
- 通过环境变量或密钥管理系统向调用程序注入 Token。
- 不要将完整 Authorization 请求头写入日志。
- 当前线上地址是 HTTP。跨公网正式使用前应配置 HTTPS，避免 Bearer Token 明文传输。

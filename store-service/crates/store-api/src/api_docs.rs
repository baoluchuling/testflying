//! Authenticated discovery of the center's public contract; no node calls.
use super::*;

const MARKDOWN: &str = include_str!("../../../docs/store-api.md");
const OPENAPI: &str = include_str!("../../../docs/openapi.yaml");
const PATH: &str = "/v1/store-management/api-docs";
const APP: &str = "/v1/store-management/apps/{platform}/{bundleIdentifier}";

#[derive(Deserialize, Default)]
pub(super) struct DocumentQuery {
    format: Option<String>,
}

pub(super) async fn read(Query(query): Query<DocumentQuery>) -> Result<Response> {
    let response = match query.format.as_deref().unwrap_or("json") {
        "json" => Json(catalog()).into_response(),
        "markdown" => (
            [(header::CONTENT_TYPE, "text/markdown; charset=utf-8")],
            MARKDOWN,
        )
            .into_response(),
        "openapi" => (
            [(header::CONTENT_TYPE, "application/yaml; charset=utf-8")],
            OPENAPI,
        )
            .into_response(),
        _ => {
            return Err(StoreError::new(
                422,
                "invalid_document_format",
                "Supported documentation formats: json, markdown, openapi",
            )
            .into());
        }
    };
    Ok(response)
}

fn catalog() -> Value {
    let locale = json!({"name":"locale","in":"query","type":"string","required":false,
        "description":"商店语言，去首尾空白、忽略大小写精确匹配；省略或空白查全部，未命中不回退"});
    let device = json!({"name":"device","in":"query","type":"string","required":false,
        "enum":["phone","tablet","foldable"],"omittedBehavior":"all",
        "description":"省略或空白查全部；Android 不支持 foldable，不传 all"});
    let mut endpoints = vec![
        endpoint(
            1,
            "GET",
            "supported-locales",
            "查询现有商店语言",
            json!([]),
            None,
        ),
        endpoint(2, "GET", "store-listings", "查询商店文案", json!([]), None),
        endpoint(
            3,
            "GET",
            "store-images",
            "按语言和设备查询图片、截图",
            json!([locale, device]),
            None,
        ),
        endpoint(
            4,
            "GET",
            "store-releases",
            "查询版本和状态；iOS 返回各版本及是否可编辑",
            json!([]),
            None,
        ),
        endpoint(
            5,
            "GET",
            "store-reviews",
            "查询评论及当前页筛选",
            json!([
                {"name":"pageSize","in":"query","type":"integer","default":50,"minimum":1,"maximumByPlatform":{"ios":200,"android":100}},
                {"name":"pageToken","in":"query","type":"string"},
                {"name":"startIndex","in":"query","type":"string","platforms":["android"]},
                {"name":"translationLanguage","in":"query","type":"string","platforms":["android"]},
                {"name":"sort","in":"query","type":"string","platforms":["ios"],"default":"-createdDate"},
                {"name":"rating","in":"query","type":"integer","minimum":1,"maximum":5},
                {"name":"locale","in":"query","type":"string"},
                {"name":"territory","in":"query","type":"string"},
                {"name":"date","in":"query","type":"string","format":"date"},
                {"name":"startDate","in":"query","type":"string","format":"date"},
                {"name":"endDate","in":"query","type":"string","format":"date"},
                {"name":"timezone","in":"query","type":"string","default":"Asia/Shanghai"}
            ]),
            None,
        ),
        endpoint(
            6,
            "GET",
            "product-page-optimizations",
            "查询 Apple PPO；Android 返回空列表",
            json!([]),
            None,
        ),
        endpoint(
            7,
            "POST",
            "product-page-optimizations",
            "创建 Apple PPO；仅 iOS",
            json!([]),
            Some("PPO"),
        ),
        endpoint(
            8,
            "POST",
            "preflight",
            "写入前检查；响应须判断 canSync",
            json!([]),
            Some("Preflight"),
        ),
        endpoint(
            9,
            "POST",
            "sync",
            "同步文案、版本说明、图片或 CPP；响应须判断 status",
            json!([]),
            Some("Sync"),
        ),
    ];
    endpoints.push(endpoint(
        10,
        "POST",
        "store-releases",
        "创建 iOS App Store 版本，不送审",
        json!([]),
        Some("CreateStoreRelease"),
    ));
    endpoints.push(json!({"id":"list-connectors","group":"node","method":"GET",
        "path":"/v1/store-management/connectors","summary":"查询全部启用的商店节点",
        "authentication":["integrationToken","businessToken"],"requiresBinding":false,"parameters":[]}));
    endpoints.push(json!({"id":"bind-connector","group":"node","method":"PUT",
        "path":"/v1/apps/{platform}/{bundleIdentifier}/connector","summary":"绑定或更换应用的商店节点",
        "authentication":["integrationToken","adminToken"],"requiresBinding":false,
        "requestBody":{"required":["connectorId"],"properties":{"connectorId":{"type":"string","format":"uuid"}}}}));
    endpoints.extend([
        json!({"id":"list-screenshot-connectors","group":"screenshot","method":"GET","path":"/v1/screenshot-connectors","summary":"列出启用的截图 Mac 节点及连接状态","authentication":["integrationToken"],"transport":["https"],"requiresBinding":false}),
        json!({"id":"bind-screenshot-connector","group":"screenshot","method":"PUT","path":"/v1/apps/{platform}/{bundleIdentifier}/screenshot-connector","summary":"绑定或更换应用的截图节点，与商店绑定独立","authentication":["integrationToken","adminToken"],"transport":["https"],"requiresBinding":false,"requestSchema":"#/components/schemas/ScreenshotBindingInput"}),
        json!({"id":"read-screenshot-binding","group":"screenshot","method":"GET","path":"/v1/apps/{platform}/{bundleIdentifier}/screenshot-connector","summary":"查询应用当前截图节点绑定","authentication":["integrationToken","adminToken"],"transport":["https"],"requiresBinding":false}),
        json!({"id":"unbind-screenshot-connector","group":"screenshot","method":"DELETE","path":"/v1/apps/{platform}/{bundleIdentifier}/screenshot-connector","summary":"解除截图绑定；已创建任务保留原节点","authentication":["integrationToken","adminToken"],"transport":["https"],"requiresBinding":false}),
        json!({"id":"submit-screenshot-job","group":"screenshot","method":"POST","path":"/v1/screenshot-jobs","summary":"按平台和包名触发截图，中心选择已绑定的 Mac","authentication":["integrationToken"],"transport":["https"],"requiresBinding":true,"bindingResource":"screenshot-connector","requestSchema":"#/components/schemas/ScreenshotJobInput"}),
        json!({"id":"read-screenshot-job","group":"screenshot","method":"GET","path":"/v1/screenshot-jobs/{jobId}","summary":"用创建任务的同一枚接入 Token 查询结果","authentication":["integrationToken"],"transport":["https"],"requiresBinding":false}),
    ]);
    json!({
        "schemaVersion":1,"contractVersion":"2026-11-11","title":"中心统一接口文档：商店、节点与截图任务",
        "scope":"center-api-contract","requiresConnectorUpgrade":true,
        "description":"中心公开接口契约；不是对节点在线状态、本地平台配置或商店账号权限的实时探测",
        "authentication":{"integrationToken":{"transport":["https"],"header":"Authorization: Bearer INTEGRATION_TOKEN"},
            "businessToken":{"transport":["http","https"],"header":"Authorization: Bearer STORE_API_TOKEN","canBind":false},
            "adminToken":{"transport":["http","https"],"canReadApiDocs":false,"canBind":true}},
        "applicationIdentity":{"platform":["ios","android"],"identifier":"iOS Bundle ID / Android 包名，大小写精确匹配"},
        "endpoints":endpoints,
        "endpointCounts":{"store":10,"node":2,"screenshot":6,"uploads":2},
        "screenshots":{"requiresConnectorUpgrade":false,"routing":"exact platform + bundleIdentifier screenshot binding; no random node selection; existing jobs keep their assigned node",
            "required":["platform","bundleIdentifier","prompt"],"connectorIdInSubmit":false,"requiresStoreBinding":false,"requiresStoreCredentials":false,
            "states":["queued","running","uploading","succeeded","failed"],"taskVisibility":"same issuing integration token only",
            "prompt":{"maxUtf8Bytes":16384,"preservedVerbatim":true,"appIdentityProvidedToWorker":true},
            "limits":{"queuedPerNode":20,"parallelJobsPerNode":1,"retentionDays":7,"maxPngBytes":16777216},
            "centerSavesImage":false,"missingUploadDestinationDoesNotBlockExecution":true,"storeForwardingSwitchesApply":false},
        "adminDiagnostics":{"authentication":["adminToken"],"method":"GET","path":"/v1/connectors/{id}/logs/latest",
            "summary":"读取指定商店节点最新 JSONL 日志文件；中心流式转发，不保存文件副本",
            "requiresConnectorUpgrade":true,"oldNodeError":{"status":409,"code":"logs_upgrade_required"}},
        "forwarding":{"administratorRead":"/v1/forwarding","administratorUpdate":"/v1/forwarding/{platform}","requestBody":{"enabled":"boolean"},"disabledError":{"status":503,"code":"platform_forwarding_disabled"},"scope":"new store reads, writes and image uploads; existing admitted requests continue; connections, diagnostics and docs remain available"},
        "iosVersions":{"creation":{"method":"POST","path":format!("{APP}/store-releases"),"platform":"ios","required":["versionString"],"releaseType":"MANUAL","submitsForReview":false},"listFields":["id","versionString","platform","state","appStoreState","appVersionState","editable","isLatest","createdDate"],"validationOwner":"connector","selection":"latest version only for writes; locked versions stop writes, read-only queries remain available"},
        "uploadEndpoints":[
            {"method":"POST","path":format!("{APP}/uploads"),"summary":"申请图片上传凭据","requestSchema":"#/components/schemas/UploadInput","authentication":["integrationToken","businessToken"]},
            {"method":"PUT","path":format!("{APP}/uploads/{{assetId}}"),"summary":"按返回的 uploadUrl 上传原始文件字节","authentication":["uploadToken"],"contentTypes":["image/png","image/jpeg"],"maxBytes":67108864}
        ],
        "imageQuery":{"resource":"store-images","parameters":[locale,device],
            "localeSource":"supported-locales 返回当前应用已有语言，不是平台全量语言字典",
            "ios":{"versionRequired":false,"devices":{"phone":["phone_screenshots"],"tablet":["tablet_screenshots"],"foldable":["foldable_screenshots"]},"provider":"apple-asset-library-placements"},
            "android":{"versionRequired":false,"devices":{"phone":["phone_screenshots"],"tablet":["seven_inch_screenshots","ten_inch_screenshots"]},"unsupportedDevice":{"foldable":{"status":422,"code":"unsupported_image_device"}}},
            "examples":[format!("{APP}/store-images?locale=en-US&device=phone"),format!("{APP}/store-images?locale=zh-Hans&device=tablet"),format!("{APP}/store-images?locale=ja&device=foldable")]
        },
        "sync":{"versionSelection":"iOS 节点自动定位最新版本；顶层 version 不接收或转发，最新版本不可编辑直接失败，不回退到旧版本","operations":["update_app_metadata","update_release_notes","update_marketing_page"],
            "androidCommit":{"changesNotSentForReview":"omitted","changesInReviewBehavior":"ERROR_IF_IN_REVIEW","maySendForReview":true,"draftOnly":false},
            "syncScopes":["metadata","description","keywords","promotional_text","short_description","title","subtitle","video","store_images","marketing_text"],
            "imageSlots":{"ios":["phone_screenshots","tablet_screenshots","foldable_screenshots"],"android":["phone_screenshots","tablet_screenshots","feature_graphic_url"]},
            "selection":"读取用 query device，写入用 metadata.storeImages 或 marketingPage.storeImages 中的槽位；未传槽位保留、显式空数组清空所选槽位",
            "textSelection":"未传字段保留；选中文字字段的显式空字符串按清空处理；版本说明通过 update_release_notes 更新",
            "appleImages":"先上传 Asset Library 并确认处理和规格，再替换 placements 并排序；多步写入非事务，不自动重放"
        },
        "limitations":["Android 不支持创建新版本、独立折叠屏图片、CPP 或创建 PPO","无独立通用送审、发布、评论回复、CPP 列表或删除、PPO 启停接口"],
        "documentation":{"formats":["json","markdown","openapi"],"markdownUrl":format!("{PATH}?format=markdown"),"openapiUrl":format!("{PATH}?format=openapi"),"markdown":MARKDOWN}
    })
}

fn endpoint(
    number: u8,
    method: &str,
    resource: &str,
    summary: &str,
    parameters: Value,
    schema: Option<&str>,
) -> Value {
    let mut value = json!({"number":number,"id":format!("{}-{resource}",method.to_ascii_lowercase()),
        "group":"store","method":method,"path":format!("{APP}/{resource}"),"summary":summary,
        "authentication":["integrationToken","businessToken"],"requiresBinding":true,"parameters":parameters});
    if let Some(schema) = schema {
        value["requestSchema"] = json!(format!("#/components/schemas/{schema}"));
    }
    value
}

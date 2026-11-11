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
            "查询 release；iOS 当前返回空列表",
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
    endpoints.push(json!({"id":"list-connectors","group":"node","method":"GET",
        "path":"/v1/store-management/connectors","summary":"查询全部启用的商店节点",
        "authentication":["integrationToken","businessToken"],"requiresBinding":false,"parameters":[]}));
    endpoints.push(json!({"id":"bind-connector","group":"node","method":"PUT",
        "path":"/v1/apps/{platform}/{bundleIdentifier}/connector","summary":"绑定或更换应用的商店节点",
        "authentication":["integrationToken","adminToken"],"requiresBinding":false,
        "requestBody":{"required":["connectorId"],"properties":{"connectorId":{"type":"string","format":"uuid"}}}}));
    json!({
        "schemaVersion":1,"contractVersion":"2026-11-11","title":"商店业务与节点接入接口",
        "scope":"center-api-contract","requiresConnectorUpgrade":true,
        "description":"中心公开接口契约；不是对节点在线状态、本地平台配置或商店账号权限的实时探测",
        "authentication":{"integrationToken":{"transport":["https"],"header":"Authorization: Bearer INTEGRATION_TOKEN"},
            "businessToken":{"transport":["http","https"],"header":"Authorization: Bearer STORE_API_TOKEN","canBind":false},
            "adminToken":{"transport":["http","https"],"canReadApiDocs":false,"canBind":true}},
        "applicationIdentity":{"platform":["ios","android"],"identifier":"iOS Bundle ID / Android 包名，大小写精确匹配"},
        "endpoints":endpoints,
        "adminDiagnostics":{"authentication":["adminToken"],"method":"GET","path":"/v1/connectors/{id}/logs/latest",
            "summary":"读取指定商店节点最新 JSONL 日志文件；中心流式转发，不保存文件副本",
            "requiresConnectorUpgrade":true,"oldNodeError":{"status":409,"code":"logs_upgrade_required"}},
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
        "limitations":["iOS release 列表未实现","Android 不支持独立折叠屏图片、CPP 或创建 PPO","无通用送审、发布、评论回复、CPP 列表或删除、PPO 启停接口"],
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

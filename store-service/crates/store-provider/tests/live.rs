use axum::{
    body::{to_bytes, Body},
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    response::Response,
    Router,
};
use p256::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use store_protocol::{Operation, Platform, StoreRequest};
use store_provider::{Gateway, ProviderConfig};

#[derive(Clone, Debug)]
struct Record {
    method: String,
    uri: String,
    headers: HeaderMap,
    bytes: Vec<u8>,
}
impl Record {
    fn body(&self) -> Value {
        serde_json::from_slice(&self.bytes).unwrap_or(Value::Null)
    }
}
type Reply = (StatusCode, Vec<(&'static str, String)>, Vec<u8>);
type Responder = Arc<dyn Fn(&Record) -> Reply + Send + Sync>;
#[derive(Clone)]
struct FakeState {
    records: Arc<Mutex<Vec<Record>>>,
    respond: Responder,
}
struct Fake {
    url: String,
    records: Arc<Mutex<Vec<Record>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fake {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Fake {
    async fn new(respond: impl Fn(&Record) -> Reply + Send + Sync + 'static) -> Self {
        let records = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new().fallback(handler).with_state(FakeState {
            records: records.clone(),
            respond: Arc::new(respond),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self { url, records, task }
    }
    fn records(&self) -> Vec<Record> {
        self.records.lock().unwrap().clone()
    }
}
async fn handler(State(state): State<FakeState>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let bytes = to_bytes(body, 70 * 1024 * 1024).await.unwrap().to_vec();
    let record = Record {
        method: parts.method.to_string(),
        uri: parts.uri.to_string(),
        headers: parts.headers,
        bytes,
    };
    state.records.lock().unwrap().push(record.clone());
    // Enforce vendor request constraints independently of scenario responses.
    // This prevents permissive fake handlers from masking malformed live calls.
    let deprecated_screenshot_request = record.uri.contains("/appScreenshot");
    let invalid_locale_creation = invalid_apple_locale_creation(&record);
    let unsupported_google_review_flag =
        record.uri.starts_with("/androidpublisher/v3/applications/")
            && record.uri.contains(":commit?")
            && reqwest::Url::parse(&format!("http://fixture{}", record.uri))
                .unwrap()
                .query_pairs()
                .any(|(key, _)| key == "changesNotSentForReview");
    let (status, headers, bytes) = if unsupported_google_review_flag {
        (StatusCode::BAD_REQUEST,vec![],br#"{"error":{"status":"INVALID_ARGUMENT","message":"changesNotSentForReview must not be set for this application"}}"#.to_vec())
    } else if deprecated_screenshot_request
        || invalid_locale_creation
        || invalid_apple_library_request(&record)
    {
        (
            StatusCode::BAD_REQUEST,
            vec![],
            b"strict fake: invalid Apple request".to_vec(),
        )
    } else {
        (state.respond)(&record)
    };
    let mut response = Response::builder().status(status);
    for (k, v) in headers {
        response = response.header(k, v);
    }
    response.body(Body::from(bytes)).unwrap()
}
fn invalid_apple_library_request(record: &Record) -> bool {
    let body = record.body();
    let data = &body["data"];
    let relations = &data["relationships"];
    if record.method == "POST" && record.uri == "/v1/appAssetLibraryImages" {
        return data["type"] != "appAssetLibraryImages"
            || data["attributes"]["category"] != "APP_SCREENSHOTS_AND_PREVIEWS"
            || data["attributes"]["fileSize"]
                .as_u64()
                .is_none_or(|size| size == 0)
            || data["attributes"].get("sourceFileChecksum").is_some()
            || relations["assetLibrary"]["data"]["type"] != "appAssetLibraries"
            || relations["assetLibrary"]["data"]["id"] != "library";
    }
    if record.method == "PATCH" && record.uri.starts_with("/v1/appAssetLibraryImages/") {
        return data["type"] != "appAssetLibraryImages"
            || data["attributes"] != json!({"uploaded":true});
    }
    if record.method == "POST"
        && matches!(
            record.uri.as_str(),
            "/v1/appAssetLibraryPlacements" | "/v1/appAssetLibraryPlacementOrderingRequests"
        )
    {
        let owner_valid = [
            (
                "appStoreVersionLocalization",
                "appStoreVersionLocalizations",
            ),
            (
                "appCustomProductPageLocalization",
                "appCustomProductPageLocalizations",
            ),
        ]
        .iter()
        .filter(|(relation, kind)| {
            relations[*relation]["data"]["type"] == *kind
                && relations[*relation]["data"]["id"]
                    .as_str()
                    .is_some_and(|id| !id.is_empty())
        })
        .count()
            == 1;
        if !owner_valid
            || relations
                .as_object()
                .is_none_or(|relations| relations.len() != 2)
            || data["attributes"]["placementGroup"]
                .as_str()
                .is_none_or(|group| group.is_empty())
        {
            return true;
        }
        if record.uri == "/v1/appAssetLibraryPlacements" {
            return data["type"] != "appAssetLibraryPlacements"
                || data["attributes"]["placementType"] != "APP_SCREENSHOT"
                || relations["image"]["data"]["type"] != "appAssetLibraryImages";
        }
        return data["type"] != "appAssetLibraryPlacementOrderingRequests"
            || relations["orderedPlacements"]["data"]
                .as_array()
                .is_none_or(|ordered| {
                    ordered.is_empty()
                        || ordered
                            .iter()
                            .any(|placement| placement["type"] != "appAssetLibraryPlacements")
                });
    }
    false
}
fn invalid_apple_locale_creation(record: &Record) -> bool {
    if record.method != "POST" {
        return false;
    }
    let (kind, relationship, owner) = match record.uri.as_str() {
        "/v1/appInfoLocalizations" => ("appInfoLocalizations", "appInfo", "appInfos"),
        "/v1/appStoreVersionLocalizations" => (
            "appStoreVersionLocalizations",
            "appStoreVersion",
            "appStoreVersions",
        ),
        "/v1/appCustomProductPageLocalizations" => (
            "appCustomProductPageLocalizations",
            "appCustomProductPageVersion",
            "appCustomProductPageVersions",
        ),
        "/v1/appStoreVersionExperimentTreatmentLocalizations" => (
            "appStoreVersionExperimentTreatmentLocalizations",
            "appStoreVersionExperimentTreatment",
            "appStoreVersionExperimentTreatments",
        ),
        _ => return false,
    };
    let body = record.body();
    let data = &body["data"];
    data["type"] != kind
        || data["attributes"]["locale"]
            .as_str()
            .is_none_or(|locale| locale.trim().is_empty())
        || data["relationships"][relationship]["data"]["type"] != owner
        || data["relationships"][relationship]["data"]["id"]
            .as_str()
            .is_none_or(|id| id.is_empty())
        || (kind == "appInfoLocalizations"
            && data["attributes"]["name"]
                .as_str()
                .is_none_or(|name| name.trim().is_empty()))
}
fn ok(v: Value) -> Reply {
    (
        StatusCode::OK,
        vec![("content-type", "application/json".into())],
        serde_json::to_vec(&v).unwrap(),
    )
}
fn request(platform: Platform, operation: Operation, payload: Value) -> StoreRequest {
    StoreRequest {
        request_id: "test-request".into(),
        platform,
        bundle_identifier: "com.example.App".into(),
        operation,
        payload,
    }
}

#[tokio::test]
async fn apple_versions_report_all_states_and_only_writes_stop_when_latest_locked() {
    let fake=Fake::new(|r| {
        if r.uri.contains("/appStoreVersions?") {
            return ok(json!({"data":[
                {"id":"older","attributes":{"platform":"IOS","versionString":"1.0","createdDate":"2025-01-01T00:00:00Z","appStoreState":"PREPARE_FOR_SUBMISSION"}},
                {"id":"latest","attributes":{"platform":"IOS","versionString":"2.0","createdDate":"2026-01-01T00:00:00Z","appStoreState":"WAITING_FOR_REVIEW","appVersionState":"WAITING_FOR_REVIEW"}}
            ]}));
        }
        apple_base(r).unwrap_or_else(||ok(json!({})))
    }).await;
    let keys = Keys::new();
    let gateway = Gateway::new(keys.config(&fake.url)).unwrap();
    let response = gateway
        .execute(&request(Platform::Ios, Operation::StoreReleases, json!({})))
        .await
        .unwrap();
    assert_eq!(response["releases"].as_array().unwrap().len(), 2);
    assert_eq!(response["releases"][0]["id"], "latest");
    assert_eq!(response["releases"][0]["state"], "WAITING_FOR_REVIEW");
    assert_eq!(response["releases"][0]["editable"], false);
    assert_eq!(response["releases"][0]["isLatest"], true);
    assert_eq!(response["releases"][1]["editable"], true);
    for operation in [
        Operation::SupportedLocales,
        Operation::StoreListings,
        Operation::StoreImages,
    ] {
        gateway
            .execute(&request(Platform::Ios, operation, json!({})))
            .await
            .unwrap();
    }
    let reads = fake.records().len();
    let preflight = gateway
        .execute(&request(
            Platform::Ios,
            Operation::Preflight,
            json!({"operation":"update_app_metadata"}),
        ))
        .await
        .unwrap();
    assert_eq!(preflight["canSync"], false);
    let sync = gateway
        .execute(&request(
            Platform::Ios,
            Operation::Sync,
            json!({"operation":"update_release_notes","locale":"en-US","releaseNotes":"notes"}),
        ))
        .await
        .unwrap();
    assert_eq!(sync["errorCode"], "store_version_not_editable");
    assert!(fake.records()[reads..].iter().all(|r| r.method == "GET"
        && (r.uri.starts_with("/v1/apps?") || r.uri.contains("/appStoreVersions?"))));
    assert!(fake.records()[..reads]
        .iter()
        .any(|r| r.uri.contains("/appStoreVersionLocalizations")));
}

#[tokio::test]
async fn apple_create_version_is_manual_without_submission_and_validates_on_the_node() {
    let fake=Fake::new(|r| {
        if r.method=="POST"&&r.uri=="/v1/appStoreVersions" {
            let body=r.body();
            assert_eq!(body["data"]["type"],"appStoreVersions");
            assert_eq!(body["data"]["attributes"],json!({"platform":"IOS","versionString":"2.1","releaseType":"MANUAL","copyright":"Example"}));
            assert_eq!(body["data"]["relationships"]["app"]["data"],json!({"type":"apps","id":"resolved-app"}));
            return ok(json!({"data":{"id":"created-version","attributes":{"versionString":"2.1","appVersionState":"PREPARE_FOR_SUBMISSION"}}}));
        }
        apple_base(r).unwrap_or_else(||ok(json!({})))
    }).await;
    let keys = Keys::new();
    let gateway = Gateway::new(keys.config(&fake.url)).unwrap();
    for (p, payload) in [
        (Platform::Ios, json!({})),
        (Platform::Ios, json!({"versionString":123})),
        (Platform::Android, json!({"versionString":"2.1"})),
    ] {
        assert_eq!(
            gateway
                .execute(&request(p, Operation::CreateStoreRelease, payload))
                .await
                .unwrap_err()
                .status,
            422
        );
    }
    assert!(fake.records().is_empty());
    let created = gateway
        .execute(&request(
            Platform::Ios,
            Operation::CreateStoreRelease,
            json!({"versionString":"2.1","copyright":"Example","appId":"untrusted"}),
        ))
        .await
        .unwrap();
    assert_eq!(created["release"]["id"], "created-version");
    assert_eq!(created["release"]["editable"], true);
    let records = fake.records();
    assert_eq!(records.len(), 2);
    assert_eq!(records.iter().filter(|r| r.method == "POST").count(), 1);
}

#[tokio::test]
async fn google_empty_commit_declares_zero_length_and_records_safe_step_logs() {
    let fake = Fake::new(|r| {
        if r.uri.contains(":commit?") {
            if r.headers
                .get("content-length")
                .and_then(|v| v.to_str().ok())
                != Some("0")
                || !r.bytes.is_empty()
            {
                return (
                    StatusCode::LENGTH_REQUIRED,
                    vec![],
                    b"private-vendor-message".to_vec(),
                );
            }
            return (
                StatusCode::BAD_REQUEST,
                vec![],
                br#"{"error":{"status":"FAILED_PRECONDITION","message":"private-vendor-message: You already have changes in review; private-copy"}}"#.to_vec(),
            );
        }
        google_base(r).unwrap_or_else(|| ok(json!({})))
    })
    .await;
    let keys = Keys::new();
    let config_path = keys.dir.path().join("connector.json");
    std::fs::write(&config_path, "{}").unwrap();
    let log = store_provider::diagnostics::Log::for_config(&config_path, "test-node").unwrap();
    let gateway = Gateway::with_log(keys.config(&fake.url), None, Some(log.clone())).unwrap();
    let mut request = request(
        Platform::Android,
        Operation::Sync,
        json!({"operation":"update_app_metadata","locale":"en-US",
        "syncScopes":["title"],"metadata":{"title":"private-copy"},"token":"private-caller-token"}),
    );
    request.request_id = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into();
    assert_eq!(gateway.execute(&request).await.unwrap_err().status, 502);
    let records = fake.records();
    assert_eq!(
        records
            .iter()
            .filter(|r| r.uri.contains(":commit?"))
            .count(),
        1
    );
    assert_eq!(records.iter().filter(|r| r.method == "DELETE").count(), 1);
    let mut entries = Vec::<Value>::new();
    let mut all = String::new();
    for entry in std::fs::read_dir(log.directory()).unwrap() {
        let text = std::fs::read_to_string(entry.unwrap().path()).unwrap();
        all.push_str(&text);
        entries.extend(
            text.lines()
                .skip(1)
                .map(|line| serde_json::from_str::<Value>(line).unwrap()),
        );
    }
    let commit = entries
        .iter()
        .find(|entry| {
            entry["event"] == "upstream.completed" && entry["details"]["step"] == "edit_commit"
        })
        .unwrap();
    assert_eq!(commit["requestId"], request.request_id);
    assert_eq!(commit["details"]["contentLengthHeader"], 0);
    assert_eq!(commit["details"]["bodyBytes"], 0);
    assert_eq!(commit["details"]["status"], 400);
    let error = entries
        .iter()
        .find(|entry| {
            entry["event"] == "upstream.error" && entry["details"]["step"] == "edit_commit"
        })
        .unwrap();
    assert_eq!(error["details"]["vendor"]["code"], "FAILED_PRECONDITION");
    assert_eq!(
        error["details"]["vendor"]["classification"],
        "changes_in_review"
    );
    assert_eq!(error["details"]["errorBodyRead"], "complete");
    assert!(entries
        .iter()
        .any(|entry| entry["details"]["step"] == "edit_discard"));
    assert!(entries
        .iter()
        .any(|entry| entry["details"]["step"] == "oauth_token"));
    for secret in [
        "private-copy",
        "private-caller-token",
        "private-vendor-message",
        "Bearer ",
        "assertion",
        "access_token",
    ] {
        assert!(!all.contains(secret), "Leaked {secret}");
    }
}

#[tokio::test]
async fn apple_without_version_reads_and_updates_only_latest_and_never_falls_back() {
    let locked = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let switch = locked.clone();
    let fake=Fake::new(move |r| {
        if r.uri.contains("/appStoreVersions?") {
            let locked=switch.load(std::sync::atomic::Ordering::Relaxed);
            assert!(!r.uri.contains("sort="));
            return ok(json!({"data":[{"id":"old","attributes":{"versionString":"2.0","createdDate":"2025-01-01T00:00:00Z","appStoreState":"PREPARE_FOR_SUBMISSION"}},
                {"id":"latest","attributes":{"versionString":"3.0","createdDate":"2026-01-01T00:00:00Z","appStoreState":if locked{"IN_REVIEW"}else{"PREPARE_FOR_SUBMISSION"}}}]}));
        }
        apple_base(r).unwrap_or_else(||ok(json!({})))
    }).await;
    let keys = Keys::new();
    let gateway = Gateway::new(keys.config(&fake.url)).unwrap();
    let payload = json!({"operation":"update_app_metadata","locale":"en-US","syncScopes":["description"],"metadata":{"description":"new"}});
    assert_eq!(
        gateway
            .execute(&request(Platform::Ios, Operation::Sync, payload.clone()))
            .await
            .unwrap()["status"],
        "succeeded"
    );
    let versions: Vec<_> = fake
        .records()
        .into_iter()
        .filter(|r| r.uri.contains("/appStoreVersions?"))
        .collect();
    assert!(versions.iter().all(|r| !r.uri.contains("sort=")
        && r.uri.contains("limit=200")
        && !r.uri.contains("filter%5BversionString%5D")));
    assert!(fake
        .records()
        .iter()
        .any(|r| r.uri.contains("/latest/appStoreVersionLocalizations")));
    assert!(!fake.records().iter().any(|r| r.uri.contains("/old/")));
    let patches = fake
        .records()
        .iter()
        .filter(|r| r.method == "PATCH")
        .count();
    locked.store(true, std::sync::atomic::Ordering::Relaxed);
    let result = gateway
        .execute(&request(Platform::Ios, Operation::Sync, payload))
        .await
        .unwrap();
    assert_eq!(result["errorCode"], "store_version_not_editable");
    assert_eq!(
        fake.records()
            .iter()
            .filter(|r| r.method == "PATCH")
            .count(),
        patches
    );
    let preflight = gateway
        .execute(&request(
            Platform::Ios,
            Operation::Preflight,
            json!({"operation":"update_app_metadata"}),
        ))
        .await
        .unwrap();
    assert_eq!(preflight["canSync"], false);
}

#[tokio::test]
async fn apple_latest_version_is_selected_across_pages_before_editability_check() {
    let fake=Fake::new(|r| {
        if r.uri.contains("/appStoreVersions?") {
            assert!(!r.uri.contains("sort="));
            if r.uri.contains("cursor=two") {
                return ok(json!({"data":[{"id":"latest","attributes":{"platform":"IOS","versionString":"3.0","createdDate":"2026-01-01T00:00:00Z","appStoreState":"IN_REVIEW"}}]}));
            }
            return ok(json!({"data":[{"id":"old","attributes":{"platform":"IOS","versionString":"2.0","createdDate":"2025-01-01T00:00:00Z","appStoreState":"PREPARE_FOR_SUBMISSION"}}],
                "links":{"next":format!("{}?cursor=two",r.uri.split('?').next().unwrap())}}));
        }
        apple_base(r).unwrap_or_else(||ok(json!({})))
    }).await;
    let keys = Keys::new();
    let gateway = Gateway::new(keys.config(&fake.url)).unwrap();
    let result=gateway.execute(&request(Platform::Ios,Operation::Sync,json!({"operation":"update_release_notes","locale":"en-US","releaseNotes":"do not change old version"}))).await.unwrap();
    assert_eq!(result["errorCode"], "store_version_not_editable");
    assert_eq!(
        fake.records()
            .iter()
            .filter(|r| r.uri.contains("/appStoreVersions?"))
            .count(),
        2
    );
    assert!(fake.records().iter().all(|r| r.method == "GET"));
}
struct Keys {
    dir: tempfile::TempDir,
    apple_public: String,
    google_public: String,
}

fn catalog_app(platform: Platform, suffix: &str) -> Value {
    match platform {
        Platform::Ios => {
            json!({"id":suffix,"attributes":{"bundleId":format!("com.example.{suffix}"),"name":format!("App {suffix}")}})
        }
        Platform::Android => {
            json!({"name":format!("apps/{suffix}"),"packageName":format!("com.example.{suffix}"),"displayName":format!("App {suffix}")})
        }
    }
}
fn catalog_page(platform: Platform, suffix: &str, next: Option<&str>) -> Value {
    match platform {
        Platform::Ios => json!({"data":[catalog_app(platform,suffix)],"links":{"next":next}}),
        Platform::Android => json!({"apps":[catalog_app(platform,suffix)],"nextPageToken":next}),
    }
}

fn catalog_log_entries(log: &store_provider::diagnostics::Log) -> (Vec<Value>, String) {
    let mut entries = Vec::new();
    let mut raw = String::new();
    for file in std::fs::read_dir(log.directory()).unwrap() {
        let content = std::fs::read_to_string(file.unwrap().path()).unwrap();
        raw.push_str(&content);
        entries.extend(
            content
                .lines()
                .skip(1)
                .map(|line| serde_json::from_str::<Value>(line).unwrap()),
        );
    }
    (entries, raw)
}

#[tokio::test]
async fn catalog_failures_log_http_and_parse_stages_without_private_response_or_token() {
    for (case, stage, reason) in [
        ("unauthorized", "apps_response", "http_error"),
        ("forbidden", "apps_response", "http_error"),
        ("agreements", "apps_response", "http_error"),
        ("invalid_json", "apps_response", "invalid_json"),
        (
            "missing_field",
            "application_fields",
            "missing_bundle_identifier",
        ),
        ("duplicate", "validation", "invalid_app_catalog"),
        ("cross_origin", "pagination", "untrusted_next_page"),
        ("oversize", "apps_response", "body_size_limit"),
    ] {
        let keys = Keys::new();
        let fake = Fake::new(move |r| {
            assert_eq!(r.method, "GET");
            match case {
                "unauthorized" | "forbidden" | "agreements" => (
                    if case == "unauthorized" {StatusCode::UNAUTHORIZED} else {StatusCode::FORBIDDEN},
                    vec![], serde_json::to_vec(&json!({"errors":[{"code": match case {
                        "unauthorized" => "NOT_AUTHORIZED",
                        "agreements" => "FORBIDDEN.REQUIRED_AGREEMENTS_MISSING_OR_EXPIRED",
                        _ => "FORBIDDEN_ERROR"
                    },"title":"private-response-title","detail":"private-response-text"}]})).unwrap()),
                "invalid_json" => (StatusCode::OK, vec![], b"private-invalid-json".to_vec()),
                "missing_field" => ok(json!({"data":[{"id":"private-id","attributes":{"name":"private-name"}}]})),
                "duplicate" => ok(json!({"data":[catalog_app(Platform::Ios,"private-app"),catalog_app(Platform::Ios,"private-app")]})),
                "cross_origin" => ok(json!({"data":[],"links":{"next":"https://private-host.invalid/v1/apps?private-cursor"}})),
                "oversize" => (StatusCode::OK,vec![],vec![b' ';2*1024*1024+1]),
                _ => unreachable!(),
            }
        }).await;
        let path = keys.dir.path().join("connector.json");
        std::fs::write(&path, "{}").unwrap();
        let log = store_provider::diagnostics::Log::for_config(&path, "catalog-node").unwrap();
        let gateway = Gateway::with_log(keys.config(&fake.url), None, Some(log.clone())).unwrap();
        let report_id = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
        let error = store_provider::diagnostics::catalog_scope(
            report_id,
            Platform::Ios,
            gateway.account_catalog(Platform::Ios),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "catalog_discovery_failed");
        let (entries, raw) = catalog_log_entries(&log);
        let failure = entries
            .iter()
            .rev()
            .find(|entry| entry["event"] == "catalog.failed")
            .unwrap();
        assert_eq!(failure["details"]["stage"], stage, "{case}");
        assert_eq!(failure["details"]["reason"], reason, "{case}");
        let http = entries
            .iter()
            .rev()
            .find(|entry| entry["event"] == "upstream.completed")
            .unwrap();
        assert_eq!(http["details"]["step"], "catalog_apps");
        assert_eq!(
            http["details"]["status"],
            if case == "unauthorized" {
                401
            } else if matches!(case, "forbidden" | "agreements") {
                403
            } else {
                200
            }
        );
        if matches!(case, "unauthorized" | "forbidden" | "agreements") {
            let vendor = entries
                .iter()
                .rev()
                .find(|entry| entry["event"] == "upstream.error")
                .unwrap();
            assert_eq!(
                vendor["details"]["vendor"]["code"],
                match case {
                    "unauthorized" => "NOT_AUTHORIZED",
                    "agreements" => "FORBIDDEN.REQUIRED_AGREEMENTS_MISSING_OR_EXPIRED",
                    _ => "FORBIDDEN_ERROR",
                }
            );
            if case == "agreements" {
                let summary = &vendor["details"]["vendor"];
                assert_eq!(summary["classification"], "agreements_required");
                assert_eq!(summary["errors"][0]["code"], summary["code"]);
                assert_eq!(summary["errorCount"], 1);
                assert_eq!(summary["errorsTruncated"], false);
                assert_eq!(summary["rawMessagesOmitted"], true);
            }
        }
        for entry in entries.iter().filter(|entry| entry["event"].is_string()) {
            assert_eq!(entry["requestId"], report_id);
            assert_eq!(entry["platform"], "ios");
            assert_eq!(entry["operation"], "account_catalog");
        }
        assert!(!raw.contains("private-"), "{case}");
        for record in fake.records() {
            assert!(!raw.contains(record.headers["authorization"].to_str().unwrap()));
        }
        assert!(!raw.contains("BEGIN PRIVATE KEY"));
    }
}
#[tokio::test]
async fn catalog_absent_credentials_and_mock_are_safe_without_network() {
    let gateway = Gateway::new(ProviderConfig::default()).unwrap();
    for platform in [Platform::Ios, Platform::Android] {
        assert_eq!(gateway.account_fingerprint(platform), None);
        assert_eq!(
            gateway.account_catalog(platform).await.unwrap_err().code,
            "catalog_not_configured"
        );
    }
    let gateway = Gateway::new(ProviderConfig {
        mode: "mock".into(),
        ..ProviderConfig::default()
    })
    .unwrap();
    assert_eq!(gateway.account_fingerprint(Platform::Ios), None);
    assert_eq!(gateway.account_fingerprint(Platform::Android), None);
    assert_eq!(
        gateway
            .account_catalog(Platform::Ios)
            .await
            .unwrap_err()
            .code,
        "catalog_unsupported"
    );
}

#[tokio::test]
async fn catalog_oauth_diagnostics_and_concurrent_platform_scopes_are_isolated() {
    let fake = Fake::new(|r| {
        if r.uri == "/token" {
            return (
                StatusCode::FORBIDDEN,
                vec![],
                br#"{"error":"invalid_grant","error_description":"private-oauth-description"}"#
                    .to_vec(),
            );
        }
        ok(catalog_page(Platform::Ios, "private-app", None))
    })
    .await;
    let keys = Keys::new();
    let path = keys.dir.path().join("connector.json");
    std::fs::write(&path, "{}").unwrap();
    let log = store_provider::diagnostics::Log::for_config(&path, "catalog-node").unwrap();
    let gateway = Gateway::with_log(keys.config(&fake.url), None, Some(log.clone())).unwrap();
    let ios_id = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
    let android_id = "bbbbbbbb-bbbb-cccc-dddd-eeeeeeeeeeee";
    let (ios, android) = tokio::join!(
        store_provider::diagnostics::catalog_scope(
            ios_id,
            Platform::Ios,
            gateway.account_catalog(Platform::Ios)
        ),
        store_provider::diagnostics::catalog_scope(
            android_id,
            Platform::Android,
            gateway.account_catalog(Platform::Android)
        )
    );
    assert_eq!(ios.unwrap().apps.len(), 1);
    assert_eq!(android.unwrap_err().code, "catalog_discovery_failed");
    let (entries, raw) = catalog_log_entries(&log);
    let oauth = entries
        .iter()
        .find(|entry| {
            entry["event"] == "upstream.error" && entry["details"]["step"] == "oauth_token"
        })
        .unwrap();
    assert_eq!(oauth["requestId"], android_id);
    assert_eq!(oauth["details"]["status"], 403);
    assert_eq!(oauth["details"]["vendor"]["code"], "invalid_grant");
    assert!(entries
        .iter()
        .any(|entry| entry["event"] == "catalog.failed"
            && entry["details"]["stage"] == "reporting_oauth_response"));
    assert!(entries
        .iter()
        .any(|entry| entry["event"] == "catalog.finished"
            && entry["requestId"] == ios_id
            && entry["details"]["appCount"] == 1));
    for entry in entries.iter().filter(|entry| entry["event"].is_string()) {
        assert_eq!(
            entry["requestId"],
            if entry["platform"] == "ios" {
                ios_id
            } else {
                android_id
            }
        );
    }
    for secret in [
        "private-",
        "fake-issuer",
        "fake-key",
        "fake@example.test",
        "BEGIN PRIVATE KEY",
        "grant_type",
        "assertion",
    ] {
        assert!(!raw.contains(secret), "{secret}");
    }
    assert_eq!(fake.records().len(), 2);
}

#[tokio::test]
async fn catalog_logs_connection_failures_and_successful_pagination_without_cursors() {
    let keys = Keys::new();
    let fake = Fake::new(|r| {
        ok(catalog_page(
            Platform::Ios,
            if r.uri.contains("private-cursor") {
                "two"
            } else {
                "one"
            },
            if r.uri.contains("private-cursor") {
                None
            } else {
                Some("/v1/apps?cursor=private-cursor")
            },
        ))
    })
    .await;
    let path = keys.dir.path().join("connector.json");
    std::fs::write(&path, "{}").unwrap();
    let log = store_provider::diagnostics::Log::for_config(&path, "catalog-node").unwrap();
    let gateway = Gateway::with_log(keys.config(&fake.url), None, Some(log.clone())).unwrap();
    assert_eq!(
        gateway
            .account_catalog(Platform::Ios)
            .await
            .unwrap()
            .apps
            .len(),
        2
    );
    let (entries, raw) = catalog_log_entries(&log);
    assert!(entries.iter().any(|e| e["event"] == "catalog.page"
        && e["details"]["page"] == 2
        && e["details"]["appCount"] == 2));
    assert!(!raw.contains("private-cursor"));
    let gateway =
        Gateway::with_log(keys.config("http://127.0.0.1:0"), None, Some(log.clone())).unwrap();
    assert_eq!(
        gateway
            .account_catalog(Platform::Ios)
            .await
            .unwrap_err()
            .code,
        "catalog_discovery_failed"
    );
    let (entries, _) = catalog_log_entries(&log);
    assert!(entries
        .iter()
        .any(|e| e["event"] == "upstream.completed" && e["details"]["error"] == "connect_failed"));
    assert!(entries
        .iter()
        .any(|e| e["event"] == "catalog.failed" && e["details"]["stage"] == "apps_request"));
}
#[tokio::test]
async fn catalog_two_pages_are_complete_and_fingerprints_are_opaque() {
    for platform in [Platform::Ios, Platform::Android] {
        let fake = Fake::new(move |r| {
            if r.uri == "/token" {
                return ok(json!({"access_token":"reporting-token","expires_in":3600}));
            }
            assert_eq!(r.method, "GET");
            assert!(r.uri.starts_with(if platform == Platform::Ios {
                "/v1/apps?"
            } else {
                "/v1beta1/apps:search?"
            }));
            let second = r.uri.contains("two");
            if platform == Platform::Android {
                assert_eq!(r.headers["authorization"], "Bearer reporting-token");
            }
            ok(catalog_page(
                platform,
                if second { "two" } else { "one" },
                if second {
                    None
                } else if platform == Platform::Ios {
                    Some("/v1/apps?cursor=two")
                } else {
                    Some("two")
                },
            ))
        })
        .await;
        let keys = Keys::new();
        let gateway = Gateway::new(keys.config(&fake.url)).unwrap();
        let local_fingerprint = gateway.account_fingerprint(platform).unwrap();
        assert!(fake.records().is_empty(), "fingerprinting must be local");
        let catalog = gateway.account_catalog(platform).await.unwrap();
        assert_eq!(catalog.fingerprint, local_fingerprint);
        assert_eq!(catalog.apps.len(), 2);
        catalog.validate().unwrap();
        use sha2::{Digest, Sha256};
        let identity = if platform == Platform::Ios {
            "fake-issuer"
        } else {
            "fake@example.test"
        };
        assert_eq!(
            catalog.fingerprint,
            hex::encode(Sha256::digest(format!("{platform}:{identity}").as_bytes()))
        );
        assert!(!serde_json::to_string(&catalog).unwrap().contains(identity));
        assert_eq!(
            fake.records().iter().filter(|r| r.method == "GET").count(),
            2
        );
    }
}
#[tokio::test]
async fn catalog_rejects_failed_second_page_cycles_duplicates_cross_origin_and_oversize() {
    let keys = Keys::new();
    for platform in [Platform::Ios, Platform::Android] {
        for scenario in [
            "second_failure",
            "cycle",
            "duplicate",
            "oversize",
            "too_many",
            "cross_origin",
        ] {
            let fake = Fake::new(move |r| {
                if r.uri == "/token" {
                    return ok(json!({"access_token":"reporting-token","expires_in":3600}));
                }
                let second = r.uri.contains("two");
                if scenario == "oversize" {
                    return (StatusCode::OK, vec![], vec![b' '; 2 * 1024 * 1024 + 1]);
                }
                if scenario == "too_many" {
                    let apps: Vec<_> = (0..10_001)
                        .map(|i| catalog_app(platform, &i.to_string()))
                        .collect();
                    return ok(if platform == Platform::Ios {
                        json!({"data":apps})
                    } else {
                        json!({"apps":apps})
                    });
                }
                if second && scenario == "second_failure" {
                    return (
                        StatusCode::FORBIDDEN,
                        vec![],
                        b"secret-upstream-body".to_vec(),
                    );
                }
                let next = if scenario == "cross_origin" && platform == Platform::Ios {
                    Some("https://untrusted.invalid/v1/apps?cursor=two")
                } else if !second || scenario == "cycle" {
                    Some(if platform == Platform::Ios {
                        "/v1/apps?cursor=two"
                    } else {
                        "two"
                    })
                } else {
                    None
                };
                ok(catalog_page(
                    platform,
                    if second && scenario != "duplicate" {
                        "two"
                    } else {
                        "one"
                    },
                    next,
                ))
            })
            .await;
            // Reporting only receives opaque page tokens; a URL token cannot change its origin.
            if scenario == "cross_origin" && platform == Platform::Android {
                continue;
            }
            let gateway = Gateway::new(keys.config(&fake.url)).unwrap();
            let fingerprint = gateway.account_fingerprint(platform).unwrap();
            assert!(fake.records().is_empty());
            let error = gateway.account_catalog(platform).await.unwrap_err();
            assert_eq!(gateway.account_fingerprint(platform), Some(fingerprint));
            assert_eq!(
                error.code, "catalog_discovery_failed",
                "{platform}/{scenario}"
            );
            assert!(!error.message.contains("secret"));
            assert!(fake.records().len() <= 4, "cycle must fail promptly");
            assert!(fake
                .records()
                .iter()
                .all(|r| r.method == "GET" || r.uri == "/token"));
        }
    }
}
fn catalog_oauth_scope(record: &Record, keys: &Keys, url: &str) -> String {
    let form = reqwest::Url::parse(&format!(
        "http://fixture/?{}",
        String::from_utf8_lossy(&record.bytes)
    ))
    .unwrap();
    let assertion = form
        .query_pairs()
        .find(|(k, _)| k == "assertion")
        .unwrap()
        .1
        .into_owned();
    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
    validation.set_audience(&[format!("{url}/token")]);
    jsonwebtoken::decode::<Value>(
        &assertion,
        &jsonwebtoken::DecodingKey::from_rsa_pem(keys.google_public.as_bytes()).unwrap(),
        &validation,
    )
    .unwrap()
    .claims["scope"]
        .as_str()
        .unwrap()
        .into()
}
#[tokio::test]
async fn catalog_reporting_and_publisher_tokens_are_isolated_and_reporting_failure_preserves_business(
) {
    let reporting_failed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let failed = reporting_failed.clone();
    let fake = Fake::new(move |r| {
        if r.uri == "/token" {
            // Inspect the synthetic assertion only to choose a distinct response token.
            let form = reqwest::Url::parse(&format!("http://fixture/?{}", String::from_utf8_lossy(&r.bytes))).unwrap();
            let assertion = form.query_pairs().find(|(k,_)| k == "assertion").unwrap().1.into_owned();
            let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
            validation.insecure_disable_signature_validation(); validation.validate_aud = false;
            let claims = jsonwebtoken::decode::<Value>(&assertion, &jsonwebtoken::DecodingKey::from_secret(b"fixture"), &validation).unwrap().claims;
            return ok(json!({"access_token":if claims["scope"] == store_provider::catalog::REPORTING_SCOPE { "reporting-token" } else { "publisher-token" },"expires_in":3600}));
        }
        if r.uri.starts_with("/v1beta1/apps:search") {
            assert_eq!(r.headers["authorization"], "Bearer reporting-token");
            if failed.load(std::sync::atomic::Ordering::SeqCst) { return (StatusCode::FORBIDDEN, vec![], b"disabled".to_vec()); }
            return ok(catalog_page(Platform::Android, "one", None));
        }
        assert_eq!(r.headers["authorization"], "Bearer publisher-token");
        google_base(r).unwrap_or_else(|| ok(json!({})))
    }).await;
    let keys = Keys::new();
    let gateway = Gateway::new(keys.config(&fake.url)).unwrap();
    let fingerprint = gateway.account_fingerprint(Platform::Android).unwrap();
    assert!(fake.records().is_empty());
    assert_eq!(
        gateway
            .account_catalog(Platform::Android)
            .await
            .unwrap()
            .fingerprint,
        fingerprint
    );
    gateway
        .execute(&request(
            Platform::Android,
            Operation::StoreListings,
            json!({}),
        ))
        .await
        .unwrap();
    reporting_failed.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(gateway.account_catalog(Platform::Android).await.is_err());
    assert_eq!(
        gateway.account_fingerprint(Platform::Android),
        Some(fingerprint)
    );
    gateway
        .execute(&request(
            Platform::Android,
            Operation::StoreListings,
            json!({}),
        ))
        .await
        .unwrap();
    let tokens: Vec<_> = fake
        .records()
        .into_iter()
        .filter(|r| r.uri == "/token")
        .collect();
    assert_eq!(tokens.len(), 2);
    let scopes: std::collections::HashSet<_> = tokens
        .iter()
        .map(|r| catalog_oauth_scope(r, &keys, &fake.url))
        .collect();
    assert!(scopes.contains(store_provider::catalog::REPORTING_SCOPE));
    assert!(scopes.contains("https://www.googleapis.com/auth/androidpublisher"));
}
#[tokio::test]
async fn catalog_reporting_oauth_failure_does_not_poison_publisher_token() {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = calls.clone();
    let fake = Fake::new(move |r| {
        if r.uri == "/token" {
            if counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                return (
                    StatusCode::FORBIDDEN,
                    vec![],
                    b"credential-error-must-stay-local".to_vec(),
                );
            }
            return ok(json!({"access_token":"publisher-token","expires_in":3600}));
        }
        assert!(
            !r.uri.starts_with("/v1beta1/"),
            "Reporting must stop after OAuth failure"
        );
        assert_eq!(r.headers["authorization"], "Bearer publisher-token");
        google_base(r).unwrap_or_else(|| ok(json!({})))
    })
    .await;
    let keys = Keys::new();
    let gateway = Gateway::new(keys.config(&fake.url)).unwrap();
    let fingerprint = gateway.account_fingerprint(Platform::Android).unwrap();
    assert!(fake.records().is_empty());
    let error = gateway
        .account_catalog(Platform::Android)
        .await
        .unwrap_err();
    assert_eq!(error.code, "catalog_discovery_failed");
    assert_eq!(
        gateway.account_fingerprint(Platform::Android),
        Some(fingerprint)
    );
    assert!(!error.message.contains("credential-error"));
    gateway
        .execute(&request(
            Platform::Android,
            Operation::StoreListings,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    let tokens: Vec<_> = fake
        .records()
        .into_iter()
        .filter(|r| r.uri == "/token")
        .collect();
    assert_eq!(
        catalog_oauth_scope(&tokens[0], &keys, &fake.url),
        store_provider::catalog::REPORTING_SCOPE
    );
    assert_eq!(
        catalog_oauth_scope(&tokens[1], &keys, &fake.url),
        "https://www.googleapis.com/auth/androidpublisher"
    );
}
impl Keys {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let apple = p256::SecretKey::from_slice(&[7; 32]).unwrap();
        std::fs::write(
            dir.path().join("apple.p8"),
            apple.to_pkcs8_pem(LineEnding::LF).unwrap().as_bytes(),
        )
        .unwrap();
        let rsa = rsa::RsaPrivateKey::new(&mut rand::thread_rng(), 2048).unwrap();
        let google = rsa.to_pkcs8_pem(LineEnding::LF).unwrap();
        std::fs::write(
            dir.path().join("google.json"),
            serde_json::to_vec(
                &json!({"client_email":"fake@example.test","private_key":google.as_str()}),
            )
            .unwrap(),
        )
        .unwrap();
        Self {
            dir,
            apple_public: apple
                .public_key()
                .to_public_key_pem(LineEnding::LF)
                .unwrap(),
            google_public: rsa::RsaPublicKey::from(&rsa)
                .to_public_key_pem(LineEnding::LF)
                .unwrap(),
        }
    }
    fn config(&self, url: &str) -> ProviderConfig {
        ProviderConfig {
            apple_issuer_id: Some("fake-issuer".into()),
            apple_key_id: Some("fake-key".into()),
            apple_private_key_path: Some(
                self.dir
                    .path()
                    .join("apple.p8")
                    .to_string_lossy()
                    .into_owned(),
            ),
            google_service_account_path: Some(
                self.dir
                    .path()
                    .join("google.json")
                    .to_string_lossy()
                    .into_owned(),
            ),
            apple_base_url: url.into(),
            google_base_url: url.into(),
            google_reporting_base_url: url.into(),
            google_upload_base_url: url.into(),
            google_token_url: format!("{url}/token"),
            ..Default::default()
        }
    }
}
fn apple_base(r: &Record) -> Option<Reply> {
    if r.uri.ends_with("/assetLibrary") {
        return Some(ok(
            json!({"data":{"type":"appAssetLibraries","id":"library"}}),
        ));
    }
    if r.uri.starts_with("/v1/appAssetLibraryRefData?") {
        return Some(ok(apple_image_reference()));
    }
    if r.uri.contains("/placements?") {
        return Some(ok(apple_image_placements(&[("shot", "phone-profile")])));
    }
    if r.uri.starts_with("/v1/apps?") {
        return Some(ok(
            json!({"data":[{"id":"resolved-app","attributes":{"bundleId":"com.example.App","primaryLocale":"en-US"}}]}),
        ));
    }
    if r.uri.contains("/appStoreVersions?") {
        return Some(ok(
            json!({"data":[{"id":"version","attributes":{"appStoreState":"PREPARE_FOR_SUBMISSION"}}]}),
        ));
    }
    if r.uri.contains("/appStoreVersionLocalizations?") {
        return Some(ok(
            json!({"data":[{"id":"version-loc","attributes":{"locale":"en-US","description":"body","keywords":"keys","whatsNew":"notes"}}]}),
        ));
    }
    if r.uri.ends_with("/appInfos?limit=1") {
        return Some(ok(json!({"data":[{"id":"info"}]})));
    }
    if r.uri.contains("/appInfoLocalizations?") {
        return Some(ok(
            json!({"data":[{"id":"info-loc","attributes":{"locale":"en-US","name":"Title","subtitle":"Subtitle"}}]}),
        ));
    }
    if r.method == "POST" && r.uri == "/v1/appInfoLocalizations" {
        return Some(ok(json!({"data":{"id":"created-info-loc"}})));
    }
    if r.method == "POST" && r.uri == "/v1/appStoreVersionLocalizations" {
        return Some(ok(json!({"data":{"id":"created-version-loc"}})));
    }
    None
}
fn apple_image_reference() -> Value {
    let profiles = [
        (
            "phone-profile",
            "IPHONE_DYNAMIC_ISLAND_LARGE_DISPLAY",
            "IPHONE",
        ),
        ("tablet-profile", "IPAD_13", "IPAD"),
        ("duo-inner-profile", "IPHONE_DUO", "IPHONE"),
        ("duo-outer-profile", "IPHONE_DUO", "IPHONE"),
    ];
    json!({"data":[{"type":"appAssetLibraryRefData","id":"1","attributes":{
        "features":(["APP_STORE_VERSIONS","CUSTOM_PRODUCT_PAGES"].iter().map(|feature|json!({"featureId":feature,"placementPolicies":[{"placementType":"APP_SCREENSHOT","groupLimits":[{"groupIds":profiles.iter().map(|(group,_,_)|group).collect::<Vec<_>>(),"maxCount":10}]}]})).collect::<Vec<_>>()),
        "placementTypes":[{"placementTypeId":"APP_SCREENSHOT","acceptsAssetCategories":["APP_SCREENSHOTS_AND_PREVIEWS"],"specMappings":profiles.iter().map(|(group,_,_)|json!({"placementGroupId":group,"specs":["spec"]})).collect::<Vec<_>>()}],
        "imageSpecs":[{"specId":"spec","dimensions":{"minWidth":2,"maxWidth":2,"minHeight":3,"maxHeight":3},"compatiblePlacementTypes":["APP_SCREENSHOT"],"alphaAllowed":false,"mimeTypes":["image/png"],"fileExtensions":[".png"],"maxFileSize":65536}],
        "placementProfileGroups":profiles.iter().map(|(group,class,_)|json!({"placementProfileGroupId":group,"displayClassId":class})).collect::<Vec<_>>(),
        "displayClasses":profiles.iter().map(|(_,class,family)|json!({"displayClassId":class,"deviceFamily":family})).collect::<Vec<_>>()
    }}]})
}
fn apple_image_placements(images: &[(&str, &str)]) -> Value {
    json!({
        "data":images.iter().map(|(id,group)|json!({"type":"appAssetLibraryPlacements","id":format!("placement-{id}"),"attributes":{"placementType":"APP_SCREENSHOT","placementGroup":group},"relationships":{"image":{"data":{"type":"appAssetLibraryImages","id":id}}}})).collect::<Vec<_>>(),
        "included":images.iter().map(|(id,_)|json!({"type":"appAssetLibraryImages","id":id,"attributes":{"fileName":format!("{id}.png"),"imageAsset":{"templateUrl":"https://cdn.test/{w}","width":1290,"height":2796}}})).collect::<Vec<_>>()
    })
}
fn apple_upload_base(record: &Record, image: &[u8], base: &str) -> Option<Reply> {
    if record.uri == "/image.png" {
        return Some((
            StatusCode::OK,
            vec![("content-type", "image/png".into())],
            image.to_vec(),
        ));
    }
    if record.method == "POST" && record.uri == "/v1/appAssetLibraryImages" {
        return Some(ok(
            json!({"data":{"type":"appAssetLibraryImages","id":"shot","attributes":{"state":"AWAITING_UPLOAD","uploadOperations":[{"method":"PUT","url":format!("{base}/chunk"),"offset":0,"length":image.len(),"requestHeaders":[{"name":"x-upload","value":"chunk-header"}]}]}}}),
        ));
    }
    if record.method == "PATCH" && record.uri == "/v1/appAssetLibraryImages/shot" {
        return Some(ok(
            json!({"data":{"type":"appAssetLibraryImages","id":"shot","attributes":{"state":"UPLOAD_COMPLETE"}}}),
        ));
    }
    if record.method == "POST" && record.uri == "/v1/appAssetLibraryPlacements" {
        return Some(ok(
            json!({"data":{"type":"appAssetLibraryPlacements","id":format!("new-{}",record.body()["data"]["attributes"]["placementGroup"].as_str().unwrap())}}),
        ));
    }
    None
}
fn google_base(r: &Record) -> Option<Reply> {
    if r.uri == "/token" {
        return Some(ok(
            json!({"access_token":"fake-access-token","expires_in":3600}),
        ));
    }
    if r.method == "POST" && r.uri.ends_with("/edits") {
        return Some(ok(json!({"id":"edit"})));
    }
    if r.method == "DELETE" && r.uri.ends_with("/edits/edit") {
        return Some(ok(json!({})));
    }
    None
}

fn google_batch_payload(url: &str) -> Value {
    json!({"operation":"update_app_metadata","localizations":[
        {"locale":"en-US","syncScopes":["title","store_images"],"metadata":{"title":"English","description":"not selected","storeImages":{"phone_screenshots":{"urls":[format!("{url}/batch-image")]}}}},
        {"locale":"ja","syncScopes":["description","store_images"],"metadata":{"description":"Japanese copy","storeImages":{"tablet_screenshots":{"urls":[format!("{url}/batch-image")]}}}}
    ]})
}

#[tokio::test]
async fn google_multilocale_text_and_images_share_one_edit_and_one_commit() {
    let fake = Fake::new(|r| {
        if r.uri == "/batch-image" {
            return (
                StatusCode::OK,
                vec![("content-type", "image/png".into())],
                png(),
            );
        }
        google_base(r).unwrap_or_else(|| ok(json!({})))
    })
    .await;
    let keys = Keys::new();
    let gateway = Gateway::new(keys.config(&fake.url)).unwrap();
    let result = gateway
        .execute(&request(
            Platform::Android,
            Operation::Sync,
            google_batch_payload(&fake.url),
        ))
        .await
        .unwrap();
    assert_eq!(result["status"], "succeeded");
    assert_eq!(result["storeState"]["locales"], json!(["en-US", "ja"]));
    let records = fake.records();
    assert_eq!(
        records
            .iter()
            .filter(|r| r.method == "POST" && r.uri.ends_with("/edits"))
            .count(),
        1
    );
    let commits: Vec<_> = records
        .iter()
        .filter(|r| r.uri.contains(":commit?"))
        .collect();
    assert_eq!(commits.len(), 1);
    assert!(commits[0].uri.contains("ERROR_IF_IN_REVIEW"));
    assert!(!commits[0].uri.contains("changesNotSentForReview"));
    let patches: Vec<_> = records.iter().filter(|r| r.method == "PATCH").collect();
    assert_eq!(patches.len(), 2);
    assert_eq!(patches[0].body(), json!({"title":"English"}));
    assert_eq!(
        patches[1].body(),
        json!({"fullDescription":"Japanese copy"})
    );
    assert!(patches[0].uri.contains("/edits/edit/listings/en%2DUS"));
    assert!(patches[1].uri.contains("/edits/edit/listings/ja"));
    let uploads: Vec<_> = records
        .iter()
        .filter(|r| r.uri.starts_with("/upload/"))
        .collect();
    assert_eq!(uploads.len(), 3);
    assert!(uploads
        .iter()
        .all(|r| r.uri.contains("/edits/edit/listings/")));
    assert!(records.iter().all(|r| !r.uri.contains("/tracks")));
    let commit_index = records
        .iter()
        .position(|r| r.uri.contains(":commit?"))
        .unwrap();
    assert!(records
        .iter()
        .enumerate()
        .filter(|(_, r)| r.uri.starts_with("/upload/") || r.method == "PATCH")
        .all(|(i, _)| i < commit_index));
}

#[tokio::test]
async fn google_multilocale_late_failure_discards_without_commit_or_replay() {
    let fake = Fake::new(|r| {
        if r.uri == "/batch-image" {
            return (StatusCode::OK, vec![], png());
        }
        if r.method == "PATCH" && r.uri.ends_with("/ja") {
            return (
                StatusCode::BAD_REQUEST,
                vec![],
                br#"{"error":{"message":"rejected"}}"#.to_vec(),
            );
        }
        google_base(r).unwrap_or_else(|| ok(json!({})))
    })
    .await;
    let keys = Keys::new();
    let gateway = Gateway::new(keys.config(&fake.url)).unwrap();
    assert!(gateway
        .execute(&request(
            Platform::Android,
            Operation::Sync,
            google_batch_payload(&fake.url)
        ))
        .await
        .is_err());
    let records = fake.records();
    assert!(!records.iter().any(|r| r.uri.contains(":commit")));
    assert_eq!(
        records
            .iter()
            .filter(|r| r.method == "DELETE" && r.uri.ends_with("/edits/edit"))
            .count(),
        1
    );
    assert_eq!(
        records
            .iter()
            .filter(|r| r.method == "PATCH" && r.uri.ends_with("/ja"))
            .count(),
        1
    );
}

#[tokio::test]
async fn google_multilocale_validates_entire_batch_before_any_network() {
    let fake = Fake::new(|_| panic!("Invalid batch reached upstream")).await;
    let keys = Keys::new();
    let gateway = Gateway::new(keys.config(&fake.url)).unwrap();
    let valid = json!({"operation":"update_app_metadata","localizations":[{"locale":"en-US","metadata":{"title":"private-copy"}},{"locale":"ja","metadata":{"title":"Japanese"}}]});
    let mut cases = vec![
        json!({"operation":"update_app_metadata","localizations":[]}),
        json!({"operation":"update_app_metadata","localizations":null}),
    ];
    for (pointer, value) in [
        ("/localizations/1/locale", json!("EN-us")),
        ("/localizations/1/locale", json!(" ja")),
        ("/localizations/1/metadata/title", json!(1)),
        ("/localizations/1/metadata", json!({})),
        ("/localizations/1/syncScopes", json!([])),
        ("/localizations/1/syncScopes", json!(["keywords"])),
        ("/locale", json!("en-US")),
        ("/operation", json!("update_release_notes")),
    ] {
        let mut bad = valid.clone();
        if pointer == "/locale" {
            bad["locale"] = value;
        } else if pointer.ends_with("syncScopes") {
            bad["localizations"][1]["syncScopes"] = value;
        } else {
            *bad.pointer_mut(pointer).unwrap() = value;
        }
        cases.push(bad);
    }
    let mut bad_image = valid.clone();
    bad_image["localizations"][1]["metadata"]["storeImages"] =
        json!({"phone_screenshots":{"urls":["file:///private/image.png"]}});
    cases.push(bad_image);
    cases.push(json!({"operation":"update_app_metadata","localizations":vec![valid["localizations"][0].clone();51]}));
    for payload in cases {
        assert_eq!(
            gateway
                .execute(&request(Platform::Android, Operation::Sync, payload))
                .await
                .unwrap_err()
                .status,
            422
        );
    }
    assert_eq!(
        gateway
            .execute(&request(Platform::Ios, Operation::Sync, valid.clone()))
            .await
            .unwrap_err()
            .status,
        422
    );
    let summary = store_provider::diagnostics::summary(&valid).to_string();
    assert!(!summary.contains("private-copy"));
    assert!(fake.records().is_empty());
}

#[tokio::test]
async fn google_multilocale_unknown_commit_is_not_replayed_or_hidden_by_cleanup() {
    let fake = Fake::new(|r| {
        if r.uri.contains(":commit?") {
            return (StatusCode::OK, vec![], b"lost-success-body".to_vec());
        }
        if r.method == "DELETE" && r.uri.ends_with("/edits/edit") {
            return (StatusCode::BAD_GATEWAY, vec![], b"cleanup failed".to_vec());
        }
        google_base(r).unwrap_or_else(|| ok(json!({})))
    })
    .await;
    let keys = Keys::new();
    let assets = store_provider::assets::AssetStore::new("batch-node".into(), "11".repeat(32));
    let gateway = Gateway::with_assets(keys.config(&fake.url), Some(assets.clone())).unwrap();
    let request = request(
        Platform::Android,
        Operation::Sync,
        json!({"operation":"update_app_metadata","localizations":[
            {"locale":"en-US","metadata":{"title":"English"}},{"locale":"ja","metadata":{"title":"Japanese"}}
        ]}),
    );
    let lease = assets.claim(&request).unwrap();
    let error = gateway
        .execute_with_assets(&request, lease)
        .await
        .unwrap_err();
    assert!(error.outcome_unknown);
    assert_ne!(error.code, "store_edit_cleanup_failed");
    assert_eq!(
        fake.records()
            .iter()
            .filter(|r| r.uri.contains(":commit?"))
            .count(),
        1
    );
}

#[tokio::test]
async fn google_multilocale_uploaded_asset_is_reusable_until_whole_execution_finishes() {
    use sha2::{Digest, Sha256};
    use store_protocol::upload::{self, UploadClaims, UploadInput};
    let fake = Fake::new(|r| google_base(r).unwrap_or_else(|| ok(json!({})))).await;
    let keys = Keys::new();
    let key = "11".repeat(32);
    let assets = store_provider::assets::AssetStore::new("batch-node".into(), key.clone());
    let image = png();
    let now = upload::now().unwrap();
    let claims = UploadClaims {
        iss: "testflying-store-api".into(),
        aud: "connector:batch-node".into(),
        iat: now,
        exp: now + upload::TOKEN_SECONDS,
        connector_id: "batch-node".into(),
        asset_id: "00000000-0000-4000-8000-000000000001".into(),
        platform: Platform::Android,
        bundle_identifier: "com.example.App".into(),
        file: UploadInput {
            file_name: "fixture.png".into(),
            content_type: "image/png".into(),
            size_bytes: image.len() as u64,
            sha256: hex::encode(Sha256::digest(&image)),
        },
    };
    let session = "00000000-0000-4000-8000-000000000002";
    assets
        .start(&upload::sign(&claims, &key).unwrap(), session)
        .unwrap();
    assets
        .chunk(
            Platform::Android,
            "com.example.App",
            &claims.asset_id,
            session,
            0,
            &image,
        )
        .unwrap();
    assets
        .finish(
            Platform::Android,
            "com.example.App",
            &claims.asset_id,
            session,
        )
        .unwrap();
    let gateway = Gateway::with_assets(keys.config(&fake.url), Some(assets.clone())).unwrap();
    let images = json!({"phone_screenshots":{"assets":[{"assetId":claims.asset_id}]}});
    let payload = json!({"operation":"update_app_metadata","localizations":[
        {"locale":"en-US","syncScopes":["store_images"],"metadata":{"storeImages":images.clone()}},
        {"locale":"ja","syncScopes":["store_images"],"metadata":{"storeImages":images}}
    ]});
    assert_eq!(
        gateway
            .execute(&request(Platform::Android, Operation::Sync, payload))
            .await
            .unwrap()["status"],
        "succeeded"
    );
    assert_eq!(assets.usage(), (0, 0));
    assert_eq!(
        fake.records()
            .iter()
            .filter(|r| r.uri.starts_with("/upload/"))
            .count(),
        2
    );
    assert_eq!(
        fake.records()
            .iter()
            .filter(|r| r.uri.contains(":commit?"))
            .count(),
        1
    );
}

#[tokio::test]
async fn async_image_mutation_response_loss_preserves_original_error_and_explicit_rejections() {
    for platform in [Platform::Ios, Platform::Android] {
        for rejected in [false, true] {
            let image = png();
            let endpoint = Arc::new(Mutex::new(String::new()));
            let upstream = endpoint.clone();
            let fake = Fake::new(move |r| {
                if r.uri == "/v1/appAssetLibraryPlacementOrderingRequests"
                    || r.uri.contains(":commit?")
                {
                    // The vendor accepted a mutation but its success body is unusable,
                    // or explicitly rejected it. These must produce different certainty.
                    return (
                        if rejected {
                            StatusCode::BAD_REQUEST
                        } else {
                            StatusCode::OK
                        },
                        vec![],
                        b"lost response".to_vec(),
                    );
                }
                if r.method == "DELETE" && r.uri.ends_with("/edits/edit") {
                    return (
                        StatusCode::BAD_GATEWAY,
                        vec![],
                        b"discard also failed".to_vec(),
                    );
                }
                apple_upload_base(r, &image, &upstream.lock().unwrap())
                    .or_else(|| apple_base(r))
                    .or_else(|| google_base(r))
                    .unwrap_or_else(|| ok(json!({})))
            })
            .await;
            *endpoint.lock().unwrap() = fake.url.clone();
            let keys = Keys::new();
            let assets =
                store_provider::assets::AssetStore::new("image-node".into(), "22".repeat(32));
            let gateway =
                Gateway::with_assets(keys.config(&fake.url), Some(assets.clone())).unwrap();
            let request = request(
                platform,
                Operation::Sync,
                json!({"operation":"update_app_metadata",
                "locale":"en-US","syncScopes":["store_images"],"metadata":{"storeImages":{
                    "phone_screenshots":{"urls":[format!("{}/image.png", fake.url)]}}}}),
            );
            let lease = assets.claim(&request).unwrap();
            let error = gateway
                .execute_with_assets(&request, lease)
                .await
                .unwrap_err();
            assert_eq!(error.code, "store_upstream_error");
            assert_eq!(
                error.message,
                if rejected {
                    "Store upstream returned HTTP 400"
                } else {
                    "Invalid store JSON response"
                }
            );
            assert_eq!(error.outcome_unknown, !rejected);
            assert_eq!(
                error.body()["outcomeUnknown"],
                if rejected { Value::Null } else { json!(true) }
            );
            let records = fake.records();
            assert_eq!(
                records
                    .iter()
                    .filter(|r| r.uri == "/v1/appAssetLibraryPlacementOrderingRequests"
                        || r.uri.contains(":commit?"))
                    .count(),
                1
            );
            if platform == Platform::Android {
                assert!(records
                    .iter()
                    .any(|r| r.method == "DELETE" && r.uri.ends_with("/edits/edit")));
            } else {
                // The original synchronous path keeps its error shape and certainty.
                assert!(!gateway.execute(&request).await.unwrap_err().outcome_unknown);
            }
        }
    }
}

#[tokio::test]
async fn async_image_download_and_validation_errors_remain_known_despite_discard_failure() {
    let fake = Fake::new(|r| {
        if r.uri == "/image.png" {
            return (StatusCode::BAD_GATEWAY, vec![], vec![]);
        }
        if r.method == "DELETE" && r.uri.ends_with("/edits/edit") {
            return (StatusCode::BAD_GATEWAY, vec![], vec![]);
        }
        google_base(r).unwrap_or_else(|| ok(json!({})))
    })
    .await;
    let keys = Keys::new();
    let assets = store_provider::assets::AssetStore::new("image-node".into(), "22".repeat(32));
    let gateway = Gateway::with_assets(keys.config(&fake.url), Some(assets.clone())).unwrap();
    let mut request = request(
        Platform::Android,
        Operation::Sync,
        json!({"operation":"update_app_metadata",
        "locale":"en-US","syncScopes":["store_images"],"metadata":{"storeImages":{
            "phone_screenshots":{"urls":[format!("{}/image.png", fake.url)]}}}}),
    );
    let error = gateway
        .execute_with_assets(&request, assets.claim(&request).unwrap())
        .await
        .unwrap_err();
    assert_eq!(error.message, "Image download returned an error");
    assert!(!error.outcome_unknown);
    let calls = fake.records().len();
    request.payload["locale"] = json!(false);
    let error = gateway
        .execute_with_assets(&request, assets.claim(&request).unwrap())
        .await
        .unwrap_err();
    assert!(!error.outcome_unknown);
    assert_eq!(fake.records().len(), calls);
}

#[test]
fn defaults_and_debug_do_not_expose_credentials() {
    let config:ProviderConfig=serde_json::from_value(json!({"appleIssuerId":"secret-issuer","appleKeyId":"secret-key","storeProxyUrl":"http://user:secret@example.test"})).unwrap();
    assert_eq!(
        config.google_token_url,
        "https://oauth2.googleapis.com/token"
    );
    let debug = format!("{config:?}");
    assert!(!debug.contains("secret"));
    assert!(!debug.contains("user:"));
    assert_eq!(
        serde_json::to_value(config).unwrap()["appleIssuerId"],
        "secret-issuer"
    );
}

#[tokio::test]
async fn apple_identity_jwt_cache_listings_and_scoped_empty_update() {
    let fake = Fake::new(|r| apple_base(r).unwrap_or_else(|| ok(json!({})))).await;
    let keys = Keys::new();
    let gateway = Gateway::new(keys.config(&fake.url)).unwrap();
    let listings = gateway
        .execute(&request(
            Platform::Ios,
            Operation::StoreListings,
            json!({"version":"1.2","app":{"storeAppId":"evil"}}),
        ))
        .await
        .unwrap();
    assert_eq!(listings["listings"][0]["title"], "Title");
    assert_eq!(listings["listings"][0]["releaseNotes"], "notes");
    gateway.execute(&request(Platform::Ios,Operation::Sync,json!({"version":"1.2","locale":"en-US","operation":"update_app_metadata",
        "syncScopes":["keywords"],"metadata":{"keywords":"","description":"must-not-write","title":"must-not-write"}}))).await.unwrap();
    let records = fake.records();
    let lookup = records
        .iter()
        .find(|r| r.uri.starts_with("/v1/apps?"))
        .unwrap();
    let url = reqwest::Url::parse(&format!("{}{}", fake.url, lookup.uri)).unwrap();
    assert!(url
        .query_pairs()
        .any(|(k, v)| k == "filter[bundleId]" && v == "com.example.App"));
    assert!(records.iter().all(|r| !r.uri.contains("evil")));
    let patch = records.iter().find(|r| r.method == "PATCH").unwrap();
    assert_eq!(patch.body()["data"]["attributes"], json!({"keywords":""}));
    let tokens = records
        .iter()
        .filter_map(|r| r.headers.get("authorization"))
        .collect::<Vec<_>>();
    assert!(tokens.windows(2).all(|pair| pair[0] == pair[1]));
    let token = tokens[0].to_str().unwrap().strip_prefix("Bearer ").unwrap();
    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::ES256);
    validation.set_audience(&["appstoreconnect-v1"]);
    validation.set_issuer(&["fake-issuer"]);
    let claims = jsonwebtoken::decode::<Value>(
        token,
        &jsonwebtoken::DecodingKey::from_ec_pem(keys.apple_public.as_bytes()).unwrap(),
        &validation,
    )
    .unwrap();
    assert_eq!(claims.header.kid.as_deref(), Some("fake-key"));
    assert_eq!(
        claims.claims["exp"].as_u64().unwrap() - claims.claims["iat"].as_u64().unwrap(),
        1200
    );
}

#[tokio::test]
async fn google_oauth_cache_targeted_release_and_safe_commit() {
    let fake=Fake::new(|r|google_base(r).unwrap_or_else(||{
        if r.uri.ends_with("/tracks") {return ok(json!({"tracks":[{"track":"production","releases":[{"name":"old","status":"completed","versionCodes":["9"],"releaseNotes":[{"language":"ja","text":"preserve"}]}]},
            {"track":"internal","releases":[{"name":"new","status":"draft","versionCodes":["20"]}]}]}));}ok(json!({}))
    })).await;
    let keys = Keys::new();
    let gateway = Gateway::new(keys.config(&fake.url)).unwrap();
    let result = gateway
        .execute(&request(
            Platform::Android,
            Operation::Sync,
            json!({"operation":"update_release_notes","locale":"en-US","releaseNotes":"New notes",
        "storeRelease":{"track":"production","versionCode":"9"},"app":{"packageName":"evil"}}),
        ))
        .await
        .unwrap();
    assert_eq!(result["storeState"]["versionCode"], "9");
    let automatic=gateway.execute(&request(Platform::Android,Operation::Sync,json!({"operation":"update_release_notes","locale":"en-US","releaseNotes":"Latest notes"}))).await.unwrap();
    assert_eq!(automatic["storeState"]["track"], "internal");
    assert_eq!(automatic["storeState"]["versionCode"], "20");
    let named = gateway.execute(&request(Platform::Android, Operation::Sync, json!({
        "operation":"update_release_notes", "locale":"en-US", "releaseNotes":"Named release notes", "storeRelease":{"versionName":"old"}
    }))).await.unwrap();
    assert_eq!(named["storeState"]["track"], "production");
    assert_eq!(named["storeState"]["versionCode"], "9");
    gateway
        .execute(&request(
            Platform::Android,
            Operation::Preflight,
            json!({"operation":"update_app_metadata"}),
        ))
        .await
        .unwrap();
    let records = fake.records();
    assert_eq!(records.iter().filter(|r| r.uri == "/token").count(), 1);
    let form = reqwest::Url::parse(&format!(
        "http://placeholder/?{}",
        String::from_utf8_lossy(&records.iter().find(|r| r.uri == "/token").unwrap().bytes)
    ))
    .unwrap();
    let assertion = form
        .query_pairs()
        .find(|(k, _)| k == "assertion")
        .unwrap()
        .1
        .into_owned();
    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
    validation.set_audience(&[format!("{}/token", fake.url)]);
    let claims = jsonwebtoken::decode::<Value>(
        &assertion,
        &jsonwebtoken::DecodingKey::from_rsa_pem(keys.google_public.as_bytes()).unwrap(),
        &validation,
    )
    .unwrap();
    assert_eq!(claims.claims["iss"], "fake@example.test");
    let put = records.iter().find(|r| r.method == "PUT").unwrap();
    assert!(put.uri.ends_with("/tracks/production"));
    assert_eq!(
        put.body()["releases"][0]["releaseNotes"],
        json!([{"language":"ja","text":"preserve"},{"language":"en-US","text":"New notes"}])
    );
    let commit = records.iter().find(|r| r.uri.contains(":commit?")).unwrap();
    assert_eq!(commit.headers.get("content-length").unwrap(), "0");
    assert!(commit.bytes.is_empty());
    assert!(!commit.uri.contains("changesNotSentForReview"));
    assert!(commit
        .uri
        .contains("changesInReviewBehavior=ERROR_IF_IN_REVIEW"));
    assert!(!records.iter().any(|r| r.uri.contains("evil")));
    assert!(records.iter().any(|r| r.method == "DELETE"));
}

#[tokio::test]
async fn google_metadata_scope_and_failed_commit_discard_edit_without_retry() {
    let fake = Fake::new(|r| {
        google_base(r).unwrap_or_else(|| {
            if r.uri.contains(":commit?") {
                return (
                    StatusCode::BAD_REQUEST,
                    vec![],
                    br#"{"error":{"status":"FAILED_PRECONDITION","message":"You already have changes in review. private upstream bearer and key material"}}"#.to_vec(),
                );
            }
            ok(json!({}))
        })
    })
    .await;
    let keys = Keys::new();
    let gateway = Gateway::new(keys.config(&fake.url)).unwrap();
    let error=gateway.execute(&request(Platform::Android,Operation::Sync,json!({"operation":"update_app_metadata","locale":"en-US","syncScopes":["title"],"metadata":{"title":"","description":"excluded"}}))).await.unwrap_err();
    assert_eq!(error.status, 502);
    assert!(!error.message.contains("private"));
    let records = fake.records();
    assert_eq!(
        records.iter().find(|r| r.method == "PATCH").unwrap().body(),
        json!({"title":""})
    );
    assert_eq!(
        records
            .iter()
            .filter(|r| r.uri.contains(":commit?"))
            .count(),
        1
    );
    assert!(records
        .iter()
        .any(|r| r.method == "DELETE" && r.uri.ends_with("/edits/edit")));
    let commit = records.iter().find(|r| r.uri.contains(":commit?")).unwrap();
    assert!(commit
        .uri
        .contains("changesInReviewBehavior=ERROR_IF_IN_REVIEW"));
    assert!(!commit.uri.contains("changesNotSentForReview"));
}

#[tokio::test]
async fn apple_missing_locale_creation_and_locked_version_preflight() {
    let locked = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let switch = locked.clone();
    let fake = Fake::new(move |r| {
        if r.uri.contains("/appStoreVersions?") && switch.load(std::sync::atomic::Ordering::Relaxed)
        {
            return ok(
                json!({"data":[{"id":"version","attributes":{"appStoreState":"IN_REVIEW"}}]}),
            );
        }
        if r.uri.contains("/appStoreVersionLocalizations?") {
            return ok(json!({"data":[]}));
        }
        if r.method == "POST" && r.uri == "/v1/appStoreVersionLocalizations" {
            return ok(json!({"data":{"id":"created-loc"}}));
        }
        apple_base(r).unwrap_or_else(|| ok(json!({})))
    })
    .await;
    let keys = Keys::new();
    let gateway = Gateway::new(keys.config(&fake.url)).unwrap();
    gateway.execute(&request(Platform::Ios,Operation::Sync,json!({"operation":"update_release_notes","version":"1.0","locale":"fr-FR","releaseNotes":"Bonjour"}))).await.unwrap();
    let records = fake.records();
    let created = records
        .iter()
        .find(|r| r.method == "POST" && r.uri == "/v1/appStoreVersionLocalizations")
        .unwrap()
        .body();
    assert_eq!(created["data"]["attributes"]["locale"], "fr-FR");
    assert_eq!(
        created["data"]["relationships"]["appStoreVersion"]["data"]["id"],
        "version"
    );
    let info = records
        .iter()
        .position(|r| r.method == "POST" && r.uri == "/v1/appInfoLocalizations")
        .unwrap();
    let version = records
        .iter()
        .position(|r| r.method == "POST" && r.uri == "/v1/appStoreVersionLocalizations")
        .unwrap();
    assert!(info < version);
    assert_eq!(
        records[info].body()["data"]["attributes"],
        json!({"locale":"fr-FR", "name":"Title"})
    );
    locked.store(true, std::sync::atomic::Ordering::Relaxed);
    let preflight = gateway
        .execute(&request(
            Platform::Ios,
            Operation::Preflight,
            json!({"operation":"update_release_notes","version":"1.0"}),
        ))
        .await
        .unwrap();
    assert_eq!(preflight["canSync"], false);
    let result=gateway.execute(&request(Platform::Ios,Operation::Sync,json!({"operation":"update_release_notes","version":"1.0","locale":"fr-FR","releaseNotes":"blocked"}))).await.unwrap();
    assert_eq!(result["errorCode"], "store_version_not_editable");
    assert_eq!(
        fake.records()
            .iter()
            .filter(|r| r.method == "PATCH")
            .count(),
        1
    );
}

#[tokio::test]
async fn apple_and_google_review_pagination_and_comment_mapping() {
    let fake=Fake::new(|r|{
        if r.uri.contains("/customerReviews?"){return ok(json!({"data":[{"id":"review","attributes":{"rating":4,"body":"Useful","createdDate":"2026-01-01T01:02:03Z","reviewerNickname":"nick"}}],"links":{"next":"https://apple.test/reviews?cursor=cursor%2Bnext"},"meta":{"paging":{"total":2}}}));}
        if r.uri.contains("/reviews?"){return ok(json!({"reviews":[{"reviewId":"g-review","authorName":"user","comments":[{"userComment":{"text":"old"}},{"developerComment":{"text":"skip"}},{"userComment":{"text":"latest","starRating":5,"lastModified":{"seconds":"1700000000"},"appVersionCode":7}}]}],"tokenPagination":{"nextPageToken":"next-google"}}));}
        apple_base(r).or_else(||google_base(r)).unwrap_or_else(||ok(json!({})))
    }).await;
    let keys = Keys::new();
    let gateway = Gateway::new(keys.config(&fake.url)).unwrap();
    let a = gateway
        .execute(&request(
            Platform::Ios,
            Operation::StoreReviews,
            json!({"pageSize":999,"pageToken":"abc+","sort":"createdDate"}),
        ))
        .await
        .unwrap();
    assert_eq!(a["nextPageToken"], "cursor+next");
    let g = gateway
        .execute(&request(
            Platform::Android,
            Operation::StoreReviews,
            json!({"pageSize":999,"pageToken":"abc+","startIndex":"10","translationLanguage":"en"}),
        ))
        .await
        .unwrap();
    assert_eq!(g["reviews"][0]["body"], "latest");
    assert_eq!(g["nextPageToken"], "next-google");
    let records = fake.records();
    assert!(records
        .iter()
        .any(|r| r.uri.contains("limit=200") && r.uri.contains("cursor=abc%2B")));
    assert!(records
        .iter()
        .any(|r| r.uri.contains("maxResults=100") && r.uri.contains("translationLanguage=en")));
}

#[tokio::test]
async fn apple_ppo_create_treatments_and_cpp_create_and_ownership() {
    let fake=Fake::new(|r|{
        if r.uri=="/v2/appStoreVersionExperiments"{return ok(json!({"data":{"id":"experiment","attributes":{"name":"Experiment","platform":"IOS","trafficProportion":50}}}));}
        if r.uri=="/v1/appStoreVersionExperimentTreatments"{return ok(json!({"data":{"id":"treatment","attributes":{"name":"Variant A"}}}));}
        if r.uri=="/v1/appCustomProductPages"{return ok(json!({"data":{"id":"page"},"included":[{"type":"appCustomProductPageLocalizations","id":"page-loc"}]}));}
        if r.uri=="/v1/appCustomProductPages/foreign/app"{return ok(json!({"data":{"id":"foreign-app"}}));}
        apple_base(r).unwrap_or_else(||ok(json!({})))
    }).await;
    let keys = Keys::new();
    let gateway = Gateway::new(keys.config(&fake.url)).unwrap();
    let ppo=gateway.execute(&request(Platform::Ios,Operation::CreateProductPageOptimization,json!({"name":"Experiment","trafficProportion":50,"locales":["ja"],"treatments":[{"name":"Variant A"}]}))).await.unwrap();
    assert_eq!(ppo["experiment"]["treatments"][0]["locales"], json!(["ja"]));
    let cpp=gateway.execute(&request(Platform::Ios,Operation::Sync,json!({"operation":"update_marketing_page","locale":"en-US","syncScopes":["marketing_text"],"marketingPage":{"pageName":"Page","promotionalText":""}}))).await.unwrap();
    assert_eq!(cpp["storeState"]["applePageId"], "page");
    assert!(gateway.execute(&request(Platform::Ios,Operation::Sync,json!({"operation":"update_marketing_page","locale":"en-US","marketingPage":{"pageName":"Page","applePageId":"foreign"}}))).await.is_err());
    let records = fake.records();
    let create = records
        .iter()
        .find(|r| r.uri == "/v1/appCustomProductPages")
        .unwrap()
        .body();
    assert_eq!(
        create["data"]["relationships"]["app"]["data"]["id"],
        "resolved-app"
    );
    assert!(records.iter().any(
        |r| r.uri == "/v1/appStoreVersionExperimentTreatmentLocalizations"
            && r.body()["data"]["attributes"]["locale"] == "ja"
    ));
    assert!(!records
        .iter()
        .any(|r| r.method == "PATCH" && r.uri.contains("foreign")));
}

fn png() -> Vec<u8> {
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(2, 3)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}
#[tokio::test]
async fn image_uploads_stage_library_assets_before_placements_and_google_multipart() {
    let image = png();
    let expected = image.clone();
    let port = Arc::new(Mutex::new(String::new()));
    let upload_base = port.clone();
    let fake = Fake::new(move |r| {
        if let Some(reply) = apple_upload_base(r, &image, &upload_base.lock().unwrap()) {
            return reply;
        }
        apple_base(r)
            .or_else(|| google_base(r))
            .unwrap_or_else(|| ok(json!({})))
    })
    .await;
    *port.lock().unwrap() = fake.url.clone();
    let keys = Keys::new();
    let gateway = Gateway::new(keys.config(&fake.url)).unwrap();
    let payload = json!({"operation":"update_app_metadata","version":"1.0","locale":"en-US","syncScopes":["store_images"],"metadata":{"storeImages":{"phone_screenshots":{"urls":[format!("{}/image.png",fake.url)]}}}});
    gateway
        .execute(&request(Platform::Ios, Operation::Sync, payload.clone()))
        .await
        .unwrap();
    gateway
        .execute(&request(Platform::Android, Operation::Sync, payload))
        .await
        .unwrap();
    let records = fake.records();
    let reservation = records
        .iter()
        .find(|r| r.uri == "/v1/appAssetLibraryImages" && r.method == "POST")
        .unwrap()
        .body();
    assert_eq!(
        reservation["data"]["relationships"]["assetLibrary"]["data"]["id"],
        "library"
    );
    assert!(reservation["data"]["attributes"]
        .get("sourceFileChecksum")
        .is_none());
    let chunk = records.iter().find(|r| r.uri == "/chunk").unwrap();
    assert_eq!(chunk.bytes, expected);
    assert!(!chunk.headers.contains_key("authorization"));
    assert_eq!(chunk.headers["x-upload"], "chunk-header");
    assert!(records.iter().any(|r| r.method == "PATCH"
        && r.uri == "/v1/appAssetLibraryImages/shot"
        && r.body()["data"]["attributes"] == json!({"uploaded":true})));
    let committed = records
        .iter()
        .position(|r| r.method == "PATCH" && r.uri == "/v1/appAssetLibraryImages/shot")
        .unwrap();
    let removed = records
        .iter()
        .position(|r| r.method == "DELETE" && r.uri.starts_with("/v1/appAssetLibraryPlacements/"))
        .unwrap();
    let placed = records
        .iter()
        .position(|r| r.method == "POST" && r.uri == "/v1/appAssetLibraryPlacements")
        .unwrap();
    assert!(committed < removed && removed < placed);
    assert!(!records
        .iter()
        .any(|r| r.method == "GET" && r.uri == "/v1/appAssetLibraryImages/shot"));
    assert_eq!(
        records[placed].body()["data"]["relationships"]["appStoreVersionLocalization"]["data"]
            ["id"],
        "version-loc"
    );
    assert!(records
        .iter()
        .any(|r| r.uri == "/v1/appAssetLibraryPlacementOrderingRequests"
            && r.body()["data"]["relationships"]["orderedPlacements"]["data"]
                .as_array()
                .unwrap()
                .len()
                == 1));
    assert!(!records.iter().any(|r| r.uri.contains("appScreenshot")));
    let upload = records
        .iter()
        .find(|r| r.uri.starts_with("/upload/"))
        .unwrap();
    assert!(upload.uri.ends_with("?uploadType=multipart"));
    assert!(upload.headers["content-type"]
        .to_str()
        .unwrap()
        .starts_with("multipart/related;"));
    assert!(upload.bytes.windows(expected.len()).any(|w| w == expected));
}

#[tokio::test]
async fn apple_library_commit_failure_or_spec_mismatch_preserves_existing_placements() {
    for failed in [true, false] {
        let image = png();
        let endpoint = Arc::new(Mutex::new(String::new()));
        let upstream = endpoint.clone();
        let fake=Fake::new(move |r| {
            if r.method=="PATCH" && r.uri=="/v1/appAssetLibraryImages/shot" {
                if failed { return (StatusCode::BAD_REQUEST, vec![], b"private vendor error".to_vec()); }
                return ok(json!({"data":{"id":"shot","attributes":{"state":"UPLOAD_COMPLETE","specId":"unexpected-spec"}}}));
            }
            apple_upload_base(r,&image,&upstream.lock().unwrap()).or_else(||apple_base(r)).unwrap_or_else(||ok(json!({})))
        }).await;
        *endpoint.lock().unwrap() = fake.url.clone();
        let keys = Keys::new();
        let gateway = Gateway::new(keys.config(&fake.url)).unwrap();
        let error=gateway.execute(&request(Platform::Ios,Operation::Sync,json!({
            "operation":"update_app_metadata","version":"1.0","locale":"en-US","syncScopes":["store_images"],
            "metadata":{"storeImages":{"phone_screenshots":{"urls":[format!("{}/image.png",fake.url)]}}}
        }))).await.unwrap_err();
        assert_eq!(error.status, 502);
        assert!(!fake.records().iter().any(|r| r.method == "DELETE"
            || r.uri == "/v1/appAssetLibraryPlacements"
            || r.uri == "/v1/appAssetLibraryPlacementOrderingRequests"));
    }
}

#[tokio::test]
async fn apple_foldable_upload_commits_without_polling_and_orders_both_runtime_groups() {
    let image = png();
    let endpoint = Arc::new(Mutex::new(String::new()));
    let upstream = endpoint.clone();
    let fake = Fake::new(move |r| {
        if r.method == "GET" && r.uri == "/v1/appAssetLibraryImages/shot" {
            panic!("Committed image must not be polled before placement");
        }
        if r.uri.contains("/placements?") {
            return ok(apple_image_placements(&[
                ("old-duo", "duo-inner-profile"),
                ("keep-phone", "phone-profile"),
                ("keep-tablet", "tablet-profile"),
            ]));
        }
        apple_upload_base(r, &image, &upstream.lock().unwrap())
            .or_else(|| apple_base(r))
            .unwrap_or_else(|| ok(json!({})))
    })
    .await;
    *endpoint.lock().unwrap() = fake.url.clone();
    let keys = Keys::new();
    let gateway = Gateway::new(keys.config(&fake.url)).unwrap();
    assert_eq!(gateway.execute(&request(Platform::Ios,Operation::Sync,json!({
        "operation":"update_app_metadata","version":"1.0","locale":"en-US","syncScopes":["store_images"],
        "metadata":{"storeImages":{"foldable_screenshots":{"urls":[format!("{}/image.png",fake.url)]}}}
    }))).await.unwrap()["status"],"succeeded");
    let records = fake.records();
    assert!(!records
        .iter()
        .any(|r| r.method == "GET" && r.uri == "/v1/appAssetLibraryImages/shot"));
    let deletes = records
        .iter()
        .filter(|r| r.method == "DELETE")
        .collect::<Vec<_>>();
    assert_eq!(deletes.len(), 1);
    assert!(deletes[0].uri.ends_with("placement%2Dold%2Dduo"));
    let placements = records
        .iter()
        .filter(|r| r.method == "POST" && r.uri == "/v1/appAssetLibraryPlacements")
        .map(Record::body)
        .collect::<Vec<_>>();
    assert_eq!(placements.len(), 2);
    assert_eq!(
        placements[0]["data"]["attributes"]["placementGroup"],
        "duo-inner-profile"
    );
    assert_eq!(
        placements[1]["data"]["attributes"]["placementGroup"],
        "duo-outer-profile"
    );
    assert!(placements
        .iter()
        .all(|placement| placement["data"]["relationships"]["image"]["data"]["id"] == "shot"));
    assert_eq!(
        records
            .iter()
            .filter(|r| r.uri == "/v1/appAssetLibraryPlacementOrderingRequests")
            .count(),
        2
    );
    assert_eq!(
        records
            .iter()
            .filter(|r| r.method == "POST" && r.uri == "/v1/appAssetLibraryImages")
            .count(),
        1
    );
    assert!(records
        .iter()
        .filter(|r| r.method == "DELETE")
        .all(|r| r.uri.starts_with("/v1/appAssetLibraryPlacements/")));
}

#[tokio::test]
async fn invalid_images_do_not_delete_existing_images_and_google_edit_is_discarded() {
    let fake = Fake::new(|r| {
        if r.uri == "/bad.png" {
            return (StatusCode::OK, vec![], b"not an image".to_vec());
        }
        google_base(r).unwrap_or_else(|| ok(json!({})))
    })
    .await;
    let keys = Keys::new();
    let gateway = Gateway::new(keys.config(&fake.url)).unwrap();
    assert!(gateway.execute(&request(Platform::Android,Operation::Sync,json!({"operation":"update_app_metadata","locale":"en-US","syncScopes":["store_images"],"metadata":{"storeImages":{"phone_screenshots":{"urls":[format!("{}/bad.png",fake.url)]}}}}))).await.is_err());
    let records = fake.records();
    assert!(!records
        .iter()
        .any(|r| r.method == "DELETE" && r.uri.contains("phoneScreenshots")));
    assert!(records
        .iter()
        .any(|r| r.method == "DELETE" && r.uri.ends_with("/edits/edit")));
    assert!(!records.iter().any(|r| r.uri.contains(":commit?")));
}

#[tokio::test]
async fn read_resources_map_all_google_images_releases_and_apple_ppo() {
    let fake=Fake::new(|r|{
        if r.uri.ends_with("/listings"){return ok(json!({"listings":[{"language":"ja","title":"Japanese"},{"language":"en-US","title":"English","fullDescription":"Full","shortDescription":"Short","video":"https://video.test"}]}));}
        if r.uri.ends_with("/tracks"){return ok(json!({"tracks":[{"track":"production","releases":[{"name":"Old","status":"completed","versionCodes":["9"]}]},{"track":"internal","releases":[{"name":"New","status":"draft","versionCodes":["10"]}]}]}));}
        if r.uri.contains("/listings/"){return ok(json!({"images":[{"id":"image","url":"https://cdn.test/image","sha1":"digest1","sha256":"digest256"}]}));}
        if r.uri.contains("/appStoreVersionExperimentsV2?"){return ok(json!({"data":[{"id":"ppo","attributes":{"name":"PPO","trafficProportion":25,"state":"PREPARE_FOR_SUBMISSION"},"relationships":{"appStoreVersionExperimentTreatments":{"data":[{"id":"variant"}]}}}],"included":[{"type":"appStoreVersionExperimentTreatments","id":"variant","attributes":{"name":"Variant","appIconName":"Icon"}}]}));}
        if r.uri.contains("/appScreenshotSets?"){return ok(json!({"data":[{"id":"set","attributes":{"screenshotDisplayType":"APP_IPHONE_67"},"relationships":{"appScreenshots":{"data":[{"id":"screenshot"}]}}}],"included":[{"id":"screenshot","type":"appScreenshots","attributes":{"fileName":"shot.png","imageAsset":{"templateUrl":"https://cdn.test/{w}","width":1290,"height":2796}}}]}));}
        apple_base(r).or_else(||google_base(r)).unwrap_or_else(||ok(json!({})))
    }).await;
    let keys = Keys::new();
    let gateway = Gateway::new(keys.config(&fake.url)).unwrap();
    let locales = gateway
        .execute(&request(
            Platform::Android,
            Operation::SupportedLocales,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(locales["locales"], json!(["en-US", "ja"]));
    let listings = gateway
        .execute(&request(
            Platform::Android,
            Operation::StoreListings,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(listings["listings"][0]["description"], "Full");
    let images = gateway
        .execute(&request(
            Platform::Android,
            Operation::StoreImages,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(images["locales"][0]["images"].as_object().unwrap().len(), 6);
    assert_eq!(
        images["locales"][0]["images"]["wear_screenshots"][0]["sha256"],
        "digest256"
    );
    let releases = gateway
        .execute(&request(
            Platform::Android,
            Operation::StoreReleases,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(releases["releases"][0]["name"], "New");
    let ppo = gateway
        .execute(&request(
            Platform::Ios,
            Operation::ProductPageOptimizations,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(ppo["experiments"][0]["treatments"][0]["name"], "Variant");
    let images = gateway
        .execute(&request(
            Platform::Ios,
            Operation::StoreImages,
            json!({"version":"1.0"}),
        ))
        .await
        .unwrap();
    assert_eq!(
        images["locales"][0]["images"]["phone_screenshots"][0]["width"],
        1290
    );
    assert_eq!(
        fake.records()
            .iter()
            .filter(|r| r.method == "DELETE" && r.uri.ends_with("/edits/edit"))
            .count(),
        4
    );
}

#[tokio::test]
async fn apple_image_queries_filter_language_and_device_before_fetching_placements() {
    let fake = Fake::new(|record| {
        if record.uri.contains("/appStoreVersionLocalizations?") {
            return ok(json!({"data":[
                {"id":"english","attributes":{"locale":"en-US"}},
                {"id":"japanese","attributes":{"locale":"ja"}}
            ]}));
        }
        if record.uri.contains("/placements?") {
            // Return extra groups to prove they cannot leak into selected results.
            return ok(apple_image_placements(&[
                ("phone", "phone-profile"),
                ("tablet", "tablet-profile"),
                ("inner", "duo-inner-profile"),
                ("outer", "duo-outer-profile"),
            ]));
        }
        apple_base(record).unwrap_or_else(|| ok(json!({})))
    })
    .await;
    let keys = Keys::new();
    let gateway = Gateway::new(keys.config(&fake.url)).unwrap();
    for (locale, device, languages, slots, groups) in [
        (
            None,
            None,
            2,
            vec![
                "phone_screenshots",
                "tablet_screenshots",
                "foldable_screenshots",
            ],
            vec![
                "duo-inner-profile",
                "duo-outer-profile",
                "phone-profile",
                "tablet-profile",
            ],
        ),
        (
            Some("  EN-us  "),
            Some("phone"),
            1,
            vec!["phone_screenshots"],
            vec!["phone-profile"],
        ),
        (
            Some("ja"),
            Some("tablet"),
            1,
            vec!["tablet_screenshots"],
            vec!["tablet-profile"],
        ),
        (
            Some("en-US"),
            Some("foldable"),
            1,
            vec!["foldable_screenshots"],
            vec!["duo-inner-profile", "duo-outer-profile"],
        ),
        (
            Some("zz-ZZ"),
            Some("foldable"),
            0,
            vec!["foldable_screenshots"],
            vec!["duo-inner-profile", "duo-outer-profile"],
        ),
        (
            Some("  "),
            Some("  "),
            2,
            vec![
                "phone_screenshots",
                "tablet_screenshots",
                "foldable_screenshots",
            ],
            vec![
                "duo-inner-profile",
                "duo-outer-profile",
                "phone-profile",
                "tablet-profile",
            ],
        ),
    ] {
        let start = fake.records().len();
        let mut payload = json!({"version":"1.0"});
        if let Some(locale) = locale {
            payload["locale"] = json!(locale);
        }
        if let Some(device) = device {
            payload["device"] = json!(device);
        }
        let reply = gateway
            .execute(&request(Platform::Ios, Operation::StoreImages, payload))
            .await
            .unwrap();
        let rows = reply["locales"].as_array().unwrap();
        assert_eq!(rows.len(), languages);
        for row in rows {
            let images = row["images"].as_object().unwrap();
            assert_eq!(images.len(), slots.len());
            for slot in &slots {
                let images = images[*slot].as_array().unwrap();
                assert_eq!(
                    images.len(),
                    if *slot == "foldable_screenshots" {
                        2
                    } else {
                        1
                    }
                );
                assert!(images
                    .iter()
                    .all(|image| groups.contains(&image["placementGroup"].as_str().unwrap())));
            }
            if let Some(locale) = locale.filter(|locale| !locale.trim().is_empty()) {
                assert!(row["locale"]
                    .as_str()
                    .unwrap()
                    .eq_ignore_ascii_case(locale.trim()));
            }
        }
        let records = fake.records();
        let reads: Vec<_> = records[start..]
            .iter()
            .filter(|record| record.uri.contains("/placements?"))
            .collect();
        assert_eq!(reads.len(), languages);
        for read in reads {
            let url = reqwest::Url::parse(&format!("{}{}", fake.url, read.uri)).unwrap();
            assert!(url
                .query_pairs()
                .any(|(key, value)| key == "filter[placementGroup]" && value == groups.join(",")));
            assert!(url
                .query_pairs()
                .any(|(key, value)| key == "sort" && value == "placementGroupPosition"));
            if locale.is_some_and(|locale| locale.trim().eq_ignore_ascii_case("en-US")) {
                assert!(read.uri.contains("/english/placements?"));
            }
        }
    }
    assert!(fake.records().iter().all(|record| record.method == "GET"));
}

#[tokio::test]
async fn google_image_queries_filter_language_and_device_and_discard_edits() {
    let fake = Fake::new(|record| {
        if record.uri.ends_with("/listings") {
            return ok(json!({"listings":[{"language":"en-US"},{"language":"ja"}]}));
        }
        if record.uri.contains("/listings/") {
            return ok(json!({"images":[{"id":"image","url":"https://cdn.test/image"}]}));
        }
        google_base(record).unwrap_or_else(|| ok(json!({})))
    })
    .await;
    let keys = Keys::new();
    let gateway = Gateway::new(keys.config(&fake.url)).unwrap();
    for (locale, device, languages, kinds) in [
        (
            None,
            None,
            2,
            vec![
                "featureGraphic",
                "phoneScreenshots",
                "sevenInchScreenshots",
                "tenInchScreenshots",
                "tvScreenshots",
                "wearScreenshots",
            ],
        ),
        (
            Some("  EN-us  "),
            Some("phone"),
            1,
            vec!["phoneScreenshots"],
        ),
        (
            Some("ja"),
            Some("tablet"),
            1,
            vec!["sevenInchScreenshots", "tenInchScreenshots"],
        ),
        (Some("zz-ZZ"), Some("phone"), 0, vec!["phoneScreenshots"]),
        (
            Some("  "),
            Some("  "),
            2,
            vec![
                "featureGraphic",
                "phoneScreenshots",
                "sevenInchScreenshots",
                "tenInchScreenshots",
                "tvScreenshots",
                "wearScreenshots",
            ],
        ),
    ] {
        let start = fake.records().len();
        let mut payload = json!({});
        if let Some(locale) = locale {
            payload["locale"] = json!(locale);
        }
        if let Some(device) = device {
            payload["device"] = json!(device);
        }
        let reply = gateway
            .execute(&request(Platform::Android, Operation::StoreImages, payload))
            .await
            .unwrap();
        let rows = reply["locales"].as_array().unwrap();
        assert_eq!(rows.len(), languages);
        for row in rows {
            assert_eq!(row["images"].as_object().unwrap().len(), kinds.len());
            if let Some(locale) = locale.filter(|locale| !locale.trim().is_empty()) {
                assert!(row["locale"]
                    .as_str()
                    .unwrap()
                    .eq_ignore_ascii_case(locale.trim()));
            }
        }
        let records = fake.records();
        let reads: Vec<_> = records[start..]
            .iter()
            .filter(|record| record.uri.contains("/listings/"))
            .collect();
        assert_eq!(reads.len(), languages * kinds.len());
        assert!(reads
            .iter()
            .all(|record| kinds.iter().any(|kind| record.uri.ends_with(kind))));
        assert_eq!(
            records[start..]
                .iter()
                .filter(|record| record.method == "DELETE" && record.uri.ends_with("/edits/edit"))
                .count(),
            1
        );
        assert!(!records[start..]
            .iter()
            .any(|record| record.uri.contains(":commit")));
    }
}

#[tokio::test]
async fn invalid_image_read_selectors_fail_before_any_upstream_request() {
    let fake = Fake::new(|_| panic!("Invalid image selectors reached upstream")).await;
    let keys = Keys::new();
    let gateway = Gateway::new(keys.config(&fake.url)).unwrap();
    for platform in [Platform::Ios, Platform::Android] {
        for payload in [
            json!({"device":"desktop"}),
            json!({"device":null}),
            json!({"device":1}),
            json!({"locale":[]}),
        ] {
            assert_eq!(
                gateway
                    .execute(&request(platform, Operation::StoreImages, payload))
                    .await
                    .unwrap_err()
                    .status,
                422
            );
        }
    }
    assert_eq!(
        gateway
            .execute(&request(
                Platform::Android,
                Operation::StoreImages,
                json!({"device":"foldable"})
            ))
            .await
            .unwrap_err()
            .code,
        "unsupported_image_device"
    );
    assert!(fake.records().is_empty());
}

#[tokio::test]
async fn apple_image_pagination_preserves_order_and_rejects_unrelated_next_links() {
    for next in [
        "?cursor=second",
        "https://untrusted.test/steal",
        "/v1/apps?steal=1",
        "?cursor=first",
    ] {
        let fake = Fake::new(move |record| {
            if record.uri.contains("/placements?") {
                if record.uri.ends_with("cursor=second") {
                    return ok(apple_image_placements(&[("second", "phone-profile")]));
                }
                let mut page = apple_image_placements(&[("first", "phone-profile")]);
                // A loop is detected before sending its duplicate request.
                page["links"] = json!({"next":next});
                return ok(page);
            }
            apple_base(record).unwrap_or_else(|| ok(json!({})))
        })
        .await;
        let keys = Keys::new();
        let gateway = Gateway::new(keys.config(&fake.url)).unwrap();
        let result = gateway
            .execute(&request(
                Platform::Ios,
                Operation::StoreImages,
                json!({"version":"1.0","device":"phone"}),
            ))
            .await;
        if next == "?cursor=second" {
            let reply = result.unwrap();
            let images = reply["locales"][0]["images"]["phone_screenshots"]
                .as_array()
                .unwrap();
            assert_eq!(
                images
                    .iter()
                    .map(|image| image["id"].as_str().unwrap())
                    .collect::<Vec<_>>(),
                vec!["first", "second"]
            );
        } else {
            assert_eq!(result.unwrap_err().status, 502);
        }
        assert!(!fake
            .records()
            .iter()
            .any(|record| record.uri.contains("steal")));
    }
}

#[tokio::test]
async fn review_filters_are_page_local_validate_timezone_and_keep_cursor() {
    let fake=Fake::new(|r|{
        if r.uri.contains("/customerReviews?"){return ok(json!({"data":[
            {"id":"selected","attributes":{"rating":5,"locale":"en-US","territory":"US","createdDate":"2026-06-23T16:00:00Z"}},
            {"id":"next-day","attributes":{"rating":5,"locale":"en-US","territory":"US","createdDate":"2026-06-24T16:00:00Z"}},
            {"id":"wrong-rating","attributes":{"rating":3,"locale":"en-US","territory":"US","createdDate":"2026-06-24T01:00:00Z"}}],"links":{"next":"https://apple.test/?cursor=next"}}));}
        apple_base(r).unwrap_or_else(||ok(json!({})))
    }).await;
    let keys = Keys::new();
    let gateway = Gateway::new(keys.config(&fake.url)).unwrap();
    let result=gateway.execute(&request(Platform::Ios,Operation::StoreReviews,json!({"date":"2026-06-24","timezone":"Asia/Shanghai","rating":5,"locale":"EN-us","territory":"us"}))).await.unwrap();
    assert_eq!(result["reviews"].as_array().unwrap().len(), 1);
    assert_eq!(result["reviews"][0]["id"], "selected");
    assert_eq!(result["nextPageToken"], "next");
    for payload in [
        json!({"timezone":"invalid/zone"}),
        json!({"date":"2026-02-30"}),
        json!({"date":"2026-01-01","startDate":"2026-01-01"}),
        json!({"rating":6}),
    ] {
        assert_eq!(
            gateway
                .execute(&request(Platform::Ios, Operation::StoreReviews, payload))
                .await
                .unwrap_err()
                .status,
            422
        );
    }
}

#[tokio::test]
async fn invalid_apple_upload_range_is_rejected_before_uploaded_completion() {
    let image = png();
    let fake=Fake::new(move |r| {
        if r.uri=="/image.png" { return (StatusCode::OK,vec![],image.clone()); }
        if r.method=="POST" && r.uri=="/v1/appAssetLibraryImages" {
            return ok(json!({"data":{"id":"shot","attributes":{"uploadOperations":[{"method":"PUT","url":"http://127.0.0.1:1/never","offset":0,"length":999999}]}}}));
        }
        apple_base(r).unwrap_or_else(||ok(json!({})))
    }).await;
    let keys = Keys::new();
    let gateway = Gateway::new(keys.config(&fake.url)).unwrap();
    assert!(gateway.execute(&request(Platform::Ios,Operation::Sync,json!({
        "operation":"update_app_metadata","version":"1.0","locale":"en-US","syncScopes":["store_images"],
        "metadata":{"storeImages":{"phone_screenshots":{"urls":[format!("{}/image.png",fake.url)]}}}
    }))).await.is_err());
    assert!(!fake.records().iter().any(|r| r.method == "DELETE"
        || r.uri == "/v1/appAssetLibraryPlacements"
        || r.method == "PATCH" && r.uri == "/v1/appAssetLibraryImages/shot"));
}

#[tokio::test]
async fn apple_committed_images_are_placed_without_polling_and_diagnostics_stay_safe() {
    for (state, safe_state) in [
        (Some("UPLOAD_COMPLETE"), "UPLOAD_COMPLETE"),
        (Some("FAILED"), "FAILED"),
        (Some("REJECTED"), "REJECTED"),
        (Some("ARCHIVED"), "ARCHIVED"),
        (Some("private-unknown-state"), "UNKNOWN"),
        (None, "MISSING"),
    ] {
        let image = png();
        let endpoint = Arc::new(Mutex::new(String::new()));
        let upstream = endpoint.clone();
        let fake = Fake::new(move |r| {
            if r.method == "GET" && r.uri == "/v1/appAssetLibraryImages/shot" {
                panic!("Successful commit must continue to placement without polling");
            }
            if r.method == "PATCH" && r.uri == "/v1/appAssetLibraryImages/shot" {
                let mut attributes = json!({
                    "specId":null,
                    "imageAsset":{"url":"https://private.invalid/private-image.png?token=private-token"},
                    "stateDetails":{"errors":[{"code":"private-vendor-code","message":"Invalid image dimensions for private-image.png; https://private.invalid/?token=private-token"}]}
                });
                if let Some(state) = state {
                    attributes["state"] = json!(state);
                }
                return ok(json!({"data":{"id":"shot","attributes":attributes}}));
            }
            if r.uri.contains("/placements?") {
                return ok(apple_image_placements(&[("old-phone", "phone-profile"), ("keep-tablet", "tablet-profile")]));
            }
            apple_upload_base(r, &image, &upstream.lock().unwrap())
                .or_else(|| apple_base(r))
                .unwrap_or_else(|| ok(json!({})))
        })
        .await;
        *endpoint.lock().unwrap() = fake.url.clone();
        let keys = Keys::new();
        let config_path = keys.dir.path().join("connector.json");
        std::fs::write(&config_path, "{}").unwrap();
        let log = store_provider::diagnostics::Log::for_config(&config_path, "test-node").unwrap();
        let gateway = Gateway::with_log(keys.config(&fake.url), None, Some(log.clone())).unwrap();
        let mut request = request(
            Platform::Ios,
            Operation::Sync,
            json!({
                "operation":"update_app_metadata","version":"1.0","locale":"en-US","syncScopes":["store_images"],
                "metadata":{"storeImages":{"phone_screenshots":{"urls":[format!("{}/image.png",fake.url)]}}}
            }),
        );
        request.request_id = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into();
        let result = gateway.execute(&request).await.unwrap();
        assert_eq!(result["status"], "succeeded");
        assert_eq!(
            result["storeState"]["storeImagesSubmission"],
            json!({"status":"accepted","processingStatus":"not_waited","assetCount":1})
        );
        let records = fake.records();
        assert!(!records
            .iter()
            .any(|r| r.method == "GET" && r.uri == "/v1/appAssetLibraryImages/shot"));
        let chunk = records.iter().find(|r| r.uri == "/chunk").unwrap();
        assert!(!chunk.headers.contains_key("authorization"));
        let removed = records
            .iter()
            .filter(|r| r.method == "DELETE")
            .collect::<Vec<_>>();
        assert_eq!(removed.len(), 1);
        assert!(removed[0].uri.ends_with("placement%2Dold%2Dphone"));
        let placements = records
            .iter()
            .filter(|r| r.method == "POST" && r.uri == "/v1/appAssetLibraryPlacements")
            .collect::<Vec<_>>();
        assert_eq!(placements.len(), 1);
        let placement = placements[0].body();
        assert_eq!(
            placement["data"]["attributes"],
            json!({"placementType":"APP_SCREENSHOT","placementGroup":"phone-profile"})
        );
        assert_eq!(
            placement["data"]["relationships"]["image"]["data"],
            json!({"type":"appAssetLibraryImages","id":"shot"})
        );
        assert_eq!(
            placement["data"]["relationships"]["appStoreVersionLocalization"]["data"],
            json!({"type":"appStoreVersionLocalizations","id":"version-loc"})
        );
        let ordering = records
            .iter()
            .find(|r| r.uri == "/v1/appAssetLibraryPlacementOrderingRequests")
            .unwrap()
            .body();
        assert_eq!(
            ordering["data"]["relationships"]["appStoreVersionLocalization"]["data"]["id"],
            "version-loc"
        );
        assert_eq!(
            ordering["data"]["relationships"]["orderedPlacements"]["data"],
            json!([{"type":"appAssetLibraryPlacements","id":"new-phone-profile"}])
        );
        let committed = records
            .iter()
            .position(|r| r.method == "PATCH" && r.uri == "/v1/appAssetLibraryImages/shot")
            .unwrap();
        let removed = records.iter().position(|r| r.method == "DELETE").unwrap();
        assert!(committed < removed);
        let mut entries = Vec::<Value>::new();
        let mut all = String::new();
        for entry in std::fs::read_dir(log.directory()).unwrap() {
            let text = std::fs::read_to_string(entry.unwrap().path()).unwrap();
            all.push_str(&text);
            entries.extend(
                text.lines()
                    .skip(1)
                    .map(|line| serde_json::from_str::<Value>(line).unwrap()),
            );
        }
        let processing = entries
            .iter()
            .find(|entry| {
                entry["event"] == "apple.image_processing"
                    && entry["details"]["phase"] == "committed"
            })
            .unwrap();
        assert_eq!(processing["requestId"], request.request_id);
        assert_eq!(processing["platform"], "ios");
        assert_eq!(processing["details"]["state"], safe_state);
        assert_eq!(
            processing["details"]["reasonClasses"],
            json!(["invalid_dimensions"])
        );
        assert_eq!(processing["details"]["stateDetailsPresent"], true);
        assert_eq!(processing["details"]["specIdPresent"], false);
        assert_eq!(processing["details"]["imageAssetPresent"], true);
        assert_eq!(processing["details"]["submissionAccepted"], true);
        assert!(!all.contains("private"));
    }
}

#[tokio::test]
async fn apple_committed_image_placement_or_ordering_rejection_still_fails_sync() {
    for fail_ordering in [false, true] {
        let image = png();
        let endpoint = Arc::new(Mutex::new(String::new()));
        let upstream = endpoint.clone();
        let fake = Fake::new(move |r| {
            let rejected_path = if fail_ordering {
                "/v1/appAssetLibraryPlacementOrderingRequests"
            } else {
                "/v1/appAssetLibraryPlacements"
            };
            if r.method == "POST" && r.uri == rejected_path {
                return (
                    StatusCode::BAD_REQUEST,
                    vec![],
                    b"private placement error".to_vec(),
                );
            }
            apple_upload_base(r, &image, &upstream.lock().unwrap())
                .or_else(|| apple_base(r))
                .unwrap_or_else(|| ok(json!({})))
        })
        .await;
        *endpoint.lock().unwrap() = fake.url.clone();
        let keys = Keys::new();
        let gateway = Gateway::new(keys.config(&fake.url)).unwrap();
        let error = gateway.execute(&request(Platform::Ios, Operation::Sync, json!({
            "operation":"update_app_metadata","version":"1.0","locale":"en-US","syncScopes":["store_images"],
            "metadata":{"storeImages":{"phone_screenshots":{"urls":[format!("{}/image.png",fake.url)]}}}
        }))).await.unwrap_err();
        assert_eq!(error.status, 502);
        assert_eq!(error.code, "store_upstream_error");
        assert!(!error.message.contains("private"));
        let records = fake.records();
        assert!(records
            .iter()
            .any(|r| r.method == "PATCH" && r.uri == "/v1/appAssetLibraryImages/shot"));
        assert!(records
            .iter()
            .any(|r| r.method == "POST" && r.uri == "/v1/appAssetLibraryPlacements"));
        assert_eq!(
            records
                .iter()
                .filter(|r| r.uri == "/v1/appAssetLibraryPlacementOrderingRequests")
                .count(),
            usize::from(fail_ordering)
        );
        assert!(!records
            .iter()
            .any(|r| r.method == "GET" && r.uri == "/v1/appAssetLibraryImages/shot"));
    }
}

#[tokio::test]
async fn apple_adaptive_rate_limit_and_identity_mismatch_fail_closed() {
    let fake = Fake::new(|r| {
        let mut reply = apple_base(r).unwrap_or_else(|| ok(json!({})));
        reply
            .1
            .push(("x-rate-limit", "user-hour-lim:3;user-hour-rem:2;".into()));
        reply
    })
    .await;
    let keys = Keys::new();
    let gateway = Gateway::new(keys.config(&fake.url)).unwrap();
    gateway
        .execute(&request(Platform::Ios, Operation::StoreReleases, json!({})))
        .await
        .unwrap();
    let error = gateway
        .execute(&request(Platform::Ios, Operation::StoreReleases, json!({})))
        .await
        .unwrap_err();
    assert_eq!(error.status, 429);
    assert_eq!(fake.records().len(), 2);
    let other = Fake::new(|_| {
        ok(json!({"data":[{"id":"foreign","attributes":{"bundleId":"com.other.App"}}]}))
    })
    .await;
    let gateway = Gateway::new(keys.config(&other.url)).unwrap();
    assert!(gateway.execute(&request(Platform::Ios,Operation::Sync,json!({"operation":"update_release_notes","version":"1.0","locale":"en-US","releaseNotes":"never"}))).await.is_err());
    assert_eq!(other.records().len(), 1);
}

#[tokio::test]
async fn apple_info_localization_creation_supplies_selected_name_and_validates_before_writes() {
    let fake = Fake::new(|r| apple_base(r).unwrap_or_else(|| ok(json!({})))).await;
    let keys = Keys::new();
    let gateway = Gateway::new(keys.config(&fake.url)).unwrap();
    gateway.execute(&request(Platform::Ios, Operation::Sync, json!({
        "operation":"update_app_metadata", "version":"1.0", "locale":"fr-FR",
        "syncScopes":["title", "description"], "metadata":{"title":"Titre", "description":"Description"}
    }))).await.unwrap();
    let records = fake.records();
    let info = records
        .iter()
        .position(|r| r.method == "POST" && r.uri == "/v1/appInfoLocalizations")
        .unwrap();
    let version = records
        .iter()
        .position(|r| r.method == "POST" && r.uri == "/v1/appStoreVersionLocalizations")
        .unwrap();
    assert!(info < version);
    assert_eq!(
        records[info].body()["data"]["attributes"],
        json!({"locale":"fr-FR", "name":"Titre"})
    );

    let no_fallback = Fake::new(|r| {
        if r.uri.contains("/appInfoLocalizations?") {
            return ok(json!({"data":[]}));
        }
        apple_base(r).unwrap_or_else(|| ok(json!({})))
    })
    .await;
    let gateway = Gateway::new(keys.config(&no_fallback.url)).unwrap();
    for payload in [
        json!({"operation":"update_release_notes", "version":"1.0", "locale":"fr-FR", "releaseNotes":"Notes"}),
        json!({"operation":"update_app_metadata", "version":"1.0", "locale":"fr-FR", "syncScopes":["description"], "metadata":{"description":"Description"}}),
        json!({"operation":"update_app_metadata", "version":"1.0", "locale":"fr-FR", "syncScopes":["title"], "metadata":{"title":""}}),
    ] {
        assert_eq!(
            gateway
                .execute(&request(Platform::Ios, Operation::Sync, payload))
                .await
                .unwrap_err()
                .status,
            422
        );
    }
    assert!(no_fallback.records().iter().all(|r| r.method == "GET"));
}

#[tokio::test]
async fn apple_version_reads_and_cpp_uploads_use_asset_library_and_correct_localization() {
    let image = png();
    let endpoint = Arc::new(Mutex::new(String::new()));
    let upstream = endpoint.clone();
    let fake = Fake::new(move |r| {
        if let Some(reply) = apple_upload_base(r, &image, &upstream.lock().unwrap()) {
            return reply;
        }
        if r.uri == "/v1/appCustomProductPages/page/app" {
            return ok(json!({"data":{"id":"resolved-app"}}));
        }
        if r.uri.contains("/appCustomProductPageVersions?") {
            return ok(json!({"data":[{"id":"cpp-version"}]}));
        }
        if r.uri.contains("/appCustomProductPageLocalizations?") {
            return ok(json!({"data":[]}));
        }
        if r.method == "POST" && r.uri == "/v1/appCustomProductPageLocalizations" {
            return ok(json!({"data":{"id":"cpp-loc"}}));
        }
        apple_base(r).unwrap_or_else(|| ok(json!({})))
    })
    .await;
    *endpoint.lock().unwrap() = fake.url.clone();
    let keys = Keys::new();
    let gateway = Gateway::new(keys.config(&fake.url)).unwrap();
    gateway
        .execute(&request(
            Platform::Ios,
            Operation::StoreImages,
            json!({"version":"1.0"}),
        ))
        .await
        .unwrap();
    gateway.execute(&request(Platform::Ios,Operation::Sync,json!({
        "operation":"update_marketing_page","locale":"fr-FR","syncScopes":["store_images"],
        "marketingPage":{"pageName":"Page","applePageId":"page","storeImages":{"phone_screenshots":{"urls":[format!("{}/image.png",fake.url)]}}}
    }))).await.unwrap();
    let records = fake.records();
    assert!(records.iter().any(|record| record
        .uri
        .contains("/appStoreVersionLocalizations/version%2Dloc/placements?")));
    assert!(!records
        .iter()
        .any(|record| record.uri.contains("appScreenshot")));
    let localization = records
        .iter()
        .find(|r| r.method == "POST" && r.uri == "/v1/appCustomProductPageLocalizations")
        .unwrap()
        .body();
    assert_eq!(
        localization["data"]["attributes"],
        json!({"locale":"fr-FR"})
    );
    let placement = records
        .iter()
        .find(|r| r.method == "POST" && r.uri == "/v1/appAssetLibraryPlacements")
        .unwrap()
        .body();
    assert_eq!(
        placement["data"]["relationships"]["appCustomProductPageLocalization"]["data"],
        json!({"type":"appCustomProductPageLocalizations","id":"cpp-loc"})
    );
    assert!(placement["data"]["relationships"]
        .get("appStoreVersionLocalization")
        .is_none());
    let order = records
        .iter()
        .find(|r| r.uri == "/v1/appAssetLibraryPlacementOrderingRequests")
        .unwrap()
        .body();
    assert_eq!(
        order["data"]["relationships"]["appCustomProductPageLocalization"]["data"]["id"],
        "cpp-loc"
    );
}

#[tokio::test]
async fn malformed_sync_selectors_fail_before_any_upstream_request() {
    let fake = Fake::new(|_| ok(json!({}))).await;
    let keys = Keys::new();
    let gateway = Gateway::new(keys.config(&fake.url)).unwrap();
    let metadata = json!({"operation":"update_app_metadata", "version":"1.0", "locale":"en-US", "metadata":{"title":"Selected", "description":"Do not broaden"}});
    for platform in [Platform::Ios, Platform::Android] {
        for selector in [
            json!("title"),
            json!(null),
            json!([]),
            json!(["title", 7]),
            json!(["unknown"]),
            json!({"title":true}),
        ] {
            let mut payload = metadata.clone();
            payload["syncScopes"] = selector;
            assert_eq!(
                gateway
                    .execute(&request(platform, Operation::Sync, payload))
                    .await
                    .unwrap_err()
                    .status,
                422
            );
        }
        for selector in [
            json!(null),
            json!([]),
            json!({}),
            json!("production"),
            json!({"versionCode":7}),
            json!({"track":true}),
            json!({"versionName":null}),
            json!({"versionCode":""}),
            json!({"track":"production", "versionCodes":["7"]}),
        ] {
            let payload = json!({"operation":"update_release_notes", "version":"1.0", "locale":"en-US", "releaseNotes":"Never apply to another release", "storeRelease":selector});
            assert_eq!(
                gateway
                    .execute(&request(platform, Operation::Sync, payload))
                    .await
                    .unwrap_err()
                    .status,
                422
            );
        }
        for (field, value) in [
            ("operation", json!("unknown")),
            ("operation", json!(7)),
            ("locale", json!(null)),
            ("locale", json!("")),
        ] {
            let mut payload = metadata.clone();
            payload[field] = value;
            assert_eq!(
                gateway
                    .execute(&request(platform, Operation::Sync, payload))
                    .await
                    .unwrap_err()
                    .status,
                422
            );
        }
    }
    for payload in [
        json!({"locale":"en-US", "version":"1.0", "metadata":{"title":"Selected"}}),
        json!({"operation":"update_app_metadata", "locale":"en-US", "metadata":null}),
        json!({"operation":"update_release_notes", "locale":"en-US", "version":"1.0"}),
        json!({"operation":"update_marketing_page", "locale":"en-US", "marketingPage":{"pageName":7}}),
    ] {
        assert_eq!(
            gateway
                .execute(&request(Platform::Ios, Operation::Sync, payload))
                .await
                .unwrap_err()
                .status,
            422
        );
    }
    assert!(fake.records().is_empty());
}

#[tokio::test]
async fn malformed_ppo_types_are_rejected_before_apple_identity_lookup() {
    let fake = Fake::new(|_| ok(json!({}))).await;
    let keys = Keys::new();
    let gateway = Gateway::new(keys.config(&fake.url)).unwrap();
    let valid = json!({"name":"Experiment", "trafficProportion":50, "locales":["en-US"], "treatments":[{"name":"Variant", "locales":["ja"]}]});
    for (field, value) in [
        ("trafficProportion", json!("50")),
        ("trafficProportion", json!(0)),
        ("trafficProportion", json!(50.5)),
        ("trafficProportion", json!(null)),
        ("locales", json!("en-US")),
        ("locales", json!(null)),
        ("locales", json!(["en-US", 7])),
        ("treatments", json!({"name":"Variant"})),
        ("treatments", json!(null)),
        ("treatments", json!([7])),
        ("treatments", json!([{"name":"Variant", "locales":"ja"}])),
        ("treatments", json!([{"name":"Variant", "locales":[null]}])),
        ("treatments", json!([{"name":"Variant", "appIconName":7}])),
        ("treatments", json!([{"name":"Variant", "locale":"ja"}])),
        ("treatments", json!([{"name":""}])),
        ("name", json!(null)),
    ] {
        let mut payload = valid.clone();
        payload[field] = value;
        assert_eq!(
            gateway
                .execute(&request(
                    Platform::Ios,
                    Operation::CreateProductPageOptimization,
                    payload
                ))
                .await
                .unwrap_err()
                .status,
            422
        );
    }
    assert!(fake.records().is_empty());
}

#[tokio::test]
async fn malformed_image_selectors_never_reach_upstream_or_delete_existing_images() {
    let fake = Fake::new(|_| ok(json!({}))).await;
    let keys = Keys::new();
    let gateway = Gateway::new(keys.config(&fake.url)).unwrap();
    let valid_url = format!("{}/image.png", fake.url);
    let invalid_images = [
        json!(null),
        json!([]),
        json!({}),
        json!({"unknown_slot":{"urls":[]}}),
        json!({"phone_screenshots":null}),
        json!({"phone_screenshots":{}}),
        json!({"phone_screenshots":{"urls":valid_url}}),
        json!({"phone_screenshots":{"assets":42}}),
        json!({"phone_screenshots":{"urls":null}}),
        json!({"phone_screenshots":{"url":valid_url}}),
        json!({"phone_screenshots":{"urls":[valid_url, 42]}}),
        json!({"phone_screenshots":{"urls":[""]}}),
        json!({"phone_screenshots":{"assets":[42]}}),
        json!({"phone_screenshots":{"assets":[{}]}}),
        json!({"phone_screenshots":{"assets":[{"downloadUrl":42}]}}),
        json!({"phone_screenshots":{"assets":[{"downloadUrl":valid_url,"fileName":42}]}}),
        json!({"phone_screenshots":{"assets":[{"downloadUrl":valid_url,"sizeBytes":"10"}]}}),
        json!({"phone_screenshots":{"assets":[{"downloadUrl":valid_url,"unknown":true}]}}),
    ];
    for platform in [Platform::Ios, Platform::Android] {
        for images in &invalid_images {
            let payload = json!({"operation":"update_app_metadata", "locale":"en-US", "version":"1.0", "syncScopes":["store_images"], "metadata":{"storeImages":images}});
            assert_eq!(
                gateway
                    .execute(&request(platform, Operation::Sync, payload))
                    .await
                    .unwrap_err()
                    .status,
                422
            );
        }
    }
    for images in &invalid_images {
        let payload = json!({"operation":"update_marketing_page", "locale":"en-US", "syncScopes":["store_images"], "marketingPage":{"pageName":"Page", "storeImages":images}});
        assert_eq!(
            gateway
                .execute(&request(Platform::Ios, Operation::Sync, payload))
                .await
                .unwrap_err()
                .status,
            422
        );
    }
    assert!(fake.records().is_empty());
}

#[tokio::test]
async fn explicit_empty_image_arrays_clear_only_the_selected_slot() {
    let fake = Fake::new(|r| {
        if r.uri.contains("/placements?") {
            return ok(apple_image_placements(&[
                ("old-phone", "phone-profile"),
                ("old-tablet", "tablet-profile"),
                ("old-duo", "duo-inner-profile"),
            ]));
        }
        apple_base(r)
            .or_else(|| google_base(r))
            .unwrap_or_else(|| ok(json!({})))
    })
    .await;
    let keys = Keys::new();
    let gateway = Gateway::new(keys.config(&fake.url)).unwrap();
    let payload = json!({"operation":"update_app_metadata", "locale":"en-US", "version":"1.0", "syncScopes":["store_images"], "metadata":{"storeImages":{"phone_screenshots":{"urls":[]}}}});
    for platform in [Platform::Ios, Platform::Android] {
        assert_eq!(
            gateway
                .execute(&request(platform, Operation::Sync, payload.clone()))
                .await
                .unwrap()["status"],
            "succeeded"
        );
    }
    let records = fake.records();
    assert!(records.iter().any(|r| r.method == "DELETE"
        && r.uri == "/v1/appAssetLibraryPlacements/placement%2Dold%2Dphone"));
    assert!(!records.iter().any(|r| r.uri.contains("old%2Dtablet")));
    assert_eq!(
        records
            .iter()
            .filter(|r| r.method == "DELETE" && r.uri.ends_with("/phoneScreenshots"))
            .count(),
        1
    );
    assert!(!records.iter().any(|r| r.method == "DELETE"
        && (r.uri.ends_with("/sevenInchScreenshots")
            || r.uri.ends_with("/tenInchScreenshots")
            || r.uri.ends_with("/featureGraphic"))));
    assert!(!records.iter().any(|r| r.uri.starts_with("/upload/")
        || (r.method == "POST" && r.uri == "/v1/appAssetLibraryImages")));
    assert!(records.iter().any(|r| r.uri.contains(":commit?")
        && r.uri.contains("changesInReviewBehavior=ERROR_IF_IN_REVIEW")
        && !r.uri.contains("changesNotSentForReview")));
}

#[tokio::test]
async fn uploaded_asset_reaches_google_multipart_and_is_released_after_sync() {
    use sha2::{Digest, Sha256};
    use store_protocol::upload::{self, UploadClaims, UploadInput};
    let image = png();
    let fake = Fake::new(|record| google_base(record).unwrap_or_else(|| ok(json!({})))).await;
    let keys = Keys::new();
    let assets = store_provider::assets::AssetStore::new("image-node".into(), "22".repeat(32));
    let time = upload::now().unwrap();
    let claims = UploadClaims {
        iss: "testflying-store-api".into(),
        aud: "connector:image-node".into(),
        iat: time,
        exp: time + 600,
        connector_id: "image-node".into(),
        asset_id: "00000000-0000-4000-8000-000000000001".into(),
        platform: Platform::Android,
        bundle_identifier: "com.example.App".into(),
        file: UploadInput {
            file_name: "local.png".into(),
            content_type: "image/png".into(),
            size_bytes: image.len() as u64,
            sha256: hex::encode(Sha256::digest(&image)),
        },
    };
    let token = upload::sign(&claims, &"22".repeat(32)).unwrap();
    assets.start(&token, &claims.asset_id).unwrap();
    assets
        .chunk(
            Platform::Android,
            "com.example.App",
            &claims.asset_id,
            &claims.asset_id,
            0,
            &image,
        )
        .unwrap();
    assets
        .finish(
            Platform::Android,
            "com.example.App",
            &claims.asset_id,
            &claims.asset_id,
        )
        .unwrap();
    let gateway = Gateway::with_assets(keys.config(&fake.url), Some(assets.clone())).unwrap();
    let result=gateway.execute(&request(Platform::Android,Operation::Sync,json!({"operation":"update_app_metadata","locale":"en-US","syncScopes":["store_images"],"metadata":{"storeImages":{"phone_screenshots":{"assets":[{"assetId":claims.asset_id}]}}}}))).await.unwrap();
    assert_eq!(result["status"], "succeeded");
    assert_eq!(assets.usage(), (0, 0));
    let records = fake.records();
    let uploaded = records
        .iter()
        .find(|r| r.uri.starts_with("/upload/"))
        .unwrap();
    assert!(uploaded
        .bytes
        .windows(image.len())
        .any(|part| part == image));
    assert!(!records
        .iter()
        .any(|r| r.method == "GET" && r.uri.ends_with(".png")));
}

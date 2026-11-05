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
    let invalid_screenshot_limit = record.uri.contains("/appScreenshotSets?")
        && reqwest::Url::parse(&format!("http://fake.test{}", record.uri))
            .unwrap()
            .query_pairs()
            .any(|(key, value)| {
                key == "limit[appScreenshots]" && value.parse::<u64>().unwrap_or(u64::MAX) > 50
            });
    let invalid_locale_creation = invalid_apple_locale_creation(&record);
    let (status, headers, bytes) = if invalid_screenshot_limit || invalid_locale_creation {
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
struct Keys {
    dir: tempfile::TempDir,
    apple_public: String,
    google_public: String,
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
            google_upload_base_url: url.into(),
            google_token_url: format!("{url}/token"),
            ..Default::default()
        }
    }
}
fn apple_base(r: &Record) -> Option<Reply> {
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
    assert!(commit.uri.contains("changesNotSentForReview=true"));
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
                    b"private upstream bearer and key material".to_vec(),
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
async fn image_uploads_reservation_chunks_checksum_completion_and_multipart() {
    let image = png();
    let expected = image.clone();
    let port = Arc::new(Mutex::new(String::new()));
    let upload_base = port.clone();
    let fake=Fake::new(move|r|{
        if r.uri=="/image.png"{return(StatusCode::OK,vec![("content-type","image/png".into())],image.clone());}
        if r.uri.contains("/appScreenshotSets?"){return ok(json!({"data":[{"id":"set","attributes":{"screenshotDisplayType":"APP_IPHONE_67"},"relationships":{"appScreenshots":{"data":[{"id":"old"}]}}}]}));}
        if r.uri=="/v1/appScreenshots"&&r.method=="POST"{return ok(json!({"data":{"id":"shot","attributes":{"uploadOperations":[{"method":"PUT","url":format!("{}/chunk",upload_base.lock().unwrap()),"offset":0,"length":image.len(),"requestHeaders":[{"name":"x-upload","value":"chunk-header"}]}]}}}));}
        apple_base(r).or_else(||google_base(r)).unwrap_or_else(||ok(json!({})))
    }).await;
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
        .find(|r| r.uri == "/v1/appScreenshots" && r.method == "POST")
        .unwrap()
        .body();
    assert_eq!(
        reservation["data"]["attributes"]["sourceFileChecksum"],
        format!("{:x}", md5::compute(&expected))
    );
    let chunk = records.iter().find(|r| r.uri == "/chunk").unwrap();
    assert_eq!(chunk.bytes, expected);
    assert!(!chunk.headers.contains_key("authorization"));
    assert_eq!(chunk.headers["x-upload"], "chunk-header");
    assert!(records.iter().any(|r| r.method == "PATCH"
        && r.uri == "/v1/appScreenshots/shot"
        && r.body()["data"]["attributes"]["uploaded"] == true));
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
    let fake=Fake::new(move|r|{
        if r.uri=="/image.png"{return(StatusCode::OK,vec![],image.clone());}
        if r.uri.contains("/appScreenshotSets?"){return ok(json!({"data":[{"id":"set","attributes":{"screenshotDisplayType":"APP_IPHONE_67"}}]}));}
        if r.uri=="/v1/appScreenshots"&&r.method=="POST"{return ok(json!({"data":{"id":"shot","attributes":{"uploadOperations":[{"method":"PUT","url":"http://127.0.0.1:1/never","offset":0,"length":999999}]}}}));}
        apple_base(r).unwrap_or_else(||ok(json!({})))
    }).await;
    let keys = Keys::new();
    let gateway = Gateway::new(keys.config(&fake.url)).unwrap();
    assert!(gateway.execute(&request(Platform::Ios,Operation::Sync,json!({"operation":"update_app_metadata","version":"1.0","locale":"en-US","syncScopes":["store_images"],"metadata":{"storeImages":{"phone_screenshots":{"urls":[format!("{}/image.png",fake.url)]}}}}))).await.is_err());
    assert!(!fake
        .records()
        .iter()
        .any(|r| r.method == "PATCH" && r.uri == "/v1/appScreenshots/shot"));
}

#[tokio::test]
async fn apple_adaptive_rate_limit_and_identity_mismatch_fail_closed() {
    let fake = Fake::new(|r| {
        let mut reply = apple_base(r).unwrap_or_else(|| ok(json!({})));
        reply
            .1
            .push(("x-rate-limit", "user-hour-lim:2;user-hour-rem:1;".into()));
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
    assert_eq!(fake.records().len(), 1);
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
async fn apple_version_and_cpp_screenshot_queries_use_vendor_limit_and_cpp_locale_schema() {
    let image = png();
    let endpoint = Arc::new(Mutex::new(String::new()));
    let upstream = endpoint.clone();
    let fake = Fake::new(move |r| {
        if r.uri == "/image.png" { return (StatusCode::OK, vec![], image.clone()); }
        if r.uri == "/v1/appCustomProductPages/page/app" { return ok(json!({"data":{"id":"resolved-app"}})); }
        if r.uri.contains("/appCustomProductPageVersions?") { return ok(json!({"data":[{"id":"cpp-version"}]})); }
        if r.uri.contains("/appCustomProductPageLocalizations?") { return ok(json!({"data":[]})); }
        if r.method == "POST" && r.uri == "/v1/appCustomProductPageLocalizations" { return ok(json!({"data":{"id":"cpp-loc"}})); }
        if r.uri.contains("/appScreenshotSets?") { return ok(json!({"data":[]})); }
        if r.method == "POST" && r.uri == "/v1/appScreenshotSets" { return ok(json!({"data":{"id":"cpp-set"}})); }
        if r.method == "POST" && r.uri == "/v1/appScreenshots" {
            return ok(json!({"data":{"id":"cpp-shot","attributes":{"uploadOperations":[{"method":"PUT","url":format!("{}/chunk",upstream.lock().unwrap()),"offset":0,"length":image.len()}]}}}));
        }
        apple_base(r).unwrap_or_else(|| ok(json!({})))
    }).await;
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
    gateway.execute(&request(Platform::Ios, Operation::Sync, json!({
        "operation":"update_marketing_page", "locale":"fr-FR", "syncScopes":["store_images"],
        "marketingPage":{"pageName":"Page", "applePageId":"page", "storeImages":{"phone_screenshots":{"urls":[format!("{}/image.png",fake.url)]}}}
    }))).await.unwrap();
    let records = fake.records();
    for owner in [
        "appStoreVersionLocalizations",
        "appCustomProductPageLocalizations",
    ] {
        let record = records
            .iter()
            .find(|r| {
                r.uri.starts_with(&format!("/v1/{owner}/")) && r.uri.contains("/appScreenshotSets?")
            })
            .unwrap();
        assert!(reqwest::Url::parse(&format!("{}{}", fake.url, record.uri))
            .unwrap()
            .query_pairs()
            .any(|(key, value)| key == "limit[appScreenshots]" && value == "50"));
    }
    let localization = records
        .iter()
        .find(|r| r.method == "POST" && r.uri == "/v1/appCustomProductPageLocalizations")
        .unwrap()
        .body();
    assert_eq!(
        localization["data"]["attributes"],
        json!({"locale":"fr-FR"})
    );
    let set = records
        .iter()
        .find(|r| r.method == "POST" && r.uri == "/v1/appScreenshotSets")
        .unwrap()
        .body();
    assert_eq!(
        set["data"]["relationships"]["appCustomProductPageLocalization"]["data"]["type"],
        "appCustomProductPageLocalizations"
    );
    assert_eq!(
        set["data"]["relationships"]["appCustomProductPageLocalization"]["data"]["id"],
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
            ("version", json!(8)),
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
        json!({"operation":"update_app_metadata", "locale":"en-US", "metadata":{"title":"Selected"}}),
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
        if r.uri.contains("/appScreenshotSets?") {
            return ok(json!({"data":[
                {"id":"phone-set", "attributes":{"screenshotDisplayType":"APP_IPHONE_67"}, "relationships":{"appScreenshots":{"data":[{"id":"old-phone"}]}}},
                {"id":"tablet-set", "attributes":{"screenshotDisplayType":"APP_IPAD_PRO_3GEN_129"}, "relationships":{"appScreenshots":{"data":[{"id":"old-tablet"}]}}}
            ]}));
        }
        apple_base(r).or_else(||google_base(r)).unwrap_or_else(||ok(json!({})))
    }).await;
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
    assert!(records
        .iter()
        .any(|r| r.method == "DELETE" && r.uri == "/v1/appScreenshots/old%2Dphone"));
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
    assert!(!records
        .iter()
        .any(|r| r.uri.starts_with("/upload/")
            || (r.method == "POST" && r.uri == "/v1/appScreenshots")));
    assert!(records
        .iter()
        .any(|r| r.uri.contains(":commit?") && r.uri.contains("changesNotSentForReview=true")));
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

//! Credential-local store implementation shared by both Connector transports.
mod apple;
mod apple_image_upload;
mod apple_images;
pub mod assets;
mod google;
mod image_query;
mod images;
mod reviews;
mod validation;

use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use reqwest::{Client, Method, Response, Url};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, VecDeque},
    fmt,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use store_protocol::{Operation, Platform, StoreError, StoreRequest};
use tokio::sync::Mutex;

type Result<T> = std::result::Result<T, StoreError>;

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProviderConfig {
    pub mode: String,
    pub apple_issuer_id: Option<String>,
    pub apple_key_id: Option<String>,
    pub apple_private_key_path: Option<String>,
    pub google_service_account_path: Option<String>,
    pub apple_base_url: String,
    pub google_base_url: String,
    pub google_upload_base_url: String,
    pub google_token_url: String,
    pub store_proxy_url: Option<String>,
}
impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            mode: "live".into(),
            apple_issuer_id: None,
            apple_key_id: None,
            apple_private_key_path: None,
            google_service_account_path: None,
            apple_base_url: "https://api.appstoreconnect.apple.com".into(),
            google_base_url: "https://androidpublisher.googleapis.com".into(),
            google_upload_base_url: "https://androidpublisher.googleapis.com".into(),
            google_token_url: "https://oauth2.googleapis.com/token".into(),
            store_proxy_url: None,
        }
    }
}
impl fmt::Debug for ProviderConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderConfig")
            .field("mode", &self.mode)
            .field("apple_configured", &self.apple_private_key_path.is_some())
            .field(
                "google_configured",
                &self.google_service_account_path.is_some(),
            )
            .finish_non_exhaustive()
    }
}
#[derive(Clone)]
pub struct Gateway {
    inner: Arc<Inner>,
}
struct Inner {
    assets: Option<Arc<assets::AssetStore>>,
    config: ProviderConfig,
    client: Client,
    apple_key: Option<EncodingKey>,
    google: Option<GoogleAccount>,
    apple_token: Mutex<Option<(String, u64)>>,
    google_token: Mutex<Option<(String, u64)>>,
    limits: Mutex<Limits>,
}
struct GoogleAccount {
    email: String,
    key: EncodingKey,
}
struct Limits {
    events: HashMap<String, VecDeque<Instant>>,
    apple_max: usize,
    apple_window: Duration,
}

impl Gateway {
    pub fn new(config: ProviderConfig) -> Result<Self> {
        Self::with_assets(config, None)
    }

    pub fn with_assets(
        config: ProviderConfig,
        assets: Option<Arc<assets::AssetStore>>,
    ) -> Result<Self> {
        if !matches!(config.mode.as_str(), "live" | "mock") {
            return Err(invalid("mode must be live or mock"));
        }
        for endpoint in [
            &config.apple_base_url,
            &config.google_base_url,
            &config.google_upload_base_url,
            &config.google_token_url,
        ] {
            let url = Url::parse(endpoint).map_err(|_| invalid("Invalid upstream endpoint"))?;
            if !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return Err(invalid(
                    "Upstream endpoints must be HTTP(S) URLs without credentials",
                ));
            }
        }
        let mut builder = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never());
        if let Some(proxy) = &config.store_proxy_url {
            builder = builder
                .proxy(reqwest::Proxy::all(proxy).map_err(|_| invalid("Invalid store proxy"))?);
        }
        let apple_key = config
            .apple_private_key_path
            .as_ref()
            .map(|path| {
                let raw =
                    std::fs::read(path).map_err(|_| credential("Cannot read Apple private key"))?;
                EncodingKey::from_ec_pem(&raw)
                    .map_err(|_| credential("Apple private key must be P-256 PKCS8 PEM"))
            })
            .transpose()?;
        let google = config
            .google_service_account_path
            .as_ref()
            .map(|path| -> Result<GoogleAccount> {
                let raw = std::fs::read(path)
                    .map_err(|_| credential("Cannot read Google service account"))?;
                let value: Value = serde_json::from_slice(&raw)
                    .map_err(|_| credential("Invalid Google service account JSON"))?;
                let email = s(&value, "client_email");
                if email.is_empty() {
                    return Err(credential("Google service account requires client_email"));
                }
                let key = EncodingKey::from_rsa_pem(s(&value, "private_key").as_bytes())
                    .map_err(|_| credential("Google private key must be RSA PEM"))?;
                Ok(GoogleAccount {
                    email: email.into(),
                    key,
                })
            })
            .transpose()?;
        let client = builder
            .build()
            .map_err(|_| invalid("Cannot configure store HTTP client"))?;
        Ok(Self {
            inner: Arc::new(Inner {
                assets,
                config,
                client,
                apple_key,
                google,
                apple_token: Mutex::new(None),
                google_token: Mutex::new(None),
                limits: Mutex::new(Limits {
                    events: HashMap::new(),
                    apple_max: 2880,
                    apple_window: Duration::from_secs(3600),
                }),
            }),
        })
    }
    pub async fn execute(&self, request: &StoreRequest) -> Result<Value> {
        request.validate()?;
        validation::validate(request)?;
        let uses_assets =
            request.operation == Operation::Sync && scopes(&request.payload, "store_images", false);
        let _assets = if uses_assets {
            if let Some(store) = &self.inner.assets {
                Some(store.claim(request)?)
            } else {
                if !assets::asset_ids(&request.payload).is_empty() {
                    return Err(invalid(
                        "Uploaded assets are not available on this Connector",
                    ));
                }
                None
            }
        } else {
            None
        };
        if self.inner.config.mode == "mock" {
            return self.mock(request);
        }
        let filters = if request.operation == Operation::StoreReviews {
            Some(reviews::Filters::parse(&request.payload)?)
        } else {
            None
        };
        let mut response = match request.platform {
            Platform::Ios => self.apple_execute(request).await,
            Platform::Android => self.google_execute(request).await,
        }?;
        if let Some(filters) = filters {
            filters.apply(&mut response);
        }
        Ok(response)
    }
    fn mock(&self, r: &StoreRequest) -> Result<Value> {
        Ok(match r.operation {
            Operation::Preflight => preflight(true, None, "mock", true),
            Operation::SupportedLocales => json!({"locales":["en-US", "zh-Hans"]}),
            Operation::StoreListings => json!({"listings":[]}),
            Operation::StoreImages => json!({"locales":[]}),
            Operation::StoreReleases => json!({"releases":[]}),
            Operation::StoreReviews => json!({"reviews":[]}),
            Operation::ProductPageOptimizations => json!({"experiments":[]}),
            Operation::CreateProductPageOptimization => {
                json!({"experiment":{"id":"mock-experiment","name":r.payload["name"],"treatments":[]}})
            }
            Operation::Sync => succeeded(json!({"mock":true})),
            Operation::UploadStart
            | Operation::UploadChunk
            | Operation::UploadFinish
            | Operation::UploadAbort => {
                return Err(invalid("Uploads must be handled by the Connector"))
            }
        })
    }
    async fn limit(&self, platform: Platform) -> Result<()> {
        let mut limits = self.inner.limits.lock().await;
        let (key, max, window) = match platform {
            Platform::Ios => ("apple", limits.apple_max, limits.apple_window),
            Platform::Android => ("google", 200, Duration::from_secs(60)),
        };
        let now = Instant::now();
        let events = limits.events.entry(key.into()).or_default();
        while events
            .front()
            .is_some_and(|at| now.duration_since(*at) >= window)
        {
            events.pop_front();
        }
        if events.len() >= max {
            return Err(StoreError::new(
                429,
                "store_rate_limited",
                "Store API rate limit reached; retry later",
            ));
        }
        events.push_back(now);
        Ok(())
    }
    async fn token(&self, platform: Platform) -> Result<String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| credential("Invalid system clock"))?
            .as_secs();
        let cache = if platform == Platform::Ios {
            &self.inner.apple_token
        } else {
            &self.inner.google_token
        };
        let mut cached = cache.lock().await;
        if let Some((token, expires)) = &*cached {
            if now + 60 < *expires {
                return Ok(token.clone());
            }
        }
        let (token, expiry) = if platform == Platform::Ios {
            let key = self
                .inner
                .apple_key
                .as_ref()
                .ok_or_else(|| credential("Apple credentials are not configured"))?;
            let issuer = self
                .inner
                .config
                .apple_issuer_id
                .as_deref()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| credential("Apple issuer ID is required"))?;
            let kid = self
                .inner
                .config
                .apple_key_id
                .clone()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| credential("Apple key ID is required"))?;
            let mut header = Header::new(Algorithm::ES256);
            header.kid = Some(kid);
            (
                encode(
                    &header,
                    &json!({"iss":issuer,"iat":now,"exp":now+1200,"aud":"appstoreconnect-v1"}),
                    key,
                )
                .map_err(|_| credential("Cannot sign Apple token"))?,
                now + 1200,
            )
        } else {
            let account = self
                .inner
                .google
                .as_ref()
                .ok_or_else(|| credential("Google credentials are not configured"))?;
            let assertion = encode(&Header::new(Algorithm::RS256), &json!({"iss":account.email,
                "scope":"https://www.googleapis.com/auth/androidpublisher", "aud":self.inner.config.google_token_url,
                "iat":now,"exp":now+3600}), &account.key).map_err(|_| credential("Cannot sign Google assertion"))?;
            let response = self
                .inner
                .client
                .post(&self.inner.config.google_token_url)
                .form(&[
                    ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                    ("assertion", &assertion),
                ])
                .send()
                .await
                .map_err(|_| upstream("Google token request failed"))?;
            let value = bounded_json(response).await?;
            let token = s(&value, "access_token");
            if token.is_empty() {
                return Err(upstream("Google token response omitted access token"));
            }
            (
                token.to_owned(),
                now.saturating_add(
                    value["expires_in"]
                        .as_u64()
                        .filter(|n| *n > 0)
                        .unwrap_or(3600)
                        .min(86400),
                ),
            )
        };
        *cached = Some((token.clone(), expiry));
        Ok(token)
    }
    async fn api(
        &self,
        platform: Platform,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        self.limit(platform).await?;
        let token = self.token(platform).await?;
        let base = match platform {
            Platform::Ios => &self.inner.config.apple_base_url,
            Platform::Android => &self.inner.config.google_base_url,
        };
        let mut request = self
            .inner
            .client
            .request(method, format!("{}{}", base.trim_end_matches('/'), path))
            .bearer_auth(token);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request
            .send()
            .await
            .map_err(|_| upstream("Store request failed"))?;
        if platform == Platform::Ios {
            if let Some(value) = response
                .headers()
                .get("x-rate-limit")
                .and_then(|v| v.to_str().ok())
            {
                for field in value.split(';') {
                    if let Some(("user-hour-lim", number)) = field
                        .trim()
                        .split_once(':')
                        .map(|(k, v)| (k.trim(), v.trim()))
                    {
                        if let Ok(number) = number.parse::<usize>() {
                            let mut limits = self.inner.limits.lock().await;
                            limits.apple_max = (number.saturating_mul(8) / 10).max(1);
                            limits.apple_window = Duration::from_secs(3600);
                        }
                    }
                }
            }
        }
        bounded_json(response).await
    }
}
async fn bounded_json(mut response: Response) -> Result<Value> {
    let status = response.status();
    // Never include upstream bodies, URLs or credentials in errors.
    if !status.is_success() {
        return Err(StoreError::new(
            if status.as_u16() == 429 { 429 } else { 502 },
            "store_upstream_error",
            format!("Store upstream returned HTTP {}", status.as_u16()),
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| upstream("Cannot read store response"))?
    {
        if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
            return Err(upstream("Store response exceeded size limit"));
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&bytes).map_err(|_| upstream("Invalid store JSON response"))
}
fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}
fn arr<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}
fn required<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| invalid(format!("{key} is required")))
}
fn id(v: &Value) -> Result<&str> {
    s(v, "id")
        .is_empty()
        .then(|| upstream("Store response omitted resource ID"))
        .map_or(Ok(s(v, "id")), Err)
}
fn escape(value: &str) -> String {
    percent_encoding::utf8_percent_encode(value, percent_encoding::NON_ALPHANUMERIC).to_string()
}
fn query(path: &str, pairs: &[(&str, &str)]) -> String {
    let mut u = Url::parse("http://placeholder.invalid").expect("static URL");
    u.query_pairs_mut().extend_pairs(pairs.iter().copied());
    format!("{path}?{}", u.query().unwrap_or(""))
}
fn invalid(message: impl Into<String>) -> StoreError {
    StoreError::new(422, "invalid_store_payload", message)
}
fn credential(message: impl Into<String>) -> StoreError {
    StoreError::new(503, "store_credentials_unavailable", message)
}
fn upstream(message: impl Into<String>) -> StoreError {
    StoreError::new(502, "store_upstream_error", message)
}
fn preflight(editable: bool, reason: Option<&str>, state: &str, exists: bool) -> Value {
    json!({"canSync":editable,"reasonCode":reason,"message":if editable {"Store content is editable"} else {"Store content is not editable"},
        "storeState":{"versionExists":exists,"editable":editable,"currentStatus":state}})
}
fn succeeded(state: Value) -> Value {
    json!({"status":"succeeded","message":"Store content synchronized","errorCode":null,"errorSummary":null,"storeState":state})
}
fn failed(code: &str, message: &str) -> Value {
    json!({"status":"failed","message":message,"errorCode":code,"errorSummary":message})
}
fn scopes(p: &Value, name: &str, all_text: bool) -> bool {
    let scopes = arr(p, "syncScopes");
    scopes.iter().any(|v| v.as_str() == Some(name))
        || (all_text
            && (scopes.is_empty() || scopes.iter().any(|v| v.as_str() == Some("metadata"))))
}
fn copy_field(
    out: &mut serde_json::Map<String, Value>,
    target: &str,
    source: &Value,
    keys: &[&str],
) -> Result<()> {
    for key in keys {
        if let Some(value) = source.get(key) {
            if !value.is_string() {
                return Err(invalid(format!("{key} must be a string")));
            }
            out.insert(target.into(), value.clone());
            break;
        }
    }
    Ok(())
}

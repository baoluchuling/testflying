use super::*;
use axum::{extract::Request, Extension};
use futures_util::StreamExt;
use registry::Integration;
use reqwest::{multipart, Client, Url};
use std::io::{Cursor, Write};
use tokio::sync::Semaphore;
use zip::{write::SimpleFileOptions, ZipWriter};

mod delivery;
pub mod notification;
pub(crate) mod oss;

const MAX_IMAGE: usize = 16 * 1024 * 1024;
const MAX_REPLY: usize = 64 * 1024;
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";
const ARM64_MACHO_HEADER: &[u8] = b"\xcf\xfa\xed\xfe\x0c\0\0\x01";

pub struct ScreenshotUploader {
    url: Url,
    token: Option<header::HeaderValue>,
    client: Client,
    transfers: Semaphore,
}

impl ScreenshotUploader {
    pub fn from_environment() -> std::result::Result<Option<Self>, StoreError> {
        let Some(url) =
            std::env::var_os("STORE_API_SCREENSHOT_UPLOAD_URL").filter(|url| !url.is_empty())
        else {
            return Ok(None);
        };
        let url = url.into_string().map_err(|_| invalid_upstream())?;
        let token = std::env::var_os("STORE_API_SCREENSHOT_UPLOAD_TOKEN")
            .map(|token| token.into_string().map_err(|_| invalid_upstream()))
            .transpose()?;
        Self::new(&url, token.as_deref()).map(Some)
    }

    pub fn new(url: &str, token: Option<&str>) -> std::result::Result<Self, StoreError> {
        let url = Url::parse(url).map_err(|_| invalid_upstream())?;
        let host = url.host_str().ok_or_else(invalid_upstream)?;
        let local = matches!(host, "localhost" | "127.0.0.1" | "[::1]");
        if !(url.scheme() == "https" || (url.scheme() == "http" && local))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(invalid_upstream());
        }
        let token = token
            .map(|token| {
                if token.is_empty() || !token.bytes().all(|byte| byte.is_ascii_graphic()) {
                    return Err(invalid_upstream());
                }
                let mut header = header::HeaderValue::from_str(&format!("Bearer {token}"))
                    .map_err(|_| invalid_upstream())?;
                header.set_sensitive(true);
                Ok(header)
            })
            .transpose()?;
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(Duration::from_secs(45))
            .build()
            .map_err(|_| invalid_upstream())?;
        Ok(Self {
            url,
            token,
            client,
            transfers: Semaphore::new(4),
        })
    }

    async fn send(&self, job: &str, bytes: Vec<u8>) -> std::result::Result<Value, StoreError> {
        let file = multipart::Part::bytes(bytes)
            .file_name("screenshot.png")
            .mime_str("image/png")
            .expect("valid media type");
        let mut request = self
            .client
            .post(self.url.clone())
            .header("Idempotency-Key", job)
            .multipart(multipart::Form::new().part("file", file));
        if let Some(token) = &self.token {
            request = request.header(header::AUTHORIZATION, token);
        }
        let mut response = request.send().await.map_err(delivery_error)?;
        if !response.status().is_success() {
            return Err(StoreError::new(
                502,
                "upstream_rejected",
                "Screenshot upload was rejected",
            ));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(delivery_error)? {
            if chunk.len() > MAX_REPLY - body.len() {
                return Err(StoreError::new(
                    502,
                    "invalid_upstream_reply",
                    "Screenshot upload reply is too large",
                ));
            }
            body.extend_from_slice(&chunk);
        }
        if body.is_empty() {
            return Ok(Value::Null);
        }
        let reply: Value = serde_json::from_slice(&body).map_err(|_| {
            StoreError::new(
                502,
                "invalid_upstream_reply",
                "Screenshot upload reply must be JSON",
            )
        })?;
        let image_url = reply
            .get("imageUrl")
            .and_then(Value::as_str)
            .filter(|value| {
                value.len() <= 2048
                    && Url::parse(value).is_ok_and(|url| {
                        url.scheme() == "https"
                            && url.host_str().is_some()
                            && url.username().is_empty()
                            && url.password().is_none()
                    })
            });
        Ok(image_url.map_or(Value::Null, |url| json!({"imageUrl":url})))
    }
}

fn invalid_upstream() -> StoreError {
    StoreError::new(
        500,
        "invalid_screenshot_upload_config",
        "Invalid screenshot upload destination or credentials",
    )
}

fn delivery_error(error: reqwest::Error) -> StoreError {
    let status = if error.is_timeout() { 504 } else { 502 };
    StoreError::new(
        status,
        "delivery_unknown",
        "Screenshot delivery could not be confirmed",
    )
}

pub(super) fn routes(
    state: Arc<ApiState>,
    uploader: Option<ScreenshotUploader>,
) -> Router<Arc<ApiState>> {
    let external = Router::new()
        .route("/v1/screenshot-connectors", get(list_nodes))
        .route("/v1/screenshot-jobs", post(submit))
        .route("/v1/screenshot-jobs/{jobId}", get(status))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            authorize_client,
        ));
    let admin = Router::new()
        .route(
            "/v1/screenshot-settings",
            get(crate::screenshot_settings::read).put(crate::screenshot_settings::update),
        )
        .route(
            "/v1/connectors/{id}/screenshot-installer",
            get(screenshot_installer),
        )
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            authorize_admin,
        ));
    let binding = Router::new()
        .route(
            "/v1/apps/{platform}/{identifier}/screenshot-connector",
            get(binding).put(bind).delete(unbind),
        )
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            integrations::authorize_binding,
        ));
    external
        .merge(admin)
        .merge(binding)
        .route("/v1/connectors/{id}/screenshot-jobs/claim", post(claim))
        .route(
            "/v1/connectors/{id}/screenshot-jobs/{job}/upload-ticket",
            post(delivery::prepare),
        )
        .route(
            "/v1/connectors/{id}/screenshot-jobs/{job}/complete",
            post(delivery::complete),
        )
        .route(
            "/v1/connectors/{id}/screenshot-jobs/{job}/failure",
            post(fail),
        )
        .route(
            "/v1/connectors/{id}/screenshot-jobs/{job}/image",
            put(image),
        )
        .route_layer(middleware::from_fn(integrations::authorize_https))
        .route_layer(middleware::map_response(|response| async {
            integrations::no_store(response)
        }))
        .layer(Extension(uploader.map(Arc::new)))
}

pub(super) async fn screenshot_installer(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<String>,
) -> Result<Response> {
    let node = state.registry.get(&id)?;
    if node.kind != ConnectorKind::Screenshot || !node.enabled {
        return Err(StoreError::new(
            422,
            "invalid_screenshot_connector",
            "Select an enabled Screenshot Connector",
        )
        .into());
    }
    let config = super::config(&state, &node);
    let config = serde_json::to_vec_pretty(&config).expect("Connector config serialization");
    let executable = state.release_root.join("darwin-arm64/screenshot-connector");
    let metadata = tokio::fs::metadata(&executable)
        .await
        .map_err(|_| release_unavailable())?;
    if !metadata.is_file() || metadata.len() > 64 * 1024 * 1024 {
        return Err(release_unavailable().into());
    }
    let executable = tokio::fs::read(executable)
        .await
        .map_err(|_| release_unavailable())?;
    if !executable.starts_with(ARM64_MACHO_HEADER) {
        return Err(release_unavailable().into());
    }
    let bytes = installer_archive(&config, &executable).map_err(|_| {
        StoreError::new(
            500,
            "installer_error",
            "Cannot package Screenshot Connector",
        )
    })?;
    let headers = [
        (header::CONTENT_TYPE, "application/zip"),
        (
            header::CONTENT_DISPOSITION,
            "attachment; filename=\"screenshot-connector-macos.zip\"",
        ),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
    ];
    Ok((headers, bytes).into_response())
}

fn release_unavailable() -> StoreError {
    StoreError::new(
        503,
        "release_unavailable",
        "macOS Screenshot Connector has not been published",
    )
}

fn installer_archive(config: &[u8], executable: &[u8]) -> zip::result::ZipResult<Vec<u8>> {
    let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
    let files: [(&str, &[u8], u32); 6] = [
        ("connector.json", config, 0o600),
        ("screenshot-connector", executable, 0o700),
        (
            "install.sh",
            include_bytes!("../../../packaging/screenshot-connector/install.sh"),
            0o700,
        ),
        (
            "run.sh",
            include_bytes!("../../../packaging/screenshot-connector/run.sh"),
            0o700,
        ),
        (
            "README.md",
            include_bytes!("../../../packaging/screenshot-connector/README.md"),
            0o600,
        ),
        (
            "testflying-prod-ca.crt",
            include_bytes!("../../../deploy/testflying-prod-ca.crt"),
            0o644,
        ),
    ];
    for (name, data, mode) in files {
        archive.start_file(name, SimpleFileOptions::default().unix_permissions(mode))?;
        archive.write_all(data)?;
    }
    Ok(archive.finish()?.into_inner())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Submit {
    target: Vec<store_protocol::screenshot::ScreenshotTarget>,
    platform: Platform,
    bundle_identifier: String,
    profile: Option<String>,
    prompt: String,
}

fn require_integration(access: Option<Extension<Integration>>) -> Result<Integration> {
    access.map(|Extension(access)| access).ok_or_else(|| {
        StoreError::new(
            403,
            "integration_token_required",
            "Screenshot jobs require an integration token",
        )
        .into()
    })
}

async fn submit(
    State(state): State<Arc<ApiState>>,
    access: Option<Extension<Integration>>,
    Json(input): Json<Submit>,
) -> Result<(StatusCode, Json<Value>)> {
    let access = require_integration(access)?;
    if input.profile.as_ref().is_some_and(|profile| {
        profile.is_empty()
            || profile.len() > 64
            || !profile
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    }) || input.prompt.trim().is_empty()
        || input.prompt.len() > 16 * 1024
    {
        return Err(StoreError::new(
            422,
            "invalid_screenshot_job",
            "Provide a prompt of at most 16 KiB and an optional valid profile",
        )
        .into());
    }
    store_protocol::screenshot::validate_targets(&input.target)?;
    let job = state.registry.enqueue_screenshot_v2(
        &access,
        input.platform,
        &input.bundle_identifier,
        input.profile.as_deref().unwrap_or_default(),
        &input.prompt,
        &input.target,
    )?;
    state.transport.notify_screenshot(
        job["connectorId"]
            .as_str()
            .expect("validated screenshot routing"),
    );
    Ok((StatusCode::ACCEPTED, Json(job)))
}

async fn list_nodes(
    State(state): State<Arc<ApiState>>,
    access: Option<Extension<Integration>>,
) -> Result<Json<Value>> {
    require_integration(access)?;
    let nodes = state
        .registry
        .list()?
        .into_iter()
        .filter(|node| node.kind == ConnectorKind::Screenshot && node.enabled)
        .map(|node| node_view(&state, node))
        .collect::<Vec<_>>();
    Ok(Json(json!({"connectors":nodes})))
}
async fn bind(
    State(state): State<Arc<ApiState>>,
    Path((platform, identifier)): Path<(Platform, String)>,
    Json(input): Json<BindInput>,
) -> Result<Json<Value>> {
    let node = state
        .registry
        .bind_screenshot(platform, &identifier, &input.connector_id)?;
    Ok(Json(
        json!({"platform":platform,"bundleIdentifier":identifier,"connector":node_view(&state,node)}),
    ))
}
async fn binding(
    State(state): State<Arc<ApiState>>,
    Path((platform, identifier)): Path<(Platform, String)>,
) -> Result<Json<Value>> {
    let node = state.registry.screenshot_binding(platform, &identifier)?;
    Ok(Json(
        json!({"platform":platform,"bundleIdentifier":identifier,"connector":node_view(&state,node)}),
    ))
}
async fn unbind(
    State(state): State<Arc<ApiState>>,
    Path((platform, identifier)): Path<(Platform, String)>,
) -> Result<StatusCode> {
    state.registry.unbind_screenshot(platform, &identifier)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn status(
    State(state): State<Arc<ApiState>>,
    access: Option<Extension<Integration>>,
    Path(job_id): Path<String>,
) -> Result<Json<Value>> {
    Ok(Json(state.registry.screenshot_status(
        &require_integration(access)?,
        &job_id,
    )?))
}

fn authorize_worker(state: &ApiState, id: &str, headers: &HeaderMap) -> Result<()> {
    let node = state.registry.authenticate(id, bearer(headers))?;
    if node.kind != registry::ConnectorKind::Screenshot {
        return Err(StoreError::new(
            403,
            "invalid_connector_kind",
            "Screenshot Connector required",
        )
        .into());
    }
    Ok(())
}

async fn claim(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Response> {
    authorize_worker(&state, &id, &headers)?;
    Ok(
        match state.registry.claim_screenshot_protocol(
            &id,
            headers
                .get("x-screenshot-protocol")
                .is_some_and(|v| v == "3"),
        )? {
            Some(job) => Json(job).into_response(),
            None => StatusCode::NO_CONTENT.into_response(),
        },
    )
}

fn lease(headers: &HeaderMap) -> Result<&str> {
    headers
        .get("x-screenshot-lease")
        .and_then(|v| v.to_str().ok())
        .filter(|v| v.len() == 36 && Uuid::parse_str(v).is_ok())
        .ok_or_else(|| {
            StoreError::new(
                422,
                "invalid_job_lease",
                "A valid screenshot job lease is required",
            )
            .into()
        })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Failure {
    code: String,
}

async fn fail(
    State(state): State<Arc<ApiState>>,
    Path((id, job)): Path<(String, String)>,
    headers: HeaderMap,
    Json(failure): Json<Failure>,
) -> Result<StatusCode> {
    authorize_worker(&state, &id, &headers)?;
    if !matches!(
        failure.code.as_str(),
        "profile_unavailable"
            | "codex_failed"
            | "selection_invalid"
            | "capture_failed"
            | "screenshot_oss_not_configured"
            | "screenshot_upload_rejected"
            | "screenshot_object_mismatch"
            | "screenshot_oss_config_changed"
            | "delivery_unknown"
            | "invalid_upload_ticket"
    ) {
        return Err(
            StoreError::new(422, "invalid_failure", "Unknown screenshot failure code").into(),
        );
    }
    state
        .registry
        .finish_screenshot(&id, &job, lease(&headers)?, Err(&failure.code), "running")?;
    Ok(StatusCode::NO_CONTENT)
}

async fn image(
    State(state): State<Arc<ApiState>>,
    Path((id, job)): Path<(String, String)>,
    Extension(uploader): Extension<Option<Arc<ScreenshotUploader>>>,
    request: Request,
) -> Result<StatusCode> {
    authorize_worker(&state, &id, request.headers())?;
    let lease = lease(request.headers())?.to_owned();
    // The legacy proxy accepts one image only; structured tasks require indexed OSS tickets.
    if state.registry.screenshot_target_index(&job, None).is_err() {
        return Err(StoreError::new(
            409,
            "screenshot_connector_upgrade_required",
            "Structured tasks require indexed OSS upload tickets",
        )
        .into());
    }
    if request
        .headers()
        .get(header::CONTENT_TYPE)
        .is_none_or(|v| v != "image/png")
    {
        return Err(StoreError::new(415, "invalid_image_type", "Send a PNG image").into());
    }
    if state.registry.screenshot_settings()?.oss.enabled {
        let error = StoreError::new(
            409,
            "screenshot_connector_upgrade_required",
            "Upgrade the Screenshot Connector to upload directly to OSS",
        );
        state.registry.begin_screenshot_upload(&id, &job, &lease)?;
        state
            .registry
            .finish_screenshot(&id, &job, &lease, Err(&error.code), "uploading")?;
        return Err(error.into());
    }
    let Some(uploader) = uploader else {
        let error = StoreError::new(
            503,
            "screenshot_upload_unconfigured",
            "Screenshot upload destination is unavailable",
        );
        state.registry.begin_screenshot_upload(&id, &job, &lease)?;
        state
            .registry
            .finish_screenshot(&id, &job, &lease, Err(&error.code), "uploading")?;
        return Err(error.into());
    };
    let _slot = uploader.transfers.try_acquire().map_err(|_| {
        StoreError::new(
            503,
            "screenshot_upload_busy",
            "Screenshot upload capacity reached",
        )
    })?;
    let mut stream = request.into_body().into_data_stream();
    let bytes = tokio::time::timeout(Duration::from_secs(45), async {
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| {
                StoreError::new(400, "invalid_image", "Screenshot upload was interrupted")
            })?;
            if chunk.len() > MAX_IMAGE - bytes.len() {
                return Err(StoreError::new(
                    413,
                    "image_too_large",
                    "Screenshot exceeds 16 MiB",
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        if !bytes.starts_with(PNG) || bytes.len() < 33 {
            return Err(StoreError::new(
                422,
                "invalid_image",
                "Invalid PNG screenshot",
            ));
        }
        Ok(bytes)
    })
    .await
    .map_err(|_| StoreError::new(408, "image_timeout", "Screenshot upload timed out"))??;
    state.registry.begin_screenshot_upload(&id, &job, &lease)?;
    let result = uploader.send(&job, bytes).await;
    state.registry.finish_screenshot(
        &id,
        &job,
        &lease,
        result.as_ref().map_err(|error| error.code.as_str()),
        "uploading",
    )?;
    result?;
    Ok(StatusCode::NO_CONTENT)
}

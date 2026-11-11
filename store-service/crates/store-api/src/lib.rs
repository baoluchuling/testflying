mod api_docs;
mod integrations;
pub mod logs;
mod node_logs;
pub mod registry;
mod screenshots;
mod setup;
pub mod transport;
mod uploads;

use axum::{
    extract::{DefaultBodyLimit, Path, Query, State, WebSocketUpgrade},
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Extension, Json, Router,
};
use registry::{Connector, ConnectorInput, ConnectorKind, Registry};
pub use screenshots::ScreenshotUploader;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};
use store_protocol::{Operation, Platform, StoreError, StoreRequest};
use transport::Transport;
use uuid::Uuid;

pub struct ApiError(pub StoreError);
impl From<StoreError> for ApiError {
    fn from(value: StoreError) -> Self {
        Self(value)
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = self.0.body();
        let summary = logs::error_summary(&self.0.code, &self.0.message);
        let mut response = (
            StatusCode::from_u16(self.0.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            Json(body.clone()),
        )
            .into_response();
        response.extensions_mut().insert(summary);
        response.extensions_mut().insert(logs::ErrorBody(body));
        response
    }
}
type Result<T> = std::result::Result<T, ApiError>;

pub struct ApiState {
    pub registry: Registry,
    pub transport: Arc<Transport>,
    admin_token: String,
    setup_sessions: setup::Sessions,
    api_token: String,
    public_url: String,
    pub release_root: PathBuf,
}

impl ApiState {
    pub fn new(
        registry: Registry,
        admin_token: String,
        api_token: String,
        public_url: String,
        release_root: PathBuf,
        timeout: Duration,
    ) -> std::result::Result<Arc<Self>, StoreError> {
        if admin_token.len() < 16 || api_token.len() < 16 {
            return Err(StoreError::new(
                500,
                "invalid_api_token",
                "API and administrator tokens must contain at least 16 bytes",
            ));
        }
        if registry::secret_eq(&admin_token, &api_token) {
            return Err(StoreError::new(
                500,
                "invalid_api_token",
                "Administrator and application tokens must be distinct",
            ));
        }
        let parsed = reqwest::Url::parse(&public_url).map_err(|_| {
            StoreError::new(500, "invalid_public_url", "Invalid Store API public URL")
        })?;
        if !matches!(parsed.scheme(), "http" | "https")
            || parsed.host_str().is_none()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return Err(StoreError::new(
                500,
                "invalid_public_url",
                "Store API public URL must be HTTP(S) without credentials",
            ));
        }
        let transport = Arc::new(Transport::with_logs(timeout, registry.logs.clone())?);
        Ok(Arc::new(Self {
            registry,
            transport,
            admin_token,
            setup_sessions: setup::Sessions::default(),
            api_token,
            public_url: public_url.trim_end_matches('/').to_owned(),
            release_root,
        }))
    }
}

fn bearer(headers: &HeaderMap) -> &str {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .unwrap_or("")
}

async fn authorize_admin(
    State(state): State<Arc<ApiState>>,
    request: axum::extract::Request,
    next: Next,
) -> Result<Response> {
    if !registry::secret_eq(&state.admin_token, bearer(request.headers())) {
        return Err(StoreError::new(401, "unauthorized", "Administrator token required").into());
    }
    let mut response = next.run(request).await;
    response.extensions_mut().insert(logs::Actor {
        kind: "administrator",
        id: None,
    });
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        "no-store".parse().expect("static header"),
    );
    Ok(response)
}
async fn authorize_client(
    State(state): State<Arc<ApiState>>,
    mut request: axum::extract::Request,
    next: Next,
) -> Result<Response> {
    let token = bearer(request.headers());
    let mut actor = logs::Actor {
        kind: "business",
        id: None,
    };
    if !registry::secret_eq(&state.api_token, token) {
        if !token.starts_with(registry::TOKEN_PREFIX) {
            return Err(StoreError::new(401, "unauthorized", "Store API token required").into());
        }
        integrations::require_https(&request)?;
        let access = state.registry.authenticate_integration(token)?;
        actor = logs::Actor {
            kind: "integration",
            id: Some(access.id.clone()),
        };
        request.extensions_mut().insert(access);
    }
    let mut response = integrations::no_store(next.run(request).await);
    response.extensions_mut().insert(actor);
    Ok(response)
}

pub fn router(state: Arc<ApiState>) -> Router {
    router_with_screenshots(state, None)
}

pub fn router_with_screenshots(
    state: Arc<ApiState>,
    uploader: Option<ScreenshotUploader>,
) -> Router {
    let management = Router::new()
        .route("/v1/logs", get(logs::read))
        .route("/v1/connectors/{id}/logs/latest", get(node_logs::latest))
        .route(
            "/v1/connectors",
            get(list_connectors).post(create_connector),
        )
        .route(
            "/v1/connectors/{id}",
            get(get_connector).patch(update_connector),
        )
        .route("/v1/connectors/{id}/rotate-token", post(rotate_token))
        .route("/v1/connectors/{id}/installation", get(installation_config))
        .route("/v1/connectors/{id}/installer/{target}", get(installer))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            authorize_admin,
        ));
    let binding = get(app_connector)
        .delete(unbind_connector)
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            authorize_admin,
        ))
        .merge(
            put(bind_connector).route_layer(middleware::from_fn_with_state(
                state.clone(),
                integrations::authorize_binding,
            )),
        );
    let public = Router::new()
        .route("/v1/store-management/api-docs", get(api_docs::read))
        .route(
            "/v1/store-management/connectors",
            get(integrations::list_nodes),
        )
        .route(
            "/v1/store-management/apps/{platform}/{identifier}/uploads",
            post(uploads::prepare),
        )
        .route(
            "/v1/store-management/apps/{platform}/{identifier}/{resource}",
            get(read_store).post(write_store),
        )
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            authorize_client,
        ));
    Router::new()
        .route(
            "/health",
            get(|State(state): State<Arc<ApiState>>| async move {
                let healthy = state.registry.logs.healthy();
                (
                    if healthy {
                        StatusCode::OK
                    } else {
                        StatusCode::SERVICE_UNAVAILABLE
                    },
                    Json(
                        json!({"status":if healthy {"ok"} else {"degraded"},"logsHealthy":healthy}),
                    ),
                )
            }),
        )
        .route("/v1/apps/{platform}/{identifier}/connector", binding)
        .route("/v1/connectors/{id}/ws", get(connect_ws))
        .route(
            "/v1/store-management/apps/{platform}/{identifier}/uploads/{asset}",
            axum::routing::put(uploads::receive).layer(DefaultBodyLimit::disable()),
        )
        .merge(management)
        .merge(public)
        .merge(integrations::admin_routes(state.clone()))
        .merge(setup::routes(state.clone()))
        .merge(screenshots::routes(state.clone(), uploader))
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
        .layer(middleware::from_fn_with_state(state.clone(), logs::observe))
        .with_state(state)
}

fn node_view(state: &ApiState, node: Connector) -> Value {
    let connected = state.transport.connected(&node.connector_id);
    let logs_available = node.kind == ConnectorKind::Store
        && (state.transport.logs_supported(&node.connector_id)
            || (!connected && node.base_url.is_some()));
    let mut value = serde_json::to_value(node).expect("Connector serialization");
    value["connected"] = json!(connected);
    value["logsAvailable"] = json!(logs_available);
    value
}

async fn list_connectors(State(state): State<Arc<ApiState>>) -> Result<Json<Value>> {
    Ok(Json(
        json!({"connectors":state.registry.list()?.into_iter().map(|n| node_view(&state,n)).collect::<Vec<_>>()}),
    ))
}
async fn get_connector(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    Ok(Json(node_view(&state, state.registry.get(&id)?)))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateConnector {
    name: String,
    #[serde(default)]
    kind: ConnectorKind,
}

async fn create_connector(
    State(state): State<Arc<ApiState>>,
    Json(input): Json<CreateConnector>,
) -> Result<(StatusCode, Json<Value>)> {
    let node = state
        .registry
        .create_with_kind(ConnectorInput { name: input.name }, input.kind)?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({"connector":node_view(&state,node.clone()),"installation":config(&state,&node)}),
        ),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnableInput {
    enabled: bool,
}
async fn update_connector(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<String>,
    Json(input): Json<EnableInput>,
) -> Result<Json<Value>> {
    let node = state.registry.set_enabled(&id, input.enabled)?;
    if !input.enabled {
        state.transport.disconnect(&id);
    }
    Ok(Json(node_view(&state, node)))
}
async fn rotate_token(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let node = state.registry.rotate(&id)?;
    state.transport.disconnect(&id);
    Ok(Json(config(&state, &node)))
}

fn config(state: &ApiState, node: &Connector) -> Value {
    if node.kind == ConnectorKind::Screenshot {
        return json!({"kind":"screenshot","connectorId":node.connector_id,
            "token":state.registry.token(node),"serverUrl":state.public_url,
            "messageKey":state.registry.purpose_key(node,"message")});
    }
    json!({"connectorId":node.connector_id,"token":state.registry.token(node),"serverUrl":state.public_url,
        "messageKey":state.registry.purpose_key(node,"message"),"uploadKey":state.registry.purpose_key(node,"upload"),
        "baseUrl":node.base_url,"listenAddr":"0.0.0.0:8100","requestTimeoutSeconds":60,"provider":{"mode":"live"}})
}
async fn installation_config(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<String>,
) -> Result<Response> {
    let node = state.registry.get(&id)?;
    if !node.enabled {
        return Err(StoreError::new(409, "connector_disabled", "Connector is disabled").into());
    }
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(config(&state, &node)),
    )
        .into_response())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BindInput {
    connector_id: String,
}
async fn bind_connector(
    State(state): State<Arc<ApiState>>,
    Path((platform, identifier)): Path<(Platform, String)>,
    Json(input): Json<BindInput>,
) -> Result<Json<Value>> {
    Ok(Json(node_view(
        &state,
        state
            .registry
            .bind(platform, &identifier, &input.connector_id)?,
    )))
}
async fn unbind_connector(
    State(state): State<Arc<ApiState>>,
    Path((platform, identifier)): Path<(Platform, String)>,
) -> Result<StatusCode> {
    state.registry.unbind(platform, &identifier)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn app_connector(
    State(state): State<Arc<ApiState>>,
    Path((platform, identifier)): Path<(Platform, String)>,
) -> Result<Json<Value>> {
    Ok(Json(node_view(
        &state,
        state.registry.resolve(platform, &identifier)?,
    )))
}

async fn connect_ws(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response> {
    let token = bearer(&headers).to_owned();
    let node = state.registry.authenticate(&id, &token)?;
    let screenshot = node.kind == ConnectorKind::Screenshot;
    if screenshot {
        integrations::require_https_headers(&headers)?;
    }
    if !headers
        .get_all(header::SEC_WEBSOCKET_PROTOCOL)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|v| v.trim() == store_protocol::crypto::SUBPROTOCOL)
    {
        return Err(StoreError::new(
            426,
            "encrypted_protocol_required",
            "Connector requires the encrypted v2 protocol",
        )
        .into());
    }
    if state.transport.connected(&id) {
        return Err(StoreError::new(
            409,
            "connector_already_connected",
            "Connector already has an active connection",
        )
        .into());
    }
    Ok(ws
        .protocols([store_protocol::crypto::SUBPROTOCOL])
        .max_message_size(store_protocol::crypto::MAX_RECORD_BYTES)
        .on_upgrade(move |socket| async move {
            if let Ok(node) = state.registry.authenticate(&id, &token) {
                let key = state.registry.purpose_key(&node, "message");
                let (transport, registration_state, registration_id) =
                    (state.transport.clone(), state.clone(), id.clone());
                transport
                    .serve(
                        id,
                        socket,
                        key,
                        screenshot.then_some(&state.registry),
                        move |endpoint| {
                            if screenshot && endpoint.is_some() {
                                return Err(store_protocol::crypto::error());
                            }
                            registration_state.registry.register_endpoint(
                                &registration_id,
                                &token,
                                endpoint.as_deref(),
                            )
                        },
                    )
                    .await;
            }
        })
        .into_response())
}

async fn execute(
    state: Arc<ApiState>,
    context: logs::RequestContext,
    platform: Platform,
    identifier: String,
    operation: Operation,
    payload: Value,
) -> Result<Response> {
    logs::business_event(
        &state.registry,
        &context,
        platform,
        &identifier,
        operation,
        &payload,
    );
    let node = state.registry.resolve(platform, &identifier)?;
    let request = StoreRequest {
        request_id: context.0.clone(),
        platform,
        bundle_identifier: identifier,
        operation,
        payload,
    };
    request.validate()?;
    let reply = state
        .transport
        .dispatch(&node, &state.registry.token(&node), request)
        .await?;
    let mut body = reply.body;
    let error = (reply.status >= 400).then(|| {
        logs::error_summary(
            body["error"]["code"].as_str().unwrap_or("request_failed"),
            body["error"]["message"].as_str().unwrap_or(""),
        )
    });
    if let Some(object) = body.get_mut("error").and_then(Value::as_object_mut) {
        object.insert("requestId".into(), json!(context.0));
    }
    let mut response = (
        StatusCode::from_u16(reply.status).unwrap_or(StatusCode::BAD_GATEWAY),
        Json(body),
    )
        .into_response();
    if let Some(error) = error {
        response.extensions_mut().insert(error);
    }
    Ok(response)
}
async fn read_store(
    State(state): State<Arc<ApiState>>,
    Extension(context): Extension<logs::RequestContext>,
    Path((platform, identifier, resource)): Path<(Platform, String, String)>,
    Query(mut query): Query<HashMap<String, String>>,
) -> Result<Response> {
    let operation = match resource.as_str() {
        "supported-locales" => Operation::SupportedLocales,
        "store-listings" => Operation::StoreListings,
        "store-images" => Operation::StoreImages,
        "store-releases" => Operation::StoreReleases,
        "store-reviews" => Operation::StoreReviews,
        "product-page-optimizations" => Operation::ProductPageOptimizations,
        _ => return Err(StoreError::new(404, "not_found", "Unknown store resource").into()),
    };
    query.remove("version");
    let payload = Value::Object(
        query
            .into_iter()
            .map(|(key, value)| {
                let parsed = if matches!(key.as_str(), "pageSize" | "rating") {
                    value
                        .parse::<u64>()
                        .map_or_else(|_| json!(value), |v| json!(v))
                } else {
                    json!(value)
                };
                (key, parsed)
            })
            .collect(),
    );
    execute(state, context, platform, identifier, operation, payload).await
}
async fn write_store(
    State(state): State<Arc<ApiState>>,
    Extension(context): Extension<logs::RequestContext>,
    Path((platform, identifier, resource)): Path<(Platform, String, String)>,
    Json(mut payload): Json<Value>,
) -> Result<Response> {
    let operation = match resource.as_str() {
        "sync" => Operation::Sync,
        "preflight" => Operation::Preflight,
        "product-page-optimizations" => Operation::CreateProductPageOptimization,
        _ => return Err(StoreError::new(404, "not_found", "Unknown store resource").into()),
    };
    if let Some(object) = payload.as_object_mut() {
        object.remove("version");
    }
    execute(state, context, platform, identifier, operation, payload).await
}

async fn installer(
    State(state): State<Arc<ApiState>>,
    Path((id, target)): Path<(String, String)>,
) -> Result<Response> {
    let binary_name = match target.as_str() {
        "linux-amd64" | "linux-arm64" | "darwin-amd64" | "darwin-arm64" => "store-connector",
        "windows-amd64" => "store-connector.exe",
        _ => {
            return Err(
                StoreError::new(422, "unsupported_target", "Unsupported Connector target").into(),
            )
        }
    };
    let node = state.registry.get(&id)?;
    if node.kind != ConnectorKind::Store {
        return Err(StoreError::new(
            422,
            "invalid_connector_kind",
            "Screenshot Connectors use the macOS worker",
        )
        .into());
    }
    if !node.enabled {
        return Err(StoreError::new(409, "connector_disabled", "Connector is disabled").into());
    }
    let release = state.release_root.join(&target).join(binary_name);
    // Only an explicit, allowlisted release binary is read; no user-supplied path or executable.
    let metadata = tokio::fs::metadata(&release).await.map_err(|_| {
        StoreError::new(
            503,
            "release_unavailable",
            "Connector binary has not been published for this target",
        )
    })?;
    if !metadata.is_file() || metadata.len() > 128 * 1024 * 1024 {
        return Err(StoreError::new(503, "invalid_release", "Connector release is invalid").into());
    }
    let binary = tokio::fs::read(release).await.map_err(|_| {
        StoreError::new(503, "release_unavailable", "Cannot read Connector release")
    })?;
    let config = serde_json::to_vec_pretty(&config(&state, &node)).expect("Config serialization");
    let data = tokio::task::spawn_blocking(move || -> std::result::Result<Vec<u8>, StoreError> {
        use std::io::{Cursor,Write};
        use zip::{write::SimpleFileOptions,ZipWriter};
        let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        archive.start_file(binary_name,options.unix_permissions(0o700)).map_err(|_| StoreError::new(500,"installer_error","Cannot package Connector"))?;
        archive.write_all(&binary).map_err(|_| StoreError::new(500,"installer_error","Cannot package Connector"))?;
        archive.start_file("connector.json",options.unix_permissions(0o600)).map_err(|_| StoreError::new(500,"installer_error","Cannot package configuration"))?;
        archive.write_all(&config).map_err(|_| StoreError::new(500,"installer_error","Cannot package configuration"))?;
        let (script_name,script) = if binary_name.ends_with(".exe") {("start.cmd", "@echo off\r\ncd /d \"%~dp0\"\r\nstore-connector.exe connector.json\r\n")} else {("start.sh", "#!/bin/sh\nset -eu\ncd -- \"$(dirname -- \"$0\")\"\nexec ./store-connector connector.json\n")};
        archive.start_file(script_name,options.unix_permissions(0o700)).map_err(|_| StoreError::new(500,"installer_error","Cannot package launcher"))?;
        archive.write_all(script.as_bytes()).map_err(|_| StoreError::new(500,"installer_error","Cannot package launcher"))?;
        Ok(archive.finish().map_err(|_| StoreError::new(500,"installer_error","Cannot finish installer"))?.into_inner())
    }).await.map_err(|_| StoreError::new(500,"installer_error","Installer task failed"))??;
    Ok((
        [
            (header::CONTENT_TYPE, "application/zip"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        data,
    )
        .into_response())
}

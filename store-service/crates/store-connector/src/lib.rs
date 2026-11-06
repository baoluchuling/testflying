use axum::{
    extract::{DefaultBodyLimit, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{sync::Arc, time::Duration};
use store_protocol::{crypto, upload, Operation, Platform, StoreError, StoreReply, StoreRequest};
use store_provider::{assets::AssetStore, Gateway, ProviderConfig};
use subtle::ConstantTimeEq;
use tokio::sync::{mpsc, oneshot, watch, Semaphore};
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{client::IntoClientRequest, Message},
};

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PlatformMode {
    #[default]
    All,
    Ios,
    Android,
}

impl PlatformMode {
    fn supports(self, platform: Platform) -> bool {
        matches!(
            (self, platform),
            (Self::All, _) | (Self::Ios, Platform::Ios) | (Self::Android, Platform::Android)
        )
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectorConfig {
    pub connector_id: String,
    pub token: String,
    #[serde(default, skip_serializing)]
    pub mode: Option<String>,
    pub server_url: String,
    pub message_key: String,
    pub upload_key: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default = "listen_addr")]
    pub listen_addr: String,
    #[serde(default = "request_timeout")]
    pub request_timeout_seconds: u64,
    #[serde(default)]
    pub platform_mode: PlatformMode,
    #[serde(default)]
    pub provider: ProviderConfig,
}
fn listen_addr() -> String {
    "0.0.0.0:8100".into()
}
fn request_timeout() -> u64 {
    60
}

pub struct Executor {
    config: ConnectorConfig,
    gateway: Gateway,
    capacity: Arc<Semaphore>,
    pub assets: Arc<AssetStore>,
}

impl Executor {
    pub fn new(mut config: ConnectorConfig) -> Result<Arc<Self>, StoreError> {
        if config.connector_id.is_empty()
            || !config
                .connector_id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
            || config.token.len() < 16
        {
            return Err(StoreError::new(
                500,
                "invalid_config",
                "Connector identity and token are required",
            ));
        }
        if !(1..=300).contains(&config.request_timeout_seconds)
            || [&config.message_key, &config.upload_key]
                .iter()
                .any(|key| hex::decode(key).map_or(true, |v| v.len() != 32))
        {
            return Err(StoreError::new(
                500,
                "invalid_config",
                "Invalid Connector keys or timeout",
            ));
        }
        let url = reqwest::Url::parse(&config.server_url)
            .map_err(|_| StoreError::new(500, "invalid_config", "Invalid Store API URL"))?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(StoreError::new(
                500,
                "invalid_config",
                "Store API URL must be HTTP(S) without credentials",
            ));
        }
        config.base_url = config
            .base_url
            .as_deref()
            .map(upload::normalize_base_url)
            .transpose()?;
        let mut provider = config.provider.clone();
        let defaults = ProviderConfig::default();
        match config.platform_mode {
            PlatformMode::All => {}
            PlatformMode::Ios => {
                provider.google_service_account_path = None;
                provider.google_base_url = defaults.google_base_url;
                provider.google_upload_base_url = defaults.google_upload_base_url;
                provider.google_token_url = defaults.google_token_url;
            }
            PlatformMode::Android => {
                provider.apple_issuer_id = None;
                provider.apple_key_id = None;
                provider.apple_private_key_path = None;
                provider.apple_base_url = defaults.apple_base_url;
            }
        }
        let assets = AssetStore::new(config.connector_id.clone(), config.upload_key.clone());
        let gateway = Gateway::with_assets(provider, Some(assets.clone()))?;
        Ok(Arc::new(Self {
            config,
            gateway,
            capacity: Arc::new(Semaphore::new(1)),
            assets,
        }))
    }

    pub async fn execute(self: Arc<Self>, request: StoreRequest) -> StoreReply {
        if let Err(error) = request.validate() {
            return StoreReply::from_result(request.request_id, Err(error));
        }
        if !self.config.platform_mode.supports(request.platform) {
            return StoreReply::from_result(
                request.request_id,
                Err(StoreError::new(
                    422,
                    "platform_not_supported",
                    format!("This Connector does not support {}", request.platform),
                )),
            );
        }
        if request.operation.is_upload() {
            return self.upload_command(request).await;
        }
        let permit = match self.capacity.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                return StoreReply::from_result(
                    request.request_id,
                    Err(StoreError::new(
                        409,
                        "connector_busy",
                        "Another request is executing for this account",
                    )),
                )
            }
        };
        let id = request.request_id.clone();
        let timeout = Duration::from_secs(self.config.request_timeout_seconds);
        let (sender, receiver) = oneshot::channel();
        // Let already-started vendor work finish cleanup even if its caller times out or disconnects.
        // The permit stays held until completion; the result is then discarded, never retried/replayed.
        tokio::spawn(async move {
            let _permit = permit;
            let result = self.gateway.execute(&request).await;
            let _ = sender.send(StoreReply::from_result(request.request_id, result));
        });
        match tokio::time::timeout(timeout, receiver).await {
            Ok(Ok(reply)) => reply,
            Ok(Err(_)) => StoreReply::from_result(
                id,
                Err(StoreError::new(
                    502,
                    "execution_failed",
                    "Connector execution stopped",
                )),
            ),
            Err(_) => StoreReply::from_result(
                id,
                Err(StoreError::new(
                    504,
                    "execution_timeout",
                    "Execution timed out; the store outcome may be unknown",
                )),
            ),
        }
    }

    pub async fn drain(&self) {
        if let Ok(permit) = self.capacity.acquire().await {
            drop(permit);
        }
    }
}

fn bearer(headers: &HeaderMap) -> &str {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .unwrap_or("")
}
pub fn http_router(executor: Arc<Executor>) -> Router {
    Router::new()
        .route("/health", get(|| async { Json(json!({"status":"ok"})) }))
        .route("/v1/execute", post(http_execute))
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
        .with_state(executor)
}

impl Executor {
    async fn upload_command(self: &Arc<Self>, request: StoreRequest) -> StoreReply {
        let id = request.request_id.clone();
        let store = self.assets.clone();
        let result = tokio::task::spawn_blocking(move || {
            let asset = request.payload["assetId"]
                .as_str()
                .filter(|id| upload::valid_asset_id(id))
                .ok_or_else(|| {
                    StoreError::new(422, "invalid_asset_id", "A valid assetId is required")
                })?;
            let session = request.payload["sessionId"]
                .as_str()
                .filter(|id| upload::valid_asset_id(id))
                .ok_or_else(|| {
                    StoreError::new(
                        422,
                        "invalid_upload_session",
                        "A valid sessionId is required",
                    )
                })?;
            let result = match request.operation {
                Operation::UploadStart => {
                    let ticket = request.payload["ticket"]
                        .as_str()
                        .ok_or_else(upload::invalid_token)?;
                    let claims = store.claims(ticket)?;
                    if claims.asset_id != asset
                        || claims.platform != request.platform
                        || claims.bundle_identifier != request.bundle_identifier
                    {
                        return Err(upload::invalid_token());
                    }
                    store.start(ticket, session)?;
                    json!({"assetId":asset})
                }
                Operation::UploadChunk => {
                    let data = request.payload["data"].as_str().ok_or_else(|| {
                        StoreError::new(422, "invalid_upload_chunk", "Chunk data is required")
                    })?;
                    let data = upload::decode_chunk(data)?;
                    let offset = request.payload["offset"].as_u64().ok_or_else(|| {
                        StoreError::new(422, "invalid_upload_offset", "Chunk offset is required")
                    })?;
                    store.chunk(
                        request.platform,
                        &request.bundle_identifier,
                        asset,
                        session,
                        offset,
                        &data,
                    )?;
                    json!({"received":offset+data.len() as u64})
                }
                Operation::UploadFinish => {
                    store.finish(request.platform, &request.bundle_identifier, asset, session)?
                }
                Operation::UploadAbort => {
                    store.abort(request.platform, &request.bundle_identifier, asset, session);
                    json!({"aborted":true})
                }
                _ => unreachable!(),
            };
            Ok(result)
        })
        .await
        .unwrap_or_else(|_| {
            Err(StoreError::new(
                500,
                "upload_worker",
                "Upload worker failed",
            ))
        });
        StoreReply::from_result(id, result)
    }
}

struct UploadGuard {
    store: Arc<AssetStore>,
    platform: store_protocol::Platform,
    app: String,
    id: String,
    session: String,
}
impl Drop for UploadGuard {
    fn drop(&mut self) {
        self.store
            .abort(self.platform, &self.app, &self.id, &self.session);
    }
}

pub async fn run(
    executor: Arc<Executor>,
    listener: tokio::net::TcpListener,
    stopped: watch::Receiver<bool>,
) -> Result<(), std::io::Error> {
    let mut http_stop = stopped.clone();
    let mut gc_stop = stopped.clone();
    let assets = executor.assets.clone();
    tokio::try_join!(
        axum::serve(listener, http_router(executor.clone())).with_graceful_shutdown(async move {
            let _ = http_stop.changed().await;
        }),
        async move {
            run_active(executor, stopped).await;
            Ok::<_, std::io::Error>(())
        },
        async move {
            let mut tick = tokio::time::interval(Duration::from_secs(30));
            loop {
                tokio::select! { _=gc_stop.changed()=>return Ok::<_,std::io::Error>(()),_=tick.tick()=>assets.purge() }
            }
        }
    )?;
    Ok(())
}
async fn http_execute(
    State(executor): State<Arc<Executor>>,
    headers: HeaderMap,
    Json(request): Json<StoreRequest>,
) -> Response {
    if !bool::from(
        executor
            .config
            .token
            .as_bytes()
            .ct_eq(bearer(&headers).as_bytes()),
    ) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(
                StoreError::new(401, "invalid_node_token", "Invalid Connector credentials").body(),
            ),
        )
            .into_response();
    }
    Json(executor.execute(request).await).into_response()
}

async fn connection(
    executor: Arc<Executor>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), StoreError> {
    let mut url = reqwest::Url::parse(&executor.config.server_url)
        .map_err(|_| StoreError::new(500, "invalid_config", "Invalid server URL"))?;
    url.set_scheme(if url.scheme() == "https" { "wss" } else { "ws" })
        .map_err(|_| StoreError::new(500, "invalid_config", "Invalid WebSocket URL"))?;
    let path = format!(
        "{}/v1/connectors/{}/ws",
        url.path().trim_end_matches('/'),
        executor.config.connector_id
    );
    url.set_path(&path);
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|_| StoreError::new(500, "invalid_config", "Invalid WebSocket request"))?;
    request.headers_mut().insert(
        header::AUTHORIZATION,
        format!("Bearer {}", executor.config.token)
            .parse()
            .map_err(|_| StoreError::new(500, "invalid_config", "Invalid node token"))?,
    );
    request.headers_mut().insert(
        header::SEC_WEBSOCKET_PROTOCOL,
        crypto::SUBPROTOCOL.parse().expect("protocol header"),
    );
    let socket_config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(crypto::MAX_RECORD_BYTES))
        .max_frame_size(Some(crypto::MAX_RECORD_BYTES));
    let (mut socket, _) = tokio::time::timeout(
        Duration::from_secs(10),
        connect_async_with_config(request, Some(socket_config), false),
    )
    .await
    .map_err(|_| StoreError::new(503, "connect_timeout", "Store API connection timed out"))?
    .map_err(|_| StoreError::new(503, "connect_failed", "Cannot connect to Store API"))?;
    let mut channel = tokio::time::timeout(Duration::from_secs(10), async {
        let mut handshake = crypto::handshake(
            &executor.config.message_key,
            &executor.config.connector_id,
            true,
        )?;
        let registration = serde_json::to_vec(
            &json!({"connectorId":executor.config.connector_id,"baseUrl":executor.config.base_url}),
        )
        .map_err(|_| crypto::error())?;
        let mut buffer = vec![0; crypto::HANDSHAKE_BYTES];
        let size = handshake
            .write_message(&registration, &mut buffer)
            .map_err(|_| crypto::error())?;
        if !bounded_send(&mut socket, Message::Binary(buffer[..size].to_vec().into())).await {
            return Err(crypto::error());
        }
        let Some(Ok(Message::Binary(reply))) = socket.next().await else {
            return Err(crypto::error());
        };
        handshake
            .read_message(&reply, &mut buffer)
            .map_err(|_| crypto::error())?;
        let mut channel = crypto::Channel::new(handshake)?;
        for record in channel.encrypt(b"confirm")? {
            if !bounded_send(&mut socket, Message::Binary(record.into())).await {
                return Err(crypto::error());
            }
        }
        let Some(Ok(Message::Binary(reply))) = socket.next().await else {
            return Err(crypto::error());
        };
        if channel.decrypt(&reply)?.as_deref() != Some(b"ready") {
            return Err(crypto::error());
        }
        Ok(channel)
    })
    .await
    .map_err(|_| crypto::error())??;
    let (mut write, mut read) = socket.split();
    let mut uploads = Vec::<UploadGuard>::new();
    let (results, mut result_receiver) = mpsc::channel::<StoreReply>(1);
    let mut heartbeat = tokio::time::interval(Duration::from_secs(10));
    let mut last_seen = tokio::time::Instant::now();
    loop {
        tokio::select! {
            _ = shutdown.changed() => {let _ = bounded_send(&mut write,Message::Close(None)).await; return Ok(());},
            _ = heartbeat.tick() => {
                if last_seen.elapsed()>Duration::from_secs(35) || !bounded_send(&mut write,Message::Ping(Vec::new().into())).await {break;}
            }
            result = result_receiver.recv() => {
                let Some(result)=result else {break;};
                let text=serde_json::to_string(&result).map_err(|_|StoreError::new(500,"serialize_error","Cannot serialize result"))?;
                let mut sent=true;
                for record in channel.encrypt(text.as_bytes())? { if !bounded_send(&mut write,Message::Binary(record.into())).await {sent=false;break;} }
                if !sent {break;}
            }
            message = read.next() => {
                let Some(Ok(message)) = message else {break;};
                last_seen=tokio::time::Instant::now();
                match message {
                    Message::Binary(bytes) => {
                        let Some(plain)=channel.decrypt(&bytes)? else {continue;};
                        let request:StoreRequest=serde_json::from_slice(&plain).map_err(|_|StoreError::new(422,"invalid_request","Invalid Store API command"))?;
                        if request.operation.is_upload() {
                            let operation=request.operation;
                            let id=request.payload["assetId"].as_str().unwrap_or("").to_owned();
                            let session=request.payload["sessionId"].as_str().unwrap_or("").to_owned();
                            let (platform,app)=(request.platform,request.bundle_identifier.clone());
                            let result=executor.clone().execute(request).await;
                            if operation==Operation::UploadStart && result.status==200 {
                                uploads.push(UploadGuard {store:executor.assets.clone(),platform,app,id,session});
                            } else if matches!(operation,Operation::UploadFinish|Operation::UploadAbort) {
                                uploads.retain(|item|item.id!=id || item.session!=session || item.platform!=platform || item.app!=app);
                            }
                            // Avoid waiting on our own single-slot result queue while reading chunks.
                            let plain=serde_json::to_vec(&result).map_err(|_|crypto::error())?;
                            for record in channel.encrypt(&plain)? { if !bounded_send(&mut write,Message::Binary(record.into())).await {return Err(crypto::error());} }
                        } else {
                            let executor=executor.clone();let sender=results.clone();
                            tokio::spawn(async move {let _=sender.send(executor.execute(request).await).await;});
                        }
                    }
                    Message::Text(_)=>return Err(crypto::error()),
                    Message::Ping(bytes) => {if !bounded_send(&mut write,Message::Pong(bytes)).await {break;}},
                    Message::Close(_) => break,
                    _=>{}
                }
            }
        }
    }
    Err(StoreError::new(
        503,
        "connection_closed",
        "Store API connection closed",
    ))
}

pub async fn run_active(executor: Arc<Executor>, mut shutdown: watch::Receiver<bool>) {
    let mut delay = Duration::from_secs(1);
    loop {
        if *shutdown.borrow() {
            return;
        }
        let started = tokio::time::Instant::now();
        let connection_shutdown = shutdown.clone();
        tokio::select! {
            _ = shutdown.changed() => return,
            result = connection(executor.clone(),connection_shutdown) => {
                if let Err(error)=result {eprintln!("{}",error.code);}
            }
        }
        if *shutdown.borrow() {
            return;
        }
        if started.elapsed() > Duration::from_secs(30) {
            delay = Duration::from_secs(1);
        }
        tokio::select! {_ = shutdown.changed() => return, _ = tokio::time::sleep(delay) => {}}
        delay = (delay * 2).min(Duration::from_secs(30));
    }
}

async fn bounded_send<S>(write: &mut S, message: Message) -> bool
where
    S: futures_util::Sink<Message> + Unpin,
{
    matches!(
        tokio::time::timeout(Duration::from_secs(5), write.send(message)).await,
        Ok(Ok(()))
    )
}

#[cfg(test)]
mod stalled_writer_tests {
    use super::*;
    use std::{
        pin::Pin,
        task::{Context, Poll},
    };
    struct Stalled;
    impl futures_util::Sink<Message> for Stalled {
        type Error = ();
        fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), ()>> {
            Poll::Pending
        }
        fn start_send(self: Pin<&mut Self>, _: Message) -> Result<(), ()> {
            Ok(())
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), ()>> {
            Poll::Pending
        }
        fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), ()>> {
            Poll::Ready(Ok(()))
        }
    }
    #[tokio::test(start_paused = true)]
    async fn stalled_socket_write_has_a_deadline() {
        let result = tokio::time::timeout(
            Duration::from_secs(6),
            bounded_send(&mut Stalled, Message::Ping(Vec::new().into())),
        )
        .await
        .expect("Socket write did not respect its deadline");
        assert!(!result);
    }
}

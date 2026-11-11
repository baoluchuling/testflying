use crate::logs::{self, Event, LogStore};
use crate::registry::{Connector, Registry};
use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use store_protocol::logs::{LogRequest, NodeRequest};
use store_protocol::{crypto, StoreError, StoreReply, StoreRequest};
use tokio::sync::{mpsc, oneshot, watch};
use uuid::Uuid;

type Result<T> = std::result::Result<T, StoreError>;
type Pending = Arc<Mutex<HashMap<String, oneshot::Sender<Result<StoreReply>>>>>;

struct Connection {
    generation: Uuid,
    sender: mpsc::Sender<NodeRequest>,
    logs_supported: bool,
    close: watch::Sender<bool>,
    screenshot_wake: watch::Sender<()>,
    pending: Pending,
}

pub struct Transport {
    logs: Option<Arc<LogStore>>,
    client: reqwest::Client,
    connections: Mutex<HashMap<String, Connection>>,
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    timeout: Duration,
}

fn disconnected() -> StoreError {
    StoreError::new(
        503,
        "connector_disconnected",
        "Connector disconnected; trigger a new request when available",
    )
}

impl Transport {
    pub fn new(timeout: Duration) -> Result<Self> {
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(5))
            .timeout(timeout)
            .build()
            .map_err(|_| {
                StoreError::new(500, "http_client", "Cannot initialize Connector transport")
            })?;
        Ok(Self {
            logs: None,
            client,
            connections: Mutex::new(HashMap::new()),
            locks: Mutex::new(HashMap::new()),
            timeout,
        })
    }

    pub(crate) fn with_logs(timeout: Duration, logs: Arc<LogStore>) -> Result<Self> {
        let mut transport = Self::new(timeout)?;
        transport.logs = Some(logs);
        Ok(transport)
    }

    fn connection_event(&self, id: &str, name: &'static str, reason: &'static str) {
        if let Some(logs) = &self.logs {
            logs.record(Event {
                name,
                connector_id: Some(id.to_owned()),
                details: serde_json::json!({"reason":reason}),
                ..Default::default()
            });
        }
    }

    pub fn connected(&self, id: &str) -> bool {
        self.connections.lock().unwrap().contains_key(id)
    }
    pub fn logs_supported(&self, id: &str) -> bool {
        self.connections
            .lock()
            .unwrap()
            .get(id)
            .is_some_and(|c| c.logs_supported)
    }

    pub async fn dispatch_logs(
        &self,
        node: &Connector,
        token: &str,
        request: LogRequest,
    ) -> Result<StoreReply> {
        request.validate()?;
        if self.connected(&node.connector_id) {
            if !self.logs_supported(&node.connector_id) {
                return Err(StoreError::new(
                    409,
                    "logs_upgrade_required",
                    "Upgrade this node to enable log retrieval",
                ));
            }
            self.websocket_request(&node.connector_id, NodeRequest::Logs(request))
                .await
        } else {
            let base = node.base_url.as_deref().ok_or_else(disconnected)?;
            let mut response = self
                .client
                .post(format!("{base}/v1/diagnostics/logs"))
                .bearer_auth(token)
                .json(&request)
                .send()
                .await
                .map_err(|_| disconnected())?;
            if response.status().as_u16() == 404 {
                return Err(StoreError::new(
                    409,
                    "logs_upgrade_required",
                    "Upgrade this node to enable log retrieval",
                ));
            }
            if !response.status().is_success() {
                return Err(StoreError::new(
                    502,
                    "connector_rejected",
                    "Node rejected the log request",
                ));
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| disconnected())? {
                if bytes.len() + chunk.len() > 128 * 1024 {
                    return Err(StoreError::new(
                        502,
                        "invalid_connector_reply",
                        "Node log reply exceeded limit",
                    ));
                }
                bytes.extend_from_slice(&chunk);
            }
            let reply: StoreReply = serde_json::from_slice(&bytes).map_err(|_| {
                StoreError::new(502, "invalid_connector_reply", "Invalid node log reply")
            })?;
            if reply.request_id != request.request_id || !(200..=599).contains(&reply.status) {
                return Err(StoreError::new(
                    502,
                    "invalid_connector_reply",
                    "Invalid node log reply",
                ));
            }
            Ok(reply)
        }
    }

    pub fn disconnect(&self, id: &str) {
        if let Some(connection) = self.connections.lock().unwrap().remove(id) {
            self.connection_event(id, "connector.disconnected", "center_closed");
            let _ = connection.close.send(true);
            for (_, sender) in connection.pending.lock().unwrap().drain() {
                let _ = sender.send(Err(disconnected()));
            }
        }
    }

    pub(crate) fn notify_screenshot(&self, id: &str) {
        if let Some(connection) = self.connections.lock().unwrap().get(id) {
            connection.screenshot_wake.send_replace(());
        }
    }

    pub fn close_all(&self) {
        let ids: Vec<_> = self.connections.lock().unwrap().keys().cloned().collect();
        for id in ids {
            self.disconnect(&id);
        }
    }

    pub async fn dispatch(
        &self,
        node: &Connector,
        token: &str,
        request: StoreRequest,
    ) -> Result<StoreReply> {
        let websocket = self.connected(&node.connector_id);
        self.dispatch_via(node, token, request, websocket).await
    }

    pub async fn dispatch_via(
        &self,
        node: &Connector,
        token: &str,
        request: StoreRequest,
        websocket: bool,
    ) -> Result<StoreReply> {
        let event = Event {
            name: "connector.dispatch",
            request_id: Some(request.request_id.clone()),
            connector_id: Some(node.connector_id.clone()),
            platform: Some(request.platform),
            identifier: Some(request.bundle_identifier.clone()),
            details: serde_json::json!({"operation":request.operation,"transport":if websocket {"wss"} else {"http"}}),
            ..Default::default()
        };
        if let Some(logs) = &self.logs {
            logs.record(event.clone());
        }
        let mut guard = DispatchGuard {
            logs: self.logs.clone(),
            event,
            start: std::time::Instant::now(),
            completed: false,
        };
        let result = self.dispatch_inner(node, token, request, websocket).await;
        guard.event.name = "connector.result";
        let (status, error) = match &result {
            Ok(reply) => (
                reply.status,
                (reply.status >= 400).then(|| {
                    logs::error_summary(
                        reply.body["error"]["code"]
                            .as_str()
                            .unwrap_or("request_failed"),
                        reply.body["error"]["message"].as_str().unwrap_or(""),
                    )
                }),
            ),
            Err(error) => (
                error.status,
                Some(logs::error_summary(&error.code, &error.message)),
            ),
        };
        guard.event.status = Some(status);
        guard.event.details["durationMs"] =
            serde_json::json!(guard.start.elapsed().as_millis() as u64);
        if let Some(error) = error {
            guard.event.details["error"] = serde_json::json!(error);
        }
        if let Ok(reply) = &result {
            guard.event.details["response"] = logs::reply_summary(&reply.body);
        }
        guard.event.details["outcomeUnknown"] = serde_json::json!(status >= 400 && status != 409);
        if let Some(logs) = &guard.logs {
            logs.record(guard.event.clone());
        }
        guard.completed = true;
        result
    }

    async fn dispatch_inner(
        &self,
        node: &Connector,
        token: &str,
        request: StoreRequest,
        websocket: bool,
    ) -> Result<StoreReply> {
        let lock = self
            .locks
            .lock()
            .unwrap()
            .entry(node.connector_id.clone())
            .or_default()
            .clone();
        let _guard = if request.operation.is_upload() {
            None
        } else {
            Some(lock.try_lock().map_err(|_| {
                StoreError::new(
                    409,
                    "connector_busy",
                    "Another request is executing for this account",
                )
            })?)
        };
        if !websocket {
            let url = format!(
                "{}/v1/execute",
                node.base_url.as_deref().ok_or_else(disconnected)?
            );
            let response = self
                .client
                .post(url)
                .bearer_auth(token)
                .json(&request)
                .send()
                .await
                .map_err(|e| {
                    if e.is_timeout() {
                        StoreError::new(
                            504,
                            "connector_timeout",
                            "Connector timed out; the store outcome may be unknown",
                        )
                    } else {
                        disconnected()
                    }
                })?;
            let status = response.status();
            if !status.is_success() {
                if let Some(logs) = &self.logs {
                    logs.record(Event {
                        name: "connector.http_rejected",
                        request_id: Some(request.request_id.clone()),
                        connector_id: Some(node.connector_id.clone()),
                        status: Some(status.as_u16()),
                        details: serde_json::json!({"transport":"http"}),
                        ..Default::default()
                    });
                }
                return Err(StoreError::new(
                    if status.as_u16() == 409 { 409 } else { 502 },
                    "connector_rejected",
                    "Connector rejected the request",
                ));
            }
            let reply: StoreReply = response.json().await.map_err(|_| {
                StoreError::new(502, "invalid_connector_reply", "Invalid Connector response")
            })?;
            if reply.request_id != request.request_id || !(200..=599).contains(&reply.status) {
                return Err(StoreError::new(
                    502,
                    "invalid_connector_reply",
                    "Connector response did not match the request",
                ));
            }
            Ok(reply)
        } else {
            self.websocket_request(&node.connector_id, NodeRequest::Store(request))
                .await
        }
    }

    async fn websocket_request(&self, node_id: &str, request: NodeRequest) -> Result<StoreReply> {
        let (sender, receiver) = oneshot::channel();
        let id = request.request_id().to_owned();
        let pending = {
            let connections = self.connections.lock().unwrap();
            let connection = connections.get(node_id).ok_or_else(disconnected)?;
            let pending = connection.pending.clone();
            pending.lock().unwrap().insert(id.clone(), sender);
            if connection.sender.try_send(request).is_err() {
                pending.lock().unwrap().remove(&id);
                return Err(disconnected());
            }
            pending
        };
        let _pending_guard = PendingGuard { pending, id };
        match tokio::time::timeout(self.timeout, receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(disconnected()),
            Err(_) => Err(StoreError::new(
                504,
                "connector_timeout",
                "Connector timed out; the store outcome may be unknown",
            )),
        }
    }

    pub async fn serve(
        self: Arc<Self>,
        id: String,
        mut socket: WebSocket,
        key: String,
        screenshot_registry: Option<&Registry>,
        register: impl FnOnce(Option<String>) -> Result<()> + Send,
    ) {
        let authenticated = tokio::time::timeout(Duration::from_secs(10), async {
            let mut handshake = crypto::handshake(&key, &id, false)?;
            let Some(Ok(Message::Binary(first))) = socket.next().await else {
                return Err(crypto::error());
            };
            if first.len() > crypto::HANDSHAKE_BYTES {
                return Err(crypto::error());
            }
            let mut plain = vec![0; crypto::HANDSHAKE_BYTES];
            let size = handshake
                .read_message(&first, &mut plain)
                .map_err(|_| crypto::error())?;
            let registration: crypto::Registration =
                serde_json::from_slice(&plain[..size]).map_err(|_| crypto::error())?;
            if registration.connector_id != id {
                return Err(crypto::error());
            }
            let endpoint = registration
                .base_url
                .as_deref()
                .map(store_protocol::upload::normalize_base_url)
                .transpose()?;
            let size = handshake
                .write_message(&[], &mut plain)
                .map_err(|_| crypto::error())?;
            if !bounded_send(&mut socket, Message::Binary(plain[..size].to_vec().into())).await {
                return Err(crypto::error());
            }
            let mut channel = crypto::Channel::new(handshake)?;
            let Some(Ok(Message::Binary(confirm))) = socket.next().await else {
                return Err(crypto::error());
            };
            if channel.decrypt(&confirm)?.as_deref() != Some(b"confirm") {
                return Err(crypto::error());
            }
            Ok((channel, endpoint, registration.logs_version == Some(1)))
        })
        .await;
        let Ok(Ok((mut channel, endpoint, logs_supported))) = authenticated else {
            self.connection_event(
                &id,
                "connector.handshake_failed",
                "handshake_rejected_or_timed_out",
            );
            return;
        };
        let (sender, mut requests) = mpsc::channel(1);
        let (close, mut closed) = watch::channel(false);
        let (screenshot_wake, mut screenshot_ready) = watch::channel(());
        let pending: Pending = Arc::default();
        let generation = Uuid::new_v4();
        {
            let mut connections = self.connections.lock().unwrap();
            if connections.contains_key(&id) {
                self.connection_event(&id, "connector.handshake_failed", "duplicate_connection");
                return;
            }
            if register(endpoint).is_err() {
                self.connection_event(&id, "connector.handshake_failed", "registration_rejected");
                return;
            }
            connections.insert(
                id.clone(),
                Connection {
                    generation,
                    sender,
                    close,
                    screenshot_wake,
                    pending: pending.clone(),
                    logs_supported,
                },
            );
        }
        self.connection_event(&id, "connector.connected", "encrypted_handshake_complete");
        let Ok(ready) = channel.encrypt(b"ready") else {
            self.disconnect(&id);
            return;
        };
        for record in ready {
            if !bounded_send(&mut socket, Message::Binary(record.into())).await {
                self.disconnect(&id);
                return;
            }
        }
        let (mut write, mut read) = socket.split();
        let mut heartbeat = tokio::time::interval(Duration::from_secs(10));
        let mut last_seen = tokio::time::Instant::now();
        loop {
            tokio::select! {
                _ = closed.changed() => break,
                _ = heartbeat.tick() => {
                    if last_seen.elapsed() > Duration::from_secs(35) {
                        self.connection_event(&id,"connector.heartbeat_failed","heartbeat_timeout");break;
                    }
                    if !bounded_send(&mut write,Message::Ping(Vec::new().into())).await {
                        self.connection_event(&id,"connector.socket_failed","write_failed_or_timed_out");break;
                    }
                    // Recover queued work after reconnect or expiration of an abandoned lease.
                    if !screenshot_ready.has_changed().unwrap_or(false)
                        && screenshot_registry.is_some_and(|registry| registry.has_claimable_screenshot(&id).unwrap_or(false)) {
                        screenshot_ready.mark_changed();
                    }
                }
                changed = screenshot_ready.changed(), if screenshot_registry.is_some() => {
                    if changed.is_err() {break;}
                    let Ok(records) = channel.encrypt(b"screenshot_job_available") else {break;};
                    let mut sent = true;
                    for record in records {
                        if !bounded_send(&mut write, Message::Binary(record.into())).await {sent=false;break;}
                    }
                    if !sent {break;}
                }
                request = requests.recv() => {
                    let Some(request) = request else {break;};
                    // A canceled HTTP caller must not cause a command to be sent later.
                    if !pending.lock().unwrap().contains_key(request.request_id()) {continue;}
                    let Ok(text) = serde_json::to_string(&request) else {break;};
                    let Ok(records)=channel.encrypt(text.as_bytes()) else {break;};
                    let mut sent=true;
                    for record in records {if !bounded_send(&mut write,Message::Binary(record.into())).await {sent=false;break;}}
                    if !sent {break;}
                }
                message = read.next() => {
                    let Some(Ok(message)) = message else {break;};
                    last_seen = tokio::time::Instant::now();
                    match message {
                        Message::Binary(bytes) => {
                            if screenshot_registry.is_some() {break;}
                            let plain=match channel.decrypt(&bytes) {Ok(Some(plain))=>plain,Ok(None)=>continue,Err(_)=>{
                                self.connection_event(&id,"connector.protocol_failed","decrypt_failed");break;
                            }};
                            let Ok(reply) = serde_json::from_slice::<StoreReply>(&plain) else {
                                self.connection_event(&id,"connector.protocol_failed","invalid_reply_json");break;
                            };
                            if !(200..=599).contains(&reply.status) {
                                self.connection_event(&id,"connector.protocol_failed","invalid_reply_status");break;
                            }
                            if let Some(sender) = pending.lock().unwrap().remove(&reply.request_id) {let _ = sender.send(Ok(reply));}
                        }
                        Message::Text(_)=>break,
                        Message::Ping(bytes) => {if !bounded_send(&mut write,Message::Pong(bytes)).await {break;}},
                        Message::Close(_) => break,
                        _ => {}
                    }
                }
            }
        }
        self.connection_event(&id, "connector.disconnected", "connection_ended");
        let mut connections = self.connections.lock().unwrap();
        if connections
            .get(&id)
            .is_some_and(|c| c.generation == generation)
        {
            connections.remove(&id);
        }
        drop(connections);
        for (_, sender) in pending.lock().unwrap().drain() {
            let _ = sender.send(Err(disconnected()));
        }
    }
}

struct DispatchGuard {
    logs: Option<Arc<LogStore>>,
    event: Event,
    start: std::time::Instant,
    completed: bool,
}
impl Drop for DispatchGuard {
    fn drop(&mut self) {
        if !self.completed {
            self.event.name = "connector.cancelled";
            self.event.status = Some(499);
            self.event.details["durationMs"] =
                serde_json::json!(self.start.elapsed().as_millis() as u64);
            self.event.details["outcomeUnknown"] = serde_json::json!(true);
            if let Some(logs) = &self.logs {
                logs.record(self.event.clone());
            }
        }
    }
}

struct PendingGuard {
    pending: Pending,
    id: String,
}
impl Drop for PendingGuard {
    fn drop(&mut self) {
        self.pending.lock().unwrap().remove(&self.id);
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
        fn poll_ready(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
        ) -> Poll<std::result::Result<(), ()>> {
            Poll::Pending
        }
        fn start_send(self: Pin<&mut Self>, _: Message) -> std::result::Result<(), ()> {
            Ok(())
        }
        fn poll_flush(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
        ) -> Poll<std::result::Result<(), ()>> {
            Poll::Pending
        }
        fn poll_close(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
        ) -> Poll<std::result::Result<(), ()>> {
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

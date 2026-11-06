use crate::registry::{Connector, Registry};
use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use store_protocol::{crypto, StoreError, StoreReply, StoreRequest};
use tokio::sync::{mpsc, oneshot, watch};
use uuid::Uuid;

type Result<T> = std::result::Result<T, StoreError>;
type Pending = Arc<Mutex<HashMap<String, oneshot::Sender<Result<StoreReply>>>>>;

struct Connection {
    generation: Uuid,
    sender: mpsc::Sender<StoreRequest>,
    close: watch::Sender<bool>,
    screenshot_wake: watch::Sender<()>,
    pending: Pending,
}

pub struct Transport {
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
            client,
            connections: Mutex::new(HashMap::new()),
            locks: Mutex::new(HashMap::new()),
            timeout,
        })
    }

    pub fn connected(&self, id: &str) -> bool {
        self.connections.lock().unwrap().contains_key(id)
    }

    pub fn disconnect(&self, id: &str) {
        if let Some(connection) = self.connections.lock().unwrap().remove(id) {
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
            let (sender, receiver) = oneshot::channel();
            let id = request.request_id.clone();
            let pending = {
                let connections = self.connections.lock().unwrap();
                let connection = connections
                    .get(&node.connector_id)
                    .ok_or_else(disconnected)?;
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
            Ok((channel, endpoint))
        })
        .await;
        let Ok(Ok((mut channel, endpoint))) = authenticated else {
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
                return;
            }
            if register(endpoint).is_err() {
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
                },
            );
        }
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
                    if last_seen.elapsed() > Duration::from_secs(35) || !bounded_send(&mut write,Message::Ping(Vec::new().into())).await {break;}
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
                    if !pending.lock().unwrap().contains_key(&request.request_id) {continue;}
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
                            let plain=match channel.decrypt(&bytes) {Ok(Some(plain))=>plain,Ok(None)=>continue,Err(_)=>break};
                            let Ok(reply) = serde_json::from_slice::<StoreReply>(&plain) else {break;};
                            if !(200..=599).contains(&reply.status) {break;}
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

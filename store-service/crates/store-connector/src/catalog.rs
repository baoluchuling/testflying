use super::*;
use store_protocol::catalog::{CatalogReport, CatalogStatus};
use tokio::task::{JoinHandle, JoinSet};

const MIN_INTERVAL: Duration = Duration::from_secs(60);
const REFRESH_INTERVAL: Duration = Duration::from_secs(3600);
const PLATFORMS: [Platform; 2] = [Platform::Ios, Platform::Android];

#[derive(Default)]
pub(super) struct CatalogCache {
    last_started: [Option<tokio::time::Instant>; 2],
    reports: [Option<CatalogReport>; 2],
}
// Dropping a connection also cancels its scheduler and all scoped discovery tasks.
pub(super) struct Session(JoinHandle<()>);
impl Drop for Session {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn reply(report: CatalogReport) -> StoreReply {
    StoreReply {
        request_id: report.report_id.clone(),
        status: 200,
        body: json!({"accountAppCatalog": report}),
    }
}
async fn discover(executor: Arc<Executor>, platform: Platform) -> CatalogReport {
    let mut report = CatalogReport {
        schema_version: 1,
        report_id: uuid::Uuid::new_v4().to_string(),
        platform,
        status: CatalogStatus::Unsupported,
        account_fingerprint: None,
        apps: Vec::new(),
        error_code: Some("catalog_unsupported".into()),
    };
    if executor.config.platform_mode.supports(platform) {
        match executor.gateway.account_catalog(platform).await {
            Ok(catalog) => {
                report.status = CatalogStatus::Ready;
                report.account_fingerprint = Some(catalog.fingerprint);
                report.apps = catalog.apps;
                report.error_code = None;
            }
            Err(error) => {
                let (status, code) = match error.code.as_str() {
                    "catalog_not_configured" => {
                        (CatalogStatus::NotConfigured, "catalog_not_configured")
                    }
                    "catalog_unsupported" => (CatalogStatus::Unsupported, "catalog_unsupported"),
                    _ => (CatalogStatus::Failed, "catalog_discovery_failed"),
                };
                report.status = status;
                report.error_code = Some(code.into());
            }
        }
    }
    // Revalidate at the final publication boundary; never publish partial or oversized data.
    if report.validate().is_err() {
        report.status = CatalogStatus::Failed;
        report.apps.clear();
        report.account_fingerprint = None;
        report.error_code = Some("catalog_discovery_failed".into());
    }
    report
}
pub(super) fn start(
    executor: Arc<Executor>,
) -> (mpsc::Sender<()>, mpsc::Receiver<StoreReply>, Session) {
    let (refresh, mut requests) = mpsc::channel(1);
    let (reports, receiver) = mpsc::channel(2);
    let task = tokio::spawn(async move {
        let mut jobs = JoinSet::new();
        let mut pending = [true; 2];
        let mut running = [false; 2];
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        // On fast reconnect replay the bounded cached report, while a fresh query waits for the interval.
        let cached = executor.catalog.lock().await.reports.clone();
        for report in cached.into_iter().flatten() {
            if reports.send(reply(report)).await.is_err() {
                return;
            }
        }
        loop {
            tokio::select! {
                request = requests.recv() => { if request.is_none() { return; } for i in 0..2 { if !running[i] { pending[i] = true; } } }
                _ = tick.tick() => {
                    let mut cache = executor.catalog.lock().await;
                    for i in 0..2 {
                        if cache.last_started[i].is_none_or(|at| at.elapsed() >= REFRESH_INTERVAL) { pending[i] = true; }
                        if pending[i] && !running[i] && cache.last_started[i].is_none_or(|at| at.elapsed() >= MIN_INTERVAL) {
                            cache.last_started[i] = Some(tokio::time::Instant::now()); pending[i] = false; running[i] = true;
                            let executor = executor.clone(); jobs.spawn(async move { (i, discover(executor, PLATFORMS[i]).await) });
                        }
                    }
                }
                result = jobs.join_next(), if !jobs.is_empty() => {
                    let Some(Ok((i, report))) = result else { return; };
                    running[i] = false;
                    executor.catalog.lock().await.reports[i] = Some(report.clone());
                    if let Some(log) = &executor.log { log.event("catalog.completed", json!({"platform":report.platform,"status":report.status,"errorCode":report.error_code})); }
                    if reports.send(reply(report)).await.is_err() { return; }
                }
            }
        }
    });
    (refresh, receiver, Session(task))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn executor() -> Arc<Executor> {
        Executor::new(serde_json::from_value(json!({"connectorId":"catalog-fixture","token":"fixture-token-0123456789",
            "messageKey":"01".repeat(32),"uploadKey":"02".repeat(32),"serverUrl":"http://127.0.0.1:1",
            "platformMode":"android","provider":{"mode":"mock"}})).unwrap()).unwrap()
    }
    #[tokio::test(start_paused = true)]
    async fn catalog_coalesces_manual_refresh_and_replays_cache_on_reconnect() {
        let executor = executor();
        let (refresh, mut reports, session) = start(executor.clone());
        let first = reports.recv().await.unwrap();
        let _ = reports.recv().await.unwrap();
        for _ in 0..100 {
            let _ = refresh.try_send(());
        }
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(59)).await;
        tokio::task::yield_now().await;
        assert!(reports.try_recv().is_err());
        tokio::time::advance(Duration::from_secs(1)).await;
        let refreshed = reports.recv().await.unwrap();
        let refreshed_other = reports.recv().await.unwrap();
        assert_ne!(first.request_id, refreshed.request_id);
        drop(session);
        drop(refresh);
        let (_refresh, mut replay, _session) = start(executor.clone());
        let replayed = replay.recv().await.unwrap();
        let replayed_other = replay.recv().await.unwrap();
        let mut expected = vec![refreshed.request_id, refreshed_other.request_id];
        expected.sort();
        let mut actual = vec![replayed.request_id.clone(), replayed_other.request_id];
        actual.sort();
        assert_eq!(actual, expected);
        assert_eq!(replayed.status, 200);
        assert!(replayed.body["accountAppCatalog"]["apps"]
            .as_array()
            .unwrap()
            .is_empty());
    }
    #[tokio::test]
    async fn catalog_does_not_use_business_capacity_and_session_cancels() {
        let executor = executor();
        let _busy = executor.capacity.clone().acquire_owned().await.unwrap();
        let (refresh, mut reports, session) = start(executor.clone());
        let report = tokio::time::timeout(Duration::from_secs(1), reports.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(report.body["accountAppCatalog"]["status"], "unsupported");
        drop(session);
        drop(refresh);
        while reports.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn encrypted_websocket_heartbeat_business_and_manual_ack_continue_during_catalog_wait() {
        use axum::{extract::Query, routing::get};
        use p256::pkcs8::{EncodePrivateKey, LineEnding};
        use std::collections::HashMap;
        let directory = tempfile::tempdir().unwrap();
        let key_path = directory.path().join("synthetic-apple.p8");
        let key = p256::SecretKey::from_slice(&[7; 32]).unwrap();
        std::fs::write(
            &key_path,
            key.to_pkcs8_pem(LineEnding::LF).unwrap().as_bytes(),
        )
        .unwrap();
        let started = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let (arrived, resume) = (started.clone(), release.clone());
        let fake_http = Router::new().route("/v1/apps", get(move |Query(query): Query<HashMap<String,String>>| {
            let (arrived, resume) = (arrived.clone(), resume.clone());
            async move {
                if !query.contains_key("filter[bundleId]") { arrived.notify_one(); resume.notified().await; }
                Json(json!({"data":[{"id":"123","attributes":{"bundleId":"com.example.app","name":"Example"}}]}))
            }
        })).fallback(|| async { Json(json!({"data":[]})) });
        let http_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let http_url = format!("http://{}", http_listener.local_addr().unwrap());
        let _http_task = Session(tokio::spawn(async move {
            axum::serve(http_listener, fake_http).await.unwrap();
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config: ConnectorConfig = serde_json::from_value(json!({"connectorId":"catalog-ws","token":"fixture-token-0123456789",
            "messageKey":"01".repeat(32),"uploadKey":"02".repeat(32),"serverUrl":format!("http://{}",listener.local_addr().unwrap()),
            "platformMode":"ios","provider":{"appleIssuerId":"fixture-issuer","appleKeyId":"fixture-key","applePrivateKeyPath":key_path,"appleBaseUrl":http_url}})).unwrap();
        let executor = Executor::new(config).unwrap();
        let (shutdown, shutdown_rx) = watch::channel(false);
        let node = tokio::spawn(async move {
            let _ = connection(executor, shutdown_rx).await;
        });
        let _node_abort = Session(node);
        tokio::time::timeout(Duration::from_secs(5), async {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_hdr_async(stream,
                |_: &tokio_tungstenite::tungstenite::handshake::server::Request, mut response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                    response.headers_mut().insert(header::SEC_WEBSOCKET_PROTOCOL, crypto::SUBPROTOCOL.parse().unwrap()); Ok(response)
                }).await.unwrap();
            let mut handshake = crypto::handshake(&"01".repeat(32), "catalog-ws", false).unwrap();
            let Message::Binary(registration) = socket.next().await.unwrap().unwrap() else { panic!("registration"); };
            let mut buffer = vec![0; crypto::HANDSHAKE_BYTES];
            let size = handshake.read_message(&registration, &mut buffer).unwrap();
            let registration: crypto::Registration = serde_json::from_slice(&buffer[..size]).unwrap();
            assert_eq!(registration.app_catalog_version, Some(1));
            let size = handshake.write_message(&[], &mut buffer).unwrap();
            socket.send(Message::Binary(buffer[..size].to_vec().into())).await.unwrap();
            let mut channel = crypto::Channel::new(handshake).unwrap();
            let Message::Binary(confirm) = socket.next().await.unwrap().unwrap() else { panic!("confirm"); };
            assert_eq!(channel.decrypt(&confirm).unwrap().unwrap(), b"confirm");
            for record in channel.encrypt(b"ready").unwrap() { socket.send(Message::Binary(record.into())).await.unwrap(); }
            started.notified().await;
            socket.send(Message::Ping(b"while-catalog-waits".to_vec().into())).await.unwrap();
            for command in [json!({"requestId":"refresh-test","catalogAction":"refresh"}), json!({"requestId":"business-test","platform":"ios","bundleIdentifier":"com.example.app","operation":"store_reviews","payload":{}})] {
                for record in channel.encrypt(&serde_json::to_vec(&command).unwrap()).unwrap() { socket.send(Message::Binary(record.into())).await.unwrap(); }
            }
            let (mut heartbeat, mut business, mut acknowledged) = (false, false, false);
            while !(heartbeat && business && acknowledged) {
                match socket.next().await.unwrap().unwrap() {
                    Message::Ping(bytes) => { socket.send(Message::Pong(bytes)).await.unwrap(); }
                    Message::Pong(bytes) => { assert_eq!(bytes.as_ref(), b"while-catalog-waits"); heartbeat = true; }
                    Message::Binary(bytes) => {
                        let Some(plain) = channel.decrypt(&bytes).unwrap() else { continue; };
                        let reply: StoreReply = serde_json::from_slice(&plain).unwrap();
                        match reply.request_id.as_str() {
                            "refresh-test" => { assert_eq!(reply.status, 202); acknowledged = true; }
                            "business-test" => { assert_eq!(reply.status, 200); business = true; }
                            _ => { assert_ne!(reply.body["accountAppCatalog"]["platform"], "ios", "no partial catalog while first page waits"); }
                        }
                    }
                    _ => {}
                }
            }
            release.notify_one();
            loop {
                match socket.next().await.unwrap().unwrap() {
                    Message::Ping(bytes) => { socket.send(Message::Pong(bytes)).await.unwrap(); }
                    Message::Binary(bytes) => {
                        let Some(plain) = channel.decrypt(&bytes).unwrap() else { continue; };
                        let reply: StoreReply = serde_json::from_slice(&plain).unwrap();
                        if reply.body["accountAppCatalog"]["platform"] == "ios" {
                            let report: CatalogReport = serde_json::from_value(reply.body["accountAppCatalog"].clone()).unwrap();
                            report.validate().unwrap(); assert_eq!(report.status, CatalogStatus::Ready); assert_eq!(report.apps.len(), 1); break;
                        }
                    }
                    _ => {}
                }
            }
        }).await.unwrap();
        shutdown.send(true).unwrap();
    }
}

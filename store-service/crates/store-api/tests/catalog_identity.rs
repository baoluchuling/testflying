//! Real provider -> encrypted connector report -> center ingestion, with only loopback upstreams.
use axum::{
    body::Body,
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    http::{Request, StatusCode, Uri},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use http_body_util::BodyExt;
use p256::pkcs8::{EncodePrivateKey, LineEnding};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::Arc, time::Duration};
use store_api::{
    registry::{ConnectorInput, ConnectorKind, Registry},
    ApiState,
};
use store_connector::{ConnectorConfig, Executor};
use store_protocol::{
    catalog::{CatalogReport, CatalogStatus},
    crypto, Platform, StoreReply,
};
use store_provider::ProviderConfig;
use tokio::sync::{mpsc, watch};
use tower::ServiceExt;

const APP: &str = "com.example.retained";
const ADMIN: &str = "catalog-identity-admin-token";

// Every fixture task is scoped even if an assertion/timeout panics.
struct Task(tokio::task::JoinHandle<()>);
impl Drop for Task {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn receive_reports(mut socket: WebSocket, sender: mpsc::Sender<CatalogReport>) {
    let mut handshake = crypto::handshake(&"01".repeat(32), "catalog-identity", false).unwrap();
    let Message::Binary(registration) = socket.recv().await.unwrap().unwrap() else {
        panic!("registration");
    };
    let mut buffer = vec![0; crypto::HANDSHAKE_BYTES];
    let size = handshake.read_message(&registration, &mut buffer).unwrap();
    let registration: crypto::Registration = serde_json::from_slice(&buffer[..size]).unwrap();
    assert_eq!(registration.app_catalog_version, Some(1));
    let size = handshake.write_message(&[], &mut buffer).unwrap();
    socket
        .send(Message::Binary(buffer[..size].to_vec().into()))
        .await
        .unwrap();
    let mut channel = crypto::Channel::new(handshake).unwrap();
    let Message::Binary(confirm) = socket.recv().await.unwrap().unwrap() else {
        panic!("confirm");
    };
    assert_eq!(channel.decrypt(&confirm).unwrap().unwrap(), b"confirm");
    for record in channel.encrypt(b"ready").unwrap() {
        socket.send(Message::Binary(record.into())).await.unwrap();
    }
    while let Some(Ok(message)) = socket.recv().await {
        match message {
            Message::Ping(bytes) => {
                if socket.send(Message::Pong(bytes)).await.is_err() {
                    return;
                }
            }
            Message::Binary(bytes) => {
                let Some(plain) = channel.decrypt(&bytes).unwrap() else {
                    continue;
                };
                let reply: StoreReply = serde_json::from_slice(&plain).unwrap();
                assert_eq!(reply.status, 200);
                let report: CatalogReport =
                    serde_json::from_value(reply.body["accountAppCatalog"].clone()).unwrap();
                assert_eq!(reply.request_id, report.report_id);
                report.validate().unwrap();
                if sender.send(report).await.is_err() {
                    return;
                }
            }
            _ => {}
        }
    }
}

async fn connector_report(
    mut provider: ProviderConfig,
    platform: Platform,
    failure: Option<&'static str>,
) -> CatalogReport {
    let (sender, mut reports) = mpsc::channel(2);
    let app = Router::new()
        .route(
            "/v1/connectors/catalog-identity/ws",
            get(move |ws: WebSocketUpgrade| {
                let sender = sender.clone();
                async move {
                    ws.protocols([crypto::SUBPROTOCOL])
                        .on_upgrade(move |socket| receive_reports(socket, sender))
                }
            }),
        )
        .fallback(move |uri: Uri| async move {
            if uri.path() == "/token" {
                return if failure == Some("oauth") {
                    (StatusCode::FORBIDDEN, "synthetic-private-oauth-error").into_response()
                } else {
                    Json(json!({"access_token":"synthetic-reporting-token","expires_in":3600}))
                        .into_response()
                };
            }
            assert_eq!(
                uri.path(),
                if platform == Platform::Ios {
                    "/v1/apps"
                } else {
                    "/v1beta1/apps:search"
                }
            );
            if failure == Some("reporting")
                || (failure == Some("second_page") && uri.query() == Some("cursor=two"))
            {
                return (StatusCode::FORBIDDEN, "synthetic-private-provider-error").into_response();
            }
            let name = if failure == Some("invalid_catalog") {
                "x".repeat(513)
            } else {
                "Retained application".into()
            };
            Json(match platform {
                Platform::Ios => json!({
                    "data":[{"id":"123","attributes":{"bundleId":APP,"name":name}}],
                    "links":{"next":if failure == Some("second_page") {
                        Some("/v1/apps?cursor=two")
                    } else { None }}
                }),
                Platform::Android => json!({
                    "apps":[{"name":"apps/123","packageName":APP,"displayName":name}]
                }),
            })
            .into_response()
        });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let _server = Task(tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    }));
    provider.apple_base_url = url.clone();
    provider.google_reporting_base_url = url.clone();
    provider.google_token_url = format!("{url}/token");
    let config: ConnectorConfig = serde_json::from_value(json!({
        "connectorId":"catalog-identity","token":"catalog-fixture-token-0123456789",
        "messageKey":"01".repeat(32),"uploadKey":"02".repeat(32),
        "serverUrl":url,"platformMode":platform.to_string(),"provider":provider
    }))
    .unwrap();
    let executor = Executor::new(config).unwrap();
    let (stop, stopped) = watch::channel(false);
    let connector = Task(tokio::spawn(store_connector::run_active(executor, stopped)));
    let report = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let report = reports.recv().await.unwrap();
            if report.platform == platform {
                break report;
            }
        }
    })
    .await
    .unwrap();
    stop.send(true).unwrap();
    drop(connector);
    report
}

async fn screenshot_route(state: Arc<ApiState>, platform: Platform) -> (StatusCode, Value) {
    let response = store_api::router(state)
        .oneshot(
            Request::builder()
                .uri(format!("/v1/apps/{platform}/{APP}/screenshot-connector"))
                .header("authorization", format!("Bearer {ADMIN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn failed_connector_discovery_retains_same_identity_but_blocks_changed_credentials() {
    let keys = tempfile::tempdir().unwrap();
    let apple = p256::SecretKey::from_slice(&[7; 32]).unwrap();
    let apple_path = keys.path().join("synthetic-apple.p8");
    std::fs::write(
        &apple_path,
        apple.to_pkcs8_pem(LineEnding::LF).unwrap().as_bytes(),
    )
    .unwrap();
    let rsa = rsa::RsaPrivateKey::new(&mut rand::thread_rng(), 2048).unwrap();
    let google_key = rsa.to_pkcs8_pem(LineEnding::LF).unwrap();
    for (platform, failure) in [
        (Platform::Android, "reporting"),
        (Platform::Android, "oauth"),
        (Platform::Ios, "second_page"),
        (Platform::Ios, "invalid_catalog"),
    ] {
        let google_path = keys.path().join("synthetic-google.json");
        let provider = |identity: &str| {
            std::fs::write(
                &google_path,
                serde_json::to_vec(&json!({
                    "client_email":identity,"private_key":google_key.as_str()
                }))
                .unwrap(),
            )
            .unwrap();
            ProviderConfig {
                apple_issuer_id: Some(identity.into()),
                apple_key_id: Some("synthetic-key".into()),
                apple_private_key_path: Some(apple_path.to_string_lossy().into_owned()),
                google_service_account_path: Some(google_path.to_string_lossy().into_owned()),
                ..Default::default()
            }
        };
        let state = ApiState::new(
            Registry::open(":memory:", "catalog-identity-signing-key-0123456789").unwrap(),
            ADMIN.into(),
            "catalog-identity-client-token".into(),
            "https://center.invalid".into(),
            PathBuf::from("unused"),
            Duration::from_secs(1),
        )
        .unwrap();
        let r = &state.registry;
        let node = r
            .create(ConnectorInput {
                name: "Store".into(),
            })
            .unwrap();
        let token = r.token(&node);
        let ready = connector_report(provider("a@example.test"), platform, None).await;
        assert_eq!(ready.status, CatalogStatus::Ready);
        r.ingest_catalog(&node.connector_id, &token, true, &ready)
            .unwrap();
        let account = r.resolve_account(platform, APP).unwrap();
        let mac = r
            .create_with_kind(
                ConnectorInput { name: "Mac".into() },
                ConnectorKind::Screenshot,
            )
            .unwrap();
        r.patch_account(
            &account.id,
            serde_json::from_value(json!({
                "screenshotConnectorId":mac.connector_id
            }))
            .unwrap(),
        )
        .unwrap();
        let same = connector_report(provider("a@example.test"), platform, Some(failure)).await;
        assert_eq!(same.status, CatalogStatus::Failed, "{platform}/{failure}");
        assert_eq!(same.account_fingerprint, ready.account_fingerprint);
        assert!(same.apps.is_empty());
        r.ingest_catalog(&node.connector_id, &token, true, &same)
            .unwrap();
        assert_eq!(r.account(&account.id).unwrap().catalog_status, "failed");
        assert_eq!(
            r.resolve(platform, APP).unwrap().connector_id,
            node.connector_id
        );
        assert_eq!(
            screenshot_route(state.clone(), platform).await.0,
            StatusCode::OK
        );

        let changed = connector_report(provider("b@example.test"), platform, Some(failure)).await;
        assert_eq!(changed.status, CatalogStatus::Failed);
        assert_eq!(
            changed.account_fingerprint,
            Some(hex::encode(Sha256::digest(
                format!("{platform}:b@example.test").as_bytes()
            )))
        );
        assert_ne!(changed.account_fingerprint, ready.account_fingerprint);
        assert!(
            changed.apps.is_empty(),
            "no partially enumerated apps may escape"
        );
        assert_eq!(
            changed.error_code.as_deref(),
            Some("catalog_discovery_failed")
        );
        let wire = serde_json::to_string(&changed).unwrap();
        for secret in [
            "a@example.test",
            "b@example.test",
            "PRIVATE KEY",
            "synthetic-private",
            "synthetic-reporting-token",
        ] {
            assert!(!wire.contains(secret));
        }
        r.ingest_catalog(&node.connector_id, &token, true, &changed)
            .unwrap();
        // Both replayed and newly received old-identity failures must leave the barrier intact.
        let mut old_failure = same.clone();
        old_failure.report_id = uuid::Uuid::new_v4().to_string();
        let mut unknown_failure = same.clone();
        unknown_failure.report_id = uuid::Uuid::new_v4().to_string();
        unknown_failure.account_fingerprint = None;
        for stale in [&same, &ready, &changed, &old_failure, &unknown_failure] {
            r.ingest_catalog(&node.connector_id, &token, true, stale)
                .unwrap();
            let retained = r.account(&account.id).unwrap();
            assert_eq!(retained.catalog_status, "identity_changed");
            assert_eq!(retained.last_catalog_at, account.last_catalog_at);
            let apps = r.account_apps(&account.id).unwrap();
            assert_eq!(apps.len(), 1);
            assert_eq!(apps[0]["bundleIdentifier"], APP);
            assert_eq!(apps[0]["name"], "Retained application");
            assert_eq!(
                r.resolve(platform, APP).unwrap_err().code,
                "identity_changed"
            );
            let (status, body) = screenshot_route(state.clone(), platform).await;
            assert_eq!(status, StatusCode::CONFLICT);
            assert_eq!(body["error"]["code"], "identity_changed");
        }
        r.patch_account(
            &account.id,
            serde_json::from_value(json!({
                "acceptIdentityChange":true
            }))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            r.account(&account.id).unwrap().catalog_status,
            "awaiting_catalog"
        );
        assert!(r.account_apps(&account.id).unwrap().is_empty());
        let fresh = connector_report(provider("b@example.test"), platform, None).await;
        r.ingest_catalog(&node.connector_id, &token, true, &fresh)
            .unwrap();
        assert_eq!(
            r.resolve(platform, APP).unwrap().connector_id,
            node.connector_id
        );
        assert_eq!(
            screenshot_route(state.clone(), platform).await.0,
            StatusCode::OK
        );
    }
}

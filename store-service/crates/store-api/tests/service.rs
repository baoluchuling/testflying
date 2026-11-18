mod support;
use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use futures_util::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc, time::Duration};
use store_api::{
    registry::{ConnectorInput, Registry},
    ApiState,
};
use store_connector::{ConnectorConfig, Executor};
use store_protocol::{crypto, Operation, Platform, StoreReply, StoreRequest};
use tokio::net::TcpListener;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};
use tower::ServiceExt;

const ADMIN: &str = "admin-test-token-0123456789";
const CLIENT: &str = "client-test-token-0123456789";
const SIGNING: &str = "signing-key-for-local-test-only-0123456789";

fn state(timeout: Duration) -> Arc<ApiState> {
    ApiState::new(
        Registry::open(":memory:", SIGNING).unwrap(),
        ADMIN.into(),
        CLIENT.into(),
        "http://127.0.0.1:8200".into(),
        PathBuf::from("nonexistent-test-releases"),
        timeout,
    )
    .unwrap()
}
async fn call(
    app: Router,
    method: &str,
    path: &str,
    token: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
async fn serve(app: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (base, handle)
}
fn input() -> ConnectorInput {
    ConnectorInput {
        name: "Account A".into(),
    }
}
fn command(id: &str) -> StoreRequest {
    StoreRequest {
        request_id: id.into(),
        platform: Platform::Ios,
        bundle_identifier: "com.example.app".into(),
        operation: Operation::SupportedLocales,
        payload: json!({}),
    }
}

#[test]
fn registry_persists_many_apps_exact_platform_and_rotated_identity() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("registry.sqlite");
    let registry = Registry::open(&path, SIGNING).unwrap();
    let first = registry.create(input()).unwrap();
    let second = registry.create(input()).unwrap();
    let token = registry.token(&first);
    support::catalog(
        &registry,
        &first.connector_id,
        Platform::Ios,
        &["com.example.app", "com.example.second"],
    );
    support::catalog(
        &registry,
        &second.connector_id,
        Platform::Android,
        &["com.example.app"],
    );
    registry
        .bind(Platform::Ios, "com.example.app", &first.connector_id)
        .unwrap();
    registry
        .bind(Platform::Ios, "com.example.second", &first.connector_id)
        .unwrap();
    registry
        .bind(Platform::Android, "com.example.app", &second.connector_id)
        .unwrap();
    assert_eq!(registry.get(&first.connector_id).unwrap().app_count, 2);
    assert!(registry.resolve(Platform::Ios, "com.Example.app").is_err());
    assert_eq!(
        registry
            .resolve(Platform::Android, "com.example.app")
            .unwrap()
            .connector_id,
        second.connector_id
    );
    assert!(!serde_json::to_string(&registry.list().unwrap())
        .unwrap()
        .contains(&token));
    drop(registry);
    let registry = Registry::open(&path, SIGNING).unwrap();
    assert_eq!(
        registry
            .resolve(Platform::Ios, "com.example.second")
            .unwrap()
            .connector_id,
        first.connector_id
    );
    assert!(registry.authenticate(&first.connector_id, &token).is_ok());
    registry.rotate(&first.connector_id).unwrap();
    assert!(registry.authenticate(&first.connector_id, &token).is_err());
    registry.set_enabled(&first.connector_id, false).unwrap();
    assert_eq!(
        registry
            .resolve(Platform::Ios, "com.example.app")
            .unwrap_err()
            .code,
        "connector_disabled"
    );
    assert_eq!(
        registry
            .unbind(Platform::Ios, "com.example.app")
            .unwrap_err()
            .status,
        410
    );
    assert_eq!(
        registry
            .resolve(Platform::Ios, "com.example.app")
            .unwrap_err()
            .code,
        "connector_disabled"
    );
}

#[tokio::test]
async fn admin_api_provisions_authenticates_and_hides_tokens() {
    let state = state(Duration::from_secs(1));
    let app = store_api::router(state.clone());
    assert_eq!(
        call(
            app.clone(),
            "POST",
            "/v1/connectors",
            CLIENT,
            json!({"name":"A"})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let (status, created) = call(
        app.clone(),
        "POST",
        "/v1/connectors",
        ADMIN,
        json!({"name":"A"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = created["connector"]["connectorId"].as_str().unwrap();
    let token = created["installation"]["token"].as_str().unwrap();
    assert!(state.registry.authenticate(id, token).is_ok());
    let (_, listed) = call(app.clone(), "GET", "/v1/connectors", ADMIN, Value::Null).await;
    assert!(!listed.to_string().contains(token));
    assert!(!listed.to_string().contains("nonce"));
    let path = "/v1/apps/ios/com.example.app/connector";
    support::catalog(&state.registry, id, Platform::Ios, &["com.example.app"]);
    assert_eq!(
        call(app.clone(), "PUT", path, ADMIN, json!({"connectorId":id}))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        call(app.clone(), "GET", path, ADMIN, Value::Null).await.1["connectorId"],
        id
    );
    assert_eq!(
        call(
            app.clone(),
            "GET",
            "/v1/store-management/apps/ios/com.example.app/supported-locales",
            CLIENT,
            Value::Null
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        call(
            app.clone(),
            "GET",
            "/v1/store-management/apps/ios/missing/supported-locales",
            "bad",
            Value::Null
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    for legacy in [
        "metadata-content-sets",
        "store-versions/1.0/draft",
        "marketing-pages",
    ] {
        let path = format!("/v1/store-management/apps/ios/com.example.app/{legacy}");
        assert_eq!(
            call(app.clone(), "POST", &path, CLIENT, json!({})).await.0,
            StatusCode::NOT_FOUND
        );
    }
    let (status, _) = call(
        app.clone(),
        "GET",
        &format!("/v1/connectors/{id}/installer/../../etc/passwd"),
        ADMIN,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, rotated) = call(
        app.clone(),
        "POST",
        &format!("/v1/connectors/{id}/rotate-token"),
        ADMIN,
        json!({}),
    )
    .await;
    assert_ne!(rotated["token"], token);
    assert!(state.registry.authenticate(id, token).is_err());
    assert_eq!(
        call(app, "DELETE", path, ADMIN, Value::Null).await.0,
        StatusCode::GONE
    );
}

#[tokio::test]
async fn http_connector_executes_shared_protocol_and_rejects_wrong_identity() {
    let config:ConnectorConfig=serde_json::from_value(json!({"connectorId":"node-http","token":"node-token-for-local-tests","serverUrl":"http://127.0.0.1:8200","messageKey":"11".repeat(32),"uploadKey":"22".repeat(32),"provider":{"mode":"mock"}})).unwrap();
    let executor = Executor::new(config).unwrap();
    let (base, server) = serve(store_connector::http_router(executor.clone())).await;
    let state = state(Duration::from_secs(1));
    let node = state.registry.create(input()).unwrap();
    state
        .registry
        .register_endpoint(
            &node.connector_id,
            &state.registry.token(&node),
            Some(&base),
        )
        .unwrap();
    let node = state.registry.get(&node.connector_id).unwrap();
    let reply = state
        .transport
        .dispatch(&node, "node-token-for-local-tests", command("http-1"))
        .await
        .unwrap();
    assert_eq!(reply.body["locales"][0], "en-US");
    assert_eq!(reply.request_id, "http-1");
    assert!(state
        .transport
        .dispatch(&node, "wrong", command("http-2"))
        .await
        .is_err());
    server.abort();
    let _ = server.await;
    executor.drain().await;
}

type RawSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
struct EncryptedSocket {
    raw: RawSocket,
    channel: crypto::Channel,
}
impl EncryptedSocket {
    async fn send(
        &mut self,
        message: Message,
    ) -> Result<(), tokio_tungstenite::tungstenite::Error> {
        if let Message::Text(text) = message {
            for record in self.channel.encrypt(text.as_bytes()).unwrap() {
                self.raw.send(Message::Binary(record.into())).await?;
            }
            Ok(())
        } else {
            self.raw.send(message).await
        }
    }
    async fn close(
        &mut self,
        frame: Option<tokio_tungstenite::tungstenite::protocol::CloseFrame>,
    ) -> Result<(), tokio_tungstenite::tungstenite::Error> {
        self.raw.close(frame).await
    }
}
async fn ws_connect(base: &str, id: &str, token: &str, key: &str) -> EncryptedSocket {
    ws_connect_catalog(base, id, token, key, false).await
}
async fn ws_connect_catalog(
    base: &str,
    id: &str,
    token: &str,
    key: &str,
    capable: bool,
) -> EncryptedSocket {
    let mut socket = ws_begin_registration(base, id, token, key, capable, None).await;
    confirm_registration(&mut socket).await;
    let Message::Binary(reply) = socket.raw.next().await.unwrap().unwrap() else {
        panic!("Expected confirmation");
    };
    assert_eq!(socket.channel.decrypt(&reply).unwrap().unwrap(), b"ready");
    socket
}
async fn ws_begin_registration(
    base: &str,
    id: &str,
    token: &str,
    key: &str,
    capable: bool,
    version: Option<&str>,
) -> EncryptedSocket {
    let mut request = format!("{}/v1/connectors/{id}/ws", base.replacen("http", "ws", 1))
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    request
        .headers_mut()
        .insert("x-forwarded-proto", "https".parse().unwrap());
    request.headers_mut().insert(
        "sec-websocket-protocol",
        crypto::SUBPROTOCOL.parse().unwrap(),
    );
    let mut raw = connect_async(request).await.unwrap().0;
    let mut handshake = crypto::handshake(key, id, true).unwrap();
    let mut buffer = vec![0; crypto::HANDSHAKE_BYTES];
    let mut registration =
        json!({"connectorId":id,"baseUrl":null,"appCatalogVersion":if capable {Some(1)}else{None}});
    if let Some(version) = version {
        registration["connectorVersion"] = json!(version);
    }
    let size = handshake
        .write_message(&serde_json::to_vec(&registration).unwrap(), &mut buffer)
        .unwrap();
    raw.send(Message::Binary(buffer[..size].to_vec().into()))
        .await
        .unwrap();
    let Message::Binary(reply) = raw.next().await.unwrap().unwrap() else {
        panic!("Expected handshake");
    };
    handshake.read_message(&reply, &mut buffer).unwrap();
    let channel = crypto::Channel::new(handshake).unwrap();
    EncryptedSocket { raw, channel }
}
async fn confirm_registration(socket: &mut EncryptedSocket) {
    for record in socket.channel.encrypt(b"confirm").unwrap() {
        socket
            .raw
            .send(Message::Binary(record.into()))
            .await
            .unwrap();
    }
}

async fn finish_registration(socket: &mut EncryptedSocket) {
    confirm_registration(socket).await;
    let Message::Binary(reply) = socket.raw.next().await.unwrap().unwrap() else {
        panic!("Expected ready");
    };
    assert_eq!(socket.channel.decrypt(&reply).unwrap().unwrap(), b"ready");
}

async fn wait_disconnected(state: &ApiState, id: &str) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while state.transport.connected(id) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn reported_versions_are_authenticated_persistent_and_legacy_reconnects_clear_them() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("registry.sqlite");
    let registry = Registry::open(&path, SIGNING).unwrap();
    let node = registry.create(input()).unwrap();
    let id = &node.connector_id;
    let token = registry.token(&node);
    assert!(registry.get(id).unwrap().connector_version.is_none());
    registry
        .register_connection(id, &token, Some("http://127.0.0.1:1234"), Some("0.10.0"))
        .unwrap();
    for (secret, version) in [
        ("wrong-token", "0.2.0"),
        (&token[..], "4294967296.0.0"),
        (&token[..], "0.02.0"),
    ] {
        assert!(registry
            .register_connection(id, secret, None, Some(version))
            .is_err());
        let persisted = registry.get(id).unwrap();
        assert_eq!(persisted.connector_version.as_deref(), Some("0.10.0"));
        assert_eq!(persisted.base_url.as_deref(), Some("http://127.0.0.1:1234"));
    }
    // Endpoint-only configuration never invents or replaces an authenticated version report.
    registry.register_endpoint(id, &token, None).unwrap();
    drop(registry);
    let reopened = Registry::open(&path, SIGNING).unwrap();
    assert_eq!(
        reopened.get(id).unwrap().connector_version.as_deref(),
        Some("0.10.0")
    );
    let state = ApiState::new(
        reopened,
        ADMIN.into(),
        CLIENT.into(),
        "http://127.0.0.1:8200".into(),
        PathBuf::from("nonexistent-test-releases"),
        Duration::from_secs(2),
    )
    .unwrap();
    let (_, view) = call(
        store_api::router(state.clone()),
        "GET",
        &format!("/v1/connectors/{id}"),
        ADMIN,
        Value::Null,
    )
    .await;
    assert_eq!(view["connected"], false);
    assert_eq!(view["connectorVersion"], "0.10.0");
    assert_eq!(view["versionStatus"], "newer");
    state
        .registry
        .register_connection(id, &token, None, None)
        .unwrap();
    assert!(state.registry.get(id).unwrap().connector_version.is_none());
}

#[tokio::test]
async fn encrypted_version_reports_require_confirmation_and_duplicate_or_stale_sessions_cannot_change_them(
) {
    let state = state(Duration::from_secs(2));
    let node = state.registry.create(input()).unwrap();
    let id = &node.connector_id;
    let token = state.registry.token(&node);
    let key = state.registry.purpose_key(&node, "message");
    let (base, server) = serve(store_api::router(state.clone())).await;
    let mut first = ws_begin_registration(&base, id, &token, &key, false, Some("0.1.0")).await;
    let mut duplicate = ws_begin_registration(&base, id, &token, &key, false, Some("0.10.0")).await;
    assert!(state.registry.get(id).unwrap().connector_version.is_none());
    assert!(!state.transport.connected(id));
    finish_registration(&mut first).await;
    confirm_registration(&mut duplicate).await;
    let rejected = tokio::time::timeout(Duration::from_secs(2), duplicate.raw.next())
        .await
        .unwrap();
    assert!(!matches!(rejected, Some(Ok(Message::Binary(_)))));
    assert_eq!(
        state.registry.get(id).unwrap().connector_version.as_deref(),
        Some("0.1.0")
    );
    first.close(None).await.unwrap();
    wait_disconnected(&state, id).await;
    let (_, view) = call(
        store_api::router(state.clone()),
        "GET",
        &format!("/v1/connectors/{id}"),
        ADMIN,
        Value::Null,
    )
    .await;
    assert_eq!(view["connected"], false);
    assert_eq!(view["connectorVersion"], "0.1.0");
    assert_eq!(
        view["latestConnectorVersion"],
        store_protocol::release::CONNECTOR_VERSION
    );
    assert_eq!(view["versionStatus"], "outdated");

    // A registration payload alone has no effect, even with valid current credentials.
    let mut incomplete = ws_begin_registration(&base, id, &token, &key, false, Some("0.3.0")).await;
    incomplete.close(None).await.unwrap();
    assert_eq!(
        state.registry.get(id).unwrap().connector_version.as_deref(),
        Some("0.1.0")
    );
    let mut stale = ws_begin_registration(&base, id, &token, &key, false, Some("0.4.0")).await;
    state.registry.rotate(id).unwrap();
    confirm_registration(&mut stale).await;
    let rejected = tokio::time::timeout(Duration::from_secs(2), stale.raw.next())
        .await
        .unwrap();
    assert!(!matches!(rejected, Some(Ok(Message::Binary(_)))));
    assert_eq!(
        state.registry.get(id).unwrap().connector_version.as_deref(),
        Some("0.1.0")
    );
    assert!(!state.transport.connected(id));
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn store_and_screenshot_node_views_compare_numeric_versions_and_accept_legacy_nodes() {
    let state = state(Duration::from_secs(2));
    let (base, server) = serve(store_api::router(state.clone())).await;
    for kind in [
        store_api::registry::ConnectorKind::Store,
        store_api::registry::ConnectorKind::Screenshot,
    ] {
        let node = state.registry.create_with_kind(input(), kind).unwrap();
        let id = &node.connector_id;
        let token = state.registry.token(&node);
        let key = state.registry.purpose_key(&node, "message");
        for (version, status) in [
            (Some("0.10.0"), "newer"),
            (Some(store_protocol::release::CONNECTOR_VERSION), "current"),
            (None, "unknown"),
        ] {
            let mut socket = ws_begin_registration(&base, id, &token, &key, false, version).await;
            finish_registration(&mut socket).await;
            let (_, view) = call(
                store_api::router(state.clone()),
                "GET",
                &format!("/v1/connectors/{id}"),
                ADMIN,
                Value::Null,
            )
            .await;
            assert_eq!(view["connected"], true);
            assert_eq!(view["connectorVersion"], json!(version));
            assert_eq!(view["versionStatus"], status);
            socket.close(None).await.unwrap();
            wait_disconnected(&state, id).await;
            assert_eq!(
                state.registry.get(id).unwrap().connector_version.as_deref(),
                version
            );
        }
    }
    server.abort();
    let _ = server.await;
}
async fn next_request(socket: &mut EncryptedSocket) -> StoreRequest {
    loop {
        match tokio::time::timeout(Duration::from_secs(2), socket.raw.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
        {
            Message::Binary(bytes) => {
                if let Some(plain) = socket.channel.decrypt(&bytes).unwrap() {
                    return serde_json::from_slice(&plain).unwrap();
                }
            }
            Message::Ping(bytes) => socket.send(Message::Pong(bytes)).await.unwrap(),
            _ => {}
        }
    }
}
async fn wait_connected(state: &ApiState, id: &str) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !state.transport.connected(id) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

async fn send_catalog(
    socket: &mut EncryptedSocket,
    report: &store_protocol::catalog::CatalogReport,
) {
    socket
        .send(Message::Text(
            serde_json::to_string(&StoreReply {
                request_id: report.report_id.clone(),
                status: 200,
                body: json!({"accountAppCatalog":report}),
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
}

async fn catalog_command(socket: &mut EncryptedSocket) -> store_protocol::catalog::CatalogRequest {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match socket.raw.next().await.unwrap().unwrap() {
                Message::Ping(bytes) => socket.raw.send(Message::Pong(bytes)).await.unwrap(),
                Message::Binary(bytes) => {
                    if let Some(plain) = socket.channel.decrypt(&bytes).unwrap() {
                        let request: store_protocol::logs::NodeRequest =
                            serde_json::from_slice(&plain).unwrap();
                        if let store_protocol::logs::NodeRequest::Catalog(request) = request {
                            return request;
                        } else {
                            panic!("Expected catalog refresh");
                        }
                    }
                }
                other => panic!("Unexpected catalog frame {other:?}"),
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn encrypted_catalog_reports_refresh_capabilities_and_business_pending_are_isolated() {
    let state = state(Duration::from_secs(3));
    let node = state.registry.create(input()).unwrap();
    let token = state.registry.token(&node);
    let key = state.registry.purpose_key(&node, "message");
    let account = state
        .registry
        .create_account(
            serde_json::from_value(
                json!({"name":"WSS account","platform":"ios","storeConnectorId":node.connector_id}),
            )
            .unwrap(),
        )
        .unwrap();
    let app = store_api::router(state.clone());
    let (base, server) = serve(app.clone()).await;
    let refresh = format!("/v1/accounts/{}/refresh", account.id);
    assert_eq!(
        call(app.clone(), "POST", &refresh, ADMIN, json!({}))
            .await
            .0,
        503
    );
    let mut old = ws_connect(&base, &node.connector_id, &token, &key).await;
    wait_connected(&state, &node.connector_id).await;
    let (status, body) = call(app.clone(), "POST", &refresh, ADMIN, json!({})).await;
    assert_eq!(status, 409);
    assert_eq!(body["error"]["code"], "catalog_upgrade_required");
    let mut report = store_protocol::catalog::CatalogReport {
        schema_version: 1,
        report_id: uuid::Uuid::new_v4().to_string(),
        platform: Platform::Ios,
        status: store_protocol::catalog::CatalogStatus::Ready,
        account_fingerprint: Some("a".repeat(64)),
        apps: vec![store_protocol::catalog::CatalogApp {
            bundle_identifier: "com.example.app".into(),
            name: "Verified WSS app".into(),
            store_app_id: Some("123".into()),
        }],
        error_code: None,
    };
    send_catalog(&mut old, &report).await;
    let (transport, copy, secret) = (state.transport.clone(), node.clone(), token.clone());
    let legacy = tokio::spawn(async move {
        transport
            .dispatch(&copy, &secret, command("legacy-business"))
            .await
    });
    let request = next_request(&mut old).await;
    old.send(Message::Text(
        serde_json::to_string(&StoreReply {
            request_id: request.request_id,
            status: 200,
            body: json!({"ok":true}),
        })
        .unwrap()
        .into(),
    ))
    .await
    .unwrap();
    assert_eq!(legacy.await.unwrap().unwrap().body["ok"], true);
    assert!(state.registry.account_apps(&account.id).unwrap().is_empty());
    old.close(None).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while state.transport.connected(&node.connector_id) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let mut socket = ws_connect_catalog(&base, &node.connector_id, &token, &key, true).await;
    wait_connected(&state, &node.connector_id).await;
    send_catalog(&mut socket, &report).await;
    tokio::time::timeout(Duration::from_secs(2), async {
        while state.registry.account_apps(&account.id).unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let business_id = uuid::Uuid::new_v4().to_string();
    let (transport, copy, secret, business) = (
        state.transport.clone(),
        node.clone(),
        token.clone(),
        command(&business_id),
    );
    let pending = tokio::spawn(async move { transport.dispatch(&copy, &secret, business).await });
    let request = next_request(&mut socket).await;
    assert_eq!(request.request_id, business_id);
    // Catalog refresh is dispatched even while the store business lock is held.
    let (status, accepted) = call(app.clone(), "POST", &refresh, ADMIN, json!({})).await;
    assert_eq!(status, 202);
    let refresh_request = catalog_command(&mut socket).await;
    assert_eq!(refresh_request.catalog_action, "refresh");
    assert_eq!(accepted["requestId"], refresh_request.request_id);
    // A valid unsolicited report using the pending request ID cannot complete the business request.
    report.report_id = business_id.clone();
    report.apps[0].name = "Updated catalog".into();
    send_catalog(&mut socket, &report).await;
    tokio::time::timeout(Duration::from_secs(2), async {
        while state.registry.account_apps(&account.id).unwrap()[0]["name"] != "Updated catalog" {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(!pending.is_finished());
    // Invalid directory content is safely discarded without closing the business channel.
    report.report_id = uuid::Uuid::new_v4().to_string();
    report.apps.push(report.apps[0].clone());
    send_catalog(&mut socket, &report).await;
    socket
        .send(Message::Text(
            serde_json::to_string(&StoreReply {
                request_id: business_id,
                status: 200,
                body: json!({"businessResult":true}),
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    assert_eq!(pending.await.unwrap().unwrap().body["businessResult"], true);
    assert!(state.transport.connected(&node.connector_id));
    assert_eq!(state.registry.account_apps(&account.id).unwrap().len(), 1);
    // Rotating credentials revokes catalog writes from the already-authenticated old session.
    state.registry.rotate(&node.connector_id).unwrap();
    report.report_id = uuid::Uuid::new_v4().to_string();
    report.apps.truncate(1);
    report.apps[0].name = "Must be rejected".into();
    send_catalog(&mut socket, &report).await;
    let (transport, copy, secret) = (state.transport.clone(), node.clone(), token.clone());
    let last = tokio::spawn(async move {
        transport
            .dispatch(&copy, &secret, command("after-rotation"))
            .await
    });
    let request = next_request(&mut socket).await;
    socket
        .send(Message::Text(
            serde_json::to_string(&StoreReply {
                request_id: request.request_id,
                status: 200,
                body: json!({}),
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    last.await.unwrap().unwrap();
    assert_eq!(
        state.registry.account_apps(&account.id).unwrap()[0]["name"],
        "Updated catalog"
    );
    socket.close(None).await.unwrap();
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn websocket_routes_multiple_apps_and_rejects_duplicate_nodes() {
    let state = state(Duration::from_secs(1));
    let node = state.registry.create(input()).unwrap();
    let token = state.registry.token(&node);
    for name in ["com.example.app", "com.example.other"] {
        support::catalog(
            &state.registry,
            &node.connector_id,
            Platform::Ios,
            &["com.example.app", "com.example.other"],
        );
        state
            .registry
            .bind(Platform::Ios, name, &node.connector_id)
            .unwrap();
    }
    let (base, server) = serve(store_api::router(state.clone())).await;
    let mut socket = ws_connect(
        &base,
        &node.connector_id,
        &token,
        &state.registry.purpose_key(&node, "message"),
    )
    .await;
    wait_connected(&state, &node.connector_id).await;
    assert!(!state.transport.logs_supported(&node.connector_id));
    let unsupported = state
        .transport
        .dispatch_logs(
            &node,
            &token,
            store_protocol::logs::LogRequest {
                request_id: "legacy-log-check".into(),
                action: store_protocol::logs::LogAction::Latest,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(unsupported.code, "logs_upgrade_required");
    let unsupported = state
        .transport
        .dispatch(
            &node,
            &token,
            StoreRequest {
                request_id: "legacy-version-check".into(),
                platform: Platform::Ios,
                bundle_identifier: "com.example.app".into(),
                operation: Operation::CreateStoreRelease,
                payload: json!({"versionString":"2.0"}),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(unsupported.code, "connector_upgrade_required");
    assert!(state.transport.connected(&node.connector_id));
    let mut duplicate = format!(
        "{}/v1/connectors/{}/ws",
        base.replacen("http", "ws", 1),
        node.connector_id
    )
    .into_client_request()
    .unwrap();
    duplicate
        .headers_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    duplicate.headers_mut().insert(
        "sec-websocket-protocol",
        crypto::SUBPROTOCOL.parse().unwrap(),
    );
    let error = connect_async(duplicate).await.unwrap_err();
    assert!(
        matches!(error,tokio_tungstenite::tungstenite::Error::Http(response) if response.status()==409)
    );
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    for name in ["com.example.app", "com.example.other"] {
        let request = client
            .get(format!(
                "{base}/v1/store-management/apps/ios/{name}/supported-locales"
            ))
            .bearer_auth(CLIENT);
        let pending =
            tokio::spawn(
                async move { request.send().await.unwrap().json::<Value>().await.unwrap() },
            );
        let received = next_request(&mut socket).await;
        assert_eq!(received.bundle_identifier, name);
        socket
            .send(Message::Text(
                serde_json::to_string(&StoreReply {
                    request_id: received.request_id,
                    status: 200,
                    body: json!({"locales":["en-US"]}),
                })
                .unwrap()
                .into(),
            ))
            .await
            .unwrap();
        assert_eq!(pending.await.unwrap()["locales"][0], "en-US");
    }
    let image_request = client
        .get(format!("{base}/v1/store-management/apps/ios/com.example.app/store-images?version=1.0&locale=ja&device=foldable"))
        .bearer_auth(CLIENT);
    let pending = tokio::spawn(async move {
        image_request
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap()
    });
    let received = next_request(&mut socket).await;
    assert_eq!(received.operation, Operation::StoreImages);
    assert_eq!(received.payload, json!({"locale":"ja","device":"foldable"}));
    let images =
        json!({"locales":[{"locale":"ja","images":{"foldable_screenshots":[{"id":"duo-image"}]}}]});
    socket
        .send(Message::Text(
            serde_json::to_string(&StoreReply {
                request_id: received.request_id,
                status: 200,
                body: images.clone(),
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    assert_eq!(pending.await.unwrap(), images);
    let (_, body) = call(
        store_api::router(state.clone()),
        "PATCH",
        &format!("/v1/connectors/{}", node.connector_id),
        ADMIN,
        json!({"enabled":false}),
    )
    .await;
    assert_eq!(body["enabled"], false);
    assert!(!state.transport.connected(&node.connector_id));
    state.transport.close_all();
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn timeout_busy_disconnect_and_reconnect_never_replay_commands() {
    let state = state(Duration::from_millis(150));
    let node = state.registry.create(input()).unwrap();
    let token = state.registry.token(&node);
    let (base, server) = serve(store_api::router(state.clone())).await;
    let mut socket = ws_connect(
        &base,
        &node.connector_id,
        &token,
        &state.registry.purpose_key(&node, "message"),
    )
    .await;
    wait_connected(&state, &node.connector_id).await;
    let (transport, copy, secret) = (state.transport.clone(), node.clone(), token.clone());
    let first = tokio::spawn(async move {
        transport
            .dispatch(&copy, &secret, command("old-request"))
            .await
    });
    assert_eq!(next_request(&mut socket).await.request_id, "old-request");
    assert_eq!(
        state
            .transport
            .dispatch(&node, &token, command("busy-request"))
            .await
            .unwrap_err()
            .code,
        "connector_busy"
    );
    assert_eq!(first.await.unwrap().unwrap_err().code, "connector_timeout");
    // Late results cannot satisfy another request.
    socket
        .send(Message::Text(
            serde_json::to_string(&StoreReply {
                request_id: "old-request".into(),
                status: 200,
                body: json!({"stale":true}),
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    let (transport, copy, secret) = (state.transport.clone(), node.clone(), token.clone());
    let pending = tokio::spawn(async move {
        transport
            .dispatch(&copy, &secret, command("disconnect-request"))
            .await
    });
    next_request(&mut socket).await;
    socket.close(None).await.unwrap();
    assert_eq!(
        pending.await.unwrap().unwrap_err().code,
        "connector_disconnected"
    );
    tokio::time::timeout(Duration::from_secs(2), async {
        while state.transport.connected(&node.connector_id) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let mut reconnected = ws_connect(
        &base,
        &node.connector_id,
        &token,
        &state.registry.purpose_key(&node, "message"),
    )
    .await;
    wait_connected(&state, &node.connector_id).await;
    let (transport, copy, secret) = (state.transport.clone(), node.clone(), token.clone());
    let fresh = tokio::spawn(async move {
        transport
            .dispatch(&copy, &secret, command("fresh-request"))
            .await
    });
    let received = next_request(&mut reconnected).await;
    assert_eq!(received.request_id, "fresh-request");
    reconnected
        .send(Message::Text(
            serde_json::to_string(&StoreReply {
                request_id: received.request_id,
                status: 200,
                body: json!({"fresh":true}),
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    assert_eq!(fresh.await.unwrap().unwrap().body, json!({"fresh":true}));
    state.transport.close_all();
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn latest_node_log_streams_over_http_and_encrypted_websocket_without_app_binding() {
    let state = state(Duration::from_secs(5));
    let node = state.registry.create(input()).unwrap();
    let token = state.registry.token(&node);
    let (base, center) = serve(store_api::router(state.clone())).await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("connector.json");
    std::fs::write(&path, "{}").unwrap();
    let log = store_provider::diagnostics::Log::for_config(&path, &node.connector_id).unwrap();
    // A log larger than one encrypted message must still stream without store credentials.
    for _ in 0..600 {
        log.event("fixture", json!({"data":"x".repeat(4096)}));
    }
    let snapshot = log.latest_snapshot().unwrap();
    let expected = std::fs::read(log.directory().join(snapshot["file"].as_str().unwrap())).unwrap();
    assert!(expected.len() > crypto::MAX_JSON_BYTES);
    let config:ConnectorConfig=serde_json::from_value(json!({"connectorId":node.connector_id,"token":token,
        "messageKey":state.registry.purpose_key(&node,"message"),"uploadKey":state.registry.purpose_key(&node,"upload"),
        "serverUrl":base,"platformMode":"android","provider":{"mode":"mock"}})).unwrap();
    let executor = Executor::with_log(config, Some(log.clone())).unwrap();
    let (direct, node_server) = serve(store_connector::http_router(executor.clone())).await;
    state
        .registry
        .register_endpoint(&node.connector_id, &token, Some(&direct))
        .unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let control = format!("{direct}/v1/diagnostics/logs");
    let latest = json!({"requestId":"fixture-control","action":{"type":"latest"}});
    assert_eq!(
        client
            .post(&control)
            .bearer_auth(CLIENT)
            .json(&latest)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let invalid = client
        .post(&control)
        .bearer_auth(&token)
        .json(&json!({"requestId":"invalid","action":{
        "type":"read","file":"../connector.json","offset":0,"length":1,"snapshotSize":1}}))
        .send()
        .await
        .unwrap()
        .json::<StoreReply>()
        .await
        .unwrap();
    assert_eq!(invalid.status, 422);
    let url = format!("{base}/v1/connectors/{}/logs/latest", node.connector_id);
    assert_eq!(
        client
            .get(&url)
            .bearer_auth(CLIENT)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let http = client.get(&url).bearer_auth(ADMIN).send().await.unwrap();
    assert_eq!(http.status(), 200);
    assert_eq!(http.headers()["cache-control"], "no-store");
    let downloaded = http.bytes().await.unwrap();
    assert_eq!(downloaded.len(), expected.len());
    assert!(
        downloaded.as_ref() == expected.as_slice(),
        "HTTP log snapshot content differs"
    );
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let active = tokio::spawn(store_connector::run_active(executor.clone(), stopped));
    wait_connected(&state, &node.connector_id).await;
    assert!(state.transport.logs_supported(&node.connector_id));
    state
        .transport
        .dispatch_logs(
            &node,
            &token,
            store_protocol::logs::LogRequest {
                request_id: "connected-barrier".into(),
                action: store_protocol::logs::LogAction::Latest,
            },
        )
        .await
        .unwrap();
    let current = log.latest_snapshot().unwrap();
    let file = current["file"].as_str().unwrap();
    let path = log.directory().join(file);
    let baseline = std::fs::read(&path).unwrap();
    assert!(baseline.len() >= expected.len());
    assert!(
        baseline.starts_with(&expected),
        "Connected log lost the original fixture prefix"
    );
    let wss = client.get(&url).bearer_auth(ADMIN).send().await.unwrap();
    assert_eq!(wss.status(), 200);
    assert_eq!(
        wss.headers()["content-disposition"],
        format!("attachment; filename=\"{file}\"")
    );
    let snapshot_length = wss.headers()["content-length"]
        .to_str()
        .unwrap()
        .parse::<usize>()
        .unwrap();
    let downloaded = wss.bytes().await.unwrap();
    assert_eq!(downloaded.len(), snapshot_length);
    assert!(downloaded.len() >= baseline.len());
    assert!(
        downloaded.starts_with(&baseline),
        "Streamed log lost its pre-request baseline prefix"
    );
    // Catalog completion may append after the baseline read. Validate every streamed byte
    // against the immutable prefix of the same source file, including those new events.
    let after_download = std::fs::read(&path).unwrap();
    assert!(after_download.len() >= downloaded.len());
    assert!(
        downloaded.as_ref() == &after_download[..downloaded.len()],
        "Encrypted log snapshot content differs"
    );
    state
        .registry
        .set_enabled(&node.connector_id, false)
        .unwrap();
    assert_eq!(
        client
            .get(&url)
            .bearer_auth(ADMIN)
            .send()
            .await
            .unwrap()
            .status(),
        409
    );
    stop.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(2), active)
        .await
        .unwrap()
        .unwrap();
    executor.drain().await;
    state.transport.close_all();
    node_server.abort();
    let _ = node_server.await;
    center.abort();
    let _ = center.await;
}

#[tokio::test]
async fn node_log_download_audits_cancellation_and_rejects_corrupt_chunks() {
    use axum::routing::post;
    let state = state(Duration::from_secs(2));
    let node = state.registry.create(input()).unwrap();
    let fake=Router::new().route("/v1/diagnostics/logs",post(|axum::Json(request):axum::Json<Value>|async move {
        let body=if request["action"]["type"]=="latest" {json!({"file":"store-connector-0000498420.jsonl","sizeBytes":1})}
            else {json!({"file":"store-connector-0000498420.jsonl","offset":999,"snapshotSize":1,"data":"eA=="})};
        axum::Json(json!({"requestId":request["requestId"],"status":200,"body":body}))
    }));
    let (base, server) = serve(fake).await;
    state
        .registry
        .register_endpoint(
            &node.connector_id,
            &state.registry.token(&node),
            Some(&base),
        )
        .unwrap();
    let app = store_api::router(state.clone());
    let path = format!("/v1/connectors/{}/logs/latest", node.connector_id);
    let request = || {
        Request::builder()
            .uri(&path)
            .header("authorization", format!("Bearer {ADMIN}"))
            .body(Body::empty())
            .unwrap()
    };
    let cancelled = app.clone().oneshot(request()).await.unwrap();
    assert_eq!(cancelled.status(), 200);
    drop(cancelled);
    let corrupt = app.clone().oneshot(request()).await.unwrap();
    assert!(corrupt.into_body().collect().await.is_err());
    let (_, entries) = call(
        app,
        "GET",
        &format!("/v1/logs?connectorId={}", node.connector_id),
        ADMIN,
        Value::Null,
    )
    .await;
    let entries = entries["entries"].as_array().unwrap();
    assert!(entries
        .iter()
        .any(|e| e["event"] == "node.logs.download_cancelled"));
    assert!(entries
        .iter()
        .any(|e| e["event"] == "node.logs.download_failed"));
    assert!(!entries
        .iter()
        .any(|e| e["event"] == "node.logs.download_completed"));
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn real_connector_active_loop_and_api_round_trip() {
    let state = state(Duration::from_secs(2));
    let node = state.registry.create(input()).unwrap();
    let token = state.registry.token(&node);
    support::catalog(
        &state.registry,
        &node.connector_id,
        Platform::Ios,
        &["com.example.app"],
    );
    state
        .registry
        .bind(Platform::Ios, "com.example.app", &node.connector_id)
        .unwrap();
    let (base, server) = serve(store_api::router(state.clone())).await;
    let config:ConnectorConfig=serde_json::from_value(json!({"connectorId":node.connector_id,"token":token,"messageKey":state.registry.purpose_key(&node,"message"),"uploadKey":state.registry.purpose_key(&node,"upload"),"serverUrl":base,"provider":{"mode":"mock"}})).unwrap();
    let executor = Executor::new(config).unwrap();
    let (_, health) = call(
        store_connector::http_router(executor.clone()),
        "GET",
        "/health",
        "",
        Value::Null,
    )
    .await;
    assert_eq!(
        health["connectorVersion"],
        store_protocol::release::CONNECTOR_VERSION
    );
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let connector = tokio::spawn(store_connector::run_active(executor.clone(), stopped));
    wait_connected(&state, &node.connector_id).await;
    assert_eq!(
        state
            .registry
            .get(&node.connector_id)
            .unwrap()
            .connector_version
            .as_deref(),
        Some(store_protocol::release::CONNECTOR_VERSION)
    );
    let response = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .get(format!(
            "{base}/v1/store-management/apps/ios/com.example.app/supported-locales"
        ))
        .bearer_auth(CLIENT)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.json::<Value>().await.unwrap()["locales"][0],
        "en-US"
    );
    let created = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .post(format!(
            "{base}/v1/store-management/apps/ios/com.example.app/store-releases"
        ))
        .bearer_auth(CLIENT)
        .json(&json!({"versionString":"2.0"}))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 200);
    assert_eq!(
        created.json::<Value>().await.unwrap()["release"]["versionString"],
        "2.0"
    );
    stop.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(2), connector)
        .await
        .unwrap()
        .unwrap();
    executor.drain().await;
    state.transport.close_all();
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn installer_contains_published_binary_private_identity_and_launcher() {
    use std::io::{Cursor, Read};
    let directory = tempfile::tempdir().unwrap();
    let releases = directory.path().join("releases");
    std::fs::create_dir_all(releases.join("linux-amd64")).unwrap();
    std::fs::write(
        releases.join("linux-amd64/store-connector"),
        b"test-binary-fixture",
    )
    .unwrap();
    let state = ApiState::new(
        Registry::open(":memory:", SIGNING).unwrap(),
        ADMIN.into(),
        CLIENT.into(),
        "https://store.example.com".into(),
        releases,
        Duration::from_secs(1),
    )
    .unwrap();
    let node = state.registry.create(input()).unwrap();
    let app = store_api::router(state.clone());
    let path = format!("/v1/connectors/{}/installer/linux-amd64", node.connector_id);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&path)
                .header("authorization", format!("Bearer {ADMIN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    assert_eq!(archive.len(), 3);
    let mut binary = Vec::new();
    archive
        .by_name("store-connector")
        .unwrap()
        .read_to_end(&mut binary)
        .unwrap();
    assert_eq!(binary, b"test-binary-fixture");
    let mut raw = String::new();
    archive
        .by_name("connector.json")
        .unwrap()
        .read_to_string(&mut raw)
        .unwrap();
    let config: ConnectorConfig = serde_json::from_str(&raw).unwrap();
    assert_eq!(config.connector_id, node.connector_id);
    assert_eq!(config.server_url, "https://store.example.com");
    assert!(state
        .registry
        .authenticate(&node.connector_id, &config.token)
        .is_ok());
    assert!(archive.by_name("start.sh").is_ok());
    let missing = format!(
        "/v1/connectors/{}/installer/windows-amd64",
        node.connector_id
    );
    assert_eq!(
        call(app.clone(), "GET", &missing, ADMIN, Value::Null)
            .await
            .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        call(app.clone(), "GET", &path, CLIENT, Value::Null).await.0,
        StatusCode::UNAUTHORIZED
    );
    state
        .registry
        .set_enabled(&node.connector_id, false)
        .unwrap();
    assert_eq!(
        call(app, "GET", &path, ADMIN, Value::Null).await.0,
        StatusCode::CONFLICT
    );
}

#[test]
fn service_credentials_cannot_collapse_admin_and_client_roles() {
    assert!(ApiState::new(
        Registry::open(":memory:", SIGNING).unwrap(),
        CLIENT.into(),
        CLIENT.into(),
        "http://127.0.0.1:8200".into(),
        PathBuf::from("releases"),
        Duration::from_secs(1)
    )
    .is_err());
}

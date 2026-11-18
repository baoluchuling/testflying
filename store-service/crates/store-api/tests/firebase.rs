mod support;
use axum::{
    body::Body,
    http::{Request, StatusCode},
    routing::{get, post},
    Json, Router,
};
use futures_util::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use store_api::{
    registry::{ConnectorInput, Registry},
    ApiState,
};
use store_connector::{ConnectorConfig, Executor};
use store_protocol::{
    crypto,
    firebase::{FirebaseAction, FirebaseRequest},
    logs::NodeRequest,
    Operation, Platform, StoreReply, StoreRequest,
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};
use tower::ServiceExt;

const ADMIN: &str = "firebase-admin-test-token";
const CLIENT: &str = "firebase-business-test-token";
struct Task(tokio::task::JoinHandle<()>);
impl Drop for Task {
    fn drop(&mut self) {
        self.0.abort();
    }
}
async fn serve(app: Router) -> (String, Task) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    (
        base,
        Task(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        })),
    )
}
fn state() -> Arc<ApiState> {
    ApiState::new(
        Registry::open(":memory:", &"44".repeat(32)).unwrap(),
        ADMIN.into(),
        CLIENT.into(),
        "https://store.test".into(),
        PathBuf::from("nonexistent-test-releases"),
        Duration::from_secs(3),
    )
    .unwrap()
}
fn request() -> FirebaseRequest {
    FirebaseRequest {
        request_id: "firebase-fixture".into(),
        firebase_action: FirebaseAction::Projects,
    }
}
fn registry_snapshot(state: &ApiState) -> Value {
    let accounts = state.registry.accounts().unwrap();
    let apps:Vec<_>=accounts.iter().map(|account|json!({"id":account.id,"apps":state.registry.account_apps(&account.id).unwrap()})).collect();
    json!({"accounts":accounts,"apps":apps,"nodes":state.registry.list().unwrap(),"forwarding":state.registry.forwarding().unwrap(),"binding":state.registry.resolve_account(Platform::Ios,"com.example.retained").unwrap().id})
}
async fn call(app: Router, path: &str, token: &str) -> (StatusCode, Value) {
    let response = app
        .oneshot(
            Request::builder()
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
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
async fn receive_command(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    channel: &mut crypto::Channel,
) -> NodeRequest {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match socket.next().await.unwrap().unwrap() {
                Message::Ping(bytes) => socket.send(Message::Pong(bytes)).await.unwrap(),
                Message::Binary(bytes) => {
                    if let Some(plain) = channel.decrypt(&bytes).unwrap() {
                        break serde_json::from_slice::<NodeRequest>(&plain).unwrap();
                    }
                }
                _ => {}
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn admin_session_http_discovery_is_read_only_and_old_http_nodes_are_not_probed() {
    let state = state();
    let node = state
        .registry
        .create(ConnectorInput {
            name: "Firebase fixture".into(),
        })
        .unwrap();
    let retained = state
        .registry
        .create(ConnectorInput {
            name: "Retained store account".into(),
        })
        .unwrap();
    support::catalog(
        &state.registry,
        &retained.connector_id,
        Platform::Ios,
        &["com.example.retained"],
    );
    state
        .registry
        .bind(
            Platform::Ios,
            "com.example.retained",
            &retained.connector_id,
        )
        .unwrap();
    let token = state.registry.token(&node);
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let health_token = token.clone();
    let node_token = token.clone();
    let (base, _server) = serve(Router::new()
        .route("/health", get(move |headers: axum::http::HeaderMap| { let token=health_token.clone(); async move {
            assert_eq!(headers["authorization"], format!("Bearer {token}"));
            Json(json!({"firebaseProtocol":1}))
        }}))
        .route("/v1/diagnostics/firebase", post(move |headers: axum::http::HeaderMap, Json(request): Json<FirebaseRequest>| { let count=count.clone(); let token=node_token.clone(); async move {
            assert_eq!(headers["authorization"], format!("Bearer {token}"));
            count.fetch_add(1, Ordering::SeqCst);
            let body=match request.firebase_action { FirebaseAction::Projects => json!({"projects":[{"projectId":"project-a","projectNumber":"123","displayName":"Fixture"}]}), FirebaseAction::AndroidApps {project_id} => json!({"projectId":project_id,"apps":[]}) };
            Json(StoreReply::from_result(request.request_id, Ok(body)))
        }}))).await;
    state
        .registry
        .register_endpoint(&node.connector_id, &token, Some(&base))
        .unwrap();
    let app = store_api::router(state.clone());
    let path = format!("/v1/connectors/{}/firebase/projects", node.connector_id);
    let before = registry_snapshot(&state);
    for token in ["", CLIENT, "tf_int_test-invalid-token"] {
        assert_eq!(call(app.clone(), &path, token).await.0, 401);
    }
    let (status, body) = call(app.clone(), &path, ADMIN).await;
    assert_eq!(status, 200);
    assert_eq!(body["projects"][0]["projectId"], "project-a");
    assert_eq!(
        call(
            app.clone(),
            &format!("{path}/project-a/android-apps"),
            ADMIN
        )
        .await
        .1,
        json!({"projectId":"project-a","apps":[]})
    );
    assert_eq!(
        call(app.clone(), &format!("{path}/123456/android-apps"), ADMIN)
            .await
            .0,
        422
    );
    let login = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/connector-setup/api/session")
                .header("authorization", format!("Bearer {ADMIN}"))
                .header("x-forwarded-proto", "https")
                .header("x-testflying-setup", "1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(login.status(), 204);
    let cookie = login.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap();
    for (https, status) in [(true, 200), (false, 403)] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/connector-setup/api/connectors/{}/firebase/projects",
                        node.connector_id
                    ))
                    .header("cookie", cookie)
                    .header("x-forwarded-proto", if https { "https" } else { "http" })
                    .header("x-testflying-setup", "1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), status);
    }
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert_eq!(before, registry_snapshot(&state));
    let old_calls = Arc::new(AtomicUsize::new(0));
    let count = old_calls.clone();
    let (old_base, _old) = serve(
        Router::new()
            .route("/health", get(|| async { Json(json!({"status":"ok"})) }))
            .route(
                "/v1/diagnostics/firebase",
                post(move || {
                    let count = count.clone();
                    async move {
                        count.fetch_add(1, Ordering::SeqCst);
                        StatusCode::OK
                    }
                }),
            ),
    )
    .await;
    state
        .registry
        .register_endpoint(&node.connector_id, &token, Some(&old_base))
        .unwrap();
    assert_eq!(
        call(app, &path, ADMIN).await.1["error"]["code"],
        "firebase_upgrade_required"
    );
    assert_eq!(old_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn node_http_auth_health_and_ios_mode_are_explicit() {
    let config:ConnectorConfig=serde_json::from_value(json!({"connectorId":"firebase-http","token":"firebase-test-node-token","serverUrl":"http://127.0.0.1:1","messageKey":"11".repeat(32),"uploadKey":"22".repeat(32),"platformMode":"ios","provider":{"mode":"mock","googleFirebaseBaseUrl":"bad-url","googleServiceAccountPath":""}})).unwrap();
    let executor = Executor::new(config).unwrap();
    let (base, _server) = serve(store_connector::http_router(executor)).await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    assert_eq!(
        client
            .get(format!("{base}/health"))
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap()["firebaseProtocol"],
        1
    );
    assert_eq!(
        client
            .post(format!("{base}/v1/diagnostics/firebase"))
            .json(&request())
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let reply = client
        .post(format!("{base}/v1/diagnostics/firebase"))
        .bearer_auth("firebase-test-node-token")
        .json(&request())
        .send()
        .await
        .unwrap()
        .json::<StoreReply>()
        .await
        .unwrap();
    assert_eq!(reply.status, 503);
    assert_eq!(reply.body["error"]["code"], "firebase_not_configured");
}

#[tokio::test]
async fn current_connector_advertises_header_and_executes_over_encrypted_transport() {
    let state = state();
    let node = state
        .registry
        .create(ConnectorInput {
            name: "current connector".into(),
        })
        .unwrap();
    let (base, _server) = serve(store_api::router(state.clone())).await;
    let token = state.registry.token(&node);
    let config:ConnectorConfig=serde_json::from_value(json!({"connectorId":node.connector_id,"token":token,"serverUrl":base,"messageKey":state.registry.purpose_key(&node,"message"),"uploadKey":state.registry.purpose_key(&node,"upload"),"platformMode":"ios","provider":{"mode":"mock"}})).unwrap();
    let executor = Executor::new(config).unwrap();
    let (_stop, stopped) = tokio::sync::watch::channel(false);
    let _connector = Task(tokio::spawn(store_connector::run_active(executor, stopped)));
    tokio::time::timeout(Duration::from_secs(3), async {
        while !state.transport.connected(&node.connector_id) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let reply = state
        .transport
        .dispatch_firebase(&node, &token, request())
        .await
        .unwrap();
    assert_eq!(reply.status, 503);
    assert_eq!(reply.body["error"]["code"], "firebase_not_configured");
    assert!(state.transport.connected(&node.connector_id));
    state.transport.close_all();
}

#[tokio::test]
async fn encrypted_capability_is_confirmed_and_legacy_connection_stays_usable() {
    for capable in [false, true] {
        let state = state();
        let node = state
            .registry
            .create(ConnectorInput {
                name: "encrypted fixture".into(),
            })
            .unwrap();
        let token = state.registry.token(&node);
        let key = state.registry.purpose_key(&node, "message");
        let (base, _server) = serve(store_api::router(state.clone())).await;
        let mut req = format!(
            "{}/v1/connectors/{}/ws",
            base.replacen("http", "ws", 1),
            node.connector_id
        )
        .into_client_request()
        .unwrap();
        req.headers_mut()
            .insert("authorization", format!("Bearer {token}").parse().unwrap());
        req.headers_mut().insert(
            "sec-websocket-protocol",
            crypto::SUBPROTOCOL.parse().unwrap(),
        );
        if capable {
            req.headers_mut()
                .insert("x-firebase-protocol", "1".parse().unwrap());
        }
        let (mut socket, _) = connect_async(req).await.unwrap();
        let mut handshake = crypto::handshake(&key, &node.connector_id, true).unwrap();
        let mut buffer = vec![0; crypto::HANDSHAKE_BYTES];
        let size = handshake
            .write_message(
                &serde_json::to_vec(
                    &json!({"connectorId":node.connector_id,"baseUrl":null,"appCatalogVersion":1}),
                )
                .unwrap(),
                &mut buffer,
            )
            .unwrap();
        socket
            .send(Message::Binary(buffer[..size].to_vec().into()))
            .await
            .unwrap();
        let Message::Binary(reply) = socket.next().await.unwrap().unwrap() else {
            panic!("handshake")
        };
        handshake.read_message(&reply, &mut buffer).unwrap();
        assert!(!state.transport.connected(&node.connector_id));
        let mut channel = crypto::Channel::new(handshake).unwrap();
        for record in channel.encrypt(b"confirm").unwrap() {
            socket.send(Message::Binary(record.into())).await.unwrap();
        }
        let Message::Binary(ready) = socket.next().await.unwrap().unwrap() else {
            panic!("ready")
        };
        assert_eq!(channel.decrypt(&ready).unwrap().unwrap(), b"ready");
        if !capable {
            assert_eq!(
                state
                    .transport
                    .dispatch_firebase(&node, &token, request())
                    .await
                    .unwrap_err()
                    .code,
                "firebase_upgrade_required"
            );
            assert!(state.transport.connected(&node.connector_id));
        }
        let transport = state.transport.clone();
        let fixture = node.clone();
        let credential = token.clone();
        let pending = tokio::spawn(async move {
            if capable {
                transport
                    .dispatch_firebase(&fixture, &credential, request())
                    .await
            } else {
                transport
                    .dispatch(
                        &fixture,
                        &credential,
                        StoreRequest {
                            request_id: "business-fixture".into(),
                            platform: Platform::Ios,
                            bundle_identifier: "com.example.app".into(),
                            operation: Operation::SupportedLocales,
                            payload: json!({}),
                        },
                    )
                    .await
            }
        });
        let command = receive_command(&mut socket, &mut channel).await;
        assert_eq!(matches!(command, NodeRequest::Firebase(_)), capable);
        let wire_id = command.request_id().to_owned();
        if capable {
            assert!(wire_id.starts_with("firebase:"));
            assert!(uuid::Uuid::parse_str(&wire_id).is_err());
        }
        let body = if capable {
            json!({"projects":[]})
        } else {
            json!({"locales":[]})
        };
        let response = StoreReply::from_result(command.request_id().to_owned(), Ok(body));
        for record in channel
            .encrypt(&serde_json::to_vec(&response).unwrap())
            .unwrap()
        {
            socket.send(Message::Binary(record.into())).await.unwrap();
        }
        let completed = pending.await.unwrap().unwrap();
        assert_eq!(completed.status, 200);
        assert_eq!(
            completed.request_id,
            if capable {
                "firebase-fixture"
            } else {
                "business-fixture"
            }
        );
        assert!(state.transport.connected(&node.connector_id));
        if capable {
            let before = json!({"accounts":state.registry.accounts().unwrap(),"nodes":state.registry.list().unwrap(),"forwarding":state.registry.forwarding().unwrap()});
            // A late/duplicate reply has no pending recipient. Its reserved non-UUID
            // wire ID still cannot be accepted as an unsolicited catalog report ID.
            let late_report = store_protocol::catalog::CatalogReport {
                schema_version: 1,
                report_id: wire_id.clone(),
                platform: Platform::Android,
                status: store_protocol::catalog::CatalogStatus::Ready,
                account_fingerprint: Some("a".repeat(64)),
                apps: vec![store_protocol::catalog::CatalogApp {
                    bundle_identifier: "com.example.injected".into(),
                    name: "Injected".into(),
                    store_app_id: Some("123".into()),
                }],
                error_code: None,
            };
            assert!(late_report.validate().is_err());
            let late_reply =
                StoreReply::from_result(wire_id, Ok(json!({"accountAppCatalog":late_report})));
            for record in channel
                .encrypt(&serde_json::to_vec(&late_reply).unwrap())
                .unwrap()
            {
                socket.send(Message::Binary(record.into())).await.unwrap();
            }
            let transport = state.transport.clone();
            let fixture = node.clone();
            let credential = token.clone();
            let injection = tokio::spawn(async move {
                transport
                    .dispatch_firebase(
                        &fixture,
                        &credential,
                        FirebaseRequest {
                            request_id: uuid::Uuid::new_v4().to_string(),
                            firebase_action: FirebaseAction::Projects,
                        },
                    )
                    .await
            });
            let command = receive_command(&mut socket, &mut channel).await;
            assert!(matches!(command, NodeRequest::Firebase(_)));
            let report = store_protocol::catalog::CatalogReport {
                schema_version: 1,
                report_id: uuid::Uuid::new_v4().to_string(),
                platform: Platform::Android,
                status: store_protocol::catalog::CatalogStatus::Ready,
                account_fingerprint: Some("a".repeat(64)),
                apps: vec![store_protocol::catalog::CatalogApp {
                    bundle_identifier: "com.example.injected".into(),
                    name: "Injected".into(),
                    store_app_id: Some("123".into()),
                }],
                error_code: None,
            };
            report.validate().unwrap();
            let reply = StoreReply::from_result(
                command.request_id().to_owned(),
                Ok(json!({"accountAppCatalog":report})),
            );
            for record in channel
                .encrypt(&serde_json::to_vec(&reply).unwrap())
                .unwrap()
            {
                socket.send(Message::Binary(record.into())).await.unwrap();
            }
            assert_eq!(
                injection.await.unwrap().unwrap_err().code,
                "firebase_invalid_response"
            );
            assert_eq!(
                before,
                json!({"accounts":state.registry.accounts().unwrap(),"nodes":state.registry.list().unwrap(),"forwarding":state.registry.forwarding().unwrap()})
            );
            assert!(state
                .registry
                .resolve_account(Platform::Android, "com.example.injected")
                .is_err());
            assert!(state.transport.connected(&node.connector_id));
            let transport = state.transport.clone();
            let fixture = node.clone();
            let credential = token.clone();
            let business = tokio::spawn(async move {
                transport
                    .dispatch(
                        &fixture,
                        &credential,
                        StoreRequest {
                            request_id: "after-rejected-catalog".into(),
                            platform: Platform::Ios,
                            bundle_identifier: "com.example.app".into(),
                            operation: Operation::SupportedLocales,
                            payload: json!({}),
                        },
                    )
                    .await
            });
            let command = receive_command(&mut socket, &mut channel).await;
            assert!(matches!(command, NodeRequest::Store(_)));
            let reply =
                StoreReply::from_result(command.request_id().to_owned(), Ok(json!({"locales":[]})));
            for record in channel
                .encrypt(&serde_json::to_vec(&reply).unwrap())
                .unwrap()
            {
                socket.send(Message::Binary(record.into())).await.unwrap();
            }
            assert_eq!(business.await.unwrap().unwrap().status, 200);
            assert!(state.transport.connected(&node.connector_id));
        }
        state.transport.close_all();
    }
}

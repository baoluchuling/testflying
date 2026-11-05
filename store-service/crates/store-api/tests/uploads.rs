use image::ImageEncoder;
use reqwest::Client;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::Arc, time::Duration};
use store_api::{registry::Registry, ApiState};
use store_connector::{ConnectorConfig, Executor};
use store_protocol::Platform;
use tokio::{
    io::AsyncWriteExt,
    net::{TcpListener, TcpStream},
    sync::watch,
    task::JoinHandle,
};

const ADMIN: &str = "upload-admin-test-0123456789";
const CLIENT: &str = "upload-client-test-0123456789";
const SIGNING: &str = "upload-signing-test-01234567890123456789";
const APP: &str = "com.example.upload";

struct Fixture {
    state: Arc<ApiState>,
    executor: Arc<Executor>,
    client: Client,
    base: String,
    id: String,
    stop: watch::Sender<bool>,
    api: JoinHandle<()>,
    node: JoinHandle<()>,
    node_http: String,
}
impl Fixture {
    async fn start(websocket: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let state = ApiState::new(
            Registry::open(":memory:", SIGNING).unwrap(),
            ADMIN.into(),
            CLIENT.into(),
            base.clone(),
            PathBuf::from("unused-releases"),
            Duration::from_secs(5),
        )
        .unwrap();
        let app = store_api::router(state.clone());
        let api = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let client = Client::builder().no_proxy().build().unwrap();
        let created = client
            .post(format!("{base}/v1/connectors"))
            .bearer_auth(ADMIN)
            .json(&json!({"name":"Upload account"}))
            .send()
            .await
            .unwrap();
        assert_eq!(created.status(), 201);
        let created: Value = created.json().await.unwrap();
        let id = created["connector"]["connectorId"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(created["installation"].get("mode").is_none());
        let mut config = created["installation"].clone();
        config["provider"] = json!({"mode":"mock"});
        let node_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let node_http = format!("http://{}", node_listener.local_addr().unwrap());
        // A successful WSS upload cannot have used this intentionally unreachable HTTP address.
        let advertised = if websocket {
            "http://127.0.0.1:1".to_owned()
        } else {
            node_http.clone()
        };
        config["baseUrl"] = json!(advertised);
        let executor =
            Executor::new(serde_json::from_value::<ConnectorConfig>(config).unwrap()).unwrap();
        let (stop, stopped) = watch::channel(false);
        let node = if websocket {
            let copy = executor.clone();
            tokio::spawn(async move {
                store_connector::run(copy, node_listener, stopped)
                    .await
                    .unwrap();
            })
        } else {
            let copy = executor.clone();
            let mut stopped = stopped;
            state
                .registry
                .register_endpoint(
                    &id,
                    &state.registry.token(&state.registry.get(&id).unwrap()),
                    Some(&advertised),
                )
                .unwrap();
            tokio::spawn(async move {
                axum::serve(node_listener, store_connector::http_router(copy))
                    .with_graceful_shutdown(async move {
                        let _ = stopped.changed().await;
                    })
                    .await
                    .unwrap();
            })
        };
        if websocket {
            tokio::time::timeout(Duration::from_secs(3), async {
                while !state.transport.connected(&id) {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
        }
        assert_eq!(
            state.registry.get(&id).unwrap().base_url.as_deref(),
            Some(advertised.as_str())
        );
        for app in [APP, "com.example.other"] {
            state.registry.bind(Platform::Ios, app, &id).unwrap();
        }
        Self {
            state,
            executor,
            client,
            base,
            id,
            stop,
            api,
            node,
            node_http,
        }
    }
    async fn prepare(&self, bytes: &[u8]) -> Value {
        let response=self.client.post(format!("{}/v1/store-management/apps/ios/{APP}/uploads",self.base)).bearer_auth(CLIENT)
            .json(&json!({"fileName":"screen.png","contentType":"image/png","sizeBytes":bytes.len(),"sha256":hex::encode(Sha256::digest(bytes))})).send().await.unwrap();
        assert_eq!(response.status(), 201);
        let prepared: Value = response.json().await.unwrap();
        assert!(prepared["uploadUrl"]
            .as_str()
            .unwrap()
            .starts_with(&self.base));
        assert!(!prepared.to_string().contains(&self.node_http));
        let key = self
            .state
            .registry
            .purpose_key(&self.state.registry.get(&self.id).unwrap(), "upload");
        let claims = store_protocol::upload::verify(
            prepared["uploadToken"].as_str().unwrap(),
            &key,
            &self.id,
        )
        .unwrap();
        assert_eq!(claims.exp - claims.iat, 1800);
        assert_eq!(prepared["expiresAt"], claims.exp);
        prepared
    }
    async fn upload(&self, prepared: &Value, bytes: Vec<u8>) -> reqwest::Response {
        self.client
            .put(prepared["uploadUrl"].as_str().unwrap())
            .bearer_auth(prepared["uploadToken"].as_str().unwrap())
            .body(bytes)
            .send()
            .await
            .unwrap()
    }
    async fn sync(&self, app: &str, id: &str) -> reqwest::Response {
        self.client.post(format!("{}/v1/store-management/apps/ios/{app}/sync",self.base)).bearer_auth(CLIENT)
            .json(&json!({"operation":"update_app_metadata","version":"1.0","locale":"en-US","syncScopes":["store_images"],"metadata":{"storeImages":{"phone_screenshots":{"assets":[{"assetId":id}]}}}})).send().await.unwrap()
    }
    async fn stop(self) {
        self.stop.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(3), self.node)
            .await
            .unwrap()
            .unwrap();
        self.executor.drain().await;
        self.state.transport.close_all();
        self.api.abort();
        let _ = self.api.await;
    }
}

fn png(large: bool) -> Vec<u8> {
    let width = if large { 1024 } else { 2 };
    let mut pixels = vec![0; width * width * 3];
    let mut seed = 0x12345678u32;
    for byte in &mut pixels {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        *byte = seed as u8;
    }
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(
            &pixels,
            width as u32,
            width as u32,
            image::ExtendedColorType::Rgb8,
        )
        .unwrap();
    if large {
        assert!(bytes.len() > 2 * 1024 * 1024);
    }
    bytes
}

#[tokio::test]
async fn central_http_upload_larger_than_json_limit_is_scoped_consumed_and_not_reusable() {
    let fixture = Fixture::start(false).await;
    let bytes = png(true);
    let prepared = fixture.prepare(&bytes).await;
    let copied = prepared.clone();
    let response = fixture.upload(&prepared, bytes.clone()).await;
    assert_eq!(response.status(), 201);
    let asset = prepared["assetId"].as_str().unwrap();
    assert_eq!(fixture.executor.assets.usage(), (1, bytes.len() as u64));
    assert_eq!(fixture.sync("com.example.other", asset).await.status(), 403);
    assert_eq!(fixture.executor.assets.usage().0, 1);
    let preflight=fixture.client.post(format!("{}/v1/store-management/apps/ios/{APP}/preflight",fixture.base)).bearer_auth(CLIENT)
        .json(&json!({"operation":"update_app_metadata","version":"1.0","locale":"en-US","syncScopes":["store_images"],"metadata":{"storeImages":{"phone_screenshots":{"assets":[{"assetId":asset}]}}}})).send().await.unwrap();
    assert_eq!(preflight.status(), 200);
    assert_eq!(fixture.executor.assets.usage().0, 1);
    let response = fixture.sync(APP, asset).await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.json::<Value>().await.unwrap()["status"],
        "succeeded"
    );
    assert_eq!(fixture.executor.assets.usage(), (0, 0));
    assert_eq!(fixture.sync(APP, asset).await.status(), 404);
    let wrong = fixture
        .client
        .put(copied["uploadUrl"].as_str().unwrap())
        .bearer_auth("invalid")
        // The server may reject the ticket before draining this large body.
        // Do not reuse that connection for the following independent JSON check.
        .header("connection", "close")
        .body(bytes)
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), 401);
    let invalid = fixture
        .client
        .post(format!("{}/v1/connectors", fixture.base))
        .bearer_auth(ADMIN)
        .json(&json!({"name":"bad","mode":"http","baseUrl":fixture.node_http}))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status(), 422);
    fixture.stop().await;
}

#[tokio::test]
async fn central_encrypted_wss_relay_works_with_unreachable_node_http_and_cleans_bad_hash() {
    let fixture = Fixture::start(true).await;
    assert_eq!(
        fixture
            .client
            .get(format!("{}/health", fixture.node_http))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    let bytes = png(true);
    let prepared = fixture.prepare(&bytes).await;
    assert_eq!(fixture.upload(&prepared, bytes.clone()).await.status(), 201);
    assert_eq!(fixture.executor.assets.usage().0, 1);
    assert_eq!(
        fixture
            .sync(APP, prepared["assetId"].as_str().unwrap())
            .await
            .status(),
        200
    );
    assert_eq!(fixture.executor.assets.usage(), (0, 0));
    let prepared = fixture.prepare(&bytes).await;
    let mut altered = bytes;
    altered[50] ^= 1;
    assert_eq!(fixture.upload(&prepared, altered).await.status(), 422);
    assert_eq!(fixture.executor.assets.usage(), (0, 0));
    fixture.stop().await;
}

#[tokio::test]
async fn interrupted_client_upload_releases_node_file_and_registration_requires_current_token() {
    let fixture = Fixture::start(true).await;
    let bytes = png(true);
    let prepared = fixture.prepare(&bytes).await;
    let url = reqwest::Url::parse(prepared["uploadUrl"].as_str().unwrap()).unwrap();
    let mut socket = TcpStream::connect(("127.0.0.1", url.port().unwrap()))
        .await
        .unwrap();
    let headers=format!("PUT {} HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {}\r\nTransfer-Encoding: chunked\r\n\r\n8000\r\n",url.path(),prepared["uploadToken"].as_str().unwrap());
    socket.write_all(headers.as_bytes()).await.unwrap();
    socket.write_all(&bytes[..32768]).await.unwrap();
    socket.write_all(b"\r\n").await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while fixture.executor.assets.usage().0 == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    // A duplicate use of the same signed ticket must not abort the first upload.
    let duplicate = fixture.upload(&prepared, bytes.clone()).await;
    assert_eq!(duplicate.status(), 409);
    tokio::task::yield_now().await;
    assert_eq!(fixture.executor.assets.usage().0, 1);
    socket.shutdown().await.unwrap();
    drop(socket);
    tokio::time::timeout(Duration::from_secs(3), async {
        while fixture.executor.assets.usage().0 != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let original = fixture.state.registry.get(&fixture.id).unwrap().base_url;
    assert!(fixture
        .state
        .registry
        .register_endpoint(&fixture.id, "wrong-token", Some("http://attacker.invalid"))
        .is_err());
    assert_eq!(
        fixture.state.registry.get(&fixture.id).unwrap().base_url,
        original
    );
    fixture.state.registry.rotate(&fixture.id).unwrap();
    assert!(fixture
        .state
        .registry
        .get(&fixture.id)
        .unwrap()
        .base_url
        .is_none());
    fixture.stop().await;
}

#[tokio::test]
async fn unconfirmed_encrypted_registration_does_not_overwrite_endpoint() {
    use futures_util::{SinkExt, StreamExt};
    use store_protocol::crypto;
    use tokio_tungstenite::{
        connect_async,
        tungstenite::{client::IntoClientRequest, Message},
    };
    let fixture = Fixture::start(false).await;
    let node = fixture.state.registry.get(&fixture.id).unwrap();
    let token = fixture.state.registry.token(&node);
    let original = node.base_url.clone();
    for wrong_key in [false, true] {
        let mut request = format!(
            "{}/v1/connectors/{}/ws",
            fixture.base.replacen("http", "ws", 1),
            fixture.id
        )
        .into_client_request()
        .unwrap();
        request
            .headers_mut()
            .insert("authorization", format!("Bearer {token}").parse().unwrap());
        request.headers_mut().insert(
            "sec-websocket-protocol",
            crypto::SUBPROTOCOL.parse().unwrap(),
        );
        let mut socket = connect_async(request).await.unwrap().0;
        let key = if wrong_key {
            "77".repeat(32)
        } else {
            fixture.state.registry.purpose_key(&node, "message")
        };
        let mut handshake = crypto::handshake(&key, &fixture.id, true).unwrap();
        let mut buffer = vec![0; crypto::HANDSHAKE_BYTES];
        let size = handshake
            .write_message(
                &serde_json::to_vec(
                    &json!({"connectorId":fixture.id,"baseUrl":"https://unconfirmed.invalid"}),
                )
                .unwrap(),
                &mut buffer,
            )
            .unwrap();
        socket
            .send(Message::Binary(buffer[..size].to_vec().into()))
            .await
            .unwrap();
        let reply = tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap();
        if !wrong_key {
            assert!(matches!(reply, Some(Ok(Message::Binary(_)))));
        }
        drop(socket);
        assert!(!fixture.state.transport.connected(&fixture.id));
        assert_eq!(
            fixture.state.registry.get(&fixture.id).unwrap().base_url,
            original
        );
    }
    fixture.stop().await;
}

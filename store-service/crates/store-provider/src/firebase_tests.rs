#[cfg(test)]
mod cases {
    use super::super::*;
    use axum::{
        body::{to_bytes, Body},
        extract::{Request, State},
        http::HeaderMap,
        response::Response as HttpResponse,
        Router,
    };
    use jsonwebtoken::{decode, DecodingKey, Validation};
    use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
    use std::sync::{Mutex as StdMutex, OnceLock};

    #[derive(Clone)]
    struct Record {
        method: String,
        uri: String,
        headers: HeaderMap,
        bytes: Vec<u8>,
    }
    type Responder = Arc<dyn Fn(&Record) -> (u16, Vec<u8>) + Send + Sync>;
    #[derive(Clone)]
    struct FakeState {
        records: Arc<StdMutex<Vec<Record>>>,
        respond: Responder,
    }
    struct Fake {
        url: String,
        records: Arc<StdMutex<Vec<Record>>>,
        task: tokio::task::JoinHandle<()>,
    }
    impl Drop for Fake {
        fn drop(&mut self) {
            self.task.abort();
        }
    }
    impl Fake {
        async fn new(respond: impl Fn(&Record) -> (u16, Vec<u8>) + Send + Sync + 'static) -> Self {
            async fn handle(State(state): State<FakeState>, request: Request) -> HttpResponse {
                let (parts, body) = request.into_parts();
                let record = Record {
                    method: parts.method.to_string(),
                    uri: parts.uri.to_string(),
                    headers: parts.headers,
                    bytes: to_bytes(body, 64 * 1024).await.unwrap().to_vec(),
                };
                state.records.lock().unwrap().push(record.clone());
                let (status, bytes) = (state.respond)(&record);
                HttpResponse::builder()
                    .status(status)
                    .body(Body::from(bytes))
                    .unwrap()
            }
            let records = Arc::new(StdMutex::new(Vec::new()));
            let router = Router::new().fallback(handle).with_state(FakeState {
                records: records.clone(),
                respond: Arc::new(respond),
            });
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let task = tokio::spawn(async move {
                axum::serve(listener, router).await.unwrap();
            });
            Self { url, records, task }
        }
        fn records(&self) -> Vec<Record> {
            self.records.lock().unwrap().clone()
        }
        fn gateway(&self) -> (Gateway, tempfile::TempDir) {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("fake-account.json");
            std::fs::write(
                &path,
                serde_json::to_vec(&json!({
                    "client_email":"fake-only@example.invalid", "private_key":keys().0,
                }))
                .unwrap(),
            )
            .unwrap();
            let gateway = Gateway::new(ProviderConfig {
                google_service_account_path: Some(path.to_string_lossy().into_owned()),
                google_token_url: format!("{}/token", self.url),
                google_firebase_base_url: self.url.clone(),
                google_reporting_base_url: self.url.clone(),
                google_base_url: self.url.clone(),
                ..Default::default()
            })
            .unwrap();
            (gateway, directory)
        }
    }
    fn keys() -> &'static (String, String) {
        static KEYS: OnceLock<(String, String)> = OnceLock::new();
        KEYS.get_or_init(|| {
            let key = rsa::RsaPrivateKey::new(&mut rand::thread_rng(), 2048).unwrap();
            (
                key.to_pkcs8_pem(LineEnding::LF).unwrap().to_string(),
                key.to_public_key()
                    .to_public_key_pem(LineEnding::LF)
                    .unwrap(),
            )
        })
    }
    fn reply(value: Value) -> (u16, Vec<u8>) {
        (200, serde_json::to_vec(&value).unwrap())
    }
    fn oauth() -> (u16, Vec<u8>) {
        reply(
            json!({"access_token":"firebase-token-secret","expires_in":3600,"token_type":"Bearer","scope":SCOPE}),
        )
    }
    fn project(id: &str) -> Value {
        json!({"projectId":id,"projectNumber":"1234567890","displayName":"Test project",
        "resources":{"hostingSite":"private"},"apiKeyId":"private-key-id"})
    }
    fn app(id: usize) -> Value {
        json!({"projectId":"project-a","appId":format!("1:123:android:{id:x}"),
        "packageName":format!("com.example.app{id}"),"displayName":null,
        "apiKeyId":"private-key-id","sha1Hashes":["private-hash"],"config":"private-config"})
    }
    fn parsed_url(record: &Record) -> Url {
        Url::parse(&format!("http://fixture.invalid{}", record.uri)).unwrap()
    }
    fn claims(record: &Record) -> Value {
        let form: std::collections::HashMap<_, _> = Url::parse(&format!(
            "http://fixture.invalid/?{}",
            String::from_utf8_lossy(&record.bytes)
        ))
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect();
        assert_eq!(
            form["grant_type"],
            "urn:ietf:params:oauth:grant-type:jwt-bearer"
        );
        let mut validation = Validation::new(Algorithm::RS256);
        validation.validate_aud = false;
        decode::<Value>(
            &form["assertion"],
            &DecodingKey::from_rsa_pem(keys().1.as_bytes()).unwrap(),
            &validation,
        )
        .unwrap()
        .claims
    }

    #[tokio::test]
    async fn paginated_projects_use_encoded_tokens_and_only_return_whitelisted_fields() {
        let opaque = "https://evil.invalid/path?token=secret&a=1#fragment +/=";
        let fake = Fake::new(move |record| {
            if record.uri == "/token" {
                return oauth();
            }
            let url = parsed_url(record);
            assert_eq!(record.method, "GET");
            assert_eq!(url.path(), "/v1beta1/projects");
            assert!(!url.query_pairs().any(|(key, _)| key == "showDeleted"));
            assert_eq!(
                record.headers["authorization"],
                "Bearer firebase-token-secret"
            );
            if let Some((_, token)) = url.query_pairs().find(|(key, _)| key == "pageToken") {
                assert_eq!(token, opaque);
                reply(json!({"results":[project("project-b")]}))
            } else {
                reply(
                    json!({"results":[project("project-a"),{"state":"DELETED","apiKeyId":"secret"}],
                "nextPageToken":opaque}),
                )
            }
        })
        .await;
        let (gateway, _directory) = fake.gateway();
        let result = gateway
            .firebase_query(&FirebaseAction::Projects)
            .await
            .unwrap();
        assert_eq!(
            result,
            json!({"projects":[
        {"projectId":"project-a","projectNumber":"1234567890","displayName":"Test project"},
        {"projectId":"project-b","projectNumber":"1234567890","displayName":"Test project"}]})
        );
        assert_eq!(fake.records().len(), 3);
        assert_eq!(claims(&fake.records()[0])["scope"], SCOPE);
    }

    #[tokio::test]
    async fn android_apps_aggregate_pages_and_empty_discovery_is_honest() {
        let fake = Fake::new(|record| {
            if record.uri == "/token" {
                return oauth();
            }
            let url = parsed_url(record);
            if url.path() == "/v1beta1/projects" {
                return reply(json!({}));
            }
            assert_eq!(url.path(), "/v1beta1/projects/project-a/androidApps");
            let page = url
                .query_pairs()
                .find(|(key, _)| key == "pageToken")
                .map(|(_, value)| value.parse::<usize>().unwrap())
                .unwrap_or(0);
            let apps = (page * 75..(page + 1) * 75).map(app).collect::<Vec<_>>();
            if page < 2 {
                reply(json!({"apps":apps,"nextPageToken":(page + 1).to_string()}))
            } else {
                reply(json!({"apps":apps}))
            }
        })
        .await;
        let (gateway, _directory) = fake.gateway();
        assert_eq!(
            gateway
                .firebase_query(&FirebaseAction::Projects)
                .await
                .unwrap(),
            json!({"projects":[]})
        );
        let action = FirebaseAction::AndroidApps {
            project_id: "project-a".into(),
        };
        let response = gateway.firebase_query(&action).await.unwrap();
        assert_eq!(response["apps"].as_array().unwrap().len(), 225);
        assert_eq!(
            response["apps"][0],
            json!({"projectId":"project-a","appId":"1:123:android:0",
        "packageName":"com.example.app0","displayName":null})
        );
        assert!(fake
            .records()
            .iter()
            .filter(|record| record.uri != "/token")
            .all(|record| record.method == "GET"));
    }

    #[tokio::test]
    async fn pagination_cycles_oversized_tokens_duplicates_and_invalid_rows_fail_without_partial_results(
    ) {
        let cases = [
            json!({"results":[project("project-a")],"nextPageToken":"repeat"}),
            json!({"results":[],"nextPageToken":"x".repeat(MAX_TOKEN_BYTES + 1)}),
            json!({"results":[project("project-a"),project("project-a")]}),
            json!({"results":[project("../unsafe")]}),
            json!({"results":null}),
            json!({"results":[],"nextPageToken":12}),
            json!({"error":{"message":"private failure"}}),
        ];
        for response in cases {
            let fake = Fake::new(move |record| {
                if record.uri == "/token" {
                    oauth()
                } else {
                    reply(response.clone())
                }
            })
            .await;
            let (gateway, _directory) = fake.gateway();
            let error = gateway
                .firebase_query(&FirebaseAction::Projects)
                .await
                .unwrap_err();
            assert_eq!(error.code, "firebase_invalid_response");
            assert!(fake.records().len() <= 3);
        }
    }

    #[tokio::test]
    async fn pages_items_total_bytes_and_final_serialized_size_are_bounded() {
        for case in 0..5 {
            let fake = Fake::new(move |record| {
            if record.uri == "/token" { return oauth(); }
            let url = parsed_url(record);
            let page = url.query_pairs().find(|(key, _)| key == "pageToken")
                .map(|(_, value)| value.parse::<usize>().unwrap()).unwrap_or(0);
            match case {
                0 => (200, vec![b' '; MAX_PAGE_BYTES + 1]),
                1 => reply(json!({"results":[],"nextPageToken":(page + 1).to_string()})),
                2 => reply(json!({"results":vec![project("project-a"); MAX_ITEMS + 1]})),
                3 => reply(json!({"results":[],"padding":"x".repeat(1_900_000),
                    "nextPageToken":(page + 1).to_string()})),
                _ => {
                    let projects = (page * 1000..(page + 1) * 1000).map(|index| {
                        json!({"projectId":format!("project-{index}"),"projectNumber":index.to_string(),
                            "displayName":"n".repeat(512)})
                    }).collect::<Vec<_>>();
                    if page < 3 { reply(json!({"results":projects,"nextPageToken":(page+1).to_string()})) }
                    else { reply(json!({"results":projects})) }
                }
            }
        }).await;
            let (gateway, _directory) = fake.gateway();
            assert_eq!(
                gateway
                    .firebase_query(&FirebaseAction::Projects)
                    .await
                    .unwrap_err()
                    .code,
                "firebase_invalid_response"
            );
            assert!(fake.records().len() <= 101);
        }
    }

    #[tokio::test]
    async fn errors_are_fixed_and_never_disclose_upstream_messages_or_secrets() {
        for (status, code, expected_status, body) in [
            (
                401,
                "firebase_auth_failed",
                502,
                json!({"error":{"message":"PRIVATE-token-key-email"}}),
            ),
            (
                403,
                "firebase_permission_denied",
                403,
                json!({"error":{"message":"PRIVATE-token-key-email"}}),
            ),
            (
                403,
                "firebase_api_disabled",
                403,
                json!({"error":{"details":[{
            "@type":"type.googleapis.com/google.rpc.ErrorInfo","reason":"SERVICE_DISABLED",
            "metadata":{"consumer":"PRIVATE-token-key-email"}}]}}),
            ),
            (
                404,
                "firebase_not_found",
                404,
                json!({"message":"PRIVATE-token-key-email"}),
            ),
            (
                429,
                "firebase_rate_limited",
                429,
                json!({"message":"PRIVATE-token-key-email"}),
            ),
            (
                500,
                "firebase_upstream_error",
                502,
                json!({"message":"PRIVATE-token-key-email"}),
            ),
            (
                302,
                "firebase_upstream_error",
                502,
                json!({"message":"PRIVATE-token-key-email"}),
            ),
        ] {
            let fake = Fake::new(move |record| {
                if record.uri == "/token" {
                    oauth()
                } else {
                    (status, serde_json::to_vec(&body).unwrap())
                }
            })
            .await;
            let (gateway, _directory) = fake.gateway();
            let error = gateway
                .firebase_query(&FirebaseAction::Projects)
                .await
                .unwrap_err();
            assert_eq!(error.code, code);
            assert_eq!(error.status, expected_status);
            let encoded = serde_json::to_string(&error).unwrap();
            assert!(!encoded.contains("PRIVATE"));
            assert!(!encoded.contains("firebase-token-secret"));
        }
    }

    #[tokio::test]
    async fn oauth_rejections_and_unexpected_granted_scopes_never_reach_firebase() {
        for response in [
            (
                400,
                serde_json::to_vec(
                    &json!({"error":"invalid_scope","error_description":"PRIVATE-key"}),
                )
                .unwrap(),
            ),
            reply(
                json!({"access_token":"PRIVATE-token","expires_in":3600,"scope":"https://www.googleapis.com/auth/firebase"}),
            ),
            reply(
                json!({"access_token":"PRIVATE-token","expires_in":3600,"scope":format!("{SCOPE} https://www.googleapis.com/auth/androidpublisher")}),
            ),
            reply(json!({"access_token":"PRIVATE-token","expires_in":0})),
        ] {
            let fake = Fake::new(move |_| response.clone()).await;
            let (gateway, _directory) = fake.gateway();
            let error = gateway
                .firebase_query(&FirebaseAction::Projects)
                .await
                .unwrap_err();
            assert_eq!(error.code, "firebase_auth_failed");
            assert!(!serde_json::to_string(&error).unwrap().contains("PRIVATE"));
            assert_eq!(fake.records().len(), 1);
        }
    }

    #[tokio::test]
    async fn firebase_publisher_and_reporting_tokens_have_independent_scopes_and_caches() {
        let fake = Fake::new(|record| {
            if record.uri == "/token" {
                let scope = claims(record)["scope"].as_str().unwrap().to_owned();
                let token = match scope.as_str() {
                    SCOPE => "firebase-token",
                    "https://www.googleapis.com/auth/androidpublisher" => "publisher-token",
                    "https://www.googleapis.com/auth/playdeveloperreporting" => "reporting-token",
                    _ => panic!("unexpected OAuth scope"),
                };
                return reply(json!({"access_token":token,"expires_in":3600,"scope":scope}));
            }
            let url = parsed_url(record);
            assert_eq!(record.method, "GET");
            match url.path() {
                "/v1beta1/projects" => {
                    assert_eq!(record.headers["authorization"], "Bearer firebase-token");
                    reply(json!({}))
                }
                "/v1beta1/apps:search" => {
                    assert_eq!(record.headers["authorization"], "Bearer reporting-token");
                    reply(json!({"apps":[]}))
                }
                _ => panic!("unexpected vendor request"),
            }
        })
        .await;
        let (gateway, _directory) = fake.gateway();
        for _ in 0..2 {
            gateway
                .firebase_query(&FirebaseAction::Projects)
                .await
                .unwrap();
            assert_eq!(
                gateway.token(Platform::Android).await.unwrap(),
                "publisher-token"
            );
            gateway.account_catalog(Platform::Android).await.unwrap();
        }
        let records = fake.records();
        assert_eq!(
            records
                .iter()
                .filter(|record| record.uri == "/token")
                .count(),
            3
        );
        assert_eq!(records.len(), 7);
        assert!(records
            .iter()
            .all(|record| record.uri == "/token" || record.method == "GET"));
        assert!(records.iter().all(|record| !record.uri.contains("edits")));
    }

    #[tokio::test]
    async fn unavailable_modes_invalid_paths_and_missing_credentials_fail_before_network() {
        let fake = Fake::new(|_| panic!("unexpected network request")).await;
        for (mode, code) in [
            ("live", "firebase_not_configured"),
            ("mock", "firebase_unsupported"),
        ] {
            let gateway = Gateway::new(ProviderConfig {
                mode: mode.into(),
                google_firebase_base_url: fake.url.clone(),
                google_token_url: fake.url.clone(),
                ..Default::default()
            })
            .unwrap();
            assert_eq!(
                gateway
                    .firebase_query(&FirebaseAction::Projects)
                    .await
                    .unwrap_err()
                    .code,
                code
            );
        }
        let (gateway, _directory) = fake.gateway();
        for project_id in [
            "../unsafe",
            "123456",
            "https://evil.invalid",
            "project-a?token=secret",
        ] {
            assert!(gateway
                .firebase_query(&FirebaseAction::AndroidApps {
                    project_id: project_id.into()
                })
                .await
                .is_err());
        }
        assert!(fake.records().is_empty());
        for endpoint in [
            "https://user:secret@example.invalid",
            "https://example.invalid/path",
            "https://example.invalid/?key=secret",
            "https://example.invalid/#secret",
        ] {
            assert!(Gateway::new(ProviderConfig {
                google_firebase_base_url: endpoint.into(),
                ..Default::default()
            })
            .is_err());
        }
    }

    #[tokio::test]
    async fn apps_reject_cross_project_rows_invalid_packages_and_duplicate_app_ids() {
        let mut cross_project = app(0);
        cross_project["projectId"] = json!("project-b");
        let mut invalid_package = app(0);
        invalid_package["packageName"] = json!("../secret");
        for response in [
            json!({"apps":[cross_project]}),
            json!({"apps":[invalid_package]}),
            json!({"apps":[app(0),app(0)]}),
        ] {
            let fake = Fake::new(move |record| {
                if record.uri == "/token" {
                    oauth()
                } else {
                    reply(response.clone())
                }
            })
            .await;
            let (gateway, _directory) = fake.gateway();
            assert_eq!(
                gateway
                    .firebase_query(&FirebaseAction::AndroidApps {
                        project_id: "project-a".into()
                    })
                    .await
                    .unwrap_err()
                    .code,
                "firebase_invalid_response"
            );
        }
        let fake = Fake::new(|record| {
            if record.uri == "/token" {
                oauth()
            } else {
                reply(json!({}))
            }
        })
        .await;
        let (gateway, _directory) = fake.gateway();
        assert_eq!(
            gateway
                .firebase_query(&FirebaseAction::AndroidApps {
                    project_id: "project-a".into()
                })
                .await
                .unwrap(),
            json!({"projectId":"project-a","apps":[]})
        );
    }

    #[tokio::test]
    async fn configured_node_logs_never_receive_firebase_credentials_or_error_bodies() {
        let fake = Fake::new(|record| {
            if record.uri == "/token" {
                oauth()
            } else {
                (
                    403,
                    br#"{"error":{"message":"PRIVATE-upstream-body-key"}}"#.to_vec(),
                )
            }
        })
        .await;
        let (gateway, directory) = fake.gateway();
        let log =
            diagnostics::Log::for_config(&directory.path().join("fake-account.json"), "fake-node")
                .unwrap();
        log.event("firebase_test", json!({"safe":"sentinel"}));
        let gateway =
            Gateway::with_log(gateway.inner.config.clone(), None, Some(log.clone())).unwrap();
        assert_eq!(
            gateway
                .firebase_query(&FirebaseAction::Projects)
                .await
                .unwrap_err()
                .code,
            "firebase_permission_denied"
        );
        let mut text = String::new();
        for entry in std::fs::read_dir(log.directory()).unwrap() {
            text.push_str(&std::fs::read_to_string(entry.unwrap().path()).unwrap());
        }
        assert!(text.contains("sentinel"));
        for secret in [
            "PRIVATE",
            "firebase-token-secret",
            "fake-only@example.invalid",
            "PRIVATE KEY",
            "assertion",
        ] {
            assert!(!text.contains(secret));
        }
    }
}

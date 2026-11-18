use super::*;
use store_protocol::firebase::{self, FirebaseRequest};

impl Executor {
    pub async fn execute_firebase(self: Arc<Self>, request: FirebaseRequest) -> StoreReply {
        let id = request.request_id.clone();
        if let Err(error) = request.validate() {
            return StoreReply::from_result(id, Err(error));
        }
        if self.config.platform_mode == PlatformMode::Ios {
            return StoreReply::from_result(
                id,
                Err(firebase::public_error("firebase_not_configured")),
            );
        }
        let Ok(_guard) = self.firebase_capacity.try_lock() else {
            return StoreReply::from_result(id, Err(firebase::public_error("firebase_busy")));
        };
        let result = self
            .gateway
            .firebase_query(&request.firebase_action)
            .await
            .and_then(|body| {
                firebase::validate_response(&request.firebase_action, &body)?;
                Ok(body)
            })
            .map_err(|error| firebase::public_error(&error.code));
        if let Some(log) = &self.log {
            let (status, count) = match &result {
                Ok(body) => (
                    200,
                    body["projects"]
                        .as_array()
                        .or_else(|| body["apps"].as_array())
                        .map_or(0, Vec::len),
                ),
                Err(error) => (error.status, 0),
            };
            log.event(
                "firebase.completed",
                json!({"status":status,"itemCount":count}),
            );
        }
        StoreReply::from_result(id, result)
    }
}

pub(super) async fn http_firebase(
    State(executor): State<Arc<Executor>>,
    headers: HeaderMap,
    Json(request): Json<FirebaseRequest>,
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
    Json(executor.execute_firebase(request).await).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use store_protocol::firebase::FirebaseAction;
    fn executor(mode: &str) -> Arc<Executor> {
        Executor::new(serde_json::from_value(json!({
            "connectorId":"firebase-test", "token":"firebase-test-token", "serverUrl":"http://127.0.0.1:1",
            "messageKey":"11".repeat(32),"uploadKey":"22".repeat(32),"platformMode":mode,
            "provider":{"mode":"mock","googleFirebaseBaseUrl":if mode == "ios" {"bad-url"} else {"https://firebase.googleapis.com"}}
        })).unwrap()).unwrap()
    }
    fn request() -> FirebaseRequest {
        FirebaseRequest {
            request_id: "fixture".into(),
            firebase_action: FirebaseAction::Projects,
        }
    }
    #[tokio::test]
    async fn ios_and_mock_reject_and_diagnostics_have_independent_capacity() {
        let ios = executor("ios");
        assert_eq!(
            ios.execute_firebase(request()).await.body["error"]["code"],
            "firebase_not_configured"
        );
        let node = executor("all");
        let _business = node.capacity.acquire().await.unwrap();
        assert_eq!(
            node.clone().execute_firebase(request()).await.body["error"]["code"],
            "firebase_unsupported"
        );
        let _firebase = node.firebase_capacity.lock().await;
        let reply = node.clone().execute_firebase(request()).await;
        assert_eq!(reply.status, 429);
        assert_eq!(reply.body["error"]["code"], "firebase_busy");
    }
}

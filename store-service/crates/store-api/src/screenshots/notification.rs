//! Callback transport never includes remote text or credentials in diagnostics.
use crate::{
    registry::screenshot_notifications::{Notification, Outcome, MAX_BODY},
    ApiState,
};
use reqwest::{Client, Url};
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};

fn accepted(value: &Value, body: &Value, reconcile: bool) -> bool {
    valid_snapshot(value, body, reconcile, false)
}

fn valid_snapshot(value: &Value, body: &Value, reconcile: bool, allow_rejected: bool) -> bool {
    let job = value.get("job").unwrap_or(value);
    if job["upstream_job_id"] != body["upstream_job_id"]
        || job["callback_revision"] != body["revision"]
        || !job["cancelled_at"].is_null()
        || value.get("ignored").is_some()
    {
        return false;
    }
    if reconcile {
        if job["callback_revision"] != body["revision"] || job["callback_status"] != body["status"]
        {
            return false;
        }
    } else if value["ok"] != true {
        return false;
    }
    let results = if reconcile {
        &job["callback_results"]
    } else {
        &value["items"]
    };
    let Some(results) = results.as_array() else {
        return false;
    };
    let Some(items) = body["items"].as_array() else {
        return false;
    };
    results.len() == items.len()
        && items.iter().enumerate().all(|(i, item)| {
            results
                .iter()
                .filter(|r| {
                    r["index"].as_u64() == Some(i as u64)
                        && r["slot"] == item["slot"]
                        && r["language"] == item["language"]
                        && (matches!(r["result"].as_str(), Some("created" | "skipped"))
                            || (allow_rejected && r["result"] == "rejected"))
                })
                .count()
                == 1
        })
}

async fn bounded(mut response: reqwest::Response) -> Option<Value> {
    let mut data = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if chunk.len() > MAX_BODY - data.len() {
            return None;
        }
        data.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&data).ok()
}

async fn send(client: &Client, n: &Notification) -> Outcome {
    let body: Value = match serde_json::from_str(&n.body) {
        Ok(v) => v,
        Err(_) => return Outcome::Failed("callback_body_invalid"),
    };
    let response = client
        .post(&n.url)
        .bearer_auth(&n.token)
        .header("content-type", "application/json")
        .body(n.body.clone())
        .send()
        .await;
    if let Ok(response) = response {
        match response.status().as_u16() {
            200 => {
                if let Some(value) = bounded(response).await {
                    if accepted(&value, &body, false) {
                        return Outcome::Delivered(safe_results(&value["items"]));
                    }
                    if valid_snapshot(&value, &body, false, true) {
                        return Outcome::RetryResults(safe_results(&value["items"]));
                    }
                    let job = value.get("job").unwrap_or(&value);
                    let same_job = job["upstream_job_id"] == body["upstream_job_id"];
                    if same_job && value["ignored"] == "stale_revision" {
                        return Outcome::Failed("callback_stale_revision");
                    }
                    if same_job && !job["cancelled_at"].is_null() {
                        return Outcome::Failed("callback_cancelled");
                    }
                    return Outcome::Retry("callback_response_invalid");
                }
            }
            401 => return Outcome::Unauthorized,
            400 | 422 => return Outcome::Failed("callback_validation_failed"),
            409 => return Outcome::Failed("callback_conflict"),
            413 => return Outcome::Failed("callback_body_too_large"),
            404 | 500..=599 => {}
            _ => return Outcome::Failed("callback_http_rejected"),
        }
    }
    // A lost response may already have committed the immutable snapshot. Coverage alone
    // cannot establish delivery; require revision, execution status and every item result.
    if let Ok(mut url) = Url::parse(&n.url) {
        if let Ok(mut segments) = url.path_segments_mut() {
            segments.pop_if_empty().push(&n.job);
        }
        if let Ok(response) = client.get(url).bearer_auth(&n.token).send().await {
            if response.status() == 200 {
                if let Some(value) = bounded(response).await {
                    if accepted(&value, &body, true) {
                        let job = value.get("job").unwrap_or(&value);
                        return Outcome::Delivered(safe_results(&job["callback_results"]));
                    }
                    if valid_snapshot(&value, &body, true, true) {
                        let job = value.get("job").unwrap_or(&value);
                        return Outcome::RetryResults(safe_results(&job["callback_results"]));
                    }
                }
            } else if response.status() == 401 {
                return Outcome::Unauthorized;
            }
        }
    }
    Outcome::Retry("callback_transport_failed")
}

fn safe_results(value: &Value) -> Value {
    json!(value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            let index = r["index"].as_u64()?;
            let result = r["result"].as_str()?;
            matches!(result, "created" | "skipped" | "rejected")
                .then(|| json!({"index":index,"result":result}))
        })
        .collect::<Vec<_>>())
}

pub async fn run(state: Arc<ApiState>) {
    let Ok(client) = Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(25))
        .build()
    else {
        return;
    };
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    loop {
        interval.tick().await;
        let time = store_protocol::upload::now().unwrap_or_default() as i64;
        match state.registry.claim_notification(time) {
            Ok(Some(n)) => {
                // One active transfer bounds concurrency. The overall budget includes reconciliation.
                let outcome = tokio::time::timeout(Duration::from_secs(25), send(&client, &n))
                    .await
                    .unwrap_or(Outcome::Retry("callback_timeout"));
                let time = store_protocol::upload::now().unwrap_or_default() as i64;
                if state
                    .registry
                    .finish_notification(&n, outcome, time)
                    .is_err()
                {
                    state.registry.logs.record(crate::logs::Event {
                        name: "screenshot.callback",
                        details: json!({"error":"callback_registry_unavailable"}),
                        ..Default::default()
                    });
                }
            }
            Ok(None) => {}
            Err(_) => state.registry.logs.record(crate::logs::Event {
                name: "screenshot.callback",
                details: json!({"error":"callback_registry_unavailable"}),
                ..Default::default()
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::IntoFuture;
    const JOB: &str = "22222222-2222-4222-8222-222222222222";
    #[test]
    fn acceptance_requires_each_item_not_coverage() {
        let body = json!({"upstream_job_id":JOB,"revision":1,"status":"partial","items":[{"slot":"details","language":"en"}]});
        let mut reply = json!({"ok":true,"job":{"upstream_job_id":JOB,"callback_revision":1},"items":[{"index":0,"slot":"details","language":"en","result":"rejected"}]});
        assert!(!accepted(&reply, &body, false));
        reply["items"][0]["result"] = json!("skipped");
        assert!(accepted(&reply, &body, false));
        let mut reconciled = json!({"upstream_job_id":JOB,"callback_revision":1,"callback_status":"partial","coverage_status":"done","received_at":"yes","callback_results":reply["items"]});
        assert!(accepted(&reconciled, &body, true));
        reconciled["callback_revision"] = json!(2);
        assert!(!accepted(&reconciled, &body, true));
        reconciled["callback_revision"] = json!(1);
        reconciled["cancelled_at"] = json!("now");
        assert!(!accepted(&reconciled, &body, true));
        reply["ignored"] = json!("stale_revision");
        assert!(!accepted(&reply, &body, false));
        reply.as_object_mut().unwrap().remove("ignored");
        reply["job"]["upstream_job_id"] = json!("another");
        assert!(!accepted(&reply, &body, false));
        assert_eq!(
            safe_results(
                &json!([{"index":{"token":"secret"},"result":"created"},{"index":1,"result":{"url":"secret"}},{"index":2,"result":"rejected","message":"secret"}])
            ),
            json!([{"index":2,"result":"rejected"}])
        );
    }

    #[tokio::test]
    async fn http_transport_retries_exact_body_and_reconciles_without_coverage_shortcut() {
        use axum::{
            extract::State,
            http::StatusCode,
            routing::{get, post},
            Router,
        };
        use std::sync::Mutex;
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        async fn receive(
            State(seen): State<Arc<Mutex<Vec<String>>>>,
            headers: axum::http::HeaderMap,
            body: String,
        ) -> StatusCode {
            assert_eq!(headers["authorization"], "Bearer fixture-token");
            let value: Value = serde_json::from_str(&body).unwrap();
            let mut keys = value
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>();
            keys.sort();
            assert_eq!(keys, vec!["items", "revision", "status", "upstream_job_id"]);
            assert!(uuid::Uuid::parse_str(value["upstream_job_id"].as_str().unwrap()).is_ok());
            let mut keys = value["items"][0]
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>();
            keys.sort();
            assert_eq!(keys, vec!["language", "slot", "url", "version"]);
            seen.lock().unwrap().push(body);
            StatusCode::SERVICE_UNAVAILABLE
        }
        async fn reconcile() -> axum::Json<Value> {
            axum::Json(
                json!({"upstream_job_id":JOB,"callback_revision":1,"callback_status":"partial","callback_results":[{"index":0,"slot":"details","language":"en","result":"skipped"}],"coverage_status":"partial"}),
            )
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(
            axum::serve(
                listener,
                Router::new()
                    .route("/api/hooks/captures", post(receive))
                    .route(&format!("/api/hooks/captures/{JOB}"), get(reconcile))
                    .with_state(seen.clone()),
            )
            .into_future(),
        );
        let body=json!({"upstream_job_id":JOB,"revision":1,"status":"partial","items":[{"slot":"details","language":"en","url":"https://images.example/a.png","version":"abc"}]}).to_string();
        let n = Notification {
            job: JOB.into(),
            body: body.clone(),
            lease: "lease".into(),
            url: format!("http://{address}/api/hooks/captures"),
            token: "fixture-token".into(),
        };
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        assert!(matches!(send(&client, &n).await, Outcome::Delivered(_)));
        assert!(matches!(send(&client, &n).await, Outcome::Delivered(_)));
        assert_eq!(*seen.lock().unwrap(), vec![body.clone(), body]);
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn http_auth_conflict_and_unknown_job_stop_or_retry_correctly() {
        use axum::{http::StatusCode, routing::post, Router};
        for (status, expected) in [
            (401, "unauthorized"),
            (409, "failed"),
            (404, "retry"),
            (413, "failed"),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let app = Router::new().route(
                "/captures",
                post(move || async move { StatusCode::from_u16(status).unwrap() }),
            );
            let server = tokio::spawn(axum::serve(listener, app).into_future());
            let n = Notification {
                job: JOB.into(),
                body: json!({"upstream_job_id":JOB,"revision":1,"status":"failed","items":[]})
                    .to_string(),
                lease: "lease".into(),
                url: format!("http://{address}/captures"),
                token: "fixture".into(),
            };
            let result = send(&Client::new(), &n).await;
            assert_eq!(
                match result {
                    Outcome::Unauthorized => "unauthorized",
                    Outcome::Failed(_) => "failed",
                    Outcome::Retry(_) => "retry",
                    _ => "delivered",
                },
                expected
            );
            server.abort();
            let _ = server.await;
        }
    }

    #[tokio::test]
    async fn http_rejected_items_and_oversized_response_are_not_delivered() {
        use axum::{routing::post, Router};
        for oversized in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let reply = if oversized {
                "x".repeat(MAX_BODY + 1)
            } else {
                json!({"ok":true,"job":{"upstream_job_id":JOB,"callback_revision":1},"items":[{"index":0,"slot":"details","language":"pt","result":"rejected","message":"secret upstream text"}]}).to_string()
            };
            let app = Router::new().route(
                "/captures",
                post(move || {
                    let reply = reply.clone();
                    async move { reply }
                }),
            );
            let server = tokio::spawn(axum::serve(listener, app).into_future());
            let n=Notification{job:JOB.into(),body:json!({"upstream_job_id":JOB,"revision":1,"status":"partial","items":[{"slot":"details","language":"pt","url":"https://images.example/a.png","version":"abc"}]}).to_string(),lease:"lease".into(),url:format!("http://{address}/captures"),token:"fixture".into()};
            match send(&Client::new(), &n).await {
                Outcome::RetryResults(results) if !oversized => {
                    assert_eq!(results, json!([{"index":0,"result":"rejected"}]))
                }
                Outcome::Retry(_) if oversized => {}
                _ => panic!("invalid callback acceptance"),
            }
            server.abort();
            let _ = server.await;
        }
    }

    #[tokio::test]
    async fn wrong_job_disposition_never_terminates_our_snapshot_and_get_retains_partial_results() {
        use axum::{
            http::StatusCode,
            routing::{get, post},
            Router,
        };
        for mode in [
            "foreign_cancelled",
            "foreign_stale",
            "matching_stale",
            "get_partial",
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let id = if mode == "matching_stale" {
                JOB
            } else {
                "33333333-3333-4333-8333-333333333333"
            };
            let reply = json!({"ok":true,"ignored":"stale_revision","job":{"upstream_job_id":id,"callback_revision":2,"cancelled_at":"now"},"items":[]});
            let partial = json!({"upstream_job_id":JOB,"callback_revision":1,"callback_status":"partial","callback_results":[{"index":0,"slot":"details","language":"pt","result":"created"},{"index":1,"slot":"ranking","language":"pt","result":"rejected","message":"secret"}]});
            let app = Router::new()
                .route(
                    "/captures",
                    post(move || {
                        let reply = reply.clone();
                        async move {
                            if mode == "get_partial" {
                                (StatusCode::SERVICE_UNAVAILABLE, axum::Json(Value::Null))
                            } else {
                                (StatusCode::OK, axum::Json(reply))
                            }
                        }
                    }),
                )
                .route(
                    &format!("/captures/{JOB}"),
                    get(move || {
                        let partial = partial.clone();
                        async move { axum::Json(partial) }
                    }),
                );
            let server = tokio::spawn(axum::serve(listener, app).into_future());
            let n=Notification{job:JOB.into(),body:json!({"upstream_job_id":JOB,"revision":1,"status":"partial","items":[{"slot":"details","language":"pt","url":"https://images.example/a.png","version":"a"},{"slot":"ranking","language":"pt","url":"https://images.example/b.png","version":"b"}]}).to_string(),lease:"lease".into(),url:format!("http://{address}/captures"),token:"fixture".into()};
            match (mode, send(&Client::new(), &n).await) {
                ("matching_stale", Outcome::Failed("callback_stale_revision")) => {}
                ("get_partial", Outcome::RetryResults(results)) => assert_eq!(
                    results,
                    json!([{"index":0,"result":"created"},{"index":1,"result":"rejected"}])
                ),
                (
                    "foreign_cancelled" | "foreign_stale",
                    Outcome::Retry("callback_response_invalid"),
                ) => {}
                _ => panic!("incorrect disposition or lost partial evidence"),
            }
            server.abort();
            let _ = server.await;
        }
    }
}

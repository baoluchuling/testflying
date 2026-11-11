//! Center-owned, bounded structured records. Never copy HTTP bodies or credentials here.
use crate::{registry::Registry, ApiState, Result};
use axum::{
    extract::{MatchedPath, Query, Request, State},
    http::{header, HeaderValue},
    middleware::Next,
    response::Response,
    Json,
};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use store_protocol::{Operation, Platform, StoreError};
use uuid::Uuid;

pub const RETENTION_MS: i64 = 7 * 24 * 60 * 60 * 1000;
pub const REQUEST_HEADER: &str = "x-testflying-request-id";

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
fn unavailable(_: rusqlite::Error) -> StoreError {
    StoreError::new(503, "logs_unavailable", "Center log storage unavailable")
}

pub struct LogStore {
    db: Arc<Mutex<Connection>>,
    healthy: AtomicBool,
}

impl LogStore {
    pub fn record_service(&self, event: &'static str) {
        let name = match event {
            "service.started" => "service.started",
            "service.stopping" => "service.stopping",
            _ => "service.event",
        };
        self.record(Event {
            name,
            details: json!({"retentionDays":7}),
            ..Default::default()
        });
    }
    pub(crate) fn new(db: Arc<Mutex<Connection>>) -> std::result::Result<Arc<Self>, StoreError> {
        {
            let connection = db.lock().unwrap();
            connection
                .pragma_update(None, "secure_delete", "ON")
                .map_err(unavailable)?;
            connection
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS center_logs (
                id INTEGER PRIMARY KEY AUTOINCREMENT, timestamp_ms INTEGER NOT NULL,
                event TEXT NOT NULL, request_id TEXT, connector_id TEXT, platform TEXT,
                identifier TEXT, status INTEGER, details TEXT NOT NULL);
                CREATE INDEX IF NOT EXISTS center_logs_time ON center_logs(timestamp_ms);
                CREATE INDEX IF NOT EXISTS center_logs_request ON center_logs(request_id,id);
                CREATE INDEX IF NOT EXISTS center_logs_connector ON center_logs(connector_id,id);",
                )
                .map_err(unavailable)?;
        }
        let logs = Arc::new(Self {
            db,
            healthy: AtomicBool::new(true),
        });
        logs.prune()?;
        Ok(logs)
    }

    pub fn healthy(&self) -> bool {
        self.healthy.load(Ordering::Relaxed)
    }

    pub fn prune(&self) -> std::result::Result<(), StoreError> {
        let result = self.prune_at(now());
        self.healthy.store(result.is_ok(), Ordering::Relaxed);
        result
    }

    fn prune_at(&self, timestamp: i64) -> std::result::Result<(), StoreError> {
        self.db
            .lock()
            .unwrap()
            .execute(
                "DELETE FROM center_logs WHERE timestamp_ms <= ?1",
                [timestamp - RETENTION_MS],
            )
            .map_err(unavailable)?;
        Ok(())
    }

    pub(crate) fn record(&self, event: Event) {
        let result = self.insert_at(event, now());
        // Do not duplicate business records into an independently retained system journal.
        if result.is_err() && self.healthy.swap(false, Ordering::Relaxed) {
            eprintln!("Center log storage unavailable");
        }
    }

    fn insert_at(&self, event: Event, timestamp: i64) -> std::result::Result<(), StoreError> {
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction().map_err(unavailable)?;
        tx.execute(
            "DELETE FROM center_logs WHERE timestamp_ms <= ?1",
            [timestamp - RETENTION_MS],
        )
        .map_err(unavailable)?;
        let details = serde_json::to_string(&event.details).expect("structured log serialization");
        // Every producer supplies fixed keys and bounded metadata, never arbitrary payloads.
        let details = if details.len() <= 16384 {
            details
        } else {
            "{\"summaryTruncated\":true}".into()
        };
        tx.execute("INSERT INTO center_logs(timestamp_ms,event,request_id,connector_id,platform,identifier,status,details)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8)", params![timestamp,event.name,event.request_id,event.connector_id,
                event.platform.map(|p| p.to_string()),event.identifier,event.status,details]).map_err(unavailable)?;
        tx.commit().map_err(unavailable)?;
        Ok(())
    }

    pub fn query(&self, query: &LogQuery) -> std::result::Result<Value, StoreError> {
        query.validate()?;
        self.prune()?;
        let db = self.db.lock().unwrap();
        let mut stmt = db.prepare("SELECT id,timestamp_ms,event,request_id,connector_id,platform,identifier,status,details
            FROM center_logs WHERE timestamp_ms > ?1 AND (?2 IS NULL OR id < ?2)
            AND (?3 IS NULL OR request_id=?3) AND (?4 IS NULL OR connector_id=?4)
            AND (?5 IS NULL OR platform=?5) AND (?6 IS NULL OR identifier=?6)
            AND (?7 IS NULL OR event=?7) AND (?8 IS NULL OR status=?8)
            AND (?9=0 OR status>=400 OR json_extract(details,'$.response.businessStatus') IN ('failed','blocked')
                OR json_extract(details,'$.response.canSync')=0) ORDER BY id DESC LIMIT ?10").map_err(unavailable)?;
        let rows = stmt.query_map(params![now()-RETENTION_MS,query.before,query.request_id,query.connector_id,
            query.platform.map(|p|p.to_string()),query.identifier,query.event,query.status,query.errors_only,query.limit+1], |row| {
            let details: String = row.get(8)?;
            Ok(json!({"id":row.get::<_,i64>(0)?,"timestampMs":row.get::<_,i64>(1)?,
                "event":row.get::<_,String>(2)?,"requestId":row.get::<_,Option<String>>(3)?,
                "connectorId":row.get::<_,Option<String>>(4)?,"platform":row.get::<_,Option<String>>(5)?,
                "identifier":row.get::<_,Option<String>>(6)?,"status":row.get::<_,Option<u16>>(7)?,
                "details":serde_json::from_str::<Value>(&details).unwrap_or(Value::Null)}))
        }).map_err(unavailable)?;
        let mut entries = rows
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(unavailable)?;
        let more = entries.len() > query.limit as usize;
        entries.truncate(query.limit as usize);
        let cursor = if more {
            entries.last().map(|entry| entry["id"].clone())
        } else {
            None
        };
        Ok(
            json!({"entries":entries,"nextCursor":cursor,"retentionDays":7,"storageHealthy":self.healthy()}),
        )
    }
}

#[derive(Clone, Default)]
pub(crate) struct Event {
    pub name: &'static str,
    pub request_id: Option<String>,
    pub connector_id: Option<String>,
    pub platform: Option<Platform>,
    pub identifier: Option<String>,
    pub status: Option<u16>,
    pub details: Value,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LogQuery {
    pub request_id: Option<String>,
    pub connector_id: Option<String>,
    pub platform: Option<Platform>,
    pub identifier: Option<String>,
    pub event: Option<String>,
    pub status: Option<u16>,
    pub before: Option<i64>,
    #[serde(default)]
    pub errors_only: bool,
    #[serde(default = "page_limit")]
    pub limit: u32,
}
fn page_limit() -> u32 {
    100
}
impl Default for LogQuery {
    fn default() -> Self {
        Self {
            request_id: None,
            connector_id: None,
            platform: None,
            identifier: None,
            event: None,
            status: None,
            before: None,
            errors_only: false,
            limit: page_limit(),
        }
    }
}
impl LogQuery {
    fn validate(&self) -> std::result::Result<(), StoreError> {
        if !(1..=200).contains(&self.limit)
            || self.before.is_some_and(|n| n <= 0)
            || self.status.is_some_and(|n| !(100..=599).contains(&n))
            || [
                &self.request_id,
                &self.connector_id,
                &self.identifier,
                &self.event,
            ]
            .into_iter()
            .flatten()
            .any(|s| s.is_empty() || s.len() > 180 || s.chars().any(char::is_control))
        {
            return Err(StoreError::new(
                422,
                "invalid_log_query",
                "Invalid log filters or page size",
            ));
        }
        Ok(())
    }
}

pub(super) async fn read(
    State(state): State<Arc<ApiState>>,
    Query(query): Query<LogQuery>,
) -> Result<Json<Value>> {
    Ok(Json(state.registry.logs.query(&query)?))
}

#[derive(Clone)]
pub(crate) struct RequestContext(pub String);

#[derive(Clone)]
pub(crate) struct ErrorBody(pub Value);

#[derive(Clone, Serialize)]
pub(crate) struct Actor {
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ErrorSummary {
    code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    upstream_status: Option<u16>,
}
pub(crate) fn error_summary(code: &str, message: &str) -> ErrorSummary {
    let code = match code {
        "store_upstream_error" => "store_upstream_error",
        "connector_timeout" => "connector_timeout",
        "connector_disconnected" => "connector_disconnected",
        "connector_busy" => "connector_busy",
        "connector_rejected" => "connector_rejected",
        "invalid_connector_reply" => "invalid_connector_reply",
        "connector_disabled" => "connector_disabled",
        "store_version_not_editable" => "store_version_not_editable",
        "store_version_missing" => "store_version_missing",
        "screenshot_app_not_found" => "screenshot_app_not_found",
        "screenshot_connector_disabled" => "screenshot_connector_disabled",
        "invalid_screenshot_connector" => "invalid_screenshot_connector",
        "connector_upgrade_required" => "connector_upgrade_required",
        "platform_forwarding_disabled" => "platform_forwarding_disabled",
        "app_not_found" => "app_not_found",
        "unauthorized" => "unauthorized",
        "forbidden" => "forbidden",
        "invalid_app_identifier" => "invalid_app_identifier",
        "invalid_payload" => "invalid_payload",
        "unsupported_platform" => "unsupported_platform",
        "invalid_locale" => "invalid_locale",
        "invalid_device" => "invalid_device",
        "rate_limited" => "rate_limited",
        "invalid_upload" => "invalid_upload",
        "upload_failed" => "upload_failed",
        "encrypted_protocol_required" => "encrypted_protocol_required",
        "connector_already_connected" => "connector_already_connected",
        "logs_unavailable" => "logs_unavailable",
        "invalid_log_query" => "invalid_log_query",
        "logs_upgrade_required" => "logs_upgrade_required",
        "invalid_log_request" => "invalid_log_request",
        _ => "request_failed",
    };
    let upstream_status = (code == "store_upstream_error")
        .then(|| {
            message
                .strip_prefix("Store upstream returned HTTP ")
                .and_then(|s| s.parse::<u16>().ok())
                .filter(|s| (300..=599).contains(s))
        })
        .flatten();
    ErrorSummary {
        code,
        upstream_status,
    }
}

pub(crate) fn reply_summary(body: &Value) -> Value {
    let mut summary = json!({});
    if let Some(status) = body.get("status").and_then(Value::as_str) {
        summary["businessStatus"] = json!(match status {
            "success" => "success",
            "succeeded" => "succeeded",
            "failed" => "failed",
            "blocked" => "blocked",
            "pending" => "pending",
            _ => "other",
        });
    }
    if let Some(can_sync) = body.get("canSync").and_then(Value::as_bool) {
        summary["canSync"] = json!(can_sync);
    }
    for name in [
        "locales",
        "listings",
        "reviews",
        "releases",
        "warnings",
        "errors",
        "blockingReasons",
    ] {
        if let Some(values) = body.get(name).and_then(Value::as_array) {
            summary[name] = json!({"count":values.len()});
        }
    }
    summary
}

struct HttpGuard {
    logs: Arc<LogStore>,
    event: Event,
    start: Instant,
    completed: bool,
}
impl Drop for HttpGuard {
    fn drop(&mut self) {
        if !self.completed {
            let mut event = self.event.clone();
            event.name = "http.cancelled";
            event.status = Some(499);
            event.details["durationMs"] = json!(self.start.elapsed().as_millis() as u64);
            event.details["outcomeUnknown"] = json!(true);
            self.logs.record(event);
        }
    }
}

pub(super) async fn observe(
    State(state): State<Arc<ApiState>>,
    mut request: Request,
    next: Next,
) -> Response {
    let id = Uuid::new_v4().to_string();
    request.extensions_mut().insert(RequestContext(id.clone()));
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map_or("unmatched", MatchedPath::as_str);
    let mut event = Event {
        name: "http.started",
        request_id: Some(id.clone()),
        details: json!({"method":request.method().as_str(),"route":route}),
        ..Default::default()
    };
    // Parse only known path positions. The path and query themselves are never copied.
    let segments: Vec<_> = request.uri().path().split('/').collect();
    if segments.get(1) == Some(&"v1")
        && segments.get(2) == Some(&"store-management")
        && segments.get(3) == Some(&"apps")
    {
        event.platform = segments.get(4).and_then(|s| match *s {
            "ios" => Some(Platform::Ios),
            "android" => Some(Platform::Android),
            _ => None,
        });
        event.identifier = segments
            .get(5)
            .filter(|s| store_protocol::validate_identifier(s).is_ok())
            .map(|s| s.to_string());
    } else if segments.get(1) == Some(&"v1") && segments.get(2) == Some(&"apps") {
        event.platform = segments.get(3).and_then(|s| match *s {
            "ios" => Some(Platform::Ios),
            "android" => Some(Platform::Android),
            _ => None,
        });
        event.identifier = segments
            .get(4)
            .filter(|s| store_protocol::validate_identifier(s).is_ok())
            .map(|s| s.to_string());
    }
    if let Some(index) = segments.iter().position(|s| *s == "connectors") {
        event.connector_id = segments
            .get(index + 1)
            .and_then(|s| Uuid::parse_str(s).ok())
            .map(|s| s.to_string());
    }
    state.registry.logs.record(event.clone());
    let mut guard = HttpGuard {
        logs: state.registry.logs.clone(),
        event,
        start: Instant::now(),
        completed: false,
    };
    let mut response = next.run(request).await;
    guard.event.name = "http.completed";
    guard.event.status = Some(response.status().as_u16());
    guard.event.details["durationMs"] = json!(guard.start.elapsed().as_millis() as u64);
    if let Some(actor) = response.extensions().get::<Actor>() {
        guard.event.details["actor"] = json!(actor);
    }
    if let Some(error) = response.extensions().get::<ErrorSummary>() {
        guard.event.details["error"] = json!(error);
    }
    if let Some(ErrorBody(mut body)) = response.extensions_mut().remove::<ErrorBody>() {
        if let Some(object) = body.get_mut("error").and_then(Value::as_object_mut) {
            object.insert("requestId".into(), json!(id));
        }
        *response.body_mut() = axum::body::Body::from(body.to_string());
        response.headers_mut().remove(header::CONTENT_LENGTH);
    }
    guard.logs.record(guard.event.clone());
    guard.completed = true;
    response.headers_mut().insert(
        REQUEST_HEADER,
        HeaderValue::from_str(&id).expect("UUID header"),
    );
    response
}

/// Select explicit non-secret selectors and text fingerprints; never traverse arbitrary JSON.
pub(crate) fn payload_summary(payload: &Value) -> Value {
    let mut out = json!({"fieldCount":payload.as_object().map_or(0,|v|v.len())});
    for name in [
        "locale",
        "language",
        "version",
        "versionString",
        "device",
        "track",
        "pageSize",
        "rating",
        "page",
        "operation",
    ] {
        if let Some(value) = payload.get(name) {
            out[name] = if let Some(s) = value.as_str() {
                if s.len() <= 80
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
                {
                    json!(s)
                } else {
                    json!({"invalid":true,"bytes":s.len()})
                }
            } else if value.is_number() || value.is_boolean() {
                value.clone()
            } else {
                json!({"invalid":true})
            };
        }
    }
    if let Some(scopes) = payload.get("syncScopes") {
        out["syncScopes"] = json!(scopes.as_array().map(|values| values
            .iter()
            .take(16)
            .map(|v| match v.as_str() {
                Some("metadata") => "metadata",
                Some("title") => "title",
                Some("subtitle") => "subtitle",
                Some("description") => "description",
                Some("short_description") => "short_description",
                Some("keywords") => "keywords",
                Some("marketing_text") => "marketing_text",
                Some("promotional_text") => "promotional_text",
                Some("video") => "video",
                Some("store_images") => "store_images",
                _ => "unknown",
            })
            .collect::<Vec<_>>()));
    }
    for container in [payload, payload.get("metadata").unwrap_or(&Value::Null)] {
        for field in [
            "name",
            "title",
            "subtitle",
            "description",
            "keywords",
            "promotionalText",
            "whatsNew",
            "shortDescription",
            "fullDescription",
            "releaseNotes",
            "marketingText",
        ] {
            if let Some(value) = container.get(field) {
                out["textFields"][field] = if let Some(s) = value.as_str() {
                    json!({"characters":s.chars().count(),"bytes":s.len(),"sha256":hex::encode(Sha256::digest(s.as_bytes()))})
                } else {
                    json!({"invalidType":true})
                };
            }
        }
    }
    if let Some(images) = payload
        .pointer("/metadata/storeImages")
        .and_then(Value::as_object)
    {
        for slot in [
            "phone_screenshots",
            "tablet_screenshots",
            "foldable_screenshots",
            "seven_inch_screenshots",
            "ten_inch_screenshots",
            "feature_graphic",
            "tv_screenshots",
            "wear_screenshots",
        ] {
            if let Some(value) = images.get(slot) {
                out["imageCounts"][slot] =
                    json!(value.get("urls").and_then(Value::as_array).map(|a| a.len()));
            }
        }
    }
    out
}

pub(crate) fn business_event(
    registry: &Registry,
    context: &RequestContext,
    platform: Platform,
    identifier: &str,
    operation: Operation,
    payload: &Value,
) {
    registry.logs.record(Event {
        name: "store.request",
        request_id: Some(context.0.clone()),
        platform: Some(platform),
        identifier: store_protocol::validate_identifier(identifier)
            .ok()
            .map(|_| identifier.to_owned()),
        details: json!({"operation":operation,"request":payload_summary(payload)}),
        ..Default::default()
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retention_boundary_and_pages_survive_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("registry.sqlite");
        let registry = Registry::open(&path, "test-signing-key-01234567890123456789").unwrap();
        let timestamp = now();
        registry
            .logs
            .insert_at(
                Event {
                    name: "old",
                    ..Default::default()
                },
                timestamp - RETENTION_MS,
            )
            .unwrap();
        registry
            .logs
            .insert_at(
                Event {
                    name: "new",
                    ..Default::default()
                },
                timestamp,
            )
            .unwrap();
        registry.logs.prune_at(timestamp).unwrap();
        let raw_count: i64 = registry
            .db
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM center_logs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(raw_count, 1);
        // Simulate an old record left by a previous process, then restart without traffic.
        registry
            .db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO center_logs(timestamp_ms,event,details) VALUES(?1,'expired','{}')",
                [timestamp - RETENTION_MS - 1],
            )
            .unwrap();
        drop(registry);
        let registry = Registry::open(&path, "test-signing-key-01234567890123456789").unwrap();
        let rows = registry.logs.query(&LogQuery::default()).unwrap();
        assert_eq!(rows["entries"].as_array().unwrap().len(), 1);
        assert_eq!(rows["entries"][0]["event"], "new");
        registry.logs.record(Event {
            name: "later",
            ..Default::default()
        });
        let first = registry
            .logs
            .query(&LogQuery {
                limit: 1,
                ..Default::default()
            })
            .unwrap();
        let second = registry
            .logs
            .query(&LogQuery {
                limit: 1,
                before: first["nextCursor"].as_i64(),
                ..Default::default()
            })
            .unwrap();
        assert_ne!(first["entries"][0]["id"], second["entries"][0]["id"]);
        assert!(second["nextCursor"].is_null());
    }

    #[test]
    fn metadata_never_copies_secret_fields_or_vendor_messages() {
        let payload = json!({"locale":"en-US","version":"1.2","title":"secret-copy","description":"商店文案",
            "token":"supersecret","credentials":{"privateKey":"secret-key"},"images":[{"url":"https://secret"}],"syncScopes":["text","secret"],"unknown":"hidden"});
        let summary = payload_summary(&payload);
        let serialized = summary.to_string();
        for secret in [
            "secret-copy",
            "商店文案",
            "supersecret",
            "secret-key",
            "https://secret",
            "hidden",
        ] {
            assert!(!serialized.contains(secret));
        }
        assert_eq!(summary["locale"], "en-US");
        assert_eq!(summary["textFields"]["description"]["characters"], 4);
        let error = error_summary("supersecret", "message-token");
        assert_eq!(
            serde_json::to_value(error).unwrap()["code"],
            "request_failed"
        );
        assert_eq!(
            error_summary("store_upstream_error", "Store upstream returned HTTP 400")
                .upstream_status,
            Some(400)
        );
    }

    #[test]
    fn cancellation_guard_writes_a_terminal_record() {
        let registry = Registry::open(":memory:", "test-signing-key-01234567890123456789").unwrap();
        let id = Uuid::new_v4().to_string();
        drop(HttpGuard {
            logs: registry.logs.clone(),
            event: Event {
                name: "http.started",
                request_id: Some(id.clone()),
                details: json!({}),
                ..Default::default()
            },
            start: Instant::now(),
            completed: false,
        });
        let result = registry
            .logs
            .query(&LogQuery {
                request_id: Some(id),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(result["entries"][0]["event"], "http.cancelled");
        assert_eq!(result["entries"][0]["status"], 499);
    }
}

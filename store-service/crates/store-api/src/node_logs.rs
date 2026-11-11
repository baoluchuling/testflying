//! Forward a fixed node-log snapshot; never save node files on the center.
use super::*;
use axum::body::Body;
use store_protocol::logs::{log_hour, LogAction, LogRequest, MAX_LOG_CHUNK};

async fn command(
    state: &ApiState,
    node: &Connector,
    action: LogAction,
) -> std::result::Result<Value, StoreError> {
    let current = state.registry.get(&node.connector_id)?;
    if !current.enabled || current.kind != ConnectorKind::Store {
        return Err(StoreError::new(
            409,
            "connector_disabled",
            "Store node is not enabled",
        ));
    }
    let reply = state
        .transport
        .dispatch_logs(
            &current,
            &state.registry.token(&current),
            LogRequest {
                request_id: Uuid::new_v4().to_string(),
                action,
            },
        )
        .await?;
    if reply.status != 200 {
        let code = match reply.body["error"]["code"].as_str() {
            Some("invalid_log_request") => "invalid_log_request",
            _ => "logs_unavailable",
        };
        return Err(StoreError::new(
            reply.status,
            code,
            "Node log file unavailable or changed",
        ));
    }
    Ok(reply.body)
}

struct Download {
    state: Arc<ApiState>,
    node: Connector,
    request_id: String,
    file: String,
    size: u64,
    offset: u64,
    completed: bool,
}
impl Download {
    fn record(&self, event: &'static str, status: u16) {
        self.state.registry.logs.record(logs::Event {
            name: event,
            request_id: Some(self.request_id.clone()),
            connector_id: Some(self.node.connector_id.clone()),
            status: Some(status),
            details: json!({"snapshotBytes":self.size,"transferredBytes":self.offset}),
            ..Default::default()
        });
    }
    fn failed(&mut self, status: u16) -> std::io::Error {
        self.completed = true;
        self.record("node.logs.download_failed", status);
        std::io::Error::other("Node log transfer failed")
    }
    async fn next(mut self) -> std::result::Result<Option<(Vec<u8>, Self)>, std::io::Error> {
        if self.offset == self.size {
            if !self.completed {
                self.completed = true;
                self.record("node.logs.download_completed", 200);
            }
            return Ok(None);
        }
        let length = (self.size - self.offset).min(MAX_LOG_CHUNK as u64) as u32;
        let body = match command(
            &self.state,
            &self.node,
            LogAction::Read {
                file: self.file.clone(),
                offset: self.offset,
                length,
                snapshot_size: self.size,
            },
        )
        .await
        {
            Ok(body) => body,
            Err(error) => return Err(self.failed(error.status)),
        };
        let bytes = store_protocol::upload::decode_chunk(body["data"].as_str().unwrap_or(""))
            .map_err(|_| self.failed(502))?;
        if body["file"].as_str() != Some(self.file.as_str())
            || body["offset"].as_u64() != Some(self.offset)
            || body["snapshotSize"].as_u64() != Some(self.size)
            || bytes.len() != length as usize
        {
            return Err(self.failed(502));
        }
        self.offset += bytes.len() as u64;
        if self.offset == self.size {
            self.completed = true;
            self.record("node.logs.download_completed", 200);
        }
        Ok(Some((bytes, self)))
    }
}
impl Drop for Download {
    fn drop(&mut self) {
        if !self.completed {
            self.record("node.logs.download_cancelled", 499);
        }
    }
}

pub(super) async fn latest(
    State(state): State<Arc<ApiState>>,
    Extension(context): Extension<logs::RequestContext>,
    Path(id): Path<String>,
) -> Result<Response> {
    let node = state.registry.get(&id)?;
    if node.kind != ConnectorKind::Store {
        return Err(StoreError::new(
            422,
            "invalid_connector_kind",
            "Node logs require a Store Connector",
        )
        .into());
    }
    let body = command(&state, &node, LogAction::Latest).await?;
    let file = body["file"]
        .as_str()
        .filter(|name| log_hour(name).is_some())
        .ok_or_else(|| StoreError::new(502, "invalid_connector_reply", "Invalid node log file"))?
        .to_owned();
    let size = body["sizeBytes"]
        .as_u64()
        .ok_or_else(|| StoreError::new(502, "invalid_connector_reply", "Invalid node log size"))?;
    let download = Download {
        state,
        node,
        request_id: context.0,
        file: file.clone(),
        size,
        offset: 0,
        completed: false,
    };
    download.record("node.logs.download_started", 200);
    let stream = futures_util::stream::try_unfold(download, Download::next);
    let mut response = Body::from_stream(stream).into_response();
    for (key, value) in [
        (
            header::CONTENT_TYPE,
            "application/x-ndjson; charset=utf-8".to_owned(),
        ),
        (
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{file}\""),
        ),
        (header::CONTENT_LENGTH, size.to_string()),
        (header::CACHE_CONTROL, "no-store".into()),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff".into()),
    ] {
        response
            .headers_mut()
            .insert(key, value.parse().expect("validated log download header"));
    }
    Ok(response)
}

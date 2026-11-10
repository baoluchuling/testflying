use super::*;
use axum::body::Body;
use futures_util::StreamExt;
use store_protocol::{
    crypto,
    upload::{self, UploadClaims, UploadInput},
    Platform,
};

pub(super) async fn prepare(
    State(state): State<Arc<ApiState>>,
    Extension(context): Extension<logs::RequestContext>,
    Path((platform, app)): Path<(Platform, String)>,
    Json(file): Json<UploadInput>,
) -> Result<(StatusCode, Json<Value>)> {
    file.validate()?;
    let node = state.registry.resolve(platform, &app)?;
    if !state.transport.connected(&node.connector_id) && node.base_url.is_none() {
        return Err(
            StoreError::new(503, "connector_disconnected", "Connector is not reachable").into(),
        );
    }
    let now = upload::now()?;
    let claims = UploadClaims {
        iss: "testflying-store-api".into(),
        aud: format!("connector:{}", node.connector_id),
        iat: now,
        exp: now + upload::TOKEN_SECONDS,
        asset_id: Uuid::new_v4().to_string(),
        connector_id: node.connector_id.clone(),
        platform,
        bundle_identifier: app.clone(),
        file,
    };
    let ticket = upload::sign(&claims, &state.registry.purpose_key(&node, "upload"))?;
    state.registry.logs.record(logs::Event {name:"upload.prepared",request_id:Some(context.0),connector_id:Some(node.connector_id.clone()),
        platform:Some(platform),identifier:Some(app.clone()),details:json!({"assetId":claims.asset_id,"sizeBytes":claims.file.size_bytes,"contentType":claims.file.content_type}),..Default::default()});
    let url = format!(
        "{}/v1/store-management/apps/{platform}/{app}/uploads/{}",
        state.public_url, claims.asset_id
    );
    Ok((
        StatusCode::CREATED,
        Json(
            json!({"assetId":claims.asset_id,"uploadUrl":url,"uploadToken":ticket,"expiresAt":claims.exp,"method":"PUT"}),
        ),
    ))
}

struct Relay {
    context: logs::RequestContext,
    state: Arc<ApiState>,
    node: Connector,
    token: String,
    claims: UploadClaims,
    websocket: bool,
    session: String,
    started: bool,
    finished: bool,
}
impl Relay {
    async fn chunk(
        &self,
        id: &str,
        offset: u64,
        bytes: &[u8],
    ) -> std::result::Result<(), StoreError> {
        self.command(
            Operation::UploadChunk,
            json!({"assetId":id,"offset":offset,"data":upload::encode_chunk(bytes)}),
        )
        .await
        .map(|_| ())
    }
    async fn command(
        &self,
        operation: Operation,
        mut payload: Value,
    ) -> std::result::Result<Value, StoreError> {
        payload["sessionId"] = json!(self.session);
        let request = StoreRequest {
            request_id: Uuid::new_v4().to_string(),
            platform: self.claims.platform,
            bundle_identifier: self.claims.bundle_identifier.clone(),
            operation,
            payload,
        };
        let command_id = request.request_id.clone();
        let mut event = logs::Event {
            name: "upload.command",
            request_id: Some(self.context.0.clone()),
            connector_id: Some(self.node.connector_id.clone()),
            platform: Some(self.claims.platform),
            identifier: Some(self.claims.bundle_identifier.clone()),
            details: json!({"assetId":self.claims.asset_id,"commandRequestId":command_id,"operation":operation}),
            ..Default::default()
        };
        self.state.registry.logs.record(event.clone());
        let result = self
            .state
            .transport
            .dispatch_via(&self.node, &self.token, request, self.websocket)
            .await;
        event.name = "upload.command_result";
        event.status = Some(match &result {
            Ok(reply) => reply.status,
            Err(error) => error.status,
        });
        if let Err(error) = &result {
            event.details["error"] = json!(logs::error_summary(&error.code, &error.message));
        }
        self.state.registry.logs.record(event);
        let reply = result?;
        if reply.status != 200 {
            let error = &reply.body["error"];
            return Err(StoreError::new(
                reply.status,
                error["code"].as_str().unwrap_or("upload_failed"),
                error["message"]
                    .as_str()
                    .unwrap_or("Connector rejected upload"),
            ));
        }
        Ok(reply.body)
    }
}
impl Drop for Relay {
    fn drop(&mut self) {
        if !self.started || self.finished {
            return;
        }
        self.state.registry.logs.record(logs::Event {
            name: "upload.abort_requested",
            request_id: Some(self.context.0.clone()),
            connector_id: Some(self.node.connector_id.clone()),
            details: json!({"assetId":self.claims.asset_id}),
            ..Default::default()
        });
        let (state, node, token, claims, websocket, session) = (
            self.state.clone(),
            self.node.clone(),
            self.token.clone(),
            self.claims.clone(),
            self.websocket,
            self.session.clone(),
        );
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let request = StoreRequest {
                    request_id: Uuid::new_v4().to_string(),
                    platform: claims.platform,
                    bundle_identifier: claims.bundle_identifier,
                    operation: Operation::UploadAbort,
                    payload: json!({"assetId":claims.asset_id,"sessionId":session}),
                };
                let _ = tokio::time::timeout(
                    Duration::from_secs(5),
                    state
                        .transport
                        .dispatch_via(&node, &token, request, websocket),
                )
                .await;
            });
        }
    }
}

pub(super) async fn receive(
    State(state): State<Arc<ApiState>>,
    Extension(context): Extension<logs::RequestContext>,
    Path((platform, app, id)): Path<(Platform, String, String)>,
    headers: HeaderMap,
    body: Body,
) -> Result<(StatusCode, Json<Value>)> {
    let node = state.registry.resolve(platform, &app)?;
    let ticket = bearer(&headers).to_owned();
    let claims = upload::verify(
        &ticket,
        &state.registry.purpose_key(&node, "upload"),
        &node.connector_id,
    )?;
    if claims.platform != platform || claims.bundle_identifier != app || claims.asset_id != id {
        return Err(upload::invalid_token().into());
    }
    if let Some(length) = headers.get(header::CONTENT_LENGTH) {
        if length.to_str().ok().and_then(|s| s.parse::<u64>().ok()) != Some(claims.file.size_bytes)
        {
            return Err(StoreError::new(
                422,
                "upload_integrity",
                "Content-Length does not match signed size",
            )
            .into());
        }
    }
    let token = state.registry.token(&node);
    let websocket = state.transport.connected(&node.connector_id);
    let mut relay = Relay {
        context,
        state,
        node,
        token,
        claims,
        websocket,
        session: Uuid::new_v4().to_string(),
        started: false,
        finished: false,
    };
    let transfer = async {
        // The route is selected before the first upload command, and never retried after dispatch.
        relay.started = true;
        relay
            .command(
                Operation::UploadStart,
                json!({"assetId":id,"ticket":ticket}),
            )
            .await?;
        let mut stream = body.into_data_stream();
        let mut offset = 0u64;
        let mut pending = Vec::with_capacity(crypto::CHUNK_BYTES);
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| {
                StoreError::new(400, "upload_interrupted", "Upload request was interrupted")
            })?;
            if offset + pending.len() as u64 + chunk.len() as u64 > relay.claims.file.size_bytes {
                return Err(StoreError::new(
                    413,
                    "upload_too_large",
                    "Upload exceeds signed file size",
                ));
            }
            let mut unread = chunk.as_ref();
            while !unread.is_empty() {
                let take = unread.len().min(crypto::CHUNK_BYTES - pending.len());
                pending.extend_from_slice(&unread[..take]);
                unread = &unread[take..];
                if pending.len() == crypto::CHUNK_BYTES {
                    relay.chunk(&id, offset, &pending).await?;
                    offset += pending.len() as u64;
                    pending.clear();
                }
            }
        }
        if offset + pending.len() as u64 != relay.claims.file.size_bytes {
            return Err(StoreError::new(
                422,
                "upload_integrity",
                "Upload body is incomplete",
            ));
        }
        if !pending.is_empty() {
            relay.chunk(&id, offset, &pending).await?;
        }
        let result = relay
            .command(Operation::UploadFinish, json!({"assetId":id}))
            .await?;
        relay.finished = true;
        Ok(result)
    };
    let result = tokio::time::timeout(Duration::from_secs(upload::TRANSFER_SECONDS), transfer)
        .await
        .map_err(|_| StoreError::new(408, "upload_timeout", "Upload timed out"))??;
    Ok((StatusCode::CREATED, Json(result)))
}

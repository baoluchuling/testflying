use super::*;
use axum::{extract::Request, routing::delete};
use registry::IntegrationInput;

pub(super) fn require_https(request: &Request) -> Result<()> {
    // The production TLS and plain HTTP proxy blocks overwrite this header.
    if request
        .headers()
        .get("x-forwarded-proto")
        .is_some_and(|v| v == "https")
    {
        return Ok(());
    }
    Err(StoreError::new(403, "https_required", "Use the HTTPS Store API address").into())
}

pub(super) fn no_store(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        "no-store".parse().expect("static header"),
    );
    response
}

pub(super) async fn authorize_binding(
    State(state): State<Arc<ApiState>>,
    mut request: Request,
    next: Next,
) -> Result<Response> {
    let token = bearer(request.headers());
    if !registry::secret_eq(&state.admin_token, token) {
        if !token.starts_with(registry::TOKEN_PREFIX) {
            return Err(StoreError::new(
                401,
                "unauthorized",
                "Administrator or integration token required",
            )
            .into());
        }
        require_https(&request)?;
        let access = state.registry.authenticate_integration(token)?;
        request.extensions_mut().insert(access);
    }
    Ok(no_store(next.run(request).await))
}

pub(super) async fn list_nodes(
    State(state): State<Arc<ApiState>>,
    integration: Option<Extension<Integration>>,
) -> Result<Json<Value>> {
    let nodes: Vec<_> = state
        .registry
        .list()?
        .into_iter()
        .filter(|node| {
            node.enabled
                && integration
                    .as_ref()
                    .is_none_or(|access| access.allows(&node.connector_id))
        })
        .map(|node| {
            json!({"connectorId":node.connector_id,"name":node.name,
            "connected":state.transport.connected(&node.connector_id),"enabled":node.enabled})
        })
        .collect();
    Ok(Json(json!({"connectors":nodes})))
}

pub(super) fn admin_routes(state: Arc<ApiState>) -> Router<Arc<ApiState>> {
    Router::new()
        .route("/v1/integrations", get(list).post(issue))
        .route("/v1/integrations/{id}", delete(revoke))
        .route("/v1/integrations/{id}/token", get(reveal))
        .route_layer(middleware::from_fn_with_state(state, authorize_admin))
        .route_layer(middleware::from_fn(
            |request: Request, next: Next| async move {
                require_https(&request)?;
                Ok::<_, ApiError>(no_store(next.run(request).await))
            },
        ))
}

async fn list(State(state): State<Arc<ApiState>>) -> Result<Json<Value>> {
    Ok(Json(json!({"integrations":state.registry.integrations()?})))
}

async fn issue(
    State(state): State<Arc<ApiState>>,
    Json(input): Json<IntegrationInput>,
) -> Result<(StatusCode, Json<Value>)> {
    let integration = state.registry.issue_integration(input)?;
    let token = state.registry.integration_token(&integration.id)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"integration":integration,"token":token})),
    ))
}

async fn reveal(State(state): State<Arc<ApiState>>, Path(id): Path<String>) -> Result<Json<Value>> {
    Ok(Json(
        json!({"token":state.registry.integration_token(&id)?}),
    ))
}

async fn revoke(State(state): State<Arc<ApiState>>, Path(id): Path<String>) -> Result<StatusCode> {
    state.registry.revoke_integration(&id)?;
    Ok(StatusCode::NO_CONTENT)
}

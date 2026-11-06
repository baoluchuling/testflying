use super::*;
use axum::{extract::Request, routing::delete};
use registry::{Integration, IntegrationInput};

pub(super) fn require_https(request: &Request) -> Result<()> {
    require_https_headers(request.headers())
}

pub(super) fn require_https_headers(headers: &HeaderMap) -> Result<()> {
    // The production TLS and plain HTTP proxy blocks overwrite this header.
    if headers
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

pub(super) async fn authorize_https(request: Request, next: Next) -> Result<Response> {
    require_https(&request)?;
    Ok(next.run(request).await)
}

pub(super) async fn authorize_binding(
    State(state): State<Arc<ApiState>>,
    request: Request,
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
        state.registry.authenticate_integration(token)?;
    }
    Ok(no_store(next.run(request).await))
}

pub(super) async fn list_nodes(State(state): State<Arc<ApiState>>) -> Result<Json<Value>> {
    let nodes: Vec<_> = state
        .registry
        .list()?
        .into_iter()
        .filter(|node| node.enabled && node.kind == ConnectorKind::Store)
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
        .route_layer(middleware::from_fn(authorize_https))
        .route_layer(middleware::map_response(no_store_response))
}

async fn no_store_response(response: Response) -> Response {
    no_store(response)
}

fn integration_view(integration: Integration) -> Value {
    json!({"id":integration.id,"name":integration.name,"allConnectors":true})
}

async fn list(State(state): State<Arc<ApiState>>) -> Result<Json<Value>> {
    let integrations: Vec<_> = state
        .registry
        .integrations()?
        .into_iter()
        .map(integration_view)
        .collect();
    Ok(Json(json!({"integrations":integrations})))
}

async fn issue(
    State(state): State<Arc<ApiState>>,
    Json(input): Json<IntegrationInput>,
) -> Result<(StatusCode, Json<Value>)> {
    let integration = state.registry.issue_integration(input)?;
    let token = state.registry.integration_token(&integration.id)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"integration":integration_view(integration),"token":token})),
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

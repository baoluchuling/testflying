use super::*;
use registry::{Account, AccountInput, AccountPatch};

pub(super) fn management_routes(path: &str) -> Router<Arc<ApiState>> {
    Router::new()
        .route(path, get(list).post(create))
        .route(&format!("{path}/{{id}}"), axum::routing::patch(update))
        .route(&format!("{path}/{{id}}/apps"), get(apps))
        .route(
            &format!("{path}/{{id}}/apps/{{platform}}/{{identifier}}/forwarding"),
            axum::routing::put(forwarding),
        )
        .route(&format!("{path}/{{id}}/refresh"), post(refresh))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ForwardingInput {
    enabled: bool,
}

async fn forwarding(
    State(state): State<Arc<ApiState>>,
    Path((id, platform, identifier)): Path<(String, Platform, String)>,
    Json(input): Json<ForwardingInput>,
) -> Result<StatusCode> {
    state
        .registry
        .set_app_enabled(&id, platform, &identifier, input.enabled)?;
    Ok(StatusCode::NO_CONTENT)
}

fn public_view(account: Account) -> Value {
    json!({"id":account.id,"name":account.name,"platform":account.platform,"enabled":account.enabled,
        "catalogStatus":account.catalog_status,"catalogError":account.catalog_error,"lastCatalogAt":account.last_catalog_at})
}

async fn list(State(state): State<Arc<ApiState>>) -> Result<Json<Value>> {
    Ok(Json(
        json!({"accounts":state.registry.accounts()?,"migrationIssues":state.registry.account_migration_issues()?}),
    ))
}

pub(super) async fn public_list(State(state): State<Arc<ApiState>>) -> Result<Json<Value>> {
    Ok(Json(
        json!({"accounts":state.registry.accounts()?.into_iter().map(public_view).collect::<Vec<_>>()}),
    ))
}

async fn create(
    State(state): State<Arc<ApiState>>,
    Json(input): Json<AccountInput>,
) -> Result<(StatusCode, Json<Value>)> {
    Ok((
        StatusCode::CREATED,
        Json(json!({"account":state.registry.create_account(input)?})),
    ))
}

async fn update(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<String>,
    Json(input): Json<AccountPatch>,
) -> Result<Json<Value>> {
    Ok(Json(
        json!({"account":state.registry.patch_account(&id,input)?}),
    ))
}

pub(super) async fn apps(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    Ok(Json(json!({"apps":state.registry.account_apps(&id)?})))
}

async fn refresh(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<Value>)> {
    let account = state.registry.account(&id)?;
    if !account.enabled {
        return Err(StoreError::new(409, "account_disabled", "Account is disabled").into());
    }
    if account.catalog_status == "identity_changed" {
        return Err(StoreError::new(
            409,
            "identity_changed",
            "Accept the changed identity before refreshing",
        )
        .into());
    }
    let node = state.registry.get(&account.store_connector_id)?;
    if !node.enabled || node.kind != ConnectorKind::Store {
        return Err(StoreError::new(
            409,
            "connector_disabled",
            "Account Store Connector is unavailable",
        )
        .into());
    }
    let request_id = state.transport.refresh_catalog(&node.connector_id)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({"requestId":request_id,"status":"accepted"})),
    ))
}

pub(super) async fn app_account(
    State(state): State<Arc<ApiState>>,
    Path((platform, identifier)): Path<(Platform, String)>,
) -> Result<Json<Value>> {
    Ok(Json(
        json!({"account":public_view(state.registry.resolve_account(platform,&identifier)?)}),
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct BindingInput {
    account_id: String,
}

pub(super) async fn bind(
    State(state): State<Arc<ApiState>>,
    Path((platform, identifier)): Path<(Platform, String)>,
    Json(input): Json<BindingInput>,
) -> Result<Json<Value>> {
    Ok(Json(
        json!({"account":public_view(state.registry.bind_account(platform,&identifier,&input.account_id)?)}),
    ))
}

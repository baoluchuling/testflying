//! Administrator-only read-only discovery. Results never enter the account registry.
use super::*;
use store_protocol::firebase::{FirebaseAction, FirebaseRequest};

async fn query(state: Arc<ApiState>, id: String, action: FirebaseAction) -> Result<Json<Value>> {
    action.validate()?;
    let node = state.registry.get(&id)?;
    let reply = state
        .transport
        .dispatch_firebase(
            &node,
            &state.registry.token(&node),
            FirebaseRequest {
                request_id: Uuid::new_v4().to_string(),
                firebase_action: action,
            },
        )
        .await?;
    if reply.status != 200 {
        return Err(store_protocol::firebase::public_error(
            reply.body["error"]["code"].as_str().unwrap_or(""),
        )
        .into());
    }
    Ok(Json(reply.body))
}
pub(super) async fn projects(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    query(state, id, FirebaseAction::Projects).await
}
pub(super) async fn android_apps(
    State(state): State<Arc<ApiState>>,
    Path((id, project_id)): Path<(String, String)>,
) -> Result<Json<Value>> {
    query(state, id, FirebaseAction::AndroidApps { project_id }).await
}

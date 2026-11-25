use super::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct DiscardInput {
    target_index: usize,
    object_key: String,
}

pub(super) async fn discard(
    State(state): State<Arc<ApiState>>,
    Path((id, job)): Path<(String, String)>,
    headers: HeaderMap,
    Json(input): Json<DiscardInput>,
) -> Result<Json<Value>> {
    authorize_worker(&state, &id, &headers)?;
    let lease = lease(&headers)?;
    state
        .registry
        .screenshot_target_index(&job, Some(input.target_index))?;
    if input.object_key.is_empty() || input.object_key.len() > 1024 {
        return Err(StoreError::new(
            422,
            "invalid_screenshot_object",
            "Provide the current objectKey",
        )
        .into());
    }
    Ok(Json(state.registry.discard_screenshot_target(
        &id,
        &job,
        lease,
        input.target_index,
        &input.object_key,
    )?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FinishInput {}

pub(super) async fn finish(
    State(state): State<Arc<ApiState>>,
    Path((id, job)): Path<(String, String)>,
    headers: HeaderMap,
    Json(_): Json<FinishInput>,
) -> Result<Json<Value>> {
    authorize_worker(&state, &id, &headers)?;
    Ok(Json(state.registry.finish_reviewed_screenshot(
        &id,
        &job,
        lease(&headers)?,
    )?))
}

//! Persisted center-wide controls; node connections and diagnostics remain available.
use super::*;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Settings {
    pub ios: bool,
    pub android: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Update {
    enabled: bool,
}

pub(super) async fn read(State(state): State<Arc<ApiState>>) -> Result<Json<Settings>> {
    Ok(Json(state.registry.forwarding()?))
}
pub(super) async fn update(
    State(state): State<Arc<ApiState>>,
    Extension(context): Extension<logs::RequestContext>,
    Path(platform): Path<Platform>,
    Json(input): Json<Update>,
) -> Result<Json<Settings>> {
    state.registry.set_forwarding(platform, input.enabled)?;
    state.registry.logs.record(logs::Event {
        name: "platform.forwarding_changed",
        request_id: Some(context.0),
        platform: Some(platform),
        status: Some(200),
        details: json!({"enabled":input.enabled}),
        ..Default::default()
    });
    Ok(Json(state.registry.forwarding()?))
}

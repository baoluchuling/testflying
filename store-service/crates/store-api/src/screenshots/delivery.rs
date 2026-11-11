use super::*;
use crate::{registry::screenshots::oss_fingerprint, screenshot_settings};
use std::collections::BTreeMap;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PrepareInput {
    size_bytes: u64,
    sha256: String,
    md5: String,
}

fn digest(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn ticket_lifetime(deadline: i64, issued_at: u64) -> std::result::Result<u64, StoreError> {
    let deadline = u64::try_from(deadline).map_err(|_| {
        StoreError::new(409, "invalid_job_lease", "Screenshot job lease has expired")
    })?;
    let ttl = deadline.saturating_sub(issued_at).min(300);
    if ttl == 0 {
        return Err(StoreError::new(
            409,
            "invalid_job_lease",
            "Screenshot job lease has expired",
        ));
    }
    Ok(ttl)
}

pub(super) async fn prepare(
    State(state): State<Arc<ApiState>>,
    Path((id, job)): Path<(String, String)>,
    headers: HeaderMap,
    Json(input): Json<PrepareInput>,
) -> Result<Json<Value>> {
    authorize_worker(&state, &id, &headers)?;
    let lease = lease(&headers)?;
    state.registry.screenshot_upload_lease(&id, &job, lease)?;
    if !(33..=MAX_IMAGE as u64).contains(&input.size_bytes)
        || !digest(&input.sha256, 64)
        || !digest(&input.md5, 32)
    {
        return Err(StoreError::new(
            422,
            "invalid_screenshot_metadata",
            "Provide PNG size and lowercase SHA256/MD5 digests within the 16 MiB limit",
        )
        .into());
    }
    let settings = state.registry.screenshot_settings()?;
    let storage = match oss::OssStorage::new(&settings.oss) {
        Ok(storage) => storage,
        Err(error) => {
            state.registry.finish_screenshot(
                &id,
                &job,
                lease,
                Err("screenshot_oss_not_configured"),
                "running",
            )?;
            return Err(error.into());
        }
    };
    let job_id = Uuid::parse_str(&job).map_err(|_| {
        StoreError::new(
            409,
            "invalid_job_lease",
            "Screenshot job is no longer assigned to this worker",
        )
    })?;
    let key = format!(
        "{}{job_id}/{}.png",
        if settings.oss.prefix.is_empty() {
            String::new()
        } else {
            format!("{}/", settings.oss.prefix)
        },
        Uuid::new_v4()
    );
    let fingerprint = oss_fingerprint(&settings.oss)?;
    let ticket = state.registry.prepare_screenshot_ticket(
        &id,
        &job,
        lease,
        input.size_bytes,
        &input.sha256,
        &input.md5,
        &fingerprint,
        &key,
    )?;
    let md5 = hex::decode(&ticket.md5)
        .map_err(|_| StoreError::new(500, "registry_error", "Invalid screenshot upload record"))?;
    let signed_headers = BTreeMap::from([
        ("content-type".to_owned(), "image/png".to_owned()),
        ("content-length".to_owned(), ticket.size_bytes.to_string()),
        (
            "content-md5".to_owned(),
            store_protocol::upload::encode_chunk(&md5),
        ),
        ("x-oss-meta-sha256".to_owned(), ticket.sha256),
        ("x-oss-meta-md5".to_owned(), ticket.md5),
        ("x-oss-forbid-overwrite".to_owned(), "true".to_owned()),
    ]);
    let issued_time = chrono::Utc::now();
    let issued_at = u64::try_from(issued_time.timestamp())
        .map_err(|_| StoreError::new(500, "invalid_clock", "System clock is invalid"))?;
    let ttl = ticket_lifetime(ticket.deadline, issued_at)?;
    let url = storage.presign_at("PUT", &ticket.object_key, &signed_headers, ttl, issued_time)?;
    Ok(Json(
        json!({"uploadUrl":url,"headers":signed_headers,"objectKey":ticket.object_key,"expiresAt":issued_at+ttl}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_ticket_cannot_outlive_its_job_lease() {
        assert_eq!(ticket_lifetime(1300, 1000).unwrap(), 300);
        assert_eq!(ticket_lifetime(2000, 1000).unwrap(), 300);
        assert_eq!(ticket_lifetime(1025, 1000).unwrap(), 25);
        assert_eq!(
            ticket_lifetime(1000, 1000).unwrap_err().code,
            "invalid_job_lease"
        );
        assert!(ticket_lifetime(999, 1000).is_err());
        assert!(ticket_lifetime(-1, 1000).is_err());
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CompleteInput {}

pub(super) async fn complete(
    State(state): State<Arc<ApiState>>,
    Path((id, job)): Path<(String, String)>,
    headers: HeaderMap,
    _input: Option<Json<CompleteInput>>,
) -> Result<Json<Value>> {
    authorize_worker(&state, &id, &headers)?;
    let lease = lease(&headers)?;
    let ticket = state.registry.screenshot_ticket(&id, &job, lease)?;
    if let Some(result) = ticket.result {
        return Ok(Json(json!({"delivered":true,"result":result})));
    }
    let settings: screenshot_settings::Settings = state.registry.screenshot_settings()?;
    let fingerprint = oss_fingerprint(&settings.oss)?;
    if fingerprint != ticket.config_fingerprint {
        return Err(StoreError::new(
            409,
            "screenshot_oss_config_changed",
            "Screenshot OSS settings changed after this upload ticket was issued",
        )
        .into());
    }
    let storage = oss::OssStorage::new(&settings.oss)?;
    storage
        .verify(
            &ticket.object_key,
            ticket.size_bytes,
            &ticket.sha256,
            &ticket.md5,
        )
        .await?;
    let mut result = storage.image_result(&ticket.object_key)?;
    result["objectKey"] = json!(ticket.object_key);
    result["sizeBytes"] = json!(ticket.size_bytes);
    result["sha256"] = json!(ticket.sha256);
    let result =
        state
            .registry
            .complete_screenshot_ticket(&id, &job, lease, &fingerprint, &result)?;
    Ok(Json(json!({"delivered":true,"result":result})))
}

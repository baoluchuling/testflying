//! Bounded, credential-local, read-only Firebase discovery.
use super::*;
use std::collections::HashSet;
use store_protocol::firebase::{valid_project_id, validate_response, FirebaseAction};

const SCOPE: &str = "https://www.googleapis.com/auth/firebase.readonly";
const MAX_PAGE_BYTES: usize = 2 * 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 16 * 1024 * 1024;
const MAX_ITEMS: usize = 10_000;
const MAX_PAGES: usize = 100;
const MAX_TOKEN_BYTES: usize = 4096;

fn failure(status: u16, code: &'static str) -> StoreError {
    StoreError::new(status, code, "Firebase query failed")
}
fn invalid_response() -> StoreError {
    failure(502, "firebase_invalid_response")
}
fn upstream_error() -> StoreError {
    failure(502, "firebase_upstream_error")
}
fn auth_error() -> StoreError {
    failure(502, "firebase_auth_failed")
}

impl Gateway {
    /// Discover Firebase projects or Android apps without reading or changing store bindings.
    pub async fn firebase_query(&self, action: &FirebaseAction) -> Result<Value> {
        action.validate()?;
        if self.inner.config.mode != "live" {
            return Err(failure(501, "firebase_unsupported"));
        }
        if self.inner.google.is_none() {
            return Err(failure(503, "firebase_not_configured"));
        }
        tokio::time::timeout(Duration::from_secs(60), self.firebase_query_inner(action))
            .await
            .map_err(|_| upstream_error())?
    }

    async fn firebase_query_inner(&self, action: &FirebaseAction) -> Result<Value> {
        let token = self.firebase_token().await?;
        let (path, field) = match action {
            FirebaseAction::Projects => ("/v1beta1/projects".to_owned(), "results"),
            FirebaseAction::AndroidApps { project_id } => (
                format!("/v1beta1/projects/{project_id}/androidApps"),
                "apps",
            ),
        };
        let mut url = Url::parse(&self.inner.config.google_firebase_base_url)
            .map_err(|_| upstream_error())?;
        url.set_path(&path);
        let mut next: Option<String> = None;
        let mut tokens = HashSet::new();
        let mut identifiers = HashSet::new();
        let mut entries = Vec::new();
        let mut total = 0;
        let mut item_count = 0usize;
        for page_index in 0..MAX_PAGES {
            url.set_query(None);
            {
                let mut query = url.query_pairs_mut();
                query.append_pair("pageSize", "100");
                if let Some(next) = &next {
                    query.append_pair("pageToken", next);
                }
            }
            // The URL always retains the configured origin and fixed resource path.
            // No vendor response is ever treated as a URL, and redirects are disabled.
            let response = self
                .inner
                .client
                .get(url.clone())
                .bearer_auth(&token)
                .send()
                .await
                .map_err(|_| upstream_error())?;
            let status = response.status();
            let bytes = read_bytes(response, MAX_PAGE_BYTES, &mut total).await?;
            if !status.is_success() {
                return Err(api_error(status.as_u16(), &bytes));
            }
            let value: Value = serde_json::from_slice(&bytes).map_err(|_| invalid_response())?;
            let object = value.as_object().ok_or_else(invalid_response)?;
            if object.contains_key("error") {
                return Err(invalid_response());
            }
            let rows = match object.get(field) {
                None => &[][..],
                Some(Value::Array(rows)) => rows.as_slice(),
                _ => return Err(invalid_response()),
            };
            item_count = item_count
                .checked_add(rows.len())
                .ok_or_else(invalid_response)?;
            if item_count > MAX_ITEMS {
                return Err(invalid_response());
            }
            for row in rows {
                if !row.is_object() {
                    return Err(invalid_response());
                }
                if row.get("state").and_then(Value::as_str) == Some("DELETED") {
                    continue;
                }
                let project_id = required_string(row, "projectId", 30)?;
                if !valid_project_id(project_id) {
                    return Err(invalid_response());
                }
                let name = match row.get("displayName") {
                    None | Some(Value::Null) => Value::Null,
                    Some(Value::String(name)) if name.len() <= 512 => Value::String(name.clone()),
                    _ => return Err(invalid_response()),
                };
                let normalized = match action {
                    FirebaseAction::Projects => {
                        let number = required_string(row, "projectNumber", 30)?;
                        if !number.bytes().all(|byte| byte.is_ascii_digit())
                            || !identifiers.insert(project_id.to_owned())
                        {
                            return Err(invalid_response());
                        }
                        json!({"projectId":project_id,"projectNumber":number,"displayName":name})
                    }
                    FirebaseAction::AndroidApps {
                        project_id: requested,
                    } => {
                        if requested != project_id {
                            return Err(invalid_response());
                        }
                        let app_id = required_string(row, "appId", 256)?;
                        let package = required_string(row, "packageName", 255)?;
                        if !identifiers.insert(app_id.to_owned()) {
                            return Err(invalid_response());
                        }
                        json!({"projectId":requested,"appId":app_id,"packageName":package,"displayName":name})
                    }
                };
                entries.push(normalized);
            }
            next = match object.get("nextPageToken") {
                None => None,
                Some(Value::String(token)) if token.is_empty() => None,
                Some(Value::String(token))
                    if token.len() <= MAX_TOKEN_BYTES
                        && !token.chars().any(char::is_control)
                        && tokens.insert(token.clone()) =>
                {
                    Some(token.clone())
                }
                _ => return Err(invalid_response()),
            };
            if next.is_none() {
                let result = match action {
                    FirebaseAction::Projects => json!({"projects":entries}),
                    FirebaseAction::AndroidApps { project_id } => {
                        json!({"projectId":project_id,"apps":entries})
                    }
                };
                validate_response(action, &result).map_err(|_| invalid_response())?;
                return Ok(result);
            }
            if page_index + 1 == MAX_PAGES {
                return Err(invalid_response());
            }
        }
        Err(invalid_response())
    }

    async fn firebase_token(&self) -> Result<String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| auth_error())?
            .as_secs();
        let mut cached = self.inner.google_firebase_token.lock().await;
        if let Some((token, expiry)) = &*cached {
            if now.saturating_add(60) < *expiry {
                return Ok(token.clone());
            }
        }
        let account = self
            .inner
            .google
            .as_ref()
            .ok_or_else(|| failure(503, "firebase_not_configured"))?;
        let assertion = encode(
            &Header::new(Algorithm::RS256),
            &json!({"iss":account.email,"scope":SCOPE,"aud":self.inner.config.google_token_url,
                "iat":now,"exp":now.saturating_add(3600)}),
            &account.key,
        )
        .map_err(|_| auth_error())?;
        let response = self
            .inner
            .client
            .post(&self.inner.config.google_token_url)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", assertion.as_str()),
            ])
            .send()
            .await
            .map_err(|_| auth_error())?;
        let status = response.status();
        let mut total = 0;
        let bytes = read_bytes(response, 64 * 1024, &mut total)
            .await
            .map_err(|_| auth_error())?;
        if !status.is_success() {
            return Err(auth_error());
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| auth_error())?;
        let token = value
            .get("access_token")
            .and_then(Value::as_str)
            .filter(|token| {
                !token.is_empty()
                    && token.len() <= 16 * 1024
                    && token.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
            })
            .ok_or_else(auth_error)?;
        if value
            .get("scope")
            .is_some_and(|scope| scope.as_str() != Some(SCOPE))
            || value
                .get("token_type")
                .is_some_and(|kind| kind.as_str() != Some("Bearer"))
        {
            return Err(auth_error());
        }
        let expiry = value
            .get("expires_in")
            .and_then(Value::as_u64)
            .filter(|expiry| *expiry > 0 && *expiry <= 86400)
            .ok_or_else(auth_error)?;
        *cached = Some((token.to_owned(), now.saturating_add(expiry)));
        Ok(token.to_owned())
    }
}

fn required_string<'a>(value: &'a Value, field: &str, max: usize) -> Result<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| {
            !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
        })
        .ok_or_else(invalid_response)
}

async fn read_bytes(mut response: Response, limit: usize, total: &mut usize) -> Result<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|size| size > limit as u64)
    {
        return Err(invalid_response());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| upstream_error())? {
        *total = total
            .checked_add(chunk.len())
            .ok_or_else(invalid_response)?;
        if bytes.len().saturating_add(chunk.len()) > limit || *total > MAX_TOTAL_BYTES {
            return Err(invalid_response());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn api_error(status: u16, bytes: &[u8]) -> StoreError {
    match status {
        401 => auth_error(),
        403 => {
            let value: Value = serde_json::from_slice(bytes).unwrap_or(Value::Null);
            let disabled = value["error"]["details"].as_array().is_some_and(|details| {
                details.iter().any(|detail| {
                    detail["@type"] == "type.googleapis.com/google.rpc.ErrorInfo"
                        && detail["reason"] == "SERVICE_DISABLED"
                })
            }) || value["error"]["errors"].as_array().is_some_and(|errors| {
                errors
                    .iter()
                    .any(|error| error["reason"] == "accessNotConfigured")
            });
            failure(
                403,
                if disabled {
                    "firebase_api_disabled"
                } else {
                    "firebase_permission_denied"
                },
            )
        }
        404 => failure(404, "firebase_not_found"),
        429 => failure(429, "firebase_rate_limited"),
        _ => upstream_error(),
    }
}

#[cfg(test)]
#[path = "firebase_tests.rs"]
mod tests;

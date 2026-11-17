//! Read-only enumeration. This path never creates Publisher edits or takes business capacity.
use crate::*;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use store_protocol::catalog::{AccountCatalog, CatalogApp, CatalogReport, CatalogStatus, MAX_APPS};

const MAX_PAGES: usize = 100;
const MAX_PAGE_BYTES: usize = 2 * 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 16 * 1024 * 1024;
pub const REPORTING_SCOPE: &str = "https://www.googleapis.com/auth/playdeveloperreporting";

fn failed() -> StoreError {
    StoreError::new(
        502,
        "catalog_discovery_failed",
        "Account catalog discovery failed",
    )
}
fn unconfigured() -> StoreError {
    StoreError::new(
        503,
        "catalog_not_configured",
        "Account catalog is not configured",
    )
}
fn fingerprint(platform: Platform, identity: &str) -> String {
    hex::encode(Sha256::digest(format!("{platform}:{identity}").as_bytes()))
}
fn trusted_next(base: &Url, current: &Url, next: &str) -> Result<Url> {
    let url = current.join(next).map_err(|_| failed())?;
    if url.origin() != base.origin()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.path() != base.path()
    {
        return Err(failed());
    }
    Ok(url)
}
async fn page(mut response: Response, total: &mut usize) -> Result<Value> {
    if !response.status().is_success() {
        return Err(failed());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| failed())? {
        if bytes.len().saturating_add(chunk.len()) > MAX_PAGE_BYTES
            || total.saturating_add(chunk.len()) > MAX_TOTAL_BYTES
        {
            return Err(failed());
        }
        *total += chunk.len();
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| failed())
}
fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value.get(key).and_then(Value::as_str).ok_or_else(failed)
}
impl Gateway {
    /// Opaque identity of the locally loaded credentials; never performs network I/O.
    pub fn account_fingerprint(&self, platform: Platform) -> Option<String> {
        if self.inner.config.mode != "live" {
            return None;
        }
        let identity = match platform {
            Platform::Ios => self.inner.config.apple_issuer_id.as_deref().filter(|s| {
                !s.is_empty()
                    && self.inner.apple_key.is_some()
                    && self
                        .inner
                        .config
                        .apple_key_id
                        .as_ref()
                        .is_some_and(|s| !s.is_empty())
            })?,
            Platform::Android => &self.inner.google.as_ref()?.email,
        };
        Some(fingerprint(platform, identity))
    }

    pub async fn account_catalog(&self, platform: Platform) -> Result<AccountCatalog> {
        // A whole-discovery deadline also bounds pathological slow pagination.
        tokio::time::timeout(Duration::from_secs(120), self.discover_catalog(platform))
            .await
            .map_err(|_| failed())?
    }
    async fn discover_catalog(&self, platform: Platform) -> Result<AccountCatalog> {
        if self.inner.config.mode != "live" {
            return Err(StoreError::new(
                501,
                "catalog_unsupported",
                "Account catalog is unsupported",
            ));
        }
        let fingerprint = self
            .account_fingerprint(platform)
            .ok_or_else(unconfigured)?;
        let token = match platform {
            Platform::Ios => self.token(platform).await.map_err(|_| failed())?,
            Platform::Android => self.reporting_token().await?,
        };
        let endpoint = match platform {
            Platform::Ios => format!(
                "{}/v1/apps",
                self.inner.config.apple_base_url.trim_end_matches('/')
            ),
            Platform::Android => format!(
                "{}/v1beta1/apps:search",
                self.inner
                    .config
                    .google_reporting_base_url
                    .trim_end_matches('/')
            ),
        };
        let base = Url::parse(&endpoint).map_err(|_| failed())?;
        let mut current = base.clone();
        match platform {
            Platform::Ios => {
                current
                    .query_pairs_mut()
                    .append_pair("limit", "200")
                    .append_pair("fields[apps]", "name,bundleId");
            }
            Platform::Android => {
                current.query_pairs_mut().append_pair("pageSize", "100");
            }
        }
        let mut seen = HashSet::new();
        let mut seen_pages = HashSet::new();
        let mut apps = Vec::new();
        let mut total = 0;
        for _ in 0..MAX_PAGES {
            if !seen.insert(current.to_string()) {
                return Err(failed());
            }
            // Deliberately avoid upstream diagnostic body logging; failures expose only fixed codes.
            let response = self
                .inner
                .client
                .get(current.clone())
                .bearer_auth(&token)
                .send()
                .await
                .map_err(|_| failed())?;
            let value = page(response, &mut total).await?;
            let entries: &[Value] = match platform {
                Platform::Ios => value
                    .get("data")
                    .and_then(Value::as_array)
                    .ok_or_else(failed)?,
                Platform::Android => match value.get("apps") {
                    None => &[][..],
                    Some(v) => v.as_array().ok_or_else(failed)?.as_slice(),
                },
            };
            let page_hash =
                Sha256::digest(serde_json::to_vec(entries).map_err(|_| failed())?).to_vec();
            if !seen_pages.insert(page_hash) {
                return Err(failed());
            }
            for app in entries {
                let (identifier, name, id) = match platform {
                    Platform::Ios => (
                        string(&app["attributes"], "bundleId")?,
                        string(&app["attributes"], "name")?,
                        string(app, "id")?,
                    ),
                    Platform::Android => (
                        string(app, "packageName")?,
                        string(app, "displayName")?,
                        string(app, "name")?,
                    ),
                };
                apps.push(CatalogApp {
                    bundle_identifier: identifier.into(),
                    name: name.into(),
                    store_app_id: Some(id.into()),
                });
                if apps.len() > MAX_APPS {
                    return Err(failed());
                }
            }
            store_protocol::catalog::validate_apps(&apps).map_err(|_| failed())?;
            let next = match platform {
                Platform::Ios => match value.get("links").and_then(|l| l.get("next")) {
                    None | Some(Value::Null) => None,
                    Some(v) => Some(v.as_str().ok_or_else(failed)?),
                },
                Platform::Android => match value.get("nextPageToken") {
                    None | Some(Value::Null) => None,
                    Some(v) => Some(v.as_str().ok_or_else(failed)?),
                },
            }
            .filter(|s| !s.is_empty());
            let Some(next) = next else {
                apps.sort_by(|a, b| a.bundle_identifier.cmp(&b.bundle_identifier));
                let catalog = AccountCatalog { fingerprint, apps };
                catalog.validate().map_err(|_| failed())?;
                // Leave enough room for the encrypted StoreReply envelope.
                CatalogReport {
                    schema_version: 1,
                    report_id: "00000000-0000-4000-8000-000000000000".into(),
                    platform,
                    status: CatalogStatus::Ready,
                    account_fingerprint: Some(catalog.fingerprint.clone()),
                    apps: catalog.apps.clone(),
                    error_code: None,
                }
                .validate()
                .map_err(|_| failed())?;
                return Ok(catalog);
            };
            if next.len() > 8192 {
                return Err(failed());
            }
            current = match platform {
                Platform::Ios => trusted_next(&base, &current, next)?,
                Platform::Android => {
                    let mut url = base.clone();
                    url.query_pairs_mut()
                        .append_pair("pageSize", "100")
                        .append_pair("pageToken", next);
                    url
                }
            };
        }
        Err(failed())
    }
    async fn reporting_token(&self) -> Result<String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| failed())?
            .as_secs();
        let mut cache = self.inner.google_reporting_token.lock().await;
        if let Some((token, expiry)) = &*cache {
            if now + 60 < *expiry {
                return Ok(token.clone());
            }
        }
        let account = self.inner.google.as_ref().ok_or_else(unconfigured)?;
        let assertion = encode(
            &Header::new(Algorithm::RS256),
            &json!({"iss":account.email,"scope":REPORTING_SCOPE,
            "aud":self.inner.config.google_token_url,"iat":now,"exp":now+3600}),
            &account.key,
        )
        .map_err(|_| failed())?;
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
            .map_err(|_| failed())?;
        let mut total = 0;
        let value = page(response, &mut total).await?;
        let token = string(&value, "access_token")?.to_owned();
        if token.is_empty() {
            return Err(failed());
        }
        *cache = Some((
            token.clone(),
            now.saturating_add(
                value["expires_in"]
                    .as_u64()
                    .filter(|n| *n > 0)
                    .unwrap_or(3600)
                    .min(86400),
            ),
        ));
        Ok(token)
    }
}

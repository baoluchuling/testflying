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
async fn page(
    mut response: Response,
    total: &mut usize,
    fail: impl Fn(&'static str) -> StoreError,
) -> Result<Value> {
    if !response.status().is_success() {
        return Err(fail("http_error"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| fail("body_read_failed"))?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_PAGE_BYTES
            || total.saturating_add(chunk.len()) > MAX_TOTAL_BYTES
        {
            return Err(fail("body_size_limit"));
        }
        *total += chunk.len();
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| fail("invalid_json"))
}
fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value.get(key).and_then(Value::as_str).ok_or_else(failed)
}
impl Gateway {
    fn catalog_log(&self, event: &'static str, platform: Platform, mut details: Value) {
        details["platform"] = json!(platform);
        if let Some(log) = &self.inner.log {
            log.context(event, details);
        }
    }
    fn catalog_failed(
        &self,
        platform: Platform,
        stage: &'static str,
        reason: &'static str,
    ) -> StoreError {
        self.catalog_log(
            "catalog.failed",
            platform,
            json!({"stage":stage,"reason":reason}),
        );
        failed()
    }
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
        self.catalog_log("catalog.started", platform, json!({}));
        let started = std::time::Instant::now();
        // A whole-discovery deadline also bounds pathological slow pagination.
        let result =
            tokio::time::timeout(Duration::from_secs(120), self.discover_catalog(platform))
                .await
                .unwrap_or_else(|_| Err(self.catalog_failed(platform, "discovery", "timeout")));
        self.catalog_log("catalog.finished",platform,json!({"status":if result.is_ok(){"ready"}else{"failed"},
            "errorCode":result.as_ref().err().map(|e|e.code.as_str()),"appCount":result.as_ref().ok().map(|c|c.apps.len()),
            "durationMs":started.elapsed().as_millis() as u64}));
        result
    }
    async fn discover_catalog(&self, platform: Platform) -> Result<AccountCatalog> {
        if self.inner.config.mode != "live" {
            self.catalog_failed(platform, "configuration", "unsupported_mode");
            return Err(StoreError::new(
                501,
                "catalog_unsupported",
                "Account catalog is unsupported",
            ));
        }
        let fingerprint = self.account_fingerprint(platform).ok_or_else(|| {
            self.catalog_failed(platform, "configuration", "credentials_not_configured");
            unconfigured()
        })?;
        let token = match platform {
            Platform::Ios => self.token(platform).await.map_err(|error| {
                let reason = match error.message.as_str() {
                    "Invalid system clock" => "invalid_system_clock",
                    "Cannot sign Apple token" => "jwt_signing_failed",
                    _ => "token_generation_failed",
                };
                self.catalog_failed(platform, "apple_token", reason)
            })?,
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
        let base = Url::parse(&endpoint)
            .map_err(|_| self.catalog_failed(platform, "request", "invalid_endpoint"))?;
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
        for page_number in 1..=MAX_PAGES {
            if !seen.insert(current.to_string()) {
                return Err(self.catalog_failed(platform, "pagination", "page_cycle"));
            }
            let response = self
                .send_logged(
                    self.inner.client.get(current.clone()).bearer_auth(&token),
                    Some(platform),
                    "catalog_apps",
                )
                .await
                .map_err(|_| self.catalog_failed(platform, "apps_request", "request_failed"))?;
            let value = page(response, &mut total, |reason| {
                self.catalog_failed(platform, "apps_response", reason)
            })
            .await?;
            let entries: &[Value] = match platform {
                Platform::Ios => value.get("data").and_then(Value::as_array).ok_or_else(|| {
                    self.catalog_failed(platform, "application_fields", "invalid_apps_array")
                })?,
                Platform::Android => match value.get("apps") {
                    None => &[][..],
                    Some(v) => v
                        .as_array()
                        .ok_or_else(|| {
                            self.catalog_failed(
                                platform,
                                "application_fields",
                                "invalid_apps_array",
                            )
                        })?
                        .as_slice(),
                },
            };
            let page_hash = Sha256::digest(serde_json::to_vec(entries).map_err(|_| {
                self.catalog_failed(platform, "application_fields", "serialization_failed")
            })?)
            .to_vec();
            if !seen_pages.insert(page_hash) {
                return Err(self.catalog_failed(platform, "pagination", "repeated_page"));
            }
            for app in entries {
                let (identifier, name, id) = match platform {
                    Platform::Ios => (
                        string(&app["attributes"], "bundleId").map_err(|_| {
                            self.catalog_failed(
                                platform,
                                "application_fields",
                                "missing_bundle_identifier",
                            )
                        })?,
                        string(&app["attributes"], "name").map_err(|_| {
                            self.catalog_failed(platform, "application_fields", "missing_app_name")
                        })?,
                        string(app, "id").map_err(|_| {
                            self.catalog_failed(platform, "application_fields", "missing_app_id")
                        })?,
                    ),
                    Platform::Android => (
                        string(app, "packageName").map_err(|_| {
                            self.catalog_failed(
                                platform,
                                "application_fields",
                                "missing_bundle_identifier",
                            )
                        })?,
                        string(app, "displayName").map_err(|_| {
                            self.catalog_failed(platform, "application_fields", "missing_app_name")
                        })?,
                        string(app, "name").map_err(|_| {
                            self.catalog_failed(platform, "application_fields", "missing_app_id")
                        })?,
                    ),
                };
                apps.push(CatalogApp {
                    bundle_identifier: identifier.into(),
                    name: name.into(),
                    store_app_id: Some(id.into()),
                });
                if apps.len() > MAX_APPS {
                    return Err(self.catalog_failed(platform, "validation", "app_count_limit"));
                }
            }
            store_protocol::catalog::validate_apps(&apps)
                .map_err(|_| self.catalog_failed(platform, "validation", "invalid_app_catalog"))?;
            self.catalog_log(
                "catalog.page",
                platform,
                json!({"page":page_number,"appCount":apps.len()}),
            );
            let next = match platform {
                Platform::Ios => match value.get("links").and_then(|l| l.get("next")) {
                    None | Some(Value::Null) => None,
                    Some(v) => Some(v.as_str().ok_or_else(|| {
                        self.catalog_failed(platform, "pagination", "invalid_next_page")
                    })?),
                },
                Platform::Android => match value.get("nextPageToken") {
                    None | Some(Value::Null) => None,
                    Some(v) => Some(v.as_str().ok_or_else(|| {
                        self.catalog_failed(platform, "pagination", "invalid_next_page")
                    })?),
                },
            }
            .filter(|s| !s.is_empty());
            let Some(next) = next else {
                apps.sort_by(|a, b| a.bundle_identifier.cmp(&b.bundle_identifier));
                let catalog = AccountCatalog { fingerprint, apps };
                catalog.validate().map_err(|_| {
                    self.catalog_failed(platform, "validation", "invalid_app_catalog")
                })?;
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
                .map_err(|_| self.catalog_failed(platform, "validation", "invalid_report"))?;
                return Ok(catalog);
            };
            if next.len() > 8192 {
                return Err(self.catalog_failed(platform, "pagination", "next_page_size_limit"));
            }
            current = match platform {
                Platform::Ios => trusted_next(&base, &current, next).map_err(|_| {
                    self.catalog_failed(platform, "pagination", "untrusted_next_page")
                })?,
                Platform::Android => {
                    let mut url = base.clone();
                    url.query_pairs_mut()
                        .append_pair("pageSize", "100")
                        .append_pair("pageToken", next);
                    url
                }
            };
        }
        Err(self.catalog_failed(platform, "pagination", "page_count_limit"))
    }
    async fn reporting_token(&self) -> Result<String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| {
                self.catalog_failed(Platform::Android, "reporting_token", "invalid_system_clock")
            })?
            .as_secs();
        let mut cache = self.inner.google_reporting_token.lock().await;
        if let Some((token, expiry)) = &*cache {
            if now + 60 < *expiry {
                return Ok(token.clone());
            }
        }
        let account = self.inner.google.as_ref().ok_or_else(|| {
            self.catalog_failed(
                Platform::Android,
                "configuration",
                "credentials_not_configured",
            );
            unconfigured()
        })?;
        let assertion = encode(
            &Header::new(Algorithm::RS256),
            &json!({"iss":account.email,"scope":REPORTING_SCOPE,
            "aud":self.inner.config.google_token_url,"iat":now,"exp":now+3600}),
            &account.key,
        )
        .map_err(|_| {
            self.catalog_failed(Platform::Android, "reporting_token", "jwt_signing_failed")
        })?;
        let response = self
            .send_logged(
                self.inner
                    .client
                    .post(&self.inner.config.google_token_url)
                    .form(&[
                        ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                        ("assertion", assertion.as_str()),
                    ]),
                Some(Platform::Android),
                "oauth_token",
            )
            .await
            .map_err(|_| {
                self.catalog_failed(
                    Platform::Android,
                    "reporting_oauth_request",
                    "request_failed",
                )
            })?;
        let mut total = 0;
        let value = page(response, &mut total, |reason| {
            self.catalog_failed(Platform::Android, "reporting_oauth_response", reason)
        })
        .await?;
        let token = string(&value, "access_token")
            .map_err(|_| {
                self.catalog_failed(
                    Platform::Android,
                    "reporting_oauth_response",
                    "missing_access_token",
                )
            })?
            .to_owned();
        if token.is_empty() {
            return Err(self.catalog_failed(
                Platform::Android,
                "reporting_oauth_response",
                "empty_access_token",
            ));
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

#[cfg(test)]
mod timeout_tests {
    use super::*;
    use p256::pkcs8::{EncodePrivateKey, LineEnding};

    #[tokio::test(start_paused = true)]
    async fn discovery_deadline_logs_timeout_when_token_generation_is_waiting() {
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join("apple.p8");
        let key = p256::SecretKey::from_slice(&[7; 32]).unwrap();
        std::fs::write(
            &key_path,
            key.to_pkcs8_pem(LineEnding::LF).unwrap().as_bytes(),
        )
        .unwrap();
        let config_path = dir.path().join("connector.json");
        std::fs::write(&config_path, "{}").unwrap();
        let log = diagnostics::Log::for_config(&config_path, "catalog-timeout").unwrap();
        let gateway = Gateway::with_log(
            ProviderConfig {
                apple_issuer_id: Some("fixture-issuer".into()),
                apple_key_id: Some("fixture-key".into()),
                apple_private_key_path: Some(key_path.to_string_lossy().into_owned()),
                ..ProviderConfig::default()
            },
            None,
            Some(log.clone()),
        )
        .unwrap();
        let lock = gateway.inner.apple_token.lock().await;
        let cloned = gateway.clone();
        let task = tokio::spawn(async move { cloned.account_catalog(Platform::Ios).await });
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(121)).await;
        assert_eq!(
            task.await.unwrap().unwrap_err().code,
            "catalog_discovery_failed"
        );
        drop(lock);
        let entries: Vec<Value> = std::fs::read_dir(log.directory())
            .unwrap()
            .flat_map(|file| {
                std::fs::read_to_string(file.unwrap().path())
                    .unwrap()
                    .lines()
                    .map(|line| serde_json::from_str::<Value>(line).unwrap())
                    .collect::<Vec<_>>()
            })
            .collect();
        assert!(entries
            .iter()
            .any(|entry| entry["event"] == "catalog.failed"
                && entry["details"]["stage"] == "discovery"
                && entry["details"]["reason"] == "timeout"));
        assert!(!entries
            .iter()
            .any(|entry| entry["event"] == "upstream.started"));
    }
}

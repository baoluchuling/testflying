//! OSS V4 URL signing and bounded HEAD verification. Never log URLs or upstream errors.
use crate::screenshot_settings::OssSettings;
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use reqwest::{header::HeaderMap, Client, StatusCode, Url};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, time::Duration};
use store_protocol::StoreError;

#[derive(Clone)]
pub(crate) struct OssStorage {
    config: OssSettings,
    client: Client,
}

fn config_error() -> StoreError {
    StoreError::new(
        503,
        "screenshot_oss_not_configured",
        "Configure and enable screenshot OSS storage before uploading",
    )
}
fn signing_error() -> StoreError {
    StoreError::new(
        422,
        "invalid_screenshot_object",
        "Screenshot object or signed headers are invalid",
    )
}
fn unavailable(status: u16) -> StoreError {
    StoreError::new(
        status,
        "screenshot_verification_unavailable",
        "Screenshot storage verification is unavailable; retry completion",
    )
}

fn encode(value: &str, slash_safe: bool) -> String {
    let mut out = String::with_capacity(value.len());
    const HEX: &[u8] = b"0123456789ABCDEF";
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'_' | b'.' | b'~')
            || (slash_safe && byte == b'/')
        {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 15) as usize] as char);
        }
    }
    out
}

fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 1024
        && !key.starts_with('/')
        && !key.chars().any(|c| c.is_control() || c == '\\')
        && !key
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
}

fn hmac(key: &[u8], value: &str) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC key");
    mac.update(value.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

fn sign_canonical(config: &OssSettings, canonical: &str, date: &str, time: &str) -> String {
    let scope = format!("{date}/{}/oss/aliyun_v4_request", config.region);
    let to_sign = format!(
        "OSS4-HMAC-SHA256\n{time}\n{scope}\n{}",
        hex::encode(Sha256::digest(canonical.as_bytes()))
    );
    let date_key = hmac(
        format!("aliyun_v4{}", config.access_key_secret).as_bytes(),
        date,
    );
    let region_key = hmac(&date_key, &config.region);
    let service_key = hmac(&region_key, "oss");
    let signing_key = hmac(&service_key, "aliyun_v4_request");
    hex::encode(hmac(&signing_key, &to_sign))
}

impl OssStorage {
    pub(crate) fn new(config: &OssSettings) -> Result<Self, StoreError> {
        config.validate().map_err(|_| config_error())?;
        if !config.enabled {
            return Err(config_error());
        }
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|_| config_error())?;
        Ok(Self {
            config: config.clone(),
            client,
        })
    }

    pub(crate) fn presign(
        &self,
        method: &str,
        key: &str,
        headers: &BTreeMap<String, String>,
        ttl: u64,
    ) -> Result<String, StoreError> {
        self.presign_at(method, key, headers, ttl, Utc::now())
    }

    pub(crate) fn presign_at(
        &self,
        method: &str,
        key: &str,
        headers: &BTreeMap<String, String>,
        ttl: u64,
        now: DateTime<Utc>,
    ) -> Result<String, StoreError> {
        if !matches!(method, "GET" | "HEAD" | "PUT")
            || !valid_key(key)
            || ttl == 0
            || ttl
                > if self.config.security_token.is_empty() {
                    604800
                } else {
                    43200
                }
        {
            return Err(signing_error());
        }
        let endpoint = Url::parse(&self.config.endpoint).map_err(|_| config_error())?;
        let hostname = endpoint.host_str().ok_or_else(config_error)?;
        let host = endpoint
            .port()
            .map_or_else(|| hostname.to_owned(), |port| format!("{hostname}:{port}"));
        let mut signed = BTreeMap::new();
        for (name, value) in headers {
            let name = name.to_ascii_lowercase();
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
                || value.bytes().any(|byte| byte.is_ascii_control())
                || signed.insert(name, value.trim().to_owned()).is_some()
            {
                return Err(signing_error());
            }
        }
        if signed.get("host").is_some_and(|value| value != &host) {
            return Err(signing_error());
        }
        signed.insert("host".into(), host);
        let additional = signed
            .keys()
            .filter(|name| {
                name.as_str() != "content-type"
                    && name.as_str() != "content-md5"
                    && !name.starts_with("x-oss-")
            })
            .cloned()
            .collect::<Vec<_>>()
            .join(";");
        let canonical_headers = signed
            .iter()
            .map(|(name, value)| format!("{name}:{value}\n"))
            .collect::<String>();
        let date = now.format("%Y%m%d").to_string();
        let time = now.format("%Y%m%dT%H%M%SZ").to_string();
        let scope = format!("{date}/{}/oss/aliyun_v4_request", self.config.region);
        let mut query = BTreeMap::from([
            ("x-oss-signature-version", "OSS4-HMAC-SHA256".to_owned()),
            (
                "x-oss-credential",
                format!("{}/{scope}", self.config.access_key_id),
            ),
            ("x-oss-date", time.clone()),
            ("x-oss-expires", ttl.to_string()),
            ("x-oss-additional-headers", additional.clone()),
        ]);
        if !self.config.security_token.is_empty() {
            query.insert("x-oss-security-token", self.config.security_token.clone());
        }
        let query = query
            .iter()
            .map(|(name, value)| format!("{}={}", encode(name, false), encode(value, false)))
            .collect::<Vec<_>>()
            .join("&");
        // Even when the HTTP hostname is a CNAME, OSS signs the resource /bucket/object.
        let canonical_uri = encode(&format!("/{}/{key}", self.config.bucket), true);
        let canonical = format!("{method}\n{canonical_uri}\n{query}\n{canonical_headers}\n{additional}\nUNSIGNED-PAYLOAD");
        let signature = sign_canonical(&self.config, &canonical, &date, &time);
        Ok(format!(
            "{}/{key}?{query}&x-oss-signature={signature}",
            endpoint.as_str().trim_end_matches('/'),
            key = encode(key, true)
        ))
    }

    pub(crate) async fn verify(
        &self,
        key: &str,
        size: u64,
        sha256: &str,
        md5: &str,
    ) -> Result<(), StoreError> {
        let url = self.presign("HEAD", key, &BTreeMap::new(), 60)?;
        let response = self
            .client
            .head(url)
            .send()
            .await
            .map_err(|error| unavailable(if error.is_timeout() { 504 } else { 502 }))?;
        verify_response(response.status(), response.headers(), size, sha256, md5)
    }

    pub(crate) fn image_result(&self, key: &str) -> Result<Value, StoreError> {
        if !valid_key(key) {
            return Err(signing_error());
        }
        let base = if self.config.public_base_url.is_empty() {
            &self.config.endpoint
        } else {
            &self.config.public_base_url
        };
        let base = Url::parse(base).map_err(|_| config_error())?;
        Ok(
            json!({"imageUrl":format!("{}/{}", base.as_str().trim_end_matches('/'), encode(key, true))}),
        )
    }
}

fn verify_response(
    status: StatusCode,
    headers: &HeaderMap,
    size: u64,
    sha256: &str,
    md5: &str,
) -> Result<(), StoreError> {
    if status == StatusCode::NOT_FOUND {
        return Err(StoreError::new(
            409,
            "screenshot_object_missing",
            "Screenshot object is not available yet; retry completion",
        ));
    }
    if status != StatusCode::OK {
        return Err(unavailable(502));
    }
    let header = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
    if header("content-length").and_then(|value| value.parse::<u64>().ok()) != Some(size)
        || header("content-type") != Some("image/png")
        || header("x-oss-meta-sha256") != Some(sha256)
        || header("x-oss-meta-md5") != Some(md5)
    {
        return Err(StoreError::new(
            422,
            "screenshot_object_mismatch",
            "Screenshot object size, type, or checksum metadata does not match the upload ticket",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn storage() -> OssStorage {
        OssStorage::new(&OssSettings {
            enabled: true,
            region: "cn-hangzhou".into(),
            bucket: "examplebucket".into(),
            endpoint: "https://examplebucket.oss-cn-hangzhou.aliyuncs.com".into(),
            access_key_id: "test-access-id".into(),
            access_key_secret: "test-access-secret".into(),
            ..OssSettings::default()
        })
        .unwrap()
    }

    #[test]
    fn signing_core_matches_official_python_sdk_fixed_vector() {
        // SDK test_auth_query: public fictional credentials and expected signature.
        // https://github.com/aliyun/alibabacloud-oss-python-sdk-v2/blob/master/tests/unit/signer/test_v4.py
        let config = OssSettings {
            access_key_id: "ak".into(),
            access_key_secret: "sk".into(),
            region: "cn-hangzhou".into(),
            ..OssSettings::default()
        };
        let canonical = concat!(
            "PUT\n/bucket/1234%2B-/123/1.txt\n",
            "%2Bparam1=value3&%2Bparam2&%7Cparam1=value4&%7Cparam2&param1=value1&param2&x-oss-credential=ak%2F20231217%2Fcn-hangzhou%2Foss%2Faliyun_v4_request&x-oss-date=20231217T025437Z&x-oss-expires=599&x-oss-signature-version=OSS4-HMAC-SHA256\n",
            "content-type:application/octet-stream\nx-oss-head1:value\n\n\nUNSIGNED-PAYLOAD"
        );
        assert_eq!(
            sign_canonical(&config, canonical, "20231217", "20231217T025437Z"),
            "a39966c61718be0d5b14e668088b3fa07601033f6518ac7b523100014269c0fe"
        );
    }

    #[test]
    fn canonical_request_matches_independent_official_example_construction() {
        // Literal canonical request independently transcribed from Aliyun's V4 Java example:
        // https://help.aliyun.com/zh/oss/developer-reference/add-signatures-to-urls
        let oss = storage();
        let now = DateTime::parse_from_rfc3339("2024-12-03T03:23:07Z")
            .unwrap()
            .with_timezone(&Utc);
        let signed = oss
            .presign_at("GET", "exampleobject", &BTreeMap::new(), 86400, now)
            .unwrap();
        let canonical = concat!(
            "GET\n/examplebucket/exampleobject\n",
            "x-oss-additional-headers=host&x-oss-credential=test-access-id%2F20241203%2Fcn-hangzhou%2Foss%2Faliyun_v4_request&x-oss-date=20241203T032307Z&x-oss-expires=86400&x-oss-signature-version=OSS4-HMAC-SHA256\n",
            "host:examplebucket.oss-cn-hangzhou.aliyuncs.com\n\nhost\nUNSIGNED-PAYLOAD"
        );
        let to_sign = format!(
            "OSS4-HMAC-SHA256\n20241203T032307Z\n20241203/cn-hangzhou/oss/aliyun_v4_request\n{}",
            hex::encode(Sha256::digest(canonical))
        );
        // Deliberately independent HMAC implementation, with each scope step explicit.
        let mac = |key: &[u8], value: &[u8]| {
            let mut value_mac = Hmac::<Sha256>::new_from_slice(key).unwrap();
            value_mac.update(value);
            value_mac.finalize().into_bytes()
        };
        let date = mac(b"aliyun_v4test-access-secret", b"20241203");
        let region = mac(&date, b"cn-hangzhou");
        let service = mac(&region, b"oss");
        let signing = mac(&service, b"aliyun_v4_request");
        let expected = hex::encode(mac(&signing, to_sign.as_bytes()));
        let url = Url::parse(&signed).unwrap();
        assert_eq!(
            url.query_pairs()
                .find(|(key, _)| key == "x-oss-signature")
                .unwrap()
                .1,
            expected
        );
    }

    #[test]
    fn put_headers_cname_sts_and_escaping_are_bound() {
        let mut oss = storage();
        oss.config.endpoint = "https://images.fixture.example".into();
        oss.config.security_token = "test-sts+/=".into();
        let headers = BTreeMap::from([
            ("content-length".into(), "128".into()),
            ("content-type".into(), "image/png".into()),
            ("content-md5".into(), "fixture-md5==".into()),
            ("x-oss-meta-sha256".into(), "fixture-sha256".into()),
            ("x-oss-forbid-overwrite".into(), "true".into()),
        ]);
        let now = DateTime::parse_from_rfc3339("2024-12-03T03:23:07Z")
            .unwrap()
            .with_timezone(&Utc);
        let signed = oss
            .presign_at("PUT", "screenshots/图 +.png", &headers, 300, now)
            .unwrap();
        let url = Url::parse(&signed).unwrap();
        assert_eq!(url.host_str(), Some("images.fixture.example"));
        assert_eq!(url.path(), "/screenshots/%E5%9B%BE%20%2B.png");
        assert!(url
            .query_pairs()
            .any(|(name, value)| name == "x-oss-additional-headers"
                && value == "content-length;host"));
        assert!(url
            .query_pairs()
            .any(|(name, value)| name == "x-oss-security-token" && value == "test-sts+/="));
        let mut changed = headers.clone();
        changed.insert("content-length".into(), "129".into());
        assert_ne!(
            signed,
            oss.presign_at("PUT", "screenshots/图 +.png", &changed, 300, now)
                .unwrap()
        );
        let mut other_bucket = oss.clone();
        other_bucket.config.bucket = "other-bucket".into();
        assert_ne!(
            signed,
            other_bucket
                .presign_at("PUT", "screenshots/图 +.png", &headers, 300, now)
                .unwrap()
        );
    }

    #[test]
    fn head_requires_exact_integrity_and_missing_is_retryable() {
        let mut headers = HeaderMap::new();
        for (name, value) in [
            ("content-length", "128"),
            ("content-type", "image/png"),
            ("x-oss-meta-sha256", "sha"),
            ("x-oss-meta-md5", "md5"),
        ] {
            headers.insert(
                reqwest::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
        }
        assert!(verify_response(StatusCode::OK, &headers, 128, "sha", "md5").is_ok());
        assert_eq!(
            verify_response(StatusCode::NOT_FOUND, &headers, 128, "sha", "md5")
                .unwrap_err()
                .code,
            "screenshot_object_missing"
        );
        assert_eq!(
            verify_response(StatusCode::OK, &headers, 129, "sha", "md5")
                .unwrap_err()
                .status,
            422
        );
        assert_eq!(
            verify_response(StatusCode::OK, &headers, 128, "other", "md5")
                .unwrap_err()
                .status,
            422
        );
        headers.remove("content-type");
        assert_eq!(
            verify_response(StatusCode::OK, &headers, 128, "sha", "md5")
                .unwrap_err()
                .status,
            422
        );
        assert_eq!(
            verify_response(StatusCode::TEMPORARY_REDIRECT, &headers, 128, "sha", "md5")
                .unwrap_err()
                .code,
            "screenshot_verification_unavailable"
        );
    }

    #[test]
    fn result_uses_stable_unsigned_endpoint_or_public_url() {
        let mut oss = storage();
        let private = oss.image_result("screenshots/fixture.png").unwrap();
        assert_eq!(
            private["imageUrl"],
            "https://examplebucket.oss-cn-hangzhou.aliyuncs.com/screenshots/fixture.png"
        );
        assert_eq!(
            private,
            oss.image_result("screenshots/fixture.png").unwrap()
        );
        assert!(private.get("imageUrlExpiresAt").is_none());
        assert!(Url::parse(private["imageUrl"].as_str().unwrap())
            .unwrap()
            .query()
            .is_none());
        oss.config.public_base_url = "https://cdn.fixture.example/images/".into();
        let public = oss.image_result("screenshots/图 +.png").unwrap();
        assert_eq!(
            public["imageUrl"],
            "https://cdn.fixture.example/images/screenshots/%E5%9B%BE%20%2B.png"
        );
        assert!(public.get("imageUrlExpiresAt").is_none());
        assert!(oss.image_result("../secret").is_err());
    }
}

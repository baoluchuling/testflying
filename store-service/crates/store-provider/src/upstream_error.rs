//! Extract bounded vendor codes and fixed explanations, never raw messages or caller content.
use serde_json::{json, Value};

pub(super) fn summarize(bytes: &[u8]) -> Value {
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        return json!({"classification":"unclassified","format":"non_json"});
    };
    if let Some(errors) = value["errors"]
        .as_array()
        .filter(|errors| !errors.is_empty())
    {
        return summarize_apple(errors);
    }
    let google = &value["error"];
    let code = google["status"].as_str().or_else(|| google.as_str());
    let code = code.filter(|code| known_code(code));
    let reason = google
        .pointer("/errors/0/reason")
        .and_then(Value::as_str)
        .filter(|code| known_code(code));
    let message = google["message"]
        .as_str()
        .or_else(|| value["error_description"].as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let field = google
        .pointer("/details/0/fieldViolations/0/field")
        .and_then(Value::as_str)
        .and_then(|p| p.rsplit('.').next())
        .filter(|field| known_field(field));
    let classification = classify_message(&message);
    json!({"code":code,"reason":reason,"field":field,"classification":classification})
}

fn summarize_apple(errors: &[Value]) -> Value {
    let summaries: Vec<_> = errors
        .iter()
        .take(8)
        .map(|error| {
            let raw_code = error["code"].as_str();
            let code = raw_code.filter(|code| apple_code(code));
            let field = error
                .pointer("/source/pointer")
                .and_then(Value::as_str)
                .and_then(|p| p.rsplit('/').next())
                .filter(|field| known_field(field));
            // Only inspect text in memory. Even titles may echo app content or credentials.
            let message = ["title", "detail"]
                .iter()
                .filter_map(|key| error[*key].as_str())
                .map(str::to_ascii_lowercase)
                .collect::<Vec<_>>()
                .join(" ");
            let classification = classify_apple(code.unwrap_or(""), &message);
            json!({
                "code":code,
                "codeStatus": if code.is_some() {"accepted"} else if raw_code.is_some() {"rejected"} else {"missing"},
                "field":field,
                "classification":classification,
                "explanation":explanation(classification),
            })
        })
        .collect();
    let first = &summaries[0];
    json!({
        "code":first["code"], "reason":null, "field":first["field"],
        "classification":first["classification"], "explanation":first["explanation"],
        "errors":summaries, "errorCount":errors.len(), "errorsTruncated":errors.len()>8,
        "explanationSource":"local_classification", "rawMessagesOmitted":true,
    })
}

fn apple_code(code: &str) -> bool {
    // Keep new subtypes in Apple's standard namespaces without copying arbitrary text,
    // URLs, JWTs, newlines or unbounded vendor values into a log record.
    if code.is_empty() || code.len() > 160 {
        return false;
    }
    let mut parts = code.split('.');
    let root = parts.next().unwrap_or_default();
    if !matches!(
        root,
        "FORBIDDEN"
            | "FORBIDDEN_ERROR"
            | "NOT_AUTHORIZED"
            | "ENTITY_ERROR"
            | "PARAMETER_ERROR"
            | "PATH_ERROR"
            | "STATE_ERROR"
            | "REQUEST_ERROR"
            | "JSON_ERROR"
            | "UNEXPECTED_ERROR"
            | "RATE_LIMIT_EXCEEDED"
            | "METHOD_NOT_ALLOWED"
    ) {
        return false;
    }
    parts.all(|part| {
        !part.is_empty()
            && part.len() <= 64
            && part
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
    })
}

fn classify_apple(code: &str, message: &str) -> &'static str {
    if code.contains("AGREEMENT")
        || code.contains("PLA_NOT_VALID")
        || message.contains("agreements are missing or expired")
        || message.contains("required agreements")
        || message.contains("program license agreement")
    {
        "agreements_required"
    } else if code.contains("REVOKED")
        || message.contains("key has been revoked")
        || message.contains("key is revoked")
    {
        "key_revoked"
    } else if code.starts_with("NOT_AUTHORIZED")
        || message.contains("token is invalid")
        || message.contains("token has expired")
        || message.contains("authentication credentials")
    {
        "authentication_failed"
    } else if code.starts_with("FORBIDDEN") {
        "permission_denied"
    } else if code.starts_with("RATE_LIMIT_EXCEEDED") {
        "rate_limited"
    } else {
        classify_message(message)
    }
}

fn explanation(classification: &str) -> &'static str {
    match classification {
        "agreements_required" => {
            "Apple 要求的协议未生效，请由 Account Holder 检查待签署或已过期协议。"
        }
        "key_revoked" => "Apple 指出密钥已撤销，请检查节点使用的密钥状态。",
        "authentication_failed" => "Apple 拒绝身份认证，请核对密钥配套、有效状态及 JWT 配置。",
        "permission_denied" => "Apple 拒绝此操作，需结合错误子码核对授权及账号限制。",
        "rate_limited" => "Apple 请求频率受限。",
        "locale_requirement" => "Apple 返回语言或地区字段相关错误。",
        "title_requirement" => "Apple 返回应用名称或标题相关错误。",
        "resource_already_exists" => "Apple 返回资源已存在错误。",
        _ => "尚未识别具体原因，请结合错误码、字段和请求步骤排查。",
    }
}

fn classify_message(message: &str) -> &'static str {
    if message.contains("changes in review") {
        "changes_in_review"
    } else if message.contains("changesnotsentforreview")
        && [
            "must not",
            "not be set",
            "must be false",
            "not supported",
            "not allowed",
        ]
        .iter()
        .any(|s| message.contains(s))
    {
        "review_flag_not_allowed"
    } else if message.contains("changesnotsentforreview") && message.contains("true") {
        "review_flag_required"
    } else if message.contains("draft app")
        || message.contains("has not been published")
        || message.contains("not yet published")
    {
        "app_not_published"
    } else if message.contains("privacy policy") {
        "privacy_policy_requirement"
    } else if message.contains("contains ads")
        || message.contains("content rating")
        || message.contains("app content")
    {
        "app_declaration_requirement"
    } else if message.contains("permission") || message.contains("forbidden") {
        "permission_denied"
    } else if message.contains("language") || message.contains("locale") {
        "locale_requirement"
    } else if message.contains("short description") || message.contains("shortdescription") {
        "short_description_requirement"
    } else if message.contains("full description") || message.contains("fulldescription") {
        "description_requirement"
    } else if message.contains("title") || message.contains("app name") {
        "title_requirement"
    } else if message.contains("already exists") {
        "resource_already_exists"
    } else {
        "unclassified"
    }
}
fn known_field(field: &str) -> bool {
    matches!(
        field,
        "name"
            | "title"
            | "subtitle"
            | "description"
            | "fullDescription"
            | "shortDescription"
            | "keywords"
            | "promotionalText"
            | "whatsNew"
            | "language"
            | "locale"
            | "video"
            | "appInfo"
            | "appStoreVersion"
            | "fileSize"
            | "fileName"
    )
}
fn known_code(code: &str) -> bool {
    matches!(
        code,
        "INVALID_ARGUMENT"
            | "FAILED_PRECONDITION"
            | "PERMISSION_DENIED"
            | "UNAUTHENTICATED"
            | "NOT_FOUND"
            | "ALREADY_EXISTS"
            | "RESOURCE_EXHAUSTED"
            | "INTERNAL"
            | "UNAVAILABLE"
            | "badRequest"
            | "invalid"
            | "forbidden"
            | "conditionNotMet"
            | "notFound"
            | "invalid_grant"
            | "invalid_client"
            | "invalid_scope"
            | "PARAMETER_ERROR.INVALID"
            | "ENTITY_ERROR.ATTRIBUTE.INVALID"
            | "ENTITY_ERROR.ATTRIBUTE.INVALID.REQUIRED"
            | "ENTITY_ERROR.ATTRIBUTE.INVALID.DUPLICATE"
            | "ENTITY_ERROR.RELATIONSHIP.INVALID"
            | "STATE_ERROR.INVALID_STATE"
            | "FORBIDDEN_ERROR"
            | "NOT_AUTHORIZED"
            | "UNEXPECTED_ERROR"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apple_agreement_subcodes_survive_with_fixed_explanations() {
        for code in [
            "FORBIDDEN.REQUIRED_AGREEMENTS_MISSING_OR_EXPIRED",
            "FORBIDDEN_ERROR.PLA_NOT_VALID",
        ] {
            let value = summarize(
                &serde_json::to_vec(&json!({"errors":[{
                    "code":code,
                    "title":"private-app-title",
                    "detail":"private-response-text https://private.invalid/?token=private-token"
                }]}))
                .unwrap(),
            );
            assert_eq!(value["code"], code);
            assert_eq!(value["classification"], "agreements_required");
            assert!(value["explanation"]
                .as_str()
                .unwrap()
                .contains("Account Holder"));
            assert_eq!(value["errors"][0]["codeStatus"], "accepted");
            assert_eq!(value["explanationSource"], "local_classification");
            assert_eq!(value["rawMessagesOmitted"], true);
            assert!(!value.to_string().contains("private-"));
        }
    }

    #[test]
    fn apple_multiple_errors_are_bounded_without_losing_unknown_safe_subtypes() {
        let mut errors = vec![
            json!({
                "code":"FORBIDDEN_ERROR.NEW_RESTRICTION",
                "detail":"private-copy"
            }),
            json!({
                "code":"ENTITY_ERROR.ATTRIBUTE.NEW_VALIDATION",
                "source":{"pointer":"/private-app/data/attributes/description"},
            "title":"private-copy"
            }),
        ];
        errors.extend((0..20).map(|_| json!({"code":"STATE_ERROR.FUTURE_STATE"})));
        let value = summarize(&serde_json::to_vec(&json!({"errors":errors})).unwrap());
        assert_eq!(value["code"], "FORBIDDEN_ERROR.NEW_RESTRICTION");
        assert_eq!(
            value["errors"][1]["code"],
            "ENTITY_ERROR.ATTRIBUTE.NEW_VALIDATION"
        );
        assert_eq!(value["errors"][1]["field"], "description");
        assert_eq!(value["errors"][1]["classification"], "unclassified");
        assert_eq!(value["errors"].as_array().unwrap().len(), 8);
        assert_eq!(value["errorCount"], 22);
        assert_eq!(value["errorsTruncated"], true);
        assert!(value.to_string().len() < 8192);
        assert!(!value.to_string().contains("private-"));
    }

    #[test]
    fn apple_unsafe_codes_and_vendor_text_never_enter_logs() {
        for code in [
            "private-token".to_owned(),
            "PRIVATE_SECRET".to_owned(),
            "FORBIDDEN_ERROR.private_token".to_owned(),
            "FORBIDDEN_ERROR.https://private.invalid/?token=secret".to_owned(),
            "FORBIDDEN_ERROR.\nPRIVATE_TOKEN".to_owned(),
            "FORBIDDEN_ERROR..INVALID".to_owned(),
            format!("FORBIDDEN_ERROR.{}", "SECRET".repeat(30)),
            "FORBIDDEN_ERROR.密钥".to_owned(),
        ] {
            let value = summarize(
                &serde_json::to_vec(&json!({"errors":[{
                    "code":code,
                    "id":"private-id", "status":"private-status", "links":{"about":"private-url"},
                    "title":"Authorization: Bearer private-token",
                    "detail":"BEGIN PRIVATE KEY private-key; private-app private-email@example.com",
                    "source":{"pointer":"/private-field"}
                }]}))
                .unwrap(),
            );
            assert!(value["code"].is_null());
            assert!(value["errors"][0]["field"].is_null());
            assert_eq!(value["errors"][0]["codeStatus"], "rejected");
            let text = value.to_string();
            for secret in [
                "private-",
                "PRIVATE_SECRET",
                "PRIVATE_TOKEN",
                "SECRETSECRET",
                "Authorization",
                "BEGIN PRIVATE KEY",
            ] {
                assert!(!text.contains(secret), "{secret}");
            }
        }
    }

    #[test]
    fn apple_text_is_classified_without_echoing_it_or_assuming_all_403_are_agreements() {
        for (code, detail, expected) in [
            (
                "FORBIDDEN_ERROR",
                "Required agreements are missing or expired private-copy",
                "agreements_required",
            ),
            (
                "FORBIDDEN_ERROR",
                "The API key has been revoked private-token",
                "key_revoked",
            ),
            ("NOT_AUTHORIZED", "private-copy", "authentication_failed"),
            ("RATE_LIMIT_EXCEEDED", "private-copy", "rate_limited"),
            ("FORBIDDEN_ERROR", "private-copy", "permission_denied"),
            ("ENTITY_ERROR.NEW_SUBTYPE", "private-copy", "unclassified"),
        ] {
            let value = summarize(
                &serde_json::to_vec(&json!({"errors":[{"code":code,"detail":detail}]})).unwrap(),
            );
            assert_eq!(value["classification"], expected);
            assert!(!value.to_string().contains("private-"));
        }
        for bytes in [
            b"not json private-token".as_slice(),
            br#"{"errors":["private-token",null,{}]}"#,
        ] {
            let value = summarize(bytes);
            assert!(!value.to_string().contains("private-token"));
        }
        let value = summarize(br#"{"errors":[{}]}"#);
        assert_eq!(value["errors"][0]["codeStatus"], "missing");
        let value = summarize(
            br#"{"errors":[],"error":{"status":"PERMISSION_DENIED","message":"private-copy"}}"#,
        );
        assert_eq!(value["code"], "PERMISSION_DENIED");
        assert!(!value.to_string().contains("private-copy"));
    }

    #[test]
    fn extracts_explanations_without_copying_messages_or_unknown_values() {
        let value=summarize(br#"{"error":{"status":"FAILED_PRECONDITION","message":"private-token: You already have changes in review; secret-copy","errors":[{"reason":"badRequest"}]}}"#);
        assert_eq!(value["classification"], "changes_in_review");
        assert_eq!(value["code"], "FAILED_PRECONDITION");
        assert!(!value.to_string().contains("private-token"));
        assert!(!value.to_string().contains("secret-copy"));
        let value=summarize(br#"{"errors":[{"code":"ENTITY_ERROR.ATTRIBUTE.INVALID","detail":"private-copy","source":{"pointer":"/data/attributes/description"}}]}"#);
        assert_eq!(value["field"], "description");
        assert!(!value.to_string().contains("private-copy"));
        assert!(
            !summarize(br#"{"error":{"status":"private-token","message":"private-copy"}}"#)
                .to_string()
                .contains("private")
        );
    }
}

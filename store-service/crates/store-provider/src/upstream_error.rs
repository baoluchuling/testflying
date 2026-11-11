//! Extract fixed diagnostic labels, never raw vendor messages or caller content.
use serde_json::{json, Value};

pub(super) fn summarize(bytes: &[u8]) -> Value {
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        return json!({"classification":"unclassified","format":"non_json"});
    };
    let apple = value["errors"].as_array().and_then(|v| v.first());
    let google = &value["error"];
    let code = apple
        .and_then(|v| v["code"].as_str())
        .or_else(|| google["status"].as_str())
        .or_else(|| google.as_str());
    let code = code.filter(|code| known_code(code));
    let reason = google
        .pointer("/errors/0/reason")
        .and_then(Value::as_str)
        .filter(|code| known_code(code));
    let message = google["message"]
        .as_str()
        .or_else(|| value["error_description"].as_str())
        .or_else(|| apple.and_then(|v| v["detail"].as_str()))
        .unwrap_or("")
        .to_ascii_lowercase();
    let field = apple
        .and_then(|v| v.pointer("/source/pointer"))
        .and_then(Value::as_str)
        .and_then(|p| p.rsplit('/').next())
        .filter(|field| known_field(field))
        .or_else(|| {
            google
                .pointer("/details/0/fieldViolations/0/field")
                .and_then(Value::as_str)
                .and_then(|p| p.rsplit('.').next())
                .filter(|field| known_field(field))
        });
    let classification = if message.contains("changes in review") {
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
    };
    json!({"code":code,"reason":reason,"field":field,"classification":classification})
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
            | "UNEXPECTED_ERROR"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
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

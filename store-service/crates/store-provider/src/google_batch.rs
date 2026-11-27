//! One application, one edit and one commit for an explicit localization batch.
use super::*;

pub fn is_google_batch_sync(request: &StoreRequest) -> bool {
    request.platform == Platform::Android
        && matches!(
            request.operation,
            Operation::Sync | Operation::StartStoreImageSync
        )
        && request.payload.get("localizations").is_some()
}

pub(super) fn validate_batch(request: &StoreRequest) -> Result<()> {
    let p = &request.payload;
    if !is_google_batch_sync(request) || s(p, "operation") != "update_app_metadata" {
        return Err(invalid(
            "localizations requires Android update_app_metadata sync",
        ));
    }
    if p.as_object().is_none_or(|fields| {
        fields
            .keys()
            .any(|k| !["operation", "localizations"].contains(&k.as_str()))
    }) {
        return Err(invalid(
            "Batch sync accepts only operation and localizations at the top level",
        ));
    }
    let items = p["localizations"]
        .as_array()
        .filter(|items| (1..=50).contains(&items.len()))
        .ok_or_else(|| invalid("localizations must contain 1 to 50 entries"))?;
    let mut seen = std::collections::HashSet::new();
    for item in items {
        if item.as_object().is_none_or(|fields| {
            fields
                .keys()
                .any(|k| !["locale", "syncScopes", "metadata"].contains(&k.as_str()))
        }) {
            return Err(invalid(
                "Each localization accepts only locale, syncScopes and metadata",
            ));
        }
        let locale = required(item, "locale")?;
        if locale.len() > 80
            || !locale
                .split('-')
                .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric()))
        {
            return Err(invalid(
                "Each batch locale must be an ASCII BCP-47 language tag",
            ));
        }
        if !seen.insert(locale.to_ascii_lowercase()) {
            return Err(invalid("Batch locales must be unique, ignoring case"));
        }
        if let Some(scopes) = item["syncScopes"].as_array() {
            if scopes.iter().any(|v| {
                !matches!(
                    v.as_str(),
                    Some(
                        "metadata"
                            | "title"
                            | "description"
                            | "short_description"
                            | "promotional_text"
                            | "video"
                            | "store_images"
                    )
                )
            }) {
                return Err(invalid(
                    "Batch syncScopes must select Android metadata or store_images",
                ));
            }
        }
        let mut single = request.clone();
        single.operation = Operation::Sync;
        single.payload = item.clone();
        single.payload["operation"] = json!("update_app_metadata");
        validation::validate(&single)?;
        let selected_text = !super::google::metadata_body(item)?.is_empty();
        let selected_images =
            scopes(item, "store_images", false) && item["metadata"].get("storeImages").is_some();
        if !selected_text && !selected_images {
            return Err(invalid(
                "Each localization must supply selected metadata or image content",
            ));
        }
    }
    Ok(())
}

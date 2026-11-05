//! Validate caller selectors before convenience accessors or upstream calls can
//! interpret malformed values as absent and broaden a requested mutation.
use super::*;

const SYNC_SCOPES: &[&str] = &[
    "metadata",
    "description",
    "keywords",
    "promotional_text",
    "short_description",
    "title",
    "subtitle",
    "video",
    "store_images",
    "marketing_text",
];

pub(super) fn validate(request: &StoreRequest) -> Result<()> {
    let payload = &request.payload;
    optional_strings(payload, &["operation", "locale", "version"])?;
    if let Some(value) = payload.get("syncScopes") {
        let values = value
            .as_array()
            .ok_or_else(|| invalid("syncScopes must be an array of known scope strings"))?;
        if values.is_empty() {
            return Err(invalid(
                "syncScopes must select at least one scope; omit it for default text scopes",
            ));
        }
        for scope in values {
            if !scope
                .as_str()
                .is_some_and(|scope| SYNC_SCOPES.contains(&scope))
            {
                return Err(invalid(
                    "syncScopes contains an unknown or non-string scope",
                ));
            }
        }
    }
    if let Some(value) = payload.get("storeRelease") {
        let fields = value
            .as_object()
            .ok_or_else(|| invalid("storeRelease must be an object"))?;
        if fields.is_empty() {
            return Err(invalid("storeRelease must include a release selector"));
        }
        for (key, value) in fields {
            if !["track", "versionCode", "versionName"].contains(&key.as_str()) {
                return Err(invalid("storeRelease contains an unknown selector"));
            }
            if value.as_str().is_none_or(|value| value.trim().is_empty()) {
                return Err(invalid(format!(
                    "storeRelease.{key} must be a nonempty string"
                )));
            }
        }
    }
    for content in [payload.get("metadata"), payload.get("marketingPage")]
        .into_iter()
        .flatten()
    {
        if let Some(images) = content.get("storeImages") {
            images::validate(images, request.platform)?;
        }
    }
    if matches!(request.operation, Operation::Preflight | Operation::Sync) {
        let operation = required(payload, "operation")?;
        if !matches!(
            operation,
            "update_app_metadata" | "update_release_notes" | "update_marketing_page"
        ) {
            return Err(invalid("Unsupported sync operation"));
        }
        if request.platform == Platform::Ios && operation != "update_marketing_page" {
            required(payload, "version")?;
        }
        if request.operation == Operation::Sync {
            match operation {
                "update_app_metadata" => {
                    required(payload, "locale")?;
                    let metadata = payload
                        .get("metadata")
                        .filter(|value| value.is_object())
                        .ok_or_else(|| invalid("metadata must be an object"))?;
                    optional_strings(
                        metadata,
                        &[
                            "title",
                            "subtitle",
                            "description",
                            "fullDescription",
                            "keywords",
                            "promotionalText",
                            "shortDescription",
                            "videoUrl",
                        ],
                    )?;
                }
                "update_release_notes" => {
                    required(payload, "locale")?;
                    required(payload, "releaseNotes")?;
                }
                "update_marketing_page" => {
                    let page = payload
                        .get("marketingPage")
                        .filter(|value| value.is_object())
                        .ok_or_else(|| invalid("marketingPage must be an object"))?;
                    optional_strings(
                        page,
                        &["pageName", "applePageId", "locale", "promotionalText"],
                    )?;
                    required(page, "pageName")?;
                    if s(page, "locale").trim().is_empty() {
                        required(payload, "locale")?;
                    }
                }
                _ => unreachable!("validated sync operation"),
            }
        }
    }
    if request.operation == Operation::CreateProductPageOptimization {
        if request.platform != Platform::Ios {
            return Err(invalid("Product page optimization requires iOS"));
        }
        required(payload, "name")?;
        if !payload["trafficProportion"]
            .as_u64()
            .is_some_and(|value| (1..=100).contains(&value))
        {
            return Err(invalid(
                "trafficProportion must be an integer from 1 to 100",
            ));
        }
        optional_locales(payload)?;
        if let Some(value) = payload.get("treatments") {
            let treatments = value
                .as_array()
                .ok_or_else(|| invalid("treatments must be an array"))?;
            for treatment in treatments {
                if !treatment.is_object() {
                    return Err(invalid("Each treatment must be an object"));
                }
                if treatment
                    .as_object()
                    .expect("validated object")
                    .keys()
                    .any(|key| !["id", "name", "appIconName", "locales"].contains(&key.as_str()))
                {
                    return Err(invalid("Treatment contains an unknown field"));
                }
                required(treatment, "name")?;
                optional_strings(treatment, &["id", "appIconName"])?;
                optional_locales(treatment)?;
            }
        }
    }
    Ok(())
}

fn optional_strings(value: &Value, keys: &[&str]) -> Result<()> {
    for key in keys {
        if value.get(key).is_some_and(|value| !value.is_string()) {
            return Err(invalid(format!("{key} must be a string")));
        }
    }
    Ok(())
}

fn optional_locales(value: &Value) -> Result<()> {
    if let Some(locales) = value.get("locales") {
        let locales = locales
            .as_array()
            .ok_or_else(|| invalid("locales must be an array of nonempty strings"))?;
        if locales.iter().any(|locale| {
            locale
                .as_str()
                .is_none_or(|locale| locale.trim().is_empty())
        }) {
            return Err(invalid("locales must contain only nonempty strings"));
        }
    }
    Ok(())
}

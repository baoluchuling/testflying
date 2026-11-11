use crate::StoreError;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(deny_unknown_fields)]
pub struct ScreenshotTarget {
    pub locale: String,
    pub page: String,
    pub device: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
}

pub fn validate_targets(targets: &[ScreenshotTarget]) -> Result<(), StoreError> {
    let invalid = || {
        StoreError::new(
            422,
            "invalid_screenshot_target",
            "Provide 1–256 unique valid screenshot targets within 64 KiB",
        )
    };
    if targets.is_empty()
        || targets.len() > 256
        || serde_json::to_vec(targets).map_err(|_| invalid())?.len() > 64 * 1024
    {
        return Err(invalid());
    }
    let mut seen = HashSet::new();
    for target in targets {
        if target.locale.is_empty()
            || target.locale.len() > 80
            || !target
                .locale
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
            || target.page.is_empty()
            || target.page.len() > 128
            || target.page.trim() != target.page
            || target.page.chars().any(char::is_control)
            || !matches!(target.device.as_str(), "phone" | "tablet" | "foldable")
            || target
                .theme
                .as_deref()
                .is_some_and(|v| !matches!(v, "light" | "dark"))
            || !seen.insert(target)
        {
            return Err(invalid());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> ScreenshotTarget {
        ScreenshotTarget {
            locale: "en-US".into(),
            page: "home".into(),
            device: "phone".into(),
            theme: None,
        }
    }

    #[test]
    fn target_validation_rejects_invalid_bounds_and_duplicate_combinations() {
        assert!(validate_targets(&[target()]).is_ok());
        assert!(validate_targets(&[]).is_err());
        assert!(validate_targets(&vec![target(); 257]).is_err());
        assert!(validate_targets(&[target(), target()]).is_err());
        for field in ["locale", "page", "device", "theme"] {
            let mut value = serde_json::to_value(target()).unwrap();
            value[field] = serde_json::json!(" invalid ");
            assert!(validate_targets(&[serde_json::from_value(value).unwrap()]).is_err());
        }
        let mut long = target();
        long.locale = "a".repeat(81);
        assert!(validate_targets(&[long]).is_err());
        let mut long = target();
        long.page = "a".repeat(129);
        assert!(validate_targets(&[long]).is_err());
        let mut unknown = serde_json::to_value(target()).unwrap();
        unknown["targetId"] = serde_json::json!("unexpected");
        assert!(serde_json::from_value::<ScreenshotTarget>(unknown).is_err());
        let targets: Vec<_> = (0..256)
            .map(|i| ScreenshotTarget {
                locale: "a".repeat(80),
                page: format!("{i}{}", "界".repeat(40)),
                device: "foldable".into(),
                theme: Some("dark".into()),
            })
            .collect();
        assert!(validate_targets(&targets).is_err());
    }
}

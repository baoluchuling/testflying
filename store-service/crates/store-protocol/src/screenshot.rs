use crate::StoreError;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(deny_unknown_fields)]
pub struct ScreenshotTarget {
    pub locale: String,
    pub page: String,
    pub device: String,
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
    let device = &targets[0].device;
    if targets.iter().any(|target| &target.device != device) {
        return Err(StoreError::new(
            422,
            "invalid_screenshot_target",
            "Each screenshot job supports one device; split targets into separate jobs per device",
        ));
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
            || !seen.insert((&target.page, &target.locale))
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
        }
    }

    #[test]
    fn target_validation_rejects_invalid_bounds_and_duplicate_combinations() {
        assert!(validate_targets(&[target()]).is_ok());
        assert!(validate_targets(&[]).is_err());
        assert!(validate_targets(&vec![target(); 257]).is_err());
        assert!(validate_targets(&[target(), target()]).is_err());
        for field in ["locale", "page", "device"] {
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
                page: format!("{i:03}{}", "\"".repeat(125)),
                device: "foldable".into(),
            })
            .collect();
        assert!(targets.iter().all(|target| target.page.len() == 128));
        assert!(serde_json::to_vec(&targets).unwrap().len() > 64 * 1024);
        assert!(validate_targets(&targets).is_err());
    }

    #[test]
    fn targets_require_one_device_and_unique_page_locale_without_theme() {
        for device in ["phone", "tablet", "foldable"] {
            let mut first = target();
            first.device = device.into();
            let mut next_page = first.clone();
            next_page.page = "settings".into();
            let mut next_locale = first.clone();
            next_locale.locale = "zh-CN".into();
            assert!(validate_targets(&[first.clone(), next_page, next_locale]).is_ok());
            assert!(validate_targets(&[first.clone(), first]).is_err());
        }
        let mut tablet = target();
        tablet.device = "tablet".into();
        let error = validate_targets(&[target(), tablet]).unwrap_err();
        assert_eq!(error.status, 422);
        assert!(error.message.contains("separate jobs per device"));
        for theme in [serde_json::json!("light"), serde_json::Value::Null] {
            let mut value = serde_json::to_value(target()).unwrap();
            value["theme"] = theme;
            assert!(serde_json::from_value::<ScreenshotTarget>(value).is_err());
        }
    }
}

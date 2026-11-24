use crate::StoreError;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const LEGACY_SCREENSHOT_TIMEOUT_SECONDS: u64 = 24 * 60;
pub const SCREENSHOT_DELIVERY_GRACE_SECONDS: u64 = 6 * 60;
pub const MAX_SCREENSHOT_EXECUTION_SECONDS: u64 = (256 + 1) * 1440 * 60;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct ScreenshotExecutionPolicy {
    pub preparation_minutes: u32,
    pub per_image_minutes: u32,
}

impl Default for ScreenshotExecutionPolicy {
    fn default() -> Self {
        Self {
            preparation_minutes: 30,
            per_image_minutes: 15,
        }
    }
}

impl ScreenshotExecutionPolicy {
    pub fn timeout_seconds(&self, targets: usize) -> Option<u64> {
        if !(1..=256).contains(&targets)
            || self.preparation_minutes > 1440
            || !(1..=1440).contains(&self.per_image_minutes)
        {
            return None;
        }
        u64::from(self.per_image_minutes)
            .checked_mul(targets as u64)?
            .checked_add(u64::from(self.preparation_minutes))?
            .checked_mul(60)
    }
}

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

    #[test]
    fn screenshot_budget_scales_with_targets_and_validates_all_bounds() {
        let policy = ScreenshotExecutionPolicy::default();
        assert_eq!(policy.timeout_seconds(1), Some(45 * 60));
        assert_eq!(policy.timeout_seconds(64), Some((30 + 64 * 15) * 60));
        assert_eq!(policy.timeout_seconds(256), Some((30 + 256 * 15) * 60));
        assert_eq!(policy.timeout_seconds(0), None);
        assert_eq!(policy.timeout_seconds(257), None);
        assert_eq!(
            ScreenshotExecutionPolicy {
                preparation_minutes: 0,
                per_image_minutes: 1
            }
            .timeout_seconds(1),
            Some(60)
        );
        assert_eq!(
            ScreenshotExecutionPolicy {
                preparation_minutes: 1440,
                per_image_minutes: 1440
            }
            .timeout_seconds(256),
            Some(MAX_SCREENSHOT_EXECUTION_SECONDS)
        );
        for invalid in [
            ScreenshotExecutionPolicy {
                preparation_minutes: 1441,
                per_image_minutes: 15,
            },
            ScreenshotExecutionPolicy {
                preparation_minutes: 30,
                per_image_minutes: 0,
            },
            ScreenshotExecutionPolicy {
                preparation_minutes: 0,
                per_image_minutes: u32::MAX,
            },
        ] {
            assert_eq!(invalid.timeout_seconds(1), None);
        }
    }

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

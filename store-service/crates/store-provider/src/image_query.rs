use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ImageDevice {
    Phone,
    Tablet,
    Foldable,
}
impl ImageDevice {
    pub fn slot(self) -> &'static str {
        match self {
            Self::Phone => "phone_screenshots",
            Self::Tablet => "tablet_screenshots",
            Self::Foldable => "foldable_screenshots",
        }
    }
}

pub(super) struct ImageQuery<'a> {
    locale: &'a str,
    device: Option<ImageDevice>,
}
impl<'a> ImageQuery<'a> {
    pub fn new(payload: &'a Value, platform: Platform) -> Result<Self> {
        let device = match payload.get("device") {
            None => "",
            Some(value) => value
                .as_str()
                .ok_or_else(|| invalid("device must be a string"))?
                .trim(),
        };
        let device = match device {
            "" => None,
            "phone" => Some(ImageDevice::Phone),
            "tablet" => Some(ImageDevice::Tablet),
            "foldable" if platform == Platform::Ios => Some(ImageDevice::Foldable),
            "foldable" => {
                return Err(StoreError::new(
                    422,
                    "unsupported_image_device",
                    "Google Play has no separate foldable screenshot type",
                ))
            }
            _ => return Err(invalid("device must be phone, tablet or foldable")),
        };
        Ok(Self {
            locale: s(payload, "locale").trim(),
            device,
        })
    }
    pub fn matches_locale(&self, locale: &str) -> bool {
        self.locale.is_empty() || self.locale.eq_ignore_ascii_case(locale)
    }
    pub fn matches_device(&self, device: ImageDevice) -> bool {
        self.device.is_none_or(|selected| selected == device)
    }
    pub fn matches_google_type(&self, kind: &str) -> bool {
        match self.device {
            None => true,
            Some(ImageDevice::Phone) => kind == "phoneScreenshots",
            Some(ImageDevice::Tablet) => {
                matches!(kind, "sevenInchScreenshots" | "tenInchScreenshots")
            }
            Some(ImageDevice::Foldable) => false,
        }
    }
}

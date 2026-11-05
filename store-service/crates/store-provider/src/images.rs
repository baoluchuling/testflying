use super::*;
pub(super) struct Downloaded {
    pub bytes: Vec<u8>,
    pub name: String,
    pub mime: String,
}
pub(super) fn sources(slot: &Value) -> Result<Vec<Value>> {
    let fields = slot
        .as_object()
        .ok_or_else(|| invalid("Image slots must be objects"))?;
    if fields.is_empty()
        || fields
            .keys()
            .any(|key| !["urls", "assets"].contains(&key.as_str()))
    {
        return Err(invalid(
            "Image slots must contain urls and/or assets arrays without unknown fields",
        ));
    }
    for key in ["urls", "assets"] {
        if slot.get(key).is_some_and(|value| !value.is_array()) {
            return Err(invalid(format!("Image slot {key} must be an array")));
        }
    }
    let mut result = Vec::new();
    for asset in arr(slot, "assets") {
        let asset_fields = asset
            .as_object()
            .ok_or_else(|| invalid("Image assets must be objects"))?;
        if let Some(id) = asset.get("assetId") {
            if asset_fields.len() != 1
                || !id
                    .as_str()
                    .is_some_and(store_protocol::upload::valid_asset_id)
            {
                return Err(invalid(
                    "An uploaded image must contain only a valid assetId",
                ));
            }
            result.push(asset.clone());
            continue;
        }
        if asset_fields.keys().any(|key| {
            ![
                "fileName",
                "contentType",
                "sizeBytes",
                "storageKey",
                "downloadUrl",
            ]
            .contains(&key.as_str())
        }) {
            return Err(invalid("Image asset contains an unknown field"));
        }
        for key in ["fileName", "contentType", "storageKey", "downloadUrl"] {
            if asset.get(key).is_some_and(|value| !value.is_string()) {
                return Err(invalid(format!("Image asset {key} must be a string")));
            }
        }
        if asset
            .get("sizeBytes")
            .is_some_and(|value| value.as_u64().is_none())
        {
            return Err(invalid(
                "Image asset sizeBytes must be a nonnegative integer",
            ));
        }
        let source = if s(asset, "downloadUrl").is_empty() {
            s(asset, "storageKey")
        } else {
            s(asset, "downloadUrl")
        };
        validate_source_url(source)?;
        let name = s(asset, "fileName");
        if name.contains(['/', '\\', '\r', '\n']) || name.len() > 255 {
            return Err(invalid("Invalid image file name"));
        }
        result.push(asset.clone());
    }
    for url in arr(slot, "urls") {
        let url = url
            .as_str()
            .ok_or_else(|| invalid("Image URLs must be strings"))?;
        validate_source_url(url)?;
        result.push(json!({"downloadUrl":url}));
    }
    if result.len() > 10 {
        return Err(invalid("At most 10 images per slot are allowed"));
    }
    Ok(result)
}
pub(super) fn validate(images: &Value, platform: Platform) -> Result<()> {
    let slots = images
        .as_object()
        .ok_or_else(|| invalid("storeImages must be an object"))?;
    if slots.is_empty() {
        return Err(invalid(
            "storeImages must contain at least one explicit image slot",
        ));
    }
    for (slot, value) in slots {
        if !(matches!(slot.as_str(), "phone_screenshots" | "tablet_screenshots")
            || platform == Platform::Android && slot == "feature_graphic_url")
        {
            return Err(invalid("storeImages contains an unsupported image slot"));
        }
        sources(value)?;
    }
    Ok(())
}
fn validate_source_url(raw: &str) -> Result<()> {
    let url =
        Url::parse(raw).map_err(|_| invalid("Image source must be a nonempty HTTP(S) URL"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(invalid(
            "Image sources require HTTP(S) URLs without credentials",
        ));
    }
    Ok(())
}
impl Gateway {
    pub(super) async fn download_image(&self, source: &Value) -> Result<Downloaded> {
        if let Some(id) = source.get("assetId").and_then(Value::as_str) {
            let store = self
                .inner
                .assets
                .clone()
                .ok_or_else(|| invalid("Uploaded image is unavailable"))?;
            let id = id.to_owned();
            let (bytes, name, mime) = tokio::task::spawn_blocking(move || store.read(&id))
                .await
                .map_err(|_| invalid("Cannot read uploaded image"))??;
            return Ok(Downloaded { bytes, name, mime });
        }
        let url = if s(source, "downloadUrl").is_empty() {
            s(source, "storageKey")
        } else {
            s(source, "downloadUrl")
        };
        let url = Url::parse(url).map_err(|_| invalid("Image download URL is invalid"))?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(invalid(
                "Image downloads require HTTP(S) URLs without credentials",
            ));
        }
        let mut response = self
            .inner
            .client
            .get(url.clone())
            .send()
            .await
            .map_err(|_| upstream("Image download failed"))?;
        if !response.status().is_success() {
            return Err(upstream("Image download returned an error"));
        }
        const MAX: usize = 64 * 1024 * 1024;
        if response
            .content_length()
            .is_some_and(|size| size > MAX as u64)
        {
            return Err(invalid("Image exceeds 64 MiB"));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| upstream("Image download interrupted"))?
        {
            if bytes.len() + chunk.len() > MAX {
                return Err(invalid("Image exceeds 64 MiB"));
            }
            bytes.extend_from_slice(&chunk);
        }
        let (mime, extension) = validate_bytes(&bytes)?;
        let mut name = if s(source, "fileName").is_empty() {
            url.path_segments()
                .and_then(|mut p| p.next_back())
                .unwrap_or("store-image")
                .to_owned()
        } else {
            s(source, "fileName").to_owned()
        };
        if name.contains(['/', '\\', '\r', '\n']) || name.len() > 255 {
            return Err(invalid("Invalid image file name"));
        }
        if name.is_empty() {
            name = "store-image".into();
        }
        if !name.contains('.') {
            name.push('.');
            name.push_str(extension);
        }
        Ok(Downloaded {
            bytes,
            name,
            mime: mime.into(),
        })
    }
}

pub(super) fn validate_bytes(bytes: &[u8]) -> Result<(&'static str, &'static str)> {
    let format = image::guess_format(bytes).map_err(|_| invalid("Image must be PNG or JPEG"))?;
    let (mime, extension) = match format {
        image::ImageFormat::Png => ("image/png", "png"),
        image::ImageFormat::Jpeg => ("image/jpeg", "jpg"),
        _ => return Err(invalid("Image must be PNG or JPEG")),
    };
    let reader = image::ImageReader::with_format(std::io::Cursor::new(&bytes), format);
    let (width, height) = reader
        .into_dimensions()
        .map_err(|_| invalid("Image header is invalid"))?;
    if width == 0
        || height == 0
        || width > 16384
        || height > 16384
        || u64::from(width) * u64::from(height) > 100_000_000
    {
        return Err(invalid("Image dimensions are invalid or too large"));
    }
    let mut decoder = image::ImageReader::with_format(std::io::Cursor::new(&bytes), format);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(128 * 1024 * 1024);
    decoder.limits(limits);
    decoder
        .decode()
        .map_err(|_| invalid("Image data is invalid or exceeds the decoding memory limit"))?;
    Ok((mime, extension))
}

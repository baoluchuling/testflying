//! Commit every image upload before replacing screenshot placements; Apple processes asynchronously.
use super::*;
use apple::resource;
use image_query::ImageDevice;
use std::collections::{BTreeMap, BTreeSet, HashSet};

/// Only fixed vocabulary and bounded counts may leave Apple's processing response.
struct ProcessingSummary {
    state: &'static str,
    reasons: BTreeSet<&'static str>,
    details_present: bool,
    strings_scanned: usize,
    scan_truncated: bool,
    spec_id_present: bool,
    image_asset_present: bool,
}

impl ProcessingSummary {
    fn from_asset(asset: &Value) -> Self {
        let attributes = &asset["attributes"];
        let state = match attributes.get("state") {
            None | Some(Value::Null) => "MISSING",
            Some(Value::String(state)) => match state.as_str() {
                "AWAITING_UPLOAD" => "AWAITING_UPLOAD",
                "UPLOAD_COMPLETE" => "UPLOAD_COMPLETE",
                "PREPARE_FOR_SUBMISSION" => "PREPARE_FOR_SUBMISSION",
                "READY_FOR_REVIEW" => "READY_FOR_REVIEW",
                "WAITING_FOR_REVIEW" => "WAITING_FOR_REVIEW",
                "IN_REVIEW" => "IN_REVIEW",
                "ACCEPTED" => "ACCEPTED",
                "APPROVED" => "APPROVED",
                "FAILED" => "FAILED",
                "REJECTED" => "REJECTED",
                "ARCHIVED" => "ARCHIVED",
                _ => "UNKNOWN",
            },
            _ => "UNKNOWN",
        };
        let details = attributes.get("stateDetails").filter(|v| !v.is_null());
        let mut scan = ProcessingDetailScan::default();
        if let Some(details) = details {
            scan.visit(details, 0);
        }
        if scan.reasons.is_empty() {
            scan.reasons.insert("unclassified");
        }
        Self {
            state,
            reasons: scan.reasons,
            details_present: details.is_some(),
            strings_scanned: scan.strings,
            scan_truncated: scan.truncated,
            spec_id_present: attributes["specId"].as_str().is_some_and(|v| !v.is_empty()),
            image_asset_present: attributes.get("imageAsset").is_some_and(|v| !v.is_null()),
        }
    }

    fn details(&self) -> Value {
        json!({
            "state": self.state,
            "reasonClasses": self.reasons,
            "stateDetailsPresent": self.details_present,
            "stateDetailsStringsScanned": self.strings_scanned,
            "stateDetailsScanTruncated": self.scan_truncated,
            "specIdPresent": self.spec_id_present,
            "imageAssetPresent": self.image_asset_present,
        })
    }
}

#[derive(Default)]
struct ProcessingDetailScan {
    reasons: BTreeSet<&'static str>,
    nodes: usize,
    strings: usize,
    bytes: usize,
    truncated: bool,
}

impl ProcessingDetailScan {
    fn visit(&mut self, value: &Value, depth: usize) {
        // Bound traversal, allocation, and inspected bytes independently of response size.
        if self.nodes >= 128 || depth > 6 || self.bytes >= 8192 {
            self.truncated = true;
            return;
        }
        self.nodes += 1;
        match value {
            Value::String(value) => {
                self.strings += 1;
                let limit = 1024.min(8192 - self.bytes);
                let count = value.len().min(limit);
                self.bytes += count;
                self.truncated |= value.len() > count;
                let text: String = value
                    .bytes()
                    .take(count)
                    .map(|b| {
                        if b.is_ascii_alphanumeric() {
                            char::from(b.to_ascii_lowercase())
                        } else {
                            ' '
                        }
                    })
                    .collect();
                let text = text.split_ascii_whitespace().collect::<Vec<_>>().join(" ");
                for (reason, phrases) in [
                    (
                        "invalid_image_format",
                        &[
                            "invalid image format",
                            "unsupported image format",
                            "invalid file format",
                            "unsupported file format",
                            "invalid mime type",
                        ][..],
                    ),
                    (
                        "invalid_dimensions",
                        &[
                            "invalid dimensions",
                            "invalid image dimensions",
                            "incorrect dimensions",
                            "unsupported dimensions",
                            "invalid dimension",
                        ][..],
                    ),
                    (
                        "invalid_file_size",
                        &[
                            "invalid file size",
                            "file size exceeds",
                            "file is too large",
                            "file too large",
                            "image size exceeds",
                        ][..],
                    ),
                    (
                        "checksum_mismatch",
                        &[
                            "checksum mismatch",
                            "invalid checksum",
                            "checksum does not match",
                        ][..],
                    ),
                    (
                        "invalid_color_profile",
                        &[
                            "invalid color profile",
                            "unsupported color profile",
                            "invalid colour profile",
                        ][..],
                    ),
                    (
                        "alpha_not_allowed",
                        &[
                            "alpha not allowed",
                            "alpha channel not allowed",
                            "alpha channel is not allowed",
                            "transparency not allowed",
                            "transparency is not allowed",
                        ][..],
                    ),
                    (
                        "processing_failed",
                        &[
                            "processing failed",
                            "image processing failed",
                            "processing error",
                        ][..],
                    ),
                ] {
                    if phrases.iter().any(|phrase| text.contains(*phrase)) {
                        self.reasons.insert(reason);
                    }
                }
            }
            Value::Array(values) => {
                for value in values {
                    if self.nodes >= 128 || self.bytes >= 8192 {
                        self.truncated = true;
                        break;
                    }
                    self.visit(value, depth + 1);
                }
            }
            Value::Object(values) => {
                for value in values.values() {
                    if self.nodes >= 128 || self.bytes >= 8192 {
                        self.truncated = true;
                        break;
                    }
                    self.visit(value, depth + 1);
                }
            }
            _ => {}
        }
    }
}

impl Gateway {
    pub(super) async fn apple_sync_images(
        &self,
        app: &str,
        relationship: &str,
        owner: &str,
        owner_id: &str,
        images: &Value,
    ) -> Result<usize> {
        let feature = match owner {
            "appStoreVersionLocalizations" => "APP_STORE_VERSIONS",
            "appCustomProductPageLocalizations" => "CUSTOM_PRODUCT_PAGES",
            _ => return Err(invalid("Unsupported screenshot placement owner")),
        };
        let catalog = self.apple_image_catalog(feature).await?;
        let mut staged = Vec::new();
        let mut selected_groups = HashSet::new();
        for device in [
            ImageDevice::Phone,
            ImageDevice::Tablet,
            ImageDevice::Foldable,
        ] {
            let Some(slot) = images.get(device.slot()) else {
                continue;
            };
            let groups: Vec<_> = catalog
                .groups
                .iter()
                .filter(|(_, group)| group.device == device)
                .collect();
            let sources = images::sources(slot)?;
            if groups.is_empty() && !sources.is_empty() {
                return Err(invalid(
                    "Apple has no screenshot groups for the selected device",
                ));
            }
            selected_groups.extend(groups.iter().map(|(id, _)| (*id).clone()));
            for source in sources {
                let image = self.download_image(&source).await?;
                let decoded = image::load_from_memory(&image.bytes)
                    .map_err(|_| invalid("Invalid screenshot image"))?;
                let allowed_specs: HashSet<String> = catalog
                    .specs
                    .iter()
                    .filter(|spec| {
                        let dimensions = &spec["dimensions"];
                        let in_range = |value: u32, min: &str, max: &str| {
                            dimensions[min]
                                .as_u64()
                                .is_some_and(|min| u64::from(value) >= min)
                                && dimensions[max]
                                    .as_u64()
                                    .is_some_and(|max| u64::from(value) <= max)
                        };
                        in_range(decoded.width(), "minWidth", "maxWidth")
                            && in_range(decoded.height(), "minHeight", "maxHeight")
                            && (spec["alphaAllowed"] == true || !decoded.color().has_alpha())
                            && spec["maxFileSize"]
                                .as_u64()
                                .is_some_and(|max| image.bytes.len() as u64 <= max)
                            && arr(spec, "mimeTypes")
                                .iter()
                                .any(|mime| mime.as_str() == Some(image.mime.as_str()))
                            && arr(spec, "compatiblePlacementTypes")
                                .iter()
                                .any(|kind| kind == "APP_SCREENSHOT")
                            && arr(spec, "fileExtensions")
                                .iter()
                                .filter_map(Value::as_str)
                                .any(|extension| {
                                    image
                                        .name
                                        .to_ascii_lowercase()
                                        .ends_with(&extension.to_ascii_lowercase())
                                })
                            && groups
                                .iter()
                                .any(|(_, group)| group.specs.contains(s(spec, "specId")))
                    })
                    .map(|spec| s(spec, "specId").to_owned())
                    .collect();
                if allowed_specs.is_empty() {
                    return Err(invalid(
                        "Screenshot does not match an Apple specification for the selected device",
                    ));
                }
                staged.push((device, image, allowed_specs));
            }
        }
        // Keep existing placements until every new upload is committed and locally validated.
        let submitted_count = staged.len();
        let mut replacement: BTreeMap<String, Vec<String>> = selected_groups
            .iter()
            .map(|group| (group.clone(), Vec::new()))
            .collect();
        if !staged.is_empty() {
            let library = self
                .apple(
                    Method::GET,
                    &format!("/v1/apps/{}/assetLibrary", escape(app)),
                    None,
                )
                .await?;
            let library_id = id(&library["data"])?;
            for (device, image, allowed_specs) in staged {
                let asset = self.apple_upload_library_image(library_id, &image).await?;
                let spec_id = asset["attributes"]["specId"]
                    .as_str()
                    .filter(|id| !id.is_empty());
                if spec_id.is_some_and(|spec| !allowed_specs.contains(spec)) {
                    return Err(upstream(
                        "Apple accepted the screenshot with an unexpected specification",
                    ));
                }
                let image_id = id(&asset)?.to_owned();
                for (group_id, group) in &catalog.groups {
                    let compatible = match spec_id {
                        Some(spec) => group.specs.contains(spec),
                        None => group.specs.iter().any(|spec| allowed_specs.contains(spec)),
                    };
                    if group.device == device && compatible {
                        let ids = replacement
                            .get_mut(group_id)
                            .expect("selected device group");
                        ids.push(image_id.clone());
                        if ids.len() > group.max_count {
                            return Err(invalid(
                                "Screenshot count exceeds the Apple placement group limit",
                            ));
                        }
                    }
                }
            }
        }
        if replacement.is_empty() {
            return Ok(submitted_count);
        }
        let groups = replacement.keys().cloned().collect::<Vec<_>>().join(",");
        let (existing, _) = self
            .apple_image_collection(&query(
                &format!("/v1/{owner}/{}/placements", escape(owner_id)),
                &[
                    ("filter[placementType]", "APP_SCREENSHOT"),
                    ("filter[placementGroup]", &groups),
                ],
            ))
            .await?;
        for placement in existing {
            if s(&placement["attributes"], "placementType") == "APP_SCREENSHOT"
                && replacement.contains_key(s(&placement["attributes"], "placementGroup"))
            {
                self.apple(
                    Method::DELETE,
                    &format!("/v1/appAssetLibraryPlacements/{}", escape(id(&placement)?)),
                    None,
                )
                .await?;
            }
        }
        for (group, images) in replacement {
            let mut ordered = Vec::new();
            for image_id in images {
                let placed=self.apple(Method::POST,"/v1/appAssetLibraryPlacements",Some(resource(
                    "appAssetLibraryPlacements",None,json!({"placementType":"APP_SCREENSHOT","placementGroup":group}),
                    json!({"image":{"data":{"type":"appAssetLibraryImages","id":image_id}},relationship:{"data":{"type":owner,"id":owner_id}}}),
                ))).await?;
                ordered.push(json!({"type":"appAssetLibraryPlacements","id":id(&placed["data"])?}));
            }
            if !ordered.is_empty() {
                self.apple(Method::POST,"/v1/appAssetLibraryPlacementOrderingRequests",Some(resource(
                    "appAssetLibraryPlacementOrderingRequests",None,json!({"placementGroup":group}),
                    json!({"orderedPlacements":{"data":ordered},relationship:{"data":{"type":owner,"id":owner_id}}}),
                ))).await?;
            }
        }
        Ok(submitted_count)
    }

    async fn apple_upload_library_image(
        &self,
        library: &str,
        image: &images::Downloaded,
    ) -> Result<Value> {
        let reserved=self.apple(Method::POST,"/v1/appAssetLibraryImages",Some(resource(
            "appAssetLibraryImages",None,
            json!({"fileName":image.name,"fileSize":image.bytes.len(),"category":"APP_SCREENSHOTS_AND_PREVIEWS"}),
            json!({"assetLibrary":{"data":{"type":"appAssetLibraries","id":library}}}),
        ))).await?;
        let image_id = id(&reserved["data"])?;
        self.apple_log_image_submission(&reserved["data"], "reserved");
        let operations = arr(&reserved["data"]["attributes"], "uploadOperations");
        if operations.is_empty() || operations.len() > 1000 {
            return Err(upstream(
                "Apple image reservation has invalid upload operations",
            ));
        }
        let mut uploads = Vec::new();
        for operation in operations {
            let start = operation["offset"]
                .as_u64()
                .and_then(|offset| usize::try_from(offset).ok())
                .ok_or_else(|| upstream("Invalid Apple upload offset"))?;
            let length = operation["length"]
                .as_u64()
                .and_then(|length| usize::try_from(length).ok())
                .filter(|length| *length > 0)
                .ok_or_else(|| upstream("Invalid Apple upload length"))?;
            let end = start
                .checked_add(length)
                .filter(|end| *end <= image.bytes.len())
                .ok_or_else(|| upstream("Invalid Apple upload range"))?;
            let url = Url::parse(required(operation, "url")?)
                .map_err(|_| upstream("Invalid Apple upload URL"))?;
            if !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return Err(upstream("Invalid Apple upload URL"));
            }
            let method = Method::from_bytes(required(operation, "method")?.as_bytes())
                .map_err(|_| upstream("Invalid Apple upload method"))?;
            if method != Method::PUT && method != Method::POST {
                return Err(upstream("Invalid Apple upload method"));
            }
            uploads.push((
                start,
                end,
                method,
                url,
                arr(operation, "requestHeaders").to_vec(),
            ));
        }
        let mut ranges = uploads
            .iter()
            .map(|(start, end, _, _, _)| (*start, *end))
            .collect::<Vec<_>>();
        ranges.sort_unstable();
        let mut covered = 0;
        for (start, end) in ranges {
            if start != covered {
                return Err(upstream(
                    "Apple image upload ranges do not exactly cover the file",
                ));
            }
            covered = end;
        }
        if covered != image.bytes.len() {
            return Err(upstream("Apple image upload ranges omit file bytes"));
        }
        for (start, end, method, url, headers) in uploads {
            let mut request = self
                .inner
                .client
                .request(method, url)
                .body(image.bytes[start..end].to_vec());
            for header in headers {
                request = request.header(s(&header, "name"), s(&header, "value"));
            }
            let response = self
                .send_logged(request, Some(Platform::Ios), "apple_image_chunk")
                .await?;
            if !response.status().is_success() {
                return Err(upstream("Apple image chunk upload failed"));
            }
        }
        let committed = self
            .apple(
                Method::PATCH,
                &format!("/v1/appAssetLibraryImages/{}", escape(image_id)),
                Some(resource(
                    "appAssetLibraryImages",
                    Some(image_id),
                    json!({"uploaded":true}),
                    json!({}),
                )),
            )
            .await?;
        let asset = &committed["data"];
        if id(asset)? != image_id {
            return Err(upstream("Apple returned an unexpected image identity"));
        }
        self.apple_log_image_submission(asset, "committed");
        // Placement creation supports assets still processing. A successful commit is
        // sufficient to continue; placement and ordering responses remain authoritative.
        Ok(asset.clone())
    }

    fn apple_log_image_submission(&self, asset: &Value, phase: &'static str) {
        if let Some(log) = &self.inner.log {
            let mut details = ProcessingSummary::from_asset(asset).details();
            details["phase"] = json!(phase);
            details["submissionAccepted"] = json!(phase == "committed");
            log.context("apple.image_processing", details);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn processing_summaries_classify_reasons_without_copying_vendor_content() {
        let cases = [
            ("INVALID_IMAGE_FORMAT", "invalid_image_format"),
            ("Invalid image dimensions", "invalid_dimensions"),
            ("File size exceeds the limit", "invalid_file_size"),
            ("CHECKSUM_MISMATCH", "checksum_mismatch"),
            ("Unsupported color profile", "invalid_color_profile"),
            ("Alpha channel is not allowed", "alpha_not_allowed"),
            ("Image processing failed", "processing_failed"),
            ("private-unrecognized-error", "unclassified"),
        ];
        for (message, expected) in cases {
            let summary = ProcessingSummary::from_asset(
                &json!({"id":"private-asset-id","attributes":{
                    "state":"FAILED","specId":"private-spec-id","imageAsset":{"url":"https://private.invalid/image.png?token=private-token"},
                    "stateDetails":{"errors":[{"code":"private-code","message":format!("{message}; private-file.png; https://private.invalid/?token=private-token")}]}
                }}),
            );
            let details = summary.details();
            assert_eq!(details["state"], "FAILED");
            assert_eq!(details["reasonClasses"], json!([expected]));
            assert_eq!(details["stateDetailsPresent"], true);
            assert_eq!(details["specIdPresent"], true);
            assert_eq!(details["imageAssetPresent"], true);
            assert!(!details.to_string().contains("private"));
        }
    }

    #[test]
    fn processing_summaries_keep_unknown_missing_and_terminal_states_distinct() {
        for (attributes, expected) in [
            (json!({"state":"private-state-token"}), "UNKNOWN"),
            (json!({"state":42}), "UNKNOWN"),
            (json!({"state":null}), "MISSING"),
            (json!({}), "MISSING"),
        ] {
            let summary = ProcessingSummary::from_asset(&json!({"attributes":attributes}));
            assert_eq!(summary.state, expected);
            assert!(!summary.details().to_string().contains("private"));
            assert_eq!(summary.details()["stateDetailsPresent"], false);
            assert_eq!(summary.details()["specIdPresent"], false);
            assert_eq!(summary.details()["imageAssetPresent"], false);
        }
        for state in ["FAILED", "REJECTED", "ARCHIVED"] {
            let summary = ProcessingSummary::from_asset(&json!({"attributes":{"state":state}}));
            assert_eq!(summary.state, state);
        }
    }

    #[test]
    fn processing_detail_scanning_is_bounded_by_depth_nodes_and_bytes() {
        let mut nested = json!("invalid dimensions");
        for _ in 0..8 {
            nested = json!([nested]);
        }
        for details in [
            nested,
            json!(vec![json!(null); 200]),
            json!("private".repeat(2000)),
            json!(vec![json!("a".repeat(1024)); 16]),
        ] {
            let mut scan = ProcessingDetailScan::default();
            scan.visit(&details, 0);
            assert!(scan.truncated);
            assert!(scan.nodes <= 128);
            assert!(scan.strings <= 128);
            assert!(scan.bytes <= 8192);
        }
    }
}

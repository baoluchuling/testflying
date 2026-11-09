//! Stage every image before replacing any screenshot placement.
use super::*;
use apple::resource;
use image_query::ImageDevice;
use std::collections::{BTreeMap, HashSet};

impl Gateway {
    pub(super) async fn apple_sync_images(
        &self,
        app: &str,
        relationship: &str,
        owner: &str,
        owner_id: &str,
        images: &Value,
    ) -> Result<()> {
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
        // Keep existing placements until every new asset is ready and validated.
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
                let spec_id = required(&asset["attributes"], "specId")?;
                if !allowed_specs.contains(spec_id) {
                    return Err(upstream(
                        "Apple processed the screenshot with an unexpected specification",
                    ));
                }
                let image_id = id(&asset)?.to_owned();
                for (group_id, group) in &catalog.groups {
                    if group.device == device && group.specs.contains(spec_id) {
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
            return Ok(());
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
        Ok(())
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
            let response = request
                .send()
                .await
                .map_err(|_| upstream("Apple image chunk upload failed"))?;
            if !response.status().is_success() {
                return Err(upstream("Apple image chunk upload failed"));
            }
        }
        self.apple(
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
        tokio::time::timeout(Duration::from_secs(30),async {
            loop {
                let response=self.apple(Method::GET,&format!("/v1/appAssetLibraryImages/{}",escape(image_id)),None).await?;
                let asset=&response["data"];
                if id(asset)?!=image_id { return Err(upstream("Apple returned an unexpected image identity")); }
                match s(&asset["attributes"],"state") {
                    "PREPARE_FOR_SUBMISSION"|"READY_FOR_REVIEW"|"WAITING_FOR_REVIEW"|"IN_REVIEW"|"ACCEPTED"|"APPROVED" => return Ok(asset.clone()),
                    "AWAITING_UPLOAD"|"UPLOAD_COMPLETE" => tokio::time::sleep(Duration::from_millis(500)).await,
                    _ => return Err(upstream("Apple image processing failed or returned an unsupported state")),
                }
            }
        }).await.map_err(|_|StoreError::new(504,"store_asset_processing_timeout","Apple image processing did not finish within 30 seconds; existing placements were preserved"))?
    }
}

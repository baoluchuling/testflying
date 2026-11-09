//! Read localized screenshots using the App Asset Library, including iPhone Duo.
use super::*;
use image_query::{ImageDevice, ImageQuery};
use std::collections::{BTreeMap, HashSet};

pub(super) struct AppleImageGroup {
    pub device: ImageDevice,
    pub display_class: String,
    pub specs: HashSet<String>,
    pub max_count: usize,
}
pub(super) struct AppleImageCatalog {
    pub groups: BTreeMap<String, AppleImageGroup>,
    pub specs: Vec<Value>,
}

impl Gateway {
    pub(super) async fn apple_images(
        &self,
        version: Option<&Value>,
        payload: &Value,
    ) -> Result<Value> {
        let Some(version) = version else {
            return Ok(json!({"locales":[]}));
        };
        let filters = ImageQuery::new(payload, Platform::Ios)?;
        let (localizations, _) = self
            .apple_image_collection(&format!(
                "/v1/appStoreVersions/{}/appStoreVersionLocalizations?limit=200",
                escape(id(version)?)
            ))
            .await?;
        let localizations: Vec<_> = localizations
            .into_iter()
            .filter(|loc| filters.matches_locale(s(&loc["attributes"], "locale")))
            .collect();
        if localizations.is_empty() {
            return Ok(json!({"locales":[]}));
        }
        let catalog = self.apple_image_catalog("APP_STORE_VERSIONS").await?;
        let groups: BTreeMap<_, _> = catalog
            .groups
            .iter()
            .filter(|(_, group)| filters.matches_device(group.device))
            .map(|(id, group)| (id.clone(), group))
            .collect();
        let selected = groups
            .keys()
            .map(|group| group.as_str())
            .collect::<Vec<_>>()
            .join(",");
        let mut rows = Vec::new();
        for loc in localizations {
            let mut images = serde_json::Map::new();
            for device in [
                ImageDevice::Phone,
                ImageDevice::Tablet,
                ImageDevice::Foldable,
            ] {
                if filters.matches_device(device) {
                    images.insert(device.slot().into(), json!([]));
                }
            }
            if !selected.is_empty() {
                let (placements, included) = self
                    .apple_image_collection(&query(
                        &format!(
                            "/v1/appStoreVersionLocalizations/{}/placements",
                            escape(id(&loc)?)
                        ),
                        &[
                            ("filter[placementType]", "APP_SCREENSHOT"),
                            ("filter[placementGroup]", &selected),
                            ("include", "image"),
                            ("sort", "placementGroupPosition"),
                        ],
                    ))
                    .await?;
                for placement in placements {
                    let attributes = &placement["attributes"];
                    if s(attributes, "placementType") != "APP_SCREENSHOT" {
                        continue;
                    }
                    let group_id = s(attributes, "placementGroup");
                    let Some(group) = groups.get(group_id) else {
                        continue;
                    };
                    let image_id = id(&placement["relationships"]["image"]["data"])?;
                    let image = included
                        .iter()
                        .find(|image| {
                            s(image, "type") == "appAssetLibraryImages"
                                && s(image, "id") == image_id
                        })
                        .ok_or_else(|| upstream("Apple screenshot placement omitted its image"))?;
                    let attributes = &image["attributes"];
                    let asset = &attributes["imageAsset"];
                    images[group.device.slot()].as_array_mut().expect("selected image slot").push(json!({
                        "id":image_id,"placementId":placement["id"],"placementGroup":group_id,"displayClass":group.display_class,
                        "fileName":attributes["fileName"],"url":asset["templateUrl"],"width":asset["width"],"height":asset["height"]
                    }));
                }
            }
            rows.push(json!({"locale":loc["attributes"]["locale"],"images":images}));
        }
        Ok(json!({"locales":rows}))
    }

    pub(super) async fn apple_image_catalog(&self, feature: &str) -> Result<AppleImageCatalog> {
        let (references, _) = self
            .apple_image_collection(&query(
                "/v1/appAssetLibraryRefData",
                &[
                    ("filter[features]", feature),
                    ("filter[placementTypes]", "APP_SCREENSHOT"),
                ],
            ))
            .await?;
        // Group identifiers are runtime reference data, not a fixed enum.
        let mut groups = BTreeMap::new();
        let mut specs = Vec::new();
        for reference in &references {
            let attributes = &reference["attributes"];
            specs.extend(arr(attributes, "imageSpecs").iter().cloned());
            let limits: HashMap<_, _> = arr(attributes, "features")
                .iter()
                .filter(|row| s(row, "featureId") == feature)
                .flat_map(|row| arr(row, "placementPolicies"))
                .filter(|row| s(row, "placementType") == "APP_SCREENSHOT")
                .flat_map(|row| arr(row, "groupLimits"))
                .flat_map(|row| {
                    arr(row, "groupIds").iter().filter_map(move |group| {
                        Some((group.as_str()?, row["maxCount"].as_u64()? as usize))
                    })
                })
                .collect();
            let screenshot_groups: HashMap<_, _> = arr(attributes, "placementTypes")
                .iter()
                .filter(|kind| s(kind, "placementTypeId") == "APP_SCREENSHOT")
                .flat_map(|kind| arr(kind, "specMappings"))
                .map(|mapping| {
                    (
                        s(mapping, "placementGroupId"),
                        arr(mapping, "specs")
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect::<HashSet<_>>(),
                    )
                })
                .collect();
            for group in arr(attributes, "placementProfileGroups") {
                let group_id = s(group, "placementProfileGroupId");
                let Some(group_specs) = screenshot_groups.get(group_id) else {
                    continue;
                };
                let Some(&max_count) = limits.get(group_id) else {
                    continue;
                };
                let class_id = s(group, "displayClassId");
                let class = arr(attributes, "displayClasses")
                    .iter()
                    .find(|class| s(class, "displayClassId") == class_id)
                    .ok_or_else(|| upstream("Apple screenshot group has no display class"))?;
                let device = if class_id == "IPHONE_DUO" {
                    ImageDevice::Foldable
                } else {
                    match s(class, "deviceFamily") {
                        "IPHONE" => ImageDevice::Phone,
                        "IPAD" => ImageDevice::Tablet,
                        _ => continue,
                    }
                };
                groups.insert(
                    group_id.to_owned(),
                    AppleImageGroup {
                        device,
                        display_class: class_id.to_owned(),
                        specs: group_specs.clone(),
                        max_count,
                    },
                );
            }
        }
        if groups.is_empty() {
            return Err(upstream(
                "Apple screenshot reference data has no supported device groups for this feature",
            ));
        }
        Ok(AppleImageCatalog { groups, specs })
    }

    pub(super) async fn apple_image_collection(
        &self,
        path: &str,
    ) -> Result<(Vec<Value>, Vec<Value>)> {
        let base = Url::parse(&self.inner.config.apple_base_url)
            .map_err(|_| invalid("Invalid Apple endpoint"))?;
        let first = base
            .join(path)
            .map_err(|_| upstream("Invalid Apple screenshot collection URL"))?;
        let mut current = first.clone();
        let mut seen = HashSet::new();
        let (mut data, mut included) = (Vec::new(), Vec::new());
        for _ in 0..20 {
            if !seen.insert(current.as_str().to_owned()) {
                return Err(upstream("Apple screenshot pagination repeated a page"));
            }
            let path = match current.query() {
                Some(query) => format!("{}?{query}", current.path()),
                None => current.path().to_owned(),
            };
            let response = self.apple(Method::GET, &path, None).await?;
            data.extend(
                response["data"]
                    .as_array()
                    .ok_or_else(|| upstream("Apple screenshot collection omitted data"))?
                    .iter()
                    .cloned(),
            );
            included.extend(arr(&response, "included").iter().cloned());
            if data.len() + included.len() > 10_000 {
                return Err(upstream(
                    "Apple screenshot collection exceeds the object limit",
                ));
            }
            let next = s(&response["links"], "next");
            if next.is_empty() {
                return Ok((data, included));
            }
            current = current
                .join(next)
                .map_err(|_| upstream("Invalid Apple screenshot pagination URL"))?;
            if current.origin() != base.origin()
                || current.path() != first.path()
                || !current.username().is_empty()
                || current.password().is_some()
                || current.fragment().is_some()
            {
                return Err(upstream("Apple screenshot pagination left its collection"));
            }
        }
        Err(upstream(
            "Apple screenshot pagination exceeds the page limit",
        ))
    }
}

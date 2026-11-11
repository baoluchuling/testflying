use super::*;
use std::collections::BTreeMap;

impl Gateway {
    pub(super) async fn apple(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        self.api(Platform::Ios, method, path, body).await
    }
    pub(super) async fn apple_execute(&self, r: &StoreRequest) -> Result<Value> {
        // Identity is resolved exclusively from the trusted envelope Bundle ID.
        let response = self
            .apple(
                Method::GET,
                &query(
                    "/v1/apps",
                    &[("filter[bundleId]", &r.bundle_identifier), ("limit", "2")],
                ),
                None,
            )
            .await?;
        let apps = arr(&response, "data");
        if apps.len() != 1 {
            return Err(StoreError::new(
                404,
                "store_app_not_found",
                "Bundle ID did not resolve to one Apple app",
            ));
        }
        let app = id(&apps[0])?;
        if let Some(bundle) = apps[0]["attributes"]
            .get("bundleId")
            .and_then(Value::as_str)
        {
            if bundle != r.bundle_identifier {
                return Err(upstream("Apple returned an unexpected app identity"));
            }
        }
        match r.operation {
            Operation::ProductPageOptimizations => self.apple_ppo(app, None).await,
            Operation::CreateProductPageOptimization => self.apple_ppo(app, Some(&r.payload)).await,
            Operation::StoreReviews => self.apple_reviews(app, &r.payload).await,
            Operation::StoreReleases => {
                let versions = self.apple_versions(app).await?;
                let body = json!({"releases":versions.iter().enumerate().map(|(i,v)|apple_release(v,i==0)).collect::<Vec<_>>()});
                if body.to_string().len() > store_protocol::crypto::MAX_JSON_BYTES - 16384 {
                    return Err(upstream("Apple version list exceeded reply limit"));
                }
                Ok(body)
            }
            Operation::CreateStoreRelease => {
                let mut attributes = json!({"platform":"IOS","versionString":required(&r.payload,"versionString")?,"releaseType":"MANUAL"});
                if let Some(copyright) = r.payload.get("copyright") {
                    attributes["copyright"] = copyright.clone();
                }
                let created=self.apple(Method::POST,"/v1/appStoreVersions",Some(json!({"data":{"type":"appStoreVersions","attributes":attributes,"relationships":{"app":{"data":{"type":"apps","id":app}}}}}))).await?;
                id(&created["data"])?;
                Ok(json!({"release":apple_release(&created["data"],true)}))
            }
            Operation::Sync if s(&r.payload, "operation") == "update_marketing_page" => {
                self.apple_marketing(app, &r.payload).await
            }
            _ => {
                let version = self.apple_version(app).await?;
                if !matches!(r.operation, Operation::Preflight | Operation::Sync)
                    && version
                        .as_ref()
                        .is_some_and(|v| !editable(apple_version_state(v)))
                {
                    return Err(StoreError::new(
                        409,
                        "store_version_not_editable",
                        "Latest store version is not editable",
                    ));
                }
                match r.operation {
                    Operation::Preflight => {
                        if s(&r.payload, "operation") == "update_marketing_page" {
                            return Ok(preflight(true, None, "marketing_page_editable", true));
                        }
                        Ok(match version {
                            Some(ref v) => {
                                let state = apple_version_state(v);
                                let editable = editable(state);
                                preflight(
                                    editable,
                                    if editable {
                                        None
                                    } else {
                                        Some("store_version_not_editable")
                                    },
                                    state,
                                    true,
                                )
                            }
                            None => {
                                preflight(false, Some("store_version_missing"), "missing", false)
                            }
                        })
                    }
                    Operation::SupportedLocales => {
                        let localizations = if let Some(v) = version {
                            self.apple_localizations(
                                "appStoreVersions",
                                id(&v)?,
                                "appStoreVersionLocalizations",
                            )
                            .await?
                        } else {
                            self.apple_info(app).await?.1
                        };
                        let mut locales = localizations
                            .iter()
                            .map(|v| s(&v["attributes"], "locale").to_owned())
                            .filter(|s| !s.is_empty())
                            .collect::<Vec<_>>();
                        locales.sort();
                        locales.dedup();
                        Ok(json!({"locales":locales}))
                    }
                    Operation::StoreListings => {
                        let mut listings = BTreeMap::<String, Value>::new();
                        for loc in self.apple_info(app).await?.1 {
                            let a = &loc["attributes"];
                            let locale = s(a, "locale");
                            listings.insert(locale.into(),json!({"locale":locale,"title":s(a,"name"),"subtitle":s(a,"subtitle")}));
                        }
                        if let Some(v) = version {
                            for loc in self
                                .apple_localizations(
                                    "appStoreVersions",
                                    id(&v)?,
                                    "appStoreVersionLocalizations",
                                )
                                .await?
                            {
                                let a = &loc["attributes"];
                                let locale = s(a, "locale");
                                let row = listings
                                    .entry(locale.into())
                                    .or_insert_with(|| json!({"locale":locale}));
                                for (target, source) in [
                                    ("description", "description"),
                                    ("fullDescription", "description"),
                                    ("keywords", "keywords"),
                                    ("promotionalText", "promotionalText"),
                                    ("releaseNotes", "whatsNew"),
                                ] {
                                    row[target] = json!(s(a, source));
                                }
                            }
                        }
                        Ok(json!({"listings":listings.into_values().collect::<Vec<_>>()}))
                    }
                    Operation::StoreImages => self.apple_images(version.as_ref(), &r.payload).await,
                    Operation::Sync => {
                        self.apple_sync(
                            app,
                            s(&apps[0]["attributes"], "primaryLocale"),
                            &r.payload,
                            version,
                        )
                        .await
                    }
                    _ => Err(invalid("Unsupported Apple operation")),
                }
            }
        }
    }
    async fn apple_versions(&self, app: &str) -> Result<Vec<Value>> {
        let path = format!("/v1/apps/{}/appStoreVersions", escape(app));
        let mut next = query(
            &path,
            &[
                ("limit", "200"),
                ("filter[platform]", "IOS"),
                (
                    "fields[appStoreVersions]",
                    "versionString,createdDate,appStoreState,appVersionState,platform",
                ),
            ],
        );
        let expected = Url::parse(&format!(
            "{}{}",
            self.inner.config.apple_base_url.trim_end_matches('/'),
            path
        ))
        .map_err(|_| upstream("Invalid Apple version endpoint"))?;
        let mut seen = std::collections::HashSet::new();
        let mut versions = Vec::new();
        loop {
            if seen.len() >= 100 || !seen.insert(next.clone()) {
                return Err(upstream("Apple version pagination did not finish"));
            }
            let response = self.apple(Method::GET, &next, None).await?;
            for item in arr(&response, "data") {
                if !s(&item["attributes"], "platform").is_empty()
                    && s(&item["attributes"], "platform") != "IOS"
                {
                    continue;
                }
                versions.push(item.clone());
            }
            let link = s(&response["links"], "next");
            if link.is_empty() {
                break;
            }
            if link.len() > 8192 {
                return Err(upstream("Apple version pagination link exceeded limit"));
            }
            let parsed = expected
                .join(link)
                .map_err(|_| upstream("Invalid Apple version pagination"))?;
            if parsed.origin() != expected.origin()
                || parsed.path() != expected.path()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
                || parsed.fragment().is_some()
            {
                return Err(upstream("Invalid Apple version pagination"));
            }
            next = format!(
                "{path}{}",
                parsed.query().map(|q| format!("?{q}")).unwrap_or_default()
            );
        }
        versions.sort_by_key(|v| std::cmp::Reverse(version_order(v)));
        Ok(versions)
    }
    async fn apple_version(&self, app: &str) -> Result<Option<Value>> {
        let version = self.apple_versions(app).await?.into_iter().next();
        if let Some(log) = &self.inner.log {
            let number = version
                .as_ref()
                .map(|v| s(&v["attributes"], "versionString"))
                .filter(|s| {
                    s.len() <= 80
                        && s.bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                });
            log.context(
                "apple.latest_version",
                json!({"versionString":number,"exists":version.is_some(),
                "editable":version.as_ref().is_some_and(|v|editable(apple_version_state(v)))}),
            );
        }
        Ok(version)
    }
    async fn apple_localizations(
        &self,
        owner: &str,
        owner_id: &str,
        kind: &str,
    ) -> Result<Vec<Value>> {
        let response = self
            .apple(
                Method::GET,
                &format!("/v1/{owner}/{}/{kind}?limit=200", escape(owner_id)),
                None,
            )
            .await?;
        Ok(arr(&response, "data").to_vec())
    }
    async fn apple_info(&self, app: &str) -> Result<(String, Vec<Value>)> {
        let response = self
            .apple(
                Method::GET,
                &format!("/v1/apps/{}/appInfos?limit=1", escape(app)),
                None,
            )
            .await?;
        let Some(info) = arr(&response, "data").first() else {
            return Ok((String::new(), vec![]));
        };
        let info = id(info)?.to_owned();
        let locales = self
            .apple_localizations("appInfos", &info, "appInfoLocalizations")
            .await?;
        Ok((info, locales))
    }
    async fn apple_ensure_locale(
        &self,
        owner_rel: &str,
        owner_type: &str,
        owner: &str,
        kind: &str,
        locale: &str,
        creation_attributes: Value,
    ) -> Result<String> {
        if let Some(loc) = self
            .apple_localizations(owner_type, owner, kind)
            .await?
            .iter()
            .find(|v| s(&v["attributes"], "locale") == locale)
        {
            return Ok(id(loc)?.into());
        }
        let mut attributes = creation_attributes.as_object().cloned().unwrap_or_default();
        attributes.insert("locale".into(), json!(locale));
        if kind == "appInfoLocalizations"
            && attributes
                .get("name")
                .and_then(Value::as_str)
                .is_none_or(|name| name.trim().is_empty())
        {
            return Err(invalid(
                "Creating an app-info localization requires a selected, nonempty title",
            ));
        }
        let response = self
            .apple(
                Method::POST,
                &format!("/v1/{kind}"),
                Some(resource(
                    kind,
                    None,
                    json!(attributes),
                    json!({owner_rel:{"data":{"type":owner_type,"id":owner}}}),
                )),
            )
            .await?;
        Ok(id(&response["data"])?.into())
    }
    async fn apple_patch(&self, kind: &str, resource_id: &str, attributes: Value) -> Result<()> {
        if attributes.as_object().is_some_and(|v| !v.is_empty()) {
            self.apple(
                Method::PATCH,
                &format!("/v1/{kind}/{}", escape(resource_id)),
                Some(resource(kind, Some(resource_id), attributes, json!({}))),
            )
            .await?;
        }
        Ok(())
    }
    async fn apple_sync(
        &self,
        app: &str,
        primary_locale: &str,
        p: &Value,
        version: Option<Value>,
    ) -> Result<Value> {
        let operation = required(p, "operation")?;
        if !matches!(operation, "update_app_metadata" | "update_release_notes") {
            return Err(invalid("Unsupported sync operation"));
        }
        let locale = required(p, "locale")?;
        let Some(version) = version else {
            return Ok(failed(
                "store_version_missing",
                "Target store version does not exist",
            ));
        };
        if !editable(apple_version_state(&version)) {
            return Ok(failed(
                "store_version_not_editable",
                "Target store version is not editable",
            ));
        }
        let version_id = id(&version)?;
        let mut attrs = serde_json::Map::new();
        let mut info_attrs = serde_json::Map::new();
        if operation == "update_release_notes" {
            required(p, "releaseNotes")?;
            attrs.insert("whatsNew".into(), p["releaseNotes"].clone());
        } else {
            let metadata = p
                .get("metadata")
                .filter(|m| m.is_object())
                .ok_or_else(|| invalid("metadata is required"))?;
            for (scope, target, keys) in [
                (
                    "description",
                    "description",
                    &["description", "fullDescription"][..],
                ),
                ("keywords", "keywords", &["keywords"][..]),
                (
                    "promotional_text",
                    "promotionalText",
                    &["promotionalText"][..],
                ),
            ] {
                if scopes(p, scope, true) {
                    copy_field(&mut attrs, target, metadata, keys)?;
                }
            }
            if scopes(p, "title", true) {
                copy_field(&mut info_attrs, "name", metadata, &["title"])?;
            }
            if scopes(p, "subtitle", true) {
                copy_field(&mut info_attrs, "subtitle", metadata, &["subtitle"])?;
            }
        }
        let has_images =
            scopes(p, "store_images", false) && p["metadata"]["storeImages"].is_object();
        if attrs.is_empty() && info_attrs.is_empty() && !has_images {
            return Ok(failed(
                "empty_sync_payload",
                "No selected store content was supplied",
            ));
        }
        let version_locales = self
            .apple_localizations(
                "appStoreVersions",
                version_id,
                "appStoreVersionLocalizations",
            )
            .await?;
        let new_version_locale = !version_locales
            .iter()
            .any(|v| s(&v["attributes"], "locale") == locale);
        // A new version locale must have matching app-info metadata. Resolve and
        // validate every required name before performing any localization writes.
        let info_plan = if new_version_locale || !info_attrs.is_empty() {
            let (info, locales) = self.apple_info(app).await?;
            if info.is_empty() {
                return Err(upstream("Apple app info is unavailable"));
            }
            let existing = locales
                .iter()
                .any(|v| s(&v["attributes"], "locale") == locale);
            let mut creation = info_attrs.clone();
            if !existing {
                if let Some(name) = creation.get("name") {
                    if name.as_str().is_none_or(|name| name.trim().is_empty()) {
                        return Err(invalid(
                            "Creating an app-info localization requires a nonempty title",
                        ));
                    }
                } else {
                    let fallback = locales.iter()
                        .find(|v| !primary_locale.is_empty() && s(&v["attributes"], "locale") == primary_locale)
                        .map(|v| s(&v["attributes"], "name"))
                        .filter(|name| !name.trim().is_empty())
                        .ok_or_else(|| invalid("New store locales require a selected title or the existing primary-locale app name"))?;
                    creation.insert("name".into(), json!(fallback));
                }
            }
            Some((info, creation))
        } else {
            None
        };
        let info_localization = if let Some((info, creation)) = info_plan {
            Some(
                self.apple_ensure_locale(
                    "appInfo",
                    "appInfos",
                    &info,
                    "appInfoLocalizations",
                    locale,
                    json!(creation),
                )
                .await?,
            )
        } else {
            None
        };
        let localization = self
            .apple_ensure_locale(
                "appStoreVersion",
                "appStoreVersions",
                version_id,
                "appStoreVersionLocalizations",
                locale,
                json!({}),
            )
            .await?;
        self.apple_patch("appStoreVersionLocalizations", &localization, json!(attrs))
            .await?;
        if let Some(loc) = info_localization {
            self.apple_patch("appInfoLocalizations", &loc, json!(info_attrs))
                .await?;
        }
        if has_images {
            self.apple_sync_images(
                app,
                "appStoreVersionLocalization",
                "appStoreVersionLocalizations",
                &localization,
                &p["metadata"]["storeImages"],
            )
            .await?;
        }
        Ok(succeeded(json!({})))
    }
    async fn apple_reviews(&self, app: &str, p: &Value) -> Result<Value> {
        let size = p["pageSize"]
            .as_u64()
            .filter(|n| *n > 0)
            .unwrap_or(50)
            .min(200)
            .to_string();
        let sort = if s(p, "sort").is_empty() {
            "-createdDate"
        } else {
            s(p, "sort")
        };
        let mut pairs = vec![("limit", size.as_str()), ("sort", sort)];
        if !s(p, "pageToken").is_empty() {
            pairs.push(("cursor", s(p, "pageToken")));
        }
        let response = self
            .apple(
                Method::GET,
                &query(&format!("/v1/apps/{}/customerReviews", escape(app)), &pairs),
                None,
            )
            .await?;
        let reviews=arr(&response,"data").iter().map(|v|{let a=&v["attributes"];json!({"id":s(v,"id"),"platform":"ios","rating":a["rating"].as_i64().unwrap_or(0),"title":s(a,"title"),"body":s(a,"body"),
            "reviewerNickname":s(a,"reviewerNickname"),"authorName":s(a,"reviewerNickname"),"locale":s(a,"locale"),"territory":s(a,"territory"),
            "appVersion":if s(a,"appVersionString").is_empty(){s(a,"appVersion")}else{s(a,"appVersionString")},"createdAt":s(a,"createdDate"),"updatedAt":s(a,"createdDate"),"raw":a})}).collect::<Vec<_>>();
        let next = s(&response["links"], "next");
        let token = Url::parse(next)
            .ok()
            .and_then(|u| {
                u.query_pairs()
                    .find(|(k, _)| k == "cursor")
                    .map(|(_, v)| v.into_owned())
            })
            .unwrap_or_else(|| next.into());
        Ok(json!({"reviews":reviews,"nextPageToken":token,"rawPageInfo":response["meta"]}))
    }
    async fn apple_ppo(&self, app: &str, p: Option<&Value>) -> Result<Value> {
        if let Some(p) = p {
            let name = required(p, "name")?;
            let traffic = p["trafficProportion"]
                .as_u64()
                .filter(|v| *v > 0 && *v <= 100)
                .ok_or_else(|| invalid("trafficProportion must be 1–100"))?;
            for treatment in arr(p, "treatments") {
                required(treatment, "name")?;
                let locales = if arr(treatment, "locales").is_empty() {
                    arr(p, "locales")
                } else {
                    arr(treatment, "locales")
                };
                for locale in locales {
                    if locale.as_str().is_none_or(|s| s.trim().is_empty()) {
                        return Err(invalid("Treatment locale must be a string"));
                    }
                }
            }
            let response = self
                .apple(
                    Method::POST,
                    "/v2/appStoreVersionExperiments",
                    Some(resource(
                        "appStoreVersionExperiments",
                        None,
                        json!({"name":name,"platform":"IOS","trafficProportion":traffic}),
                        json!({"app":{"data":{"type":"apps","id":app}}}),
                    )),
                )
                .await?;
            let mut experiment = ppo(&response["data"]);
            let experiment_id = id(&response["data"])?;
            let mut treatments = Vec::new();
            for treatment in arr(p, "treatments") {
                let mut attrs = json!({"name":treatment["name"]});
                if !s(treatment, "appIconName").is_empty() {
                    attrs["appIconName"] = treatment["appIconName"].clone();
                }
                let response=self.apple(Method::POST,"/v1/appStoreVersionExperimentTreatments",Some(resource("appStoreVersionExperimentTreatments",None,attrs,json!({"appStoreVersionExperimentV2":{"data":{"type":"appStoreVersionExperiments","id":experiment_id}}})))).await?;
                let treatment_id = id(&response["data"])?;
                let locales = if arr(treatment, "locales").is_empty() {
                    arr(p, "locales")
                } else {
                    arr(treatment, "locales")
                };
                for locale in locales {
                    let locale = locale
                        .as_str()
                        .filter(|s| !s.trim().is_empty())
                        .ok_or_else(|| invalid("Treatment locale must be a string"))?;
                    self.apple(Method::POST,"/v1/appStoreVersionExperimentTreatmentLocalizations",Some(resource("appStoreVersionExperimentTreatmentLocalizations",None,json!({"locale":locale}),json!({"appStoreVersionExperimentTreatment":{"data":{"type":"appStoreVersionExperimentTreatments","id":treatment_id}}})))).await?;
                }
                treatments.push(json!({"id":treatment_id,"name":response["data"]["attributes"]["name"],"appIconName":response["data"]["attributes"]["appIconName"],"locales":locales}));
            }
            experiment["treatments"] = json!(treatments);
            return Ok(json!({"experiment":experiment}));
        }
        let response = self
            .apple(
                Method::GET,
                &query(
                    &format!("/v1/apps/{}/appStoreVersionExperimentsV2", escape(app)),
                    &[
                        ("include", "appStoreVersionExperimentTreatments"),
                        ("limit", "200"),
                        ("limit[appStoreVersionExperimentTreatments]", "50"),
                    ],
                ),
                None,
            )
            .await?;
        let experiments=arr(&response,"data").iter().map(|v|{let mut experiment=ppo(v);let mut treatments=Vec::new();
            for link in arr(&v["relationships"]["appStoreVersionExperimentTreatments"],"data"){
                if let Some(t)=arr(&response,"included").iter().find(|t|s(t,"id")==s(link,"id")){
                    treatments.push(json!({"id":t["id"],"name":t["attributes"]["name"],"appIconName":t["attributes"]["appIconName"]}));
                }
            }experiment["treatments"]=json!(treatments);experiment}).collect::<Vec<_>>();
        Ok(json!({"experiments":experiments}))
    }
    async fn apple_marketing(&self, app: &str, p: &Value) -> Result<Value> {
        let page = p
            .get("marketingPage")
            .filter(|p| p.is_object())
            .ok_or_else(|| invalid("marketingPage is required"))?;
        let name = required(page, "pageName")?;
        let locale = if s(page, "locale").is_empty() {
            required(p, "locale")?
        } else {
            s(page, "locale")
        };
        let mut page_id = s(page, "applePageId").to_owned();
        let created = page_id.is_empty();
        let mut localization = None;
        if created {
            let version = self
                .apple_version(app)
                .await?
                .ok_or_else(|| invalid("Custom pages require an existing store version"))?;
            let mut attrs = json!({"locale":locale});
            if arr(p, "syncScopes").is_empty() || scopes(p, "marketing_text", false) {
                if let Some(text) = page.get("promotionalText") {
                    if !text.is_string() {
                        return Err(invalid("promotionalText must be a string"));
                    }
                    attrs["promotionalText"] = text.clone();
                }
            }
            let mut body = resource(
                "appCustomProductPages",
                None,
                json!({"name":name}),
                json!({"app":{"data":{"type":"apps","id":app}},
                "appStoreVersionTemplate":{"data":{"type":"appStoreVersions","id":id(&version)?}},
                "appCustomProductPageVersions":{"data":[{"type":"appCustomProductPageVersions","id":"new-version"}]}}),
            );
            body["included"] = json!([{"type":"appCustomProductPageVersions","id":"new-version","relationships":{"appCustomProductPageLocalizations":{"data":[{"type":"appCustomProductPageLocalizations","id":"new-locale"}]}}},
                {"type":"appCustomProductPageLocalizations","id":"new-locale","attributes":attrs}]);
            let response = self
                .apple(Method::POST, "/v1/appCustomProductPages", Some(body))
                .await?;
            page_id = id(&response["data"])?.into();
            localization = arr(&response, "included")
                .iter()
                .find(|v| s(v, "type") == "appCustomProductPageLocalizations")
                .map(|v| s(v, "id").to_owned())
                .filter(|s| !s.is_empty());
        } else {
            // Verify the supplied page belongs to the envelope-resolved app before mutation.
            let response = self
                .apple(
                    Method::GET,
                    &format!("/v1/appCustomProductPages/{}/app", escape(&page_id)),
                    None,
                )
                .await?;
            if id(&response["data"])? != app {
                return Err(invalid(
                    "Custom product page does not belong to the requested app",
                ));
            }
        }
        let localization = if let Some(loc) = localization {
            loc
        } else {
            let response = self
                .apple(
                    Method::GET,
                    &format!(
                        "/v1/appCustomProductPages/{}/appCustomProductPageVersions?limit=1",
                        escape(&page_id)
                    ),
                    None,
                )
                .await?;
            let version = arr(&response, "data")
                .first()
                .ok_or_else(|| upstream("Custom page version missing"))?;
            self.apple_ensure_locale(
                "appCustomProductPageVersion",
                "appCustomProductPageVersions",
                id(version)?,
                "appCustomProductPageLocalizations",
                locale,
                json!({}),
            )
            .await?
        };
        if arr(p, "syncScopes").is_empty() || scopes(p, "marketing_text", false) {
            let mut attrs = serde_json::Map::new();
            copy_field(&mut attrs, "promotionalText", page, &["promotionalText"])?;
            self.apple_patch(
                "appCustomProductPageLocalizations",
                &localization,
                json!(attrs),
            )
            .await?;
        }
        if scopes(p, "store_images", false) && page["storeImages"].is_object() {
            self.apple_sync_images(
                app,
                "appCustomProductPageLocalization",
                "appCustomProductPageLocalizations",
                &localization,
                &page["storeImages"],
            )
            .await?;
        }
        Ok(succeeded(json!({"applePageId":page_id,"created":created})))
    }
}
pub(super) fn resource(
    kind: &str,
    id: Option<&str>,
    attributes: Value,
    relationships: Value,
) -> Value {
    let mut data = json!({"type":kind,"attributes":attributes});
    if let Some(id) = id {
        data["id"] = json!(id);
    }
    if relationships.as_object().is_some_and(|r| !r.is_empty()) {
        data["relationships"] = relationships;
    }
    json!({"data":data})
}
fn editable(state: &str) -> bool {
    matches!(
        state,
        "PREPARE_FOR_SUBMISSION"
            | "DEVELOPER_REJECTED"
            | "REJECTED"
            | "METADATA_REJECTED"
            | "INVALID_BINARY"
            | "READY_FOR_REVIEW"
    )
}
fn apple_version_state(version: &Value) -> &str {
    let attributes = &version["attributes"];
    let state = s(attributes, "appVersionState");
    if state.is_empty() {
        s(attributes, "appStoreState")
    } else {
        state
    }
}
fn apple_release(version: &Value, is_latest: bool) -> Value {
    let attributes = &version["attributes"];
    let state = apple_version_state(version);
    json!({"id":s(version,"id"),"versionString":s(attributes,"versionString"),"platform":"ios",
        "state":state,"appStoreState":attributes["appStoreState"],"appVersionState":attributes["appVersionState"],
        "editable":editable(state),"isLatest":is_latest,"createdDate":attributes["createdDate"]})
}
fn version_order(value: &Value) -> (Option<i64>, Vec<u64>) {
    let attributes = &value["attributes"];
    let time = chrono::DateTime::parse_from_rfc3339(s(attributes, "createdDate"))
        .ok()
        .map(|d| d.timestamp_millis());
    let number = s(attributes, "versionString")
        .split('.')
        .map(|n| n.parse::<u64>().unwrap_or(0))
        .collect();
    (time, number)
}
fn ppo(v: &Value) -> Value {
    let a = &v["attributes"];
    json!({"id":s(v,"id"),"name":s(a,"name"),"platform":s(a,"platform"),"state":s(a,"state"),"trafficProportion":a["trafficProportion"].as_u64().unwrap_or(0),
    "reviewRequired":a["reviewRequired"].as_bool().unwrap_or(false),"startDate":s(a,"startDate"),"endDate":s(a,"endDate"),"treatments":[]})
}

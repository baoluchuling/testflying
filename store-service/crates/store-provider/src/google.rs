use super::*;

const IMAGE_TYPES: &[(&str, &str)] = &[
    ("featureGraphic", "feature_graphic_url"),
    ("phoneScreenshots", "phone_screenshots"),
    ("sevenInchScreenshots", "seven_inch_screenshots"),
    ("tenInchScreenshots", "ten_inch_screenshots"),
    ("tvScreenshots", "tv_screenshots"),
    ("wearScreenshots", "wear_screenshots"),
];
impl Gateway {
    pub(super) async fn google_execute(&self, r: &StoreRequest) -> Result<Value> {
        if r.operation == Operation::CreateStoreRelease {
            return Err(invalid("Creating a store version requires iOS"));
        }
        if r.operation == Operation::StoreReviews {
            return self.google_reviews(r).await;
        }
        if r.operation == Operation::ProductPageOptimizations {
            return Ok(json!({"experiments":[]}));
        }
        if r.operation == Operation::CreateProductPageOptimization {
            return Err(invalid("Product page optimization requires iOS"));
        }
        if r.operation == Operation::Sync {
            let operation = required(&r.payload, "operation")?;
            if operation == "update_marketing_page" {
                return Ok(failed(
                    "google_marketing_page_unsupported",
                    "Marketing pages require iOS",
                ));
            }
            if !matches!(operation, "update_release_notes" | "update_app_metadata") {
                return Err(invalid("Unsupported sync operation"));
            }
            required(&r.payload, "locale")?;
        }
        let root = format!(
            "/androidpublisher/v3/applications/{}",
            escape(&r.bundle_identifier)
        );
        let edit = self
            .api(
                Platform::Android,
                Method::POST,
                &format!("{root}/edits"),
                Some(json!({})),
            )
            .await?;
        let path = format!("{root}/edits/{}", escape(id(&edit)?));
        let outcome = self.google_edit(r, &path).await;
        let committed = matches!(&outcome, Ok(v) if r.operation == Operation::Sync && v["status"] == "succeeded");
        if !committed {
            let cleanup = self.google_discard_edit(&path).await.map_err(|_| {
                StoreError::new(
                    502,
                    "store_edit_cleanup_failed",
                    "Google edit could not be discarded after the store operation",
                )
            });
            // The original image error describes the write boundary, including uncertainty.
            // A failed best-effort discard must not replace that evidence.
            if !(IMAGE_EXECUTION.try_with(|_| ()).is_ok() && outcome.is_err()) {
                cleanup?;
            }
        }
        outcome
    }
    async fn google_discard_edit(&self, path: &str) -> Result<()> {
        // Cleanup must remain possible when ordinary calls exhaust the local quota.
        // Count it, but never block disposal solely on that local safety limit.
        self.inner
            .limits
            .lock()
            .await
            .events
            .entry("google".into())
            .or_default()
            .push_back(Instant::now());
        let token = self.token(Platform::Android).await?;
        let request = self
            .inner
            .client
            .delete(format!(
                "{}{}",
                self.inner.config.google_base_url.trim_end_matches('/'),
                path
            ))
            .bearer_auth(token);
        let response = self
            .send_logged(request, Some(Platform::Android), "edit_discard")
            .await?;
        bounded_json(response).await?;
        Ok(())
    }
    async fn google_edit(&self, r: &StoreRequest, path: &str) -> Result<Value> {
        match r.operation {
            Operation::Preflight => Ok(preflight(true, None, "editable", true)),
            Operation::SupportedLocales | Operation::StoreListings | Operation::StoreImages => {
                let response = self
                    .api(
                        Platform::Android,
                        Method::GET,
                        &format!("{path}/listings"),
                        None,
                    )
                    .await?;
                let mut listings = arr(&response, "listings").to_vec();
                listings.sort_by(|a, b| s(a, "language").cmp(s(b, "language")));
                if r.operation == Operation::SupportedLocales {
                    return Ok(
                        json!({"locales":listings.iter().map(|v|s(v,"language")).filter(|s|!s.is_empty()).collect::<Vec<_>>()}),
                    );
                }
                if r.operation == Operation::StoreListings {
                    return Ok(
                        json!({"listings": listings.iter().filter(|v| !s(v,"language").is_empty()).map(|v|json!({"locale":s(v,"language"),"title":s(v,"title"),
                        "shortDescription":s(v,"shortDescription"),"fullDescription":s(v,"fullDescription"),"description":s(v,"fullDescription"),"videoUrl":s(v,"video")})).collect::<Vec<_>>()}),
                    );
                }
                let mut locales = Vec::new();
                let filters = image_query::ImageQuery::new(&r.payload, r.platform)?;
                for listing in listings {
                    let locale = s(&listing, "language");
                    if locale.is_empty() || !filters.matches_locale(locale) {
                        continue;
                    }
                    let mut images = serde_json::Map::new();
                    for (kind, slot) in IMAGE_TYPES {
                        if !filters.matches_google_type(kind) {
                            continue;
                        }
                        let response = self
                            .api(
                                Platform::Android,
                                Method::GET,
                                &format!("{path}/listings/{}/{kind}", escape(locale)),
                                None,
                            )
                            .await?;
                        images.insert((*slot).into(), json!(arr(&response, "images")));
                    }
                    locales.push(json!({"locale":locale,"images":images}));
                }
                Ok(json!({"locales":locales}))
            }
            Operation::StoreReleases => {
                let response = self
                    .api(
                        Platform::Android,
                        Method::GET,
                        &format!("{path}/tracks"),
                        None,
                    )
                    .await?;
                let mut releases = Vec::new();
                for track in arr(&response, "tracks") {
                    for release in arr(track, "releases") {
                        if arr(release, "versionCodes").is_empty() {
                            continue;
                        }
                        let mut release = release.clone();
                        release["track"] = track["track"].clone();
                        releases.push(release);
                    }
                }
                releases.sort_by(|a, b| {
                    max_code(b)
                        .cmp(&max_code(a))
                        .then(s(a, "track").cmp(s(b, "track")))
                });
                Ok(json!({"releases":releases}))
            }
            Operation::Sync => {
                let p = &r.payload;
                let state = if s(p, "operation") == "update_release_notes" {
                    required(p, "releaseNotes")?;
                    let response = self
                        .api(
                            Platform::Android,
                            Method::GET,
                            &format!("{path}/tracks"),
                            None,
                        )
                        .await?;
                    let mut tracks = arr(&response, "tracks").to_vec();
                    let Some((ti, ri, code)) = release_target(&tracks, &p["storeRelease"]) else {
                        return Ok(failed(
                            "google_release_target_missing",
                            "Google Play has no matching release",
                        ));
                    };
                    let locale = s(p, "locale");
                    let release = &mut tracks[ti]["releases"][ri];
                    let mut notes = arr(release, "releaseNotes").to_vec();
                    if let Some(note) = notes
                        .iter_mut()
                        .find(|v| s(v, "language").eq_ignore_ascii_case(locale))
                    {
                        note["language"] = json!(locale);
                        note["text"] = p["releaseNotes"].clone();
                    } else {
                        notes.push(json!({"language":locale,"text":p["releaseNotes"]}));
                    }
                    release["releaseNotes"] = json!(notes);
                    let name = release["name"].clone();
                    let track = s(&tracks[ti], "track").to_owned();
                    self.api(
                        Platform::Android,
                        Method::PUT,
                        &format!("{path}/tracks/{}", escape(&track)),
                        Some(tracks[ti].clone()),
                    )
                    .await?;
                    json!({"track":track,"versionCode":code,"versionName":name})
                } else {
                    let metadata = p
                        .get("metadata")
                        .filter(|m| m.is_object())
                        .ok_or_else(|| invalid("metadata is required"))?;
                    let mut body = serde_json::Map::new();
                    for (scope, target, keys) in [
                        ("title", "title", &["title"][..]),
                        (
                            "description",
                            "fullDescription",
                            &["fullDescription", "description"][..],
                        ),
                        (
                            "short_description",
                            "shortDescription",
                            &["shortDescription", "promotionalText"][..],
                        ),
                        ("video", "video", &["videoUrl"][..]),
                    ] {
                        if scopes(p, scope, true)
                            || (scope == "short_description"
                                && scopes(p, "promotional_text", false))
                        {
                            copy_field(&mut body, target, metadata, keys)?;
                        }
                    }
                    let mut changed = !body.is_empty();
                    if changed {
                        self.api(
                            Platform::Android,
                            Method::PATCH,
                            &format!("{path}/listings/{}", escape(s(p, "locale"))),
                            Some(json!(body)),
                        )
                        .await?;
                    }
                    if scopes(p, "store_images", false) {
                        if let Some(images) = metadata.get("storeImages").filter(|v| v.is_object())
                        {
                            changed |= self
                                .google_sync_images(path, s(p, "locale"), images)
                                .await?;
                        }
                    }
                    if !changed {
                        return Ok(failed(
                            "empty_sync_payload",
                            "No selected store content was supplied",
                        ));
                    }
                    json!({})
                };
                self.api(
                    Platform::Android,
                    Method::POST,
                    &format!("{path}:commit?changesInReviewBehavior=ERROR_IF_IN_REVIEW"),
                    None,
                )
                .await?;
                Ok(succeeded(state))
            }
            _ => Err(invalid("Unsupported Google edit operation")),
        }
    }
    async fn google_sync_images(&self, path: &str, locale: &str, images: &Value) -> Result<bool> {
        let mut changed = false;
        for (kind, key) in [
            ("featureGraphic", "feature_graphic_url"),
            ("phoneScreenshots", "phone_screenshots"),
            ("sevenInchScreenshots", "tablet_screenshots"),
            ("tenInchScreenshots", "tablet_screenshots"),
        ] {
            let Some(slot) = images.get(key) else {
                continue;
            };
            let sources = images::sources(slot)?;
            let mut downloaded = Vec::new();
            for source in sources {
                downloaded.push(self.download_image(&source).await?);
            }
            let image_path = format!("{path}/listings/{}/{kind}", escape(locale));
            self.api(Platform::Android, Method::DELETE, &image_path, None)
                .await?;
            for image in downloaded {
                self.limit(Platform::Android).await?;
                let token = self.token(Platform::Android).await?;
                let endpoint = format!(
                    "{}/upload{}?uploadType=multipart",
                    self.inner
                        .config
                        .google_upload_base_url
                        .trim_end_matches('/'),
                    image_path
                );
                let mut boundary = format!("store-image-{:x}", md5::compute(&image.bytes));
                while image
                    .bytes
                    .windows(boundary.len())
                    .any(|v| v == boundary.as_bytes())
                {
                    boundary.push('x');
                }
                let mut body=format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{{}}\r\n--{boundary}\r\nContent-Type: {}\r\n\r\n",image.mime).into_bytes();
                body.extend_from_slice(&image.bytes);
                body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
                let request = self
                    .inner
                    .client
                    .post(endpoint)
                    .bearer_auth(token)
                    .header(
                        "content-type",
                        format!("multipart/related; boundary={boundary}"),
                    )
                    .body(body);
                let response = self
                    .send_logged(request, Some(Platform::Android), "google_image_upload")
                    .await?;
                mutation_json(response, true).await?;
            }
            changed = true;
        }
        Ok(changed)
    }
    async fn google_reviews(&self, r: &StoreRequest) -> Result<Value> {
        let p = &r.payload;
        let size = p["pageSize"]
            .as_u64()
            .filter(|n| *n > 0)
            .unwrap_or(50)
            .min(100)
            .to_string();
        let mut pairs = vec![("maxResults", size.as_str())];
        for (key, target) in [
            ("pageToken", "token"),
            ("startIndex", "startIndex"),
            ("translationLanguage", "translationLanguage"),
        ] {
            if !s(p, key).is_empty() {
                pairs.push((target, s(p, key)));
            }
        }
        let response = self
            .api(
                Platform::Android,
                Method::GET,
                &query(
                    &format!(
                        "/androidpublisher/v3/applications/{}/reviews",
                        escape(&r.bundle_identifier)
                    ),
                    &pairs,
                ),
                None,
            )
            .await?;
        let mut reviews = Vec::new();
        for item in arr(&response, "reviews") {
            let Some(comment) = arr(item, "comments")
                .iter()
                .rev()
                .find_map(|c| c.get("userComment"))
            else {
                continue;
            };
            let seconds = s(&comment["lastModified"], "seconds")
                .parse::<i64>()
                .unwrap_or(0);
            let nanos = comment["lastModified"]["nanos"].as_u64().unwrap_or(0);
            let timestamp = u32::try_from(nanos)
                .ok()
                .and_then(|nanos| chrono::DateTime::from_timestamp(seconds, nanos))
                .filter(|_| seconds > 0)
                .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true))
                .unwrap_or_default();
            reviews.push(json!({"id":s(item,"reviewId"),"platform":"android","rating":comment["starRating"].as_i64().unwrap_or(0),"body":s(comment,"text"),
                "authorName":s(item,"authorName"),"locale":s(comment,"reviewerLanguage"),"appVersion":s(comment,"appVersionName"),
                "appVersionCode":comment["appVersionCode"].as_i64().unwrap_or(0).to_string(),"createdAt":timestamp,"updatedAt":timestamp,"originalText":s(comment,"originalText")}));
        }
        Ok(
            json!({"reviews":reviews,"nextPageToken":s(&response["tokenPagination"],"nextPageToken")}),
        )
    }
}
fn max_code(release: &Value) -> i64 {
    arr(release, "versionCodes")
        .iter()
        .filter_map(|v| v.as_str()?.trim().parse().ok())
        .max()
        .unwrap_or(-1)
}
fn release_target(tracks: &[Value], target: &Value) -> Option<(usize, usize, String)> {
    let mut best = None;
    let mut best_code = -1i64;
    for (ti, track) in tracks.iter().enumerate() {
        if !s(target, "track").is_empty() && s(target, "track") != s(track, "track") {
            continue;
        }
        for (ri, release) in arr(track, "releases").iter().enumerate() {
            if !s(target, "versionName").is_empty()
                && s(target, "versionName") != s(release, "name")
            {
                continue;
            }
            for code in arr(release, "versionCodes")
                .iter()
                .filter_map(Value::as_str)
            {
                if !s(target, "versionCode").is_empty() {
                    if code == s(target, "versionCode") {
                        return Some((ti, ri, code.into()));
                    }
                    continue;
                }
                if let Ok(value) = code.trim().parse::<i64>() {
                    if value > best_code {
                        best_code = value;
                        best = Some((ti, ri, code.into()));
                    }
                }
            }
        }
    }
    best
}

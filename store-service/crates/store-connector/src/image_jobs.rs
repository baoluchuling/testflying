//! Node-owned tasks survive the transport caller and retain only bounded result metadata.
use super::*;
use sha2::{Digest, Sha256};
use std::{collections::HashMap, sync::Mutex, time::Instant};

const QUEUE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const RESULT_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_COMPLETED: usize = 512;
const MAX_RESULT: usize = 64 * 1024;

struct Job {
    platform: Platform,
    app: String,
    digest: [u8; 32],
    snapshot: serde_json::Value,
    completed: Option<Instant>,
}
pub(super) struct ImageJobs {
    jobs: Mutex<HashMap<String, Job>>,
    slots: Arc<Semaphore>,
}
impl Default for ImageJobs {
    fn default() -> Self {
        Self {
            jobs: Mutex::default(),
            slots: Arc::new(Semaphore::new(20)),
        }
    }
}
fn not_found() -> StoreError {
    StoreError::new(
        404,
        "store_image_job_not_found",
        "Image sync job was not found",
    )
}
fn prune(jobs: &mut HashMap<String, Job>) {
    jobs.retain(|_, job| job.completed.is_none_or(|at| at.elapsed() < RESULT_TTL));
    while jobs.values().filter(|job| job.completed.is_some()).count() > MAX_COMPLETED {
        let oldest = jobs
            .iter()
            .filter_map(|(id, job)| job.completed.map(|at| (id.clone(), at)))
            .min_by_key(|(_, at)| *at)
            .map(|(id, _)| id);
        if let Some(id) = oldest {
            jobs.remove(&id);
        } else {
            break;
        }
    }
}
impl ImageJobs {
    pub(super) async fn drain(&self) {
        if let Ok(permits) = self.slots.acquire_many(20).await {
            drop(permits);
        }
    }
    fn status(&self, request: &StoreRequest) -> Result<serde_json::Value, StoreError> {
        let id = request.payload["jobId"].as_str().ok_or_else(not_found)?;
        let mut jobs = self.jobs.lock().unwrap();
        prune(&mut jobs);
        let job = jobs
            .get(id)
            .filter(|job| job.platform == request.platform && job.app == request.bundle_identifier)
            .ok_or_else(not_found)?;
        Ok(job.snapshot.clone())
    }
    fn complete(&self, id: &str, reply: StoreReply) {
        let mut jobs = self.jobs.lock().unwrap();
        if let Some(job) = jobs.get_mut(id) {
            let unknown = reply.body["outcomeUnknown"].as_bool().unwrap_or(false);
            let failed = reply.status >= 400 || reply.body["status"] == "failed";
            let code = reply.body["error"]["code"]
                .as_str()
                .or_else(|| reply.body["errorCode"].as_str());
            let mut snapshot = json!({"jobId":id,"status":if unknown {"unknown"} else if failed {"failed"} else {"succeeded"},
                "result":reply.body,"httpStatus":reply.status,"errorCode":code,"outcomeUnknown":unknown});
            if serde_json::to_vec(&snapshot).map_or(true, |bytes| bytes.len() > MAX_RESULT) {
                snapshot = json!({"jobId":id,"status":"unknown","result":null,"httpStatus":null,
                    "errorCode":"store_image_result_too_large","outcomeUnknown":true});
            }
            job.snapshot = snapshot;
            job.completed = Some(Instant::now());
        }
        prune(&mut jobs);
    }
}
// A canceled/panicking worker releases its lease and records uncertainty without replaying writes.
struct CompletionGuard {
    executor: Arc<Executor>,
    id: String,
    completed: bool,
}
impl CompletionGuard {
    fn finish(mut self, reply: StoreReply) {
        self.executor.image_jobs.complete(&self.id, reply);
        self.completed = true;
    }
}
impl Drop for CompletionGuard {
    fn drop(&mut self) {
        if !self.completed {
            self.executor.image_jobs.complete(&self.id, StoreReply { request_id: self.id.clone(), status: 502,
                body: json!({"outcomeUnknown":true,"error":{"code":"image_worker_stopped","message":"Image sync stopped; verify the store outcome manually"}}) });
        }
    }
}
impl Executor {
    pub(super) async fn image_job(self: &Arc<Self>, request: StoreRequest) -> StoreReply {
        let id = request.request_id.clone();
        if request.operation == Operation::StoreImageSyncStatus {
            return StoreReply::from_result(id, self.image_jobs.status(&request));
        }
        let result = (|| {
            store_provider::validate_request(&request)?;
            let digest: [u8; 32] = Sha256::digest(serde_json::to_vec(&request).map_err(|_| {
                StoreError::new(422, "invalid_payload", "Cannot encode image request")
            })?)
            .into();
            // The map mutex serializes duplicate checks, capacity reservation and asset transfer.
            let mut jobs = self.image_jobs.jobs.lock().unwrap();
            prune(&mut jobs);
            if let Some(job) = jobs.get(&id) {
                if job.digest != digest
                    || job.platform != request.platform
                    || job.app != request.bundle_identifier
                {
                    return Err(StoreError::new(
                        409,
                        "store_image_job_conflict",
                        "This jobId already identifies another request",
                    ));
                }
                return Ok(
                    json!({"status":"succeeded","jobId":id,"executionStatus":job.snapshot["status"],
                    "message":"Image sync was validated and accepted for background execution","errorCode":null,"errorSummary":null,"storeState":null}),
                );
            }
            let slot = self
                .image_jobs
                .slots
                .clone()
                .try_acquire_owned()
                .map_err(|_| {
                    StoreError::new(429, "store_image_queue_full", "Image sync queue is full")
                })?;
            let lease = self.assets.claim(&request)?;
            jobs.insert(id.clone(), Job {platform:request.platform, app:request.bundle_identifier.clone(), digest,
                snapshot:json!({"jobId":id,"status":"queued","result":null,"httpStatus":null,"errorCode":null,"outcomeUnknown":false}), completed:None});
            drop(jobs);
            let executor = self.clone();
            let mut normalized = request.clone();
            normalized.operation = Operation::Sync;
            let guard = CompletionGuard {
                executor: executor.clone(),
                id: id.clone(),
                completed: false,
            };
            tokio::spawn(async move {
                let _slot = slot;
                let result = match tokio::time::timeout(
                    QUEUE_TIMEOUT,
                    executor.capacity.clone().acquire_owned(),
                )
                .await
                {
                    Ok(Ok(permit)) => {
                        let _permit = permit;
                        if let Some(job) = executor
                            .image_jobs
                            .jobs
                            .lock()
                            .unwrap()
                            .get_mut(&normalized.request_id)
                        {
                            job.snapshot["status"] = json!("running");
                        }
                        if let Some(log) = &executor.log {
                            log.task(
                                &normalized,
                                "task.execution_started",
                                json!({"background":true}),
                            );
                        }
                        executor
                            .gateway
                            .execute_with_assets(&normalized, lease)
                            .await
                    }
                    _ => {
                        drop(lease);
                        Err(StoreError::new(
                            504,
                            "store_image_queue_timeout",
                            "Image sync expired before execution",
                        ))
                    }
                };
                let reply = StoreReply::from_result(normalized.request_id.clone(), result);
                if let Some(log) = &executor.log {
                    log.task(&normalized,"task.execution_completed",json!({"background":true,"status":reply.status,"businessStatus":reply.body["status"]}));
                }
                // Consume the whole guard so Rust 2021 captures its ownership in the future.
                guard.finish(reply);
            });
            Ok(
                json!({"status":"succeeded","jobId":id,"executionStatus":"queued",
                "message":"Image sync was validated and accepted for background execution","errorCode":null,"errorSummary":null,"storeState":null}),
            )
        })();
        StoreReply::from_result(id, result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;
    fn executor() -> Arc<Executor> {
        Executor::new(serde_json::from_value(json!({"connectorId":"image-node","token":"fixture-token-0123456789",
            "messageKey":"01".repeat(32),"uploadKey":"02".repeat(32),"serverUrl":"http://127.0.0.1:1",
            "provider":{"mode":"mock"}})).unwrap()).unwrap()
    }
    fn start(id: &str) -> StoreRequest {
        StoreRequest {
            request_id: id.into(),
            platform: Platform::Ios,
            bundle_identifier: "com.example.app".into(),
            operation: Operation::StartStoreImageSync,
            payload: json!({"operation":"update_app_metadata","locale":"en-US",
                "syncScopes":["store_images"],"metadata":{"storeImages":{"phone_screenshots":{"urls":["http://127.0.0.1:1/image.png"]}}}}),
        }
    }
    fn status(id: &str) -> StoreRequest {
        let mut r = start("status-request");
        r.operation = Operation::StoreImageSyncStatus;
        r.payload = json!({"jobId":id});
        r
    }
    #[tokio::test]
    async fn unsupported_android_marketing_rejects_before_claim_and_asset_remains_reusable() {
        let executor = executor();
        let mut image = Vec::new();
        image::codecs::png::PngEncoder::new(&mut image)
            .write_image(&[0; 12], 2, 2, image::ExtendedColorType::Rgb8)
            .unwrap();
        let now = upload::now().unwrap();
        let claims = upload::UploadClaims {
            iss: "testflying-store-api".into(),
            aud: "connector:image-node".into(),
            iat: now,
            exp: now + upload::TOKEN_SECONDS,
            connector_id: "image-node".into(),
            asset_id: "00000000-0000-4000-8000-000000000099".into(),
            platform: Platform::Android,
            bundle_identifier: "com.example.app".into(),
            file: upload::UploadInput {
                file_name: "image.png".into(),
                content_type: "image/png".into(),
                size_bytes: image.len() as u64,
                sha256: hex::encode(Sha256::digest(&image)),
            },
        };
        let session = "00000000-0000-4000-8000-000000000098";
        executor
            .assets
            .start(&upload::sign(&claims, &"02".repeat(32)).unwrap(), session)
            .unwrap();
        executor
            .assets
            .chunk(
                Platform::Android,
                "com.example.app",
                &claims.asset_id,
                session,
                0,
                &image,
            )
            .unwrap();
        executor
            .assets
            .finish(
                Platform::Android,
                "com.example.app",
                &claims.asset_id,
                session,
            )
            .unwrap();
        let images = json!({"phone_screenshots":{"assets":[{"assetId":claims.asset_id}]}});
        let mut request = start("unsupported");
        request.platform = Platform::Android;
        request.payload = json!({"operation":"update_marketing_page","locale":"en-US",
            "syncScopes":["store_images"],"marketingPage":{"pageName":"Page","storeImages":images}});
        let reply = executor.clone().execute(request).await;
        assert_eq!(reply.status, 422);
        assert_eq!(
            reply.body["error"]["code"],
            "google_marketing_page_unsupported"
        );
        assert_eq!(executor.assets.usage(), (1, image.len() as u64));
        assert!(executor.image_jobs.jobs.lock().unwrap().is_empty());
        let mut reuse = start("supported");
        reuse.platform = Platform::Android;
        reuse.payload["metadata"]["storeImages"] = images;
        assert_eq!(executor.clone().execute(reuse).await.status, 200);
        executor.drain().await;
        assert_eq!(executor.assets.usage(), (0, 0));
    }
    #[test]
    fn worker_preserves_original_error_and_uncertainty_in_final_snapshot() {
        for unknown in [false, true] {
            let executor = executor();
            executor.image_jobs.jobs.lock().unwrap().insert(
                "result".into(),
                Job {
                    platform: Platform::Ios,
                    app: "com.example.app".into(),
                    digest: [0; 32],
                    snapshot: json!({}),
                    completed: None,
                },
            );
            let mut error = StoreError::new(502, "store_upstream_error", "Store request failed");
            error.outcome_unknown = unknown;
            executor.image_jobs.complete(
                "result",
                StoreReply::from_result("result".into(), Err(error)),
            );
            let snapshot = executor.image_jobs.status(&status("result")).unwrap();
            assert_eq!(
                snapshot["status"],
                if unknown { "unknown" } else { "failed" }
            );
            assert_eq!(snapshot["outcomeUnknown"], unknown);
            assert_eq!(snapshot["httpStatus"], 502);
            assert_eq!(
                snapshot["result"]["error"]["message"],
                "Store request failed"
            );
        }
    }
    #[tokio::test]
    async fn early_accept_status_isolation_and_duplicate_are_independent_of_business_lock() {
        let executor = executor();
        let busy = executor.capacity.clone().acquire_owned().await.unwrap();
        let reply = tokio::time::timeout(
            Duration::from_secs(1),
            executor.clone().execute(start("job-a")),
        )
        .await
        .unwrap();
        assert_eq!(reply.status, 200);
        assert_eq!(reply.body["executionStatus"], "queued");
        // Let the worker start waiting: admission's guard must still belong to that future.
        tokio::task::yield_now().await;
        assert_eq!(
            executor.clone().execute(status("job-a")).await.body["status"],
            "queued"
        );
        assert_eq!(executor.clone().execute(start("job-a")).await.status, 200);
        assert_eq!(executor.image_jobs.jobs.lock().unwrap().len(), 1);
        let mut conflict = start("job-a");
        conflict.payload["locale"] = json!("fr-FR");
        assert_eq!(executor.clone().execute(conflict).await.status, 409);
        let mut foreign = status("job-a");
        foreign.bundle_identifier = "com.other.app".into();
        assert_eq!(executor.clone().execute(foreign).await.status, 404);
        let mut foreign = status("job-a");
        foreign.platform = Platform::Android;
        assert_eq!(executor.clone().execute(foreign).await.status, 404);
        drop(busy);
        executor.drain().await;
        assert_eq!(
            executor.clone().execute(status("job-a")).await.body["status"],
            "succeeded"
        );
        // A retained terminal duplicate must never restart the write.
        assert_eq!(
            executor.clone().execute(start("job-a")).await.body["executionStatus"],
            "succeeded"
        );
    }
    #[tokio::test]
    async fn worker_guard_is_retained_until_cancellation() {
        let executor = executor();
        executor.image_jobs.jobs.lock().unwrap().insert("canceled".into(), Job {
            platform:Platform::Ios, app:"com.example.app".into(), digest:[0;32],
            snapshot:json!({"jobId":"canceled","status":"running","result":null,"httpStatus":null,"errorCode":null,"outcomeUnknown":false}),
            completed:None,
        });
        let guard = CompletionGuard {
            executor: executor.clone(),
            id: "canceled".into(),
            completed: false,
        };
        let worker = tokio::spawn(async move {
            std::future::pending::<()>().await;
            guard.finish(StoreReply::from_result(
                "canceled".into(),
                Ok(json!({"status":"succeeded"})),
            ));
        });
        tokio::task::yield_now().await;
        assert_eq!(
            executor.image_jobs.status(&status("canceled")).unwrap()["status"],
            "running"
        );
        worker.abort();
        assert!(worker.await.unwrap_err().is_cancelled());
        let snapshot = executor.image_jobs.status(&status("canceled")).unwrap();
        assert_eq!(snapshot["status"], "unknown");
        assert_eq!(snapshot["errorCode"], "image_worker_stopped");
    }
    #[tokio::test]
    async fn full_queue_is_checked_before_any_asset_claim() {
        let executor = executor();
        let _all_slots = executor
            .image_jobs
            .slots
            .clone()
            .acquire_many_owned(20)
            .await
            .unwrap();
        let now = upload::now().unwrap();
        let claims = upload::UploadClaims {
            iss: "testflying-store-api".into(),
            aud: "connector:image-node".into(),
            iat: now,
            exp: now + upload::TOKEN_SECONDS,
            connector_id: "image-node".into(),
            asset_id: "00000000-0000-4000-8000-000000000099".into(),
            platform: Platform::Ios,
            bundle_identifier: "com.example.app".into(),
            file: upload::UploadInput {
                file_name: "image.png".into(),
                content_type: "image/png".into(),
                size_bytes: 1,
                sha256: "11".repeat(32),
            },
        };
        executor
            .assets
            .start(
                &upload::sign(&claims, &"02".repeat(32)).unwrap(),
                "00000000-0000-4000-8000-000000000098",
            )
            .unwrap();
        let mut request = start("full");
        request.payload["metadata"]["storeImages"]["phone_screenshots"] =
            json!({"assets":[{"assetId":"00000000-0000-4000-8000-000000000099"}]});
        let reply = executor.clone().execute(request).await;
        assert_eq!(reply.status, 429);
        assert_eq!(reply.body["error"]["code"], "store_image_queue_full");
        assert_eq!(executor.assets.usage(), (1, 1));
        assert!(executor.image_jobs.jobs.lock().unwrap().is_empty());
    }
    #[tokio::test(start_paused = true)]
    async fn queue_deadline_is_terminal_and_releases_slot() {
        let executor = executor();
        let _busy = executor.capacity.clone().acquire_owned().await.unwrap();
        assert_eq!(executor.clone().execute(start("expired")).await.status, 200);
        tokio::task::yield_now().await;
        tokio::time::advance(QUEUE_TIMEOUT + Duration::from_secs(1)).await;
        tokio::task::yield_now().await;
        let reply = executor.clone().execute(status("expired")).await;
        assert_eq!(reply.body["status"], "failed");
        assert_eq!(reply.body["errorCode"], "store_image_queue_timeout");
        assert_eq!(executor.image_jobs.slots.available_permits(), 20);
    }
    #[tokio::test]
    async fn mixed_and_text_start_are_rejected_without_jobs() {
        let executor = executor();
        let mut request = start("mixed");
        request.payload["syncScopes"] = json!(["store_images", "title"]);
        assert_eq!(executor.clone().execute(request.clone()).await.status, 422);
        request.operation = Operation::Sync;
        assert_eq!(executor.clone().execute(request).await.status, 200);
        let mut request = start("text");
        request.payload["syncScopes"] = json!(["title"]);
        assert_eq!(executor.clone().execute(request.clone()).await.status, 422);
        request.operation = Operation::Sync;
        assert_eq!(executor.clone().execute(request).await.status, 200);
        assert!(executor.image_jobs.jobs.lock().unwrap().is_empty());
    }
    #[test]
    fn terminal_results_are_bounded_and_expire() {
        let executor = executor();
        let mut jobs = executor.image_jobs.jobs.lock().unwrap();
        jobs.insert(
            "large".into(),
            Job {
                platform: Platform::Ios,
                app: "com.example.app".into(),
                digest: [0; 32],
                snapshot: json!({}),
                completed: None,
            },
        );
        drop(jobs);
        executor.image_jobs.complete(
            "large",
            StoreReply::from_result("large".into(), Ok(json!({"body":"x".repeat(MAX_RESULT)}))),
        );
        assert_eq!(
            executor.image_jobs.status(&status("large")).unwrap()["status"],
            "unknown"
        );
        executor
            .image_jobs
            .jobs
            .lock()
            .unwrap()
            .get_mut("large")
            .unwrap()
            .completed = Some(Instant::now() - RESULT_TTL);
        assert!(executor.image_jobs.status(&status("large")).is_err());
    }
}

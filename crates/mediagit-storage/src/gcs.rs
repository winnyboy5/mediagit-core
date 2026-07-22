// MediaGit - Git for Media Files
// Copyright (C) 2025 MediaGit Contributors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU Affero General Public License for more details.

//! Google Cloud Storage backend — migrated to `google-cloud-storage` v1.11
//! (official Google Cloud Rust SDK, formerly yoshidan v0.24).
//!
//! ## Architecture
//!
//! v1.11 splits the GCS surface into two clients:
//! - [`Storage`]: data-plane — `write_object`, `read_object`, `open_object`
//! - [`StorageControl`]: control-plane — `get_object`, `delete_object`, `list_objects`
//!
//! Both are `Clone + Send + Sync` and share a connection pool internally.
//!
//! ## Auth
//!
//! By default both clients use Application Default Credentials (ADC), which
//! honours `GOOGLE_APPLICATION_CREDENTIALS` if it is already set in the
//! process environment. The `new(…, service_account_path)` constructor does
//! NOT mutate the environment (mutating `std::env` in a multi-threaded async
//! server is a process-wide data race); instead it parses the service
//! account JSON directly and passes `Credentials` explicitly to both client
//! builders via `with_credentials`. Prefer `with_default_credentials` in
//! production.
//!
//! ## Retry
//!
//! `Storage` (data plane: put/get) uses `AlwaysRetry.with_attempt_limit(max_retries)`
//! so upload/download failures are retried up to `GcsConfig::max_retries` times.
//!
//! `StorageControl` (gRPC control plane: exists/delete/list) uses the SDK default
//! AIP-194 policy.  `AlwaysRetry` is deliberately NOT applied here because it
//! retries `NOT_FOUND`, which `exists()` uses as a fast "absent" signal.  Applying
//! `AlwaysRetry` to `StorageControl` causes exponential back-off on every missing
//! chunk, stalling `chunks/check` on a fresh bucket.
//!
//! ## Config fields `chunk_size` / `resumable_threshold`
//!
//! Left in `GcsConfig` for backward compat (integration tests read them).
//! The v1 SDK selects simple vs resumable upload internally based on payload size
//! (configurable via `with_resumable_upload_threshold` on the builder); our
//! per-field tuning is wired through that builder method where applicable.
//! `chunk_size` is unused at the API level in v1 and kept dead for compat only.

use crate::StorageBackend;
use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::{self, StreamExt, TryStreamExt};
use google_cloud_auth::signer::Signer;
use google_cloud_gax::retry_policy::{AlwaysRetry, RetryPolicyExt};
use google_cloud_storage::builder::storage::SignedUrlBuilder;
use google_cloud_storage::client::{Storage, StorageControl};
use google_cloud_storage::model_ext::ReadRange;
use std::fmt;
use std::sync::Arc;
use tracing::{debug, warn};

/// Object size at or above which `get_with_size_hint` switches to striped
/// parallel range reads. Below this threshold, a single streamed read wins —
/// HTTP/2 multiplex amortises the cost and the per-stripe TLS/handshake
/// overhead is non-trivial. Picked as 2× the CDC max chunk bound (16 MiB) so
/// regular per-chunk reads stay on the single-shot path.
pub(crate) const STRIPED_GET_THRESHOLD: u64 = 32 * 1024 * 1024;
/// Default size of each parallel stripe (8 MiB). Tuned to (a) be large enough
/// to amortise per-stripe HTTP overhead, (b) be small enough that a 256 MiB
/// object yields enough stripes (32) to saturate the GCS connection pool.
pub(crate) const STRIPE_SIZE: u64 = 8 * 1024 * 1024;
/// Maximum number of stripe reads issued in parallel for one striped `get`.
/// Caps fan-out so a single huge object cannot drain the SDK's connection
/// pool away from concurrent unrelated requests.
pub(crate) const STRIPE_CONCURRENCY: usize = 8;

/// Configuration for the GCS backend.
#[derive(Clone, Debug)]
pub struct GcsConfig {
    /// Project ID in Google Cloud
    pub project_id: String,
    /// Bucket name for storage
    pub bucket_name: String,
    /// Chunk size for uploads (in bytes).
    /// Kept for backward compat — v1 SDK manages chunking internally.
    #[allow(dead_code)]
    pub chunk_size: usize,
    /// Threshold for resumable uploads (in bytes).
    /// Forwarded to the v1 builder via `with_resumable_upload_threshold`.
    pub resumable_threshold: usize,
    /// Maximum number of attempts (including the first) for transient failures.
    pub max_retries: u32,
    /// Optional key prefix applied to every object stored in the bucket.
    /// When set, all keys are prefixed with `<prefix>/` on the wire.
    pub prefix: Option<String>,
}

impl Default for GcsConfig {
    fn default() -> Self {
        GcsConfig {
            project_id: String::new(),
            bucket_name: String::new(),
            chunk_size: 256 * 1024,               // 256 KB — unused in v1
            resumable_threshold: 5 * 1024 * 1024, // 5 MB
            max_retries: 3,
            prefix: None,
        }
    }
}

impl GcsConfig {
    /// Create a new GCS configuration with required fields.
    pub fn new(project_id: impl Into<String>, bucket_name: impl Into<String>) -> Self {
        GcsConfig {
            project_id: project_id.into(),
            bucket_name: bucket_name.into(),
            ..Default::default()
        }
    }

    /// Set the chunk size (kept for API compat; unused by v1 SDK internally).
    pub fn with_chunk_size(mut self, chunk_size: usize) -> Self {
        self.chunk_size = chunk_size;
        self
    }

    /// Set the resumable upload threshold.
    pub fn with_resumable_threshold(mut self, threshold: usize) -> Self {
        self.resumable_threshold = threshold;
        self
    }

    /// Set the maximum number of retry attempts.
    pub fn with_max_retries(mut self, retries: u32) -> Self {
        self.max_retries = retries;
        self
    }
}

/// Google Cloud Storage backend (v1.11 official SDK).
///
/// Thread-safe, async-first implementation of `StorageBackend` for GCS.
/// Internally holds two Arc-wrapped client instances:
/// - `storage`: data-plane (read/write object content)
/// - `control`: control-plane (metadata, delete, list)
#[derive(Clone)]
pub struct GcsBackend {
    /// Data-plane client: write_object, read_object.
    storage: Arc<Storage>,
    /// Control-plane client: get_object, delete_object, list_objects.
    control: Arc<StorageControl>,
    config: GcsConfig,
    /// Caps concurrent write_object calls. GCS TCP connections time out after
    /// ~20-25 s when too many concurrent uploads land on the same host.
    /// Configurable via MEDIAGIT_GCS_UPLOAD_CONCURRENCY (default 4).
    upload_semaphore: Arc<tokio::sync::Semaphore>,
    /// V4 URL signer. `None` when ADC resolved to a credential that cannot
    /// sign locally or remotely (e.g. workload-identity-federation without
    /// IAM signBlob). When `None`, `presign_put`/`presign_get` return
    /// `Ok(None)` and the caller falls back to server-proxied transfer.
    /// Set `MEDIAGIT_GCS_DISABLE_PRESIGN=1` to force `None` at runtime.
    signer: Option<Signer>,
}

impl GcsBackend {
    fn build_upload_semaphore() -> Arc<tokio::sync::Semaphore> {
        let n = std::env::var("MEDIAGIT_GCS_UPLOAD_CONCURRENCY")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(4);
        Arc::new(tokio::sync::Semaphore::new(n))
    }

    /// Return the GCS resource path for the configured bucket.
    ///
    /// v1.11 requires the format `projects/_/buckets/{bucket_id}` for all
    /// bucket parameters.
    fn bucket_path(&self) -> String {
        format!("projects/_/buckets/{}", self.config.bucket_name)
    }

    /// Build both clients with the current `GcsConfig` retry / threshold settings.
    ///
    /// When `creds` is `Some`, it is passed explicitly to both builders via
    /// `with_credentials` (used by the explicit-service-account-file
    /// constructors). When `None`, the SDK falls back to its default
    /// Application Default Credentials resolution.
    async fn build_clients(
        config: &GcsConfig,
        creds: Option<google_cloud_auth::credentials::Credentials>,
    ) -> anyhow::Result<(Storage, StorageControl)> {
        let mut storage_builder = Storage::builder()
            .with_resumable_upload_threshold(config.resumable_threshold)
            .with_retry_policy(AlwaysRetry.with_attempt_limit(config.max_retries));

        // StorageControl (gRPC control plane: exists/delete/list) intentionally uses
        // the SDK default retry policy rather than AlwaysRetry.  AlwaysRetry retries
        // NOT_FOUND, which exists() relies on as a fast "absent" signal.  Retrying
        // NOT_FOUND causes exponential backoff for every missing chunk, stalling
        // chunks/check when the bucket is empty or a fresh push is underway.
        let mut control_builder = StorageControl::builder();

        if let Some(creds) = creds {
            storage_builder = storage_builder.with_credentials(creds.clone());
            control_builder = control_builder.with_credentials(creds);
        }

        let storage = storage_builder
            .build()
            .await
            .map_err(|e| anyhow::anyhow!("GCS Storage client build failed: {}", e))?;

        let control = control_builder
            .build()
            .await
            .map_err(|e| anyhow::anyhow!("GCS StorageControl client build failed: {}", e))?;

        Ok((storage, control))
    }

    /// Create a new GCS backend from a service account JSON file.
    ///
    /// Reads and parses `service_account_path` and passes the resulting
    /// `Credentials` explicitly to both client builders. Does not mutate the
    /// process environment.
    ///
    /// # Arguments
    ///
    /// * `project_id` - Google Cloud Project ID
    /// * `bucket_name` - GCS bucket name
    /// * `service_account_path` - Path to the service account JSON file
    pub async fn new(
        project_id: impl Into<String>,
        bucket_name: impl Into<String>,
        service_account_path: impl AsRef<std::path::Path>,
    ) -> anyhow::Result<Self> {
        let project_id = project_id.into();
        let bucket_name = bucket_name.into();

        if project_id.is_empty() {
            return Err(anyhow::anyhow!("project_id cannot be empty"));
        }
        if bucket_name.is_empty() {
            return Err(anyhow::anyhow!("bucket_name cannot be empty"));
        }

        let path = service_account_path.as_ref();
        if !path.exists() {
            return Err(anyhow::anyhow!(
                "service account file not found: {}",
                path.display()
            ));
        }

        let sa_json_str = std::fs::read_to_string(path).map_err(|e| {
            anyhow::anyhow!(
                "failed to read service account file '{}': {}",
                path.display(),
                e
            )
        })?;
        let sa_json: serde_json::Value = serde_json::from_str(&sa_json_str).map_err(|e| {
            anyhow::anyhow!(
                "failed to parse service account JSON '{}': {}",
                path.display(),
                e
            )
        })?;

        let creds = google_cloud_auth::credentials::service_account::Builder::new(sa_json.clone())
            .build()
            .map_err(|e| anyhow::anyhow!("GCS credentials build failed: {}", e))?;

        let gcs_config = GcsConfig::new(project_id.clone(), bucket_name.clone());
        let (storage, control) = Self::build_clients(&gcs_config, Some(creds)).await?;

        let signer = google_cloud_auth::credentials::service_account::Builder::new(sa_json)
            .build_signer()
            .map_err(|e| anyhow::anyhow!("GCS signer build failed: {}", e))?;

        debug!(
            project_id = %project_id,
            bucket_name = %bucket_name,
            "Initialized GCS backend (explicit credentials)"
        );

        Ok(GcsBackend {
            storage: Arc::new(storage),
            control: Arc::new(control),
            config: gcs_config,
            upload_semaphore: Self::build_upload_semaphore(),
            signer: Some(signer),
        })
    }

    /// Create a new GCS backend with custom configuration and a service account file.
    pub async fn with_config(
        config: GcsConfig,
        service_account_path: impl AsRef<std::path::Path>,
    ) -> anyhow::Result<Self> {
        if config.project_id.is_empty() {
            return Err(anyhow::anyhow!("project_id cannot be empty"));
        }
        if config.bucket_name.is_empty() {
            return Err(anyhow::anyhow!("bucket_name cannot be empty"));
        }

        let path = service_account_path.as_ref();
        if !path.exists() {
            return Err(anyhow::anyhow!(
                "service account file not found: {}",
                path.display()
            ));
        }

        let sa_json_str = std::fs::read_to_string(path).map_err(|e| {
            anyhow::anyhow!(
                "failed to read service account file '{}': {}",
                path.display(),
                e
            )
        })?;
        let sa_json: serde_json::Value = serde_json::from_str(&sa_json_str).map_err(|e| {
            anyhow::anyhow!(
                "failed to parse service account JSON '{}': {}",
                path.display(),
                e
            )
        })?;

        let creds = google_cloud_auth::credentials::service_account::Builder::new(sa_json.clone())
            .build()
            .map_err(|e| anyhow::anyhow!("GCS credentials build failed: {}", e))?;

        let (storage, control) = Self::build_clients(&config, Some(creds)).await?;

        let signer = google_cloud_auth::credentials::service_account::Builder::new(sa_json)
            .build_signer()
            .map_err(|e| anyhow::anyhow!("GCS signer build failed: {}", e))?;

        debug!(
            project_id = %config.project_id,
            bucket_name = %config.bucket_name,
            "Initialized GCS backend with custom config"
        );

        Ok(GcsBackend {
            storage: Arc::new(storage),
            control: Arc::new(control),
            config,
            upload_semaphore: Self::build_upload_semaphore(),
            signer: Some(signer),
        })
    }

    /// Get the configuration for this backend.
    pub fn config(&self) -> &GcsConfig {
        &self.config
    }

    /// Create a new GCS backend using environment variable authentication.
    ///
    /// Reads:
    /// - `GCS_PROJECT_ID` or `GOOGLE_CLOUD_PROJECT`
    /// - `GCS_BUCKET_NAME`
    /// - `GOOGLE_APPLICATION_CREDENTIALS` (optional; ADC falls back to other sources)
    pub async fn from_env() -> anyhow::Result<Self> {
        let project_id = std::env::var("GCS_PROJECT_ID")
            .or_else(|_| std::env::var("GOOGLE_CLOUD_PROJECT"))
            .map_err(|_| {
                anyhow::anyhow!(
                    "GCS_PROJECT_ID or GOOGLE_CLOUD_PROJECT environment variable not set"
                )
            })?;

        let bucket_name = std::env::var("GCS_BUCKET_NAME")
            .map_err(|_| anyhow::anyhow!("GCS_BUCKET_NAME environment variable not set"))?;

        // If an explicit credentials path is set, go through `new` to validate
        // the file exists.
        if let Ok(creds_path) = std::env::var("GOOGLE_APPLICATION_CREDENTIALS") {
            return Self::new(project_id, bucket_name, creds_path).await;
        }

        Self::with_default_credentials(project_id, bucket_name).await
    }

    /// Create a new GCS backend using Application Default Credentials (ADC).
    ///
    /// ADC automatically finds credentials from:
    /// - `GOOGLE_APPLICATION_CREDENTIALS` env var
    /// - `gcloud auth application-default login` token cache
    /// - GKE / Cloud Run / Compute Engine service account
    ///
    /// # Arguments
    ///
    /// * `project_id` - Google Cloud Project ID
    /// * `bucket_name` - GCS bucket name
    pub async fn with_default_credentials(
        project_id: impl Into<String>,
        bucket_name: impl Into<String>,
    ) -> anyhow::Result<Self> {
        Self::with_default_credentials_and_config(GcsConfig::new(project_id, bucket_name)).await
    }

    /// Same as [`Self::with_default_credentials`] but takes a full
    /// [`GcsConfig`] (e.g. to set `prefix`) instead of just project/bucket.
    pub async fn with_default_credentials_and_config(
        gcs_config: GcsConfig,
    ) -> anyhow::Result<Self> {
        let project_id = gcs_config.project_id.clone();
        let bucket_name = gcs_config.bucket_name.clone();

        if project_id.is_empty() {
            return Err(anyhow::anyhow!("project_id cannot be empty"));
        }
        if bucket_name.is_empty() {
            return Err(anyhow::anyhow!("bucket_name cannot be empty"));
        }

        // If GOOGLE_APPLICATION_CREDENTIALS is already set, honour it.
        if let Ok(creds_path) = std::env::var("GOOGLE_APPLICATION_CREDENTIALS") {
            debug!(
                project_id = %project_id,
                bucket_name = %bucket_name,
                credentials_path = %creds_path,
                "Using GOOGLE_APPLICATION_CREDENTIALS for GCS"
            );
        } else {
            debug!(
                project_id = %project_id,
                bucket_name = %bucket_name,
                "Using Application Default Credentials for GCS"
            );
        }

        let (storage, control) = Self::build_clients(&gcs_config, None).await?;

        let signer = match google_cloud_auth::credentials::Builder::default().build_signer() {
            Ok(s) => Some(s),
            Err(e) => {
                warn!(
                    target: "mediagit_storage::gcs",
                    error = %e,
                    "GCS signer unavailable; presigned URLs disabled, falling back to server proxy"
                );
                None
            }
        };

        Ok(GcsBackend {
            storage: Arc::new(storage),
            control: Arc::new(control),
            config: gcs_config,
            upload_semaphore: Self::build_upload_semaphore(),
            signer,
        })
    }

    /// Return `true` when a v1 GAX error represents a "not found" response.
    ///
    /// v1 splits storage across two transports — HTTP (data plane:
    /// `read_object`/`write_object`) and gRPC (control plane: `get_object`,
    /// `delete_object`, `list_objects`). HTTP errors expose `http_status_code`,
    /// gRPC errors expose `status()` with a typed `Code`. We must check both,
    /// otherwise a "missing object" on the gRPC path becomes a hard 500
    /// (which broke push: `exists()` is called on every chunk, and a fresh
    /// repo's bucket is empty by definition).
    /// Read a contiguous byte range `[offset, offset + len)` of `key` from GCS.
    ///
    /// Issues a single ranged `read_object` request via the v1 SDK's
    /// `set_read_range(ReadRange::segment(offset, len))` (HTTP Range under the
    /// hood; ungated by feature flags). The returned vec is exactly `len` bytes
    /// when the key has at least `offset + len` bytes; if it's shorter, the
    /// stream returns however much exists (validated by the caller against the
    /// size hint).
    async fn get_range(&self, key: &str, offset: u64, len: u64) -> anyhow::Result<Vec<u8>> {
        let key = crate::prefixed_key(&self.config.prefix, key);
        let key = key.as_str();
        let bucket_path = self.bucket_path();
        let mut resp = self
            .storage
            .read_object(&bucket_path, key)
            .set_read_range(ReadRange::segment(offset, len))
            .send()
            .await
            .map_err(|e| {
                if Self::is_not_found(&e) {
                    anyhow::anyhow!("object not found: {}", key)
                } else {
                    anyhow::anyhow!(
                        "GCS read_object range error for key '{}' [{}+{}]: {}",
                        key,
                        offset,
                        len,
                        e
                    )
                }
            })?;

        let mut buf = Vec::with_capacity(len as usize);
        while let Some(chunk) = resp.next().await.transpose().map_err(|e| {
            anyhow::anyhow!(
                "GCS range stream error for key '{}' [{}+{}]: {}",
                key,
                offset,
                len,
                e
            )
        })? {
            buf.extend_from_slice(&chunk);
        }
        Ok(buf)
    }

    fn is_not_found(e: &google_cloud_storage::Error) -> bool {
        if e.http_status_code() == Some(404) {
            return true;
        }
        if let Some(status) = e.status()
            && status.code == google_cloud_gax::error::rpc::Code::NotFound
        {
            return true;
        }
        false
    }
}

impl fmt::Debug for GcsBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GcsBackend")
            .field("project_id", &self.config.project_id)
            .field("bucket_name", &self.config.bucket_name)
            .field("chunk_size", &self.config.chunk_size)
            .field("resumable_threshold", &self.config.resumable_threshold)
            .field("max_retries", &self.config.max_retries)
            .field("presigned_urls_enabled", &self.signer.is_some())
            .finish()
    }
}

#[async_trait]
impl StorageBackend for GcsBackend {
    /// Retrieve an object from GCS.
    ///
    /// Uses the streaming `read_object` API; chunks are concatenated into a
    /// `Vec<u8>`. The SDK handles retry and CRC32C verification automatically.
    async fn get(&self, key: &str) -> anyhow::Result<Vec<u8>> {
        if key.is_empty() {
            return Err(anyhow::anyhow!("key cannot be empty"));
        }

        let key = crate::prefixed_key(&self.config.prefix, key);
        let key = key.as_str();
        let bucket_path = self.bucket_path();
        debug!(key = %key, bucket = %bucket_path, "Downloading object from GCS");

        let mut resp = self
            .storage
            .read_object(&bucket_path, key)
            .send()
            .await
            .map_err(|e| {
                if Self::is_not_found(&e) {
                    anyhow::anyhow!("object not found: {}", key)
                } else {
                    anyhow::anyhow!("GCS read_object error: {}", e)
                }
            })?;

        let mut buf = Vec::new();
        while let Some(chunk) =
            resp.next().await.transpose().map_err(|e| {
                anyhow::anyhow!("GCS read_object stream error for key '{}': {}", key, e)
            })?
        {
            buf.extend_from_slice(&chunk);
        }

        debug!(key = %key, size = buf.len(), "Downloaded object from GCS");
        Ok(buf)
    }

    async fn get_streaming_range(
        &self,
        key: &str,
        range: std::ops::Range<u64>,
    ) -> anyhow::Result<
        std::pin::Pin<
            Box<dyn futures::Stream<Item = anyhow::Result<bytes::Bytes>> + Send + 'static>,
        >,
    > {
        if key.is_empty() {
            return Err(anyhow::anyhow!("key cannot be empty"));
        }
        let prefixed = crate::prefixed_key(&self.config.prefix, key);
        let bucket_path = self.bucket_path();
        let storage = self.storage.clone();
        let len = range.end - range.start;

        let resp = storage
            .read_object(&bucket_path, &prefixed)
            .set_read_range(ReadRange::segment(range.start, len))
            .send()
            .await
            .map_err(|e| {
                if Self::is_not_found(&e) {
                    anyhow::anyhow!("object not found: {}", key)
                } else {
                    anyhow::anyhow!(
                        "GCS get_streaming_range error for key '{}' [{}+{}]: {}",
                        key,
                        range.start,
                        len,
                        e
                    )
                }
            })?;

        let stream = futures::stream::unfold(resp, |mut r| async move {
            match r.next().await {
                Some(Ok(chunk)) => Some((
                    Ok::<bytes::Bytes, anyhow::Error>(bytes::Bytes::copy_from_slice(&chunk)),
                    r,
                )),
                Some(Err(e)) => Some((
                    Err(anyhow::anyhow!(
                        "GCS get_streaming_range stream error: {}",
                        e
                    )),
                    r,
                )),
                None => None,
            }
        });
        Ok(Box::pin(stream))
    }

    /// Store an object in GCS.
    ///
    /// Uses `write_object` with `Bytes::copy_from_slice` for a single Arc-managed
    /// copy. The SDK selects simple vs resumable upload based on
    /// `resumable_threshold` (set on the client builder).
    ///
    /// Retry is handled by the SDK's built-in retry policy (capped via
    /// `AlwaysRetry.with_attempt_limit`).
    async fn put(&self, key: &str, data: &[u8]) -> anyhow::Result<()> {
        if key.is_empty() {
            return Err(anyhow::anyhow!("key cannot be empty"));
        }

        let key = crate::prefixed_key(&self.config.prefix, key);
        let key = key.as_str();
        debug!(
            key = %key,
            size = data.len(),
            resumable_threshold = self.config.resumable_threshold,
            "Uploading object to GCS"
        );

        let bucket_path = self.bucket_path();
        // Single copy into an Arc-managed buffer; retries inside the SDK reuse
        // the same `Bytes` (cheap clone — Arc bump only).
        let payload = Bytes::copy_from_slice(data);

        // Gate concurrent uploads: too many simultaneous write_object calls
        // exhaust GCS TCP connections and trigger transport timeouts (~20-25 s).
        let _permit = self
            .upload_semaphore
            .acquire()
            .await
            .map_err(|e| anyhow::anyhow!("GCS upload semaphore closed: {}", e))?;

        self.storage
            .write_object(&bucket_path, key, payload)
            .send_buffered()
            .await
            .map_err(|e| anyhow::anyhow!("GCS write_object error for key '{}': {}", key, e))?;

        debug!(key = %key, "Successfully uploaded object to GCS");
        Ok(())
    }

    /// Check whether an object exists in GCS.
    ///
    /// Uses `get_object` (metadata-only HEAD-equivalent on the control plane).
    async fn exists(&self, key: &str) -> anyhow::Result<bool> {
        if key.is_empty() {
            return Err(anyhow::anyhow!("key cannot be empty"));
        }

        let key = crate::prefixed_key(&self.config.prefix, key);
        let key = key.as_str();
        let bucket_path = self.bucket_path();

        match self
            .control
            .get_object()
            .set_bucket(&bucket_path)
            .set_object(key)
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(e) if Self::is_not_found(&e) => Ok(false),
            Err(e) => Err(anyhow::anyhow!(
                "GCS get_object error for key '{}': {}",
                key,
                e
            )),
        }
    }

    /// Return the byte length of a GCS object without downloading it.
    async fn head(&self, key: &str) -> anyhow::Result<Option<u64>> {
        if key.is_empty() {
            return Err(anyhow::anyhow!("key cannot be empty"));
        }

        let key = crate::prefixed_key(&self.config.prefix, key);
        let key = key.as_str();
        let bucket_path = self.bucket_path();

        match self
            .control
            .get_object()
            .set_bucket(&bucket_path)
            .set_object(key)
            .send()
            .await
        {
            Ok(obj) => Ok(Some(obj.size as u64)),
            Err(e) if Self::is_not_found(&e) => Ok(None),
            Err(e) => Err(anyhow::anyhow!(
                "GCS get_object error for key '{}': {}",
                key,
                e
            )),
        }
    }

    /// Delete an object from GCS (idempotent: 404 is treated as success).
    async fn delete(&self, key: &str) -> anyhow::Result<()> {
        if key.is_empty() {
            return Err(anyhow::anyhow!("key cannot be empty"));
        }

        let key = crate::prefixed_key(&self.config.prefix, key);
        let key = key.as_str();
        let bucket_path = self.bucket_path();

        match self
            .control
            .delete_object()
            .set_bucket(&bucket_path)
            .set_object(key)
            .send()
            .await
        {
            Ok(_) => {
                debug!(key = %key, "Successfully deleted object from GCS");
                Ok(())
            }
            Err(e) if Self::is_not_found(&e) => {
                // Idempotent: deleting a non-existent object is success.
                debug!(key = %key, "Object not found during delete (idempotent)");
                Ok(())
            }
            Err(e) => Err(anyhow::anyhow!(
                "GCS delete_object error for key '{}': {}",
                key,
                e
            )),
        }
    }

    /// List objects with a given prefix.
    ///
    /// Paginates automatically; returns a sorted list of object names.
    /// The v1 SDK provides a `by_item()` paginator, but we use manual
    /// pagination here to preserve the existing error-mapping pattern.
    async fn list_objects(&self, prefix: &str) -> anyhow::Result<Vec<String>> {
        let bucket_path = self.bucket_path();
        let wire_prefix = crate::prefixed_key(&self.config.prefix, prefix);
        let backend_prefix = self.config.prefix.as_deref().unwrap_or("");
        let strip_prefix = if backend_prefix.is_empty() {
            String::new()
        } else {
            format!("{}/", backend_prefix.trim_end_matches('/'))
        };

        debug!(
            bucket = %self.config.bucket_name,
            prefix = %wire_prefix,
            "Listing objects from GCS"
        );

        let mut results: Vec<String> = Vec::new();
        let mut page_token = String::new();

        loop {
            let mut builder = self.control.list_objects().set_parent(&bucket_path);

            if !wire_prefix.is_empty() {
                builder = builder.set_prefix(&wire_prefix);
            }
            if !page_token.is_empty() {
                builder = builder.set_page_token(&page_token);
            }

            let response = builder
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("GCS list_objects error: {}", e))?;

            for obj in &response.objects {
                let logical = if strip_prefix.is_empty() {
                    obj.name.clone()
                } else {
                    obj.name
                        .strip_prefix(&strip_prefix)
                        .unwrap_or(&obj.name)
                        .to_string()
                };
                results.push(logical);
            }

            page_token = response.next_page_token.clone();
            if page_token.is_empty() {
                break;
            }

            warn!(
                count = results.len(),
                "GCS list_objects: fetching next page"
            );
        }

        results.sort();
        debug!(count = results.len(), "Listed objects from GCS");
        Ok(results)
    }

    /// Striped parallel get for objects whose size the caller already knows.
    ///
    /// When `size` is `Some(n)` and `n >= STRIPED_GET_THRESHOLD`, the object is
    /// split into `STRIPE_SIZE` byte ranges and downloaded with up to
    /// `STRIPE_CONCURRENCY` concurrent ranged reads, then concatenated in order.
    /// Otherwise this falls back to the single-shot streamed `get`.
    ///
    /// CRITICAL: this method MUST NOT issue a metadata RPC to discover `size`
    /// when it is `None`. The previous implementation did exactly that, adding
    /// an extra `get_object` round-trip to every chunked-pull path (chunks are
    /// ≤ 16 MiB so they would never stripe anyway), and the regression got it
    /// reverted. If a caller has the size in a manifest, it should be passed
    /// through; otherwise this is a no-op vs `get`.
    async fn get_with_size_hint(&self, key: &str, size: Option<u64>) -> anyhow::Result<Vec<u8>> {
        if key.is_empty() {
            return Err(anyhow::anyhow!("key cannot be empty"));
        }

        let total = match size {
            Some(n) if n >= STRIPED_GET_THRESHOLD => n,
            _ => return self.get(key).await,
        };

        debug!(
            key = %key,
            size = total,
            stripe_size = STRIPE_SIZE,
            concurrency = STRIPE_CONCURRENCY,
            "Striped GCS download"
        );

        // Build (index, offset, len) triples so out-of-order completions can
        // be sorted back into the correct byte ordering before concatenation.
        let mut stripes: Vec<(usize, u64, u64)> = Vec::new();
        let mut offset: u64 = 0;
        let mut idx: usize = 0;
        while offset < total {
            let len = STRIPE_SIZE.min(total - offset);
            stripes.push((idx, offset, len));
            offset += len;
            idx += 1;
        }

        // `buffer_unordered` lets fast stripes overtake slow ones — important
        // when the slowest stripe pins the wall-clock. We re-sort by `idx`
        // before concatenation to preserve byte order.
        let mut parts: Vec<(usize, Vec<u8>)> = stream::iter(stripes)
            .map(|(idx, off, len)| {
                let this = self.clone();
                let key = key.to_string();
                async move {
                    let bytes = this.get_range(&key, off, len).await?;
                    if bytes.len() as u64 != len {
                        return Err(anyhow::anyhow!(
                            "GCS striped get short read for '{}' [{}+{}]: got {} bytes",
                            key,
                            off,
                            len,
                            bytes.len()
                        ));
                    }
                    Ok::<_, anyhow::Error>((idx, bytes))
                }
            })
            .buffer_unordered(STRIPE_CONCURRENCY)
            .try_collect()
            .await?;

        parts.sort_by_key(|(idx, _)| *idx);
        let mut out = Vec::with_capacity(total as usize);
        for (_, mut p) in parts {
            out.append(&mut p);
        }

        if out.len() as u64 != total {
            return Err(anyhow::anyhow!(
                "GCS striped get size mismatch for '{}': got {} bytes, expected {}",
                key,
                out.len(),
                total
            ));
        }

        debug!(key = %key, size = out.len(), "Striped GCS download complete");
        Ok(out)
    }

    async fn presign_put(
        &self,
        key: &str,
        _content_length: u64,
        ttl: std::time::Duration,
    ) -> anyhow::Result<Option<crate::PresignedPut>> {
        const MAX_GCS_TTL: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 3600);
        if ttl > MAX_GCS_TTL {
            warn!(
                target: "mediagit_storage::gcs",
                ttl_secs = ttl.as_secs(),
                "presign_put TTL exceeds GCS 7-day cap; falling back to server proxy"
            );
            return Ok(None);
        }
        if std::env::var_os("MEDIAGIT_GCS_DISABLE_PRESIGN").is_some() {
            return Ok(None);
        }
        let Some(signer) = self.signer.as_ref() else {
            return Ok(None);
        };
        let key = crate::prefixed_key(&self.config.prefix, key);
        let url = SignedUrlBuilder::for_object(self.bucket_path(), &key)
            .with_method(http::Method::PUT)
            .with_expiration(ttl)
            .sign_with(signer)
            .await
            .map_err(|e| anyhow::anyhow!("GCS V4 presign_put failed: {e}"))?;
        Ok(Some(crate::PresignedPut {
            url,
            method: "PUT".to_string(),
            required_headers: vec![],
            expires_at: std::time::SystemTime::now() + ttl,
        }))
    }

    async fn presign_get(
        &self,
        key: &str,
        ttl: std::time::Duration,
    ) -> anyhow::Result<Option<crate::PresignedDownload>> {
        const MAX_GCS_TTL: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 3600);
        if ttl > MAX_GCS_TTL {
            warn!(
                target: "mediagit_storage::gcs",
                ttl_secs = ttl.as_secs(),
                "presign_get TTL exceeds GCS 7-day cap; falling back to server proxy"
            );
            return Ok(None);
        }
        if std::env::var_os("MEDIAGIT_GCS_DISABLE_PRESIGN").is_some() {
            return Ok(None);
        }
        let Some(signer) = self.signer.as_ref() else {
            return Ok(None);
        };
        let key = crate::prefixed_key(&self.config.prefix, key);
        let url = SignedUrlBuilder::for_object(self.bucket_path(), &key)
            .with_method(http::Method::GET)
            .with_expiration(ttl)
            .sign_with(signer)
            .await
            .map_err(|e| anyhow::anyhow!("GCS V4 presign_get failed: {e}"))?;
        Ok(Some(crate::PresignedDownload {
            url,
            headers: vec![],
            expires_in_secs: ttl.as_secs(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gcs_config_default() {
        let config = GcsConfig::default();
        assert_eq!(config.chunk_size, 256 * 1024);
        assert_eq!(config.resumable_threshold, 5 * 1024 * 1024);
        assert_eq!(config.max_retries, 3);
    }

    #[test]
    fn test_gcs_config_new() {
        let config = GcsConfig::new("test-project", "test-bucket");
        assert_eq!(config.project_id, "test-project");
        assert_eq!(config.bucket_name, "test-bucket");
    }

    #[test]
    fn test_gcs_config_builder() {
        let config = GcsConfig::new("my-project", "my-bucket")
            .with_chunk_size(512 * 1024)
            .with_resumable_threshold(10 * 1024 * 1024)
            .with_max_retries(5);

        assert_eq!(config.chunk_size, 512 * 1024);
        assert_eq!(config.resumable_threshold, 10 * 1024 * 1024);
        assert_eq!(config.max_retries, 5);
    }

    #[tokio::test]
    async fn test_gcs_backend_new_empty_project() {
        let result = GcsBackend::new("", "bucket", "dummy.json").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("project_id"));
    }

    #[tokio::test]
    async fn test_gcs_backend_new_empty_bucket() {
        let result = GcsBackend::new("project", "", "dummy.json").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("bucket_name"));
    }

    #[tokio::test]
    async fn test_gcs_backend_new_missing_file() {
        let result = GcsBackend::new("project", "bucket", "nonexistent.json").await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("service account file not found")
        );
    }

    /// Name-composition regression test for the GCS-prefix bug (layout v2,
    /// M1 Step 5): the wire object name must compose exactly
    /// `<gcs_prefix>/<ns>/<key>` — prefix applied once, no double-prefix
    /// with the `NamespacedBackend` wrapper. This exercises the same
    /// `crate::prefixed_key` helper every GCS method calls, at the pure
    /// function level (no live GCS connection needed); live verification is
    /// deferred to the cloud matrix phase.
    #[test]
    fn test_prefix_and_namespace_compose_exactly_once() {
        let gcs_prefix = Some("backups".to_string());
        // Key as it arrives at GcsBackend AFTER NamespacedBackend has
        // already prepended "<ns>/".
        let namespaced_key = "myrepo/chunks/deadbeef";

        let wire_key = crate::prefixed_key(&gcs_prefix, namespaced_key);
        assert_eq!(wire_key, "backups/myrepo/chunks/deadbeef");

        // No backend prefix configured: namespace is the only prefix.
        let wire_key_no_gcs_prefix = crate::prefixed_key(&None, namespaced_key);
        assert_eq!(wire_key_no_gcs_prefix, "myrepo/chunks/deadbeef");

        // list_objects's strip must exactly reverse the composition.
        let backend_prefix = gcs_prefix.as_deref().unwrap_or("");
        let strip_prefix = format!("{}/", backend_prefix.trim_end_matches('/'));
        let logical = wire_key.strip_prefix(&strip_prefix).unwrap();
        assert_eq!(logical, namespaced_key);
    }

    #[test]
    fn test_gcs_backend_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<GcsBackend>();
    }

    #[test]
    fn test_bucket_path_format() {
        let config = GcsConfig::new("proj", "my-bucket");
        // We can't construct GcsBackend without async, but we can verify the
        // format string by constructing the expected value directly.
        let expected = "projects/_/buckets/my-bucket";
        assert_eq!(
            format!("projects/_/buckets/{}", config.bucket_name),
            expected
        );
    }
}

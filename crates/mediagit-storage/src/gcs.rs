// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

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
//! `StorageControl` (gRPC control plane: exists/delete/list) uses
//! [`ControlPlaneRetry`], a policy written for this exact call pattern.
//! `AlwaysRetry` must NOT be used here because it retries `NOT_FOUND`, which
//! `exists()` relies on as a fast "absent" signal - that caused exponential
//! back-off on every missing chunk and stalled `chunks/check` on a fresh
//! bucket.  But the SDK default (`Aip194Strict`) over-corrected in the other
//! direction: it treats anything that is not `Unavailable`, HTTP 503, or an io
//! error as permanent, so a gRPC `Cancelled` from a dropped connection is
//! never retried at all.  See [`ControlPlaneRetry`] for what that cost.
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
use google_cloud_gax::error::Error as GaxError;
use google_cloud_gax::error::rpc::Code;
use google_cloud_gax::retry_policy::{AlwaysRetry, RetryPolicy, RetryPolicyExt};
use google_cloud_gax::retry_result::RetryResult;
use google_cloud_gax::retry_state::RetryState;
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

        // StorageControl (gRPC control plane: exists/delete/list). NOT AlwaysRetry
        // - that retries NOT_FOUND, which exists() relies on as a fast "absent"
        // signal, and caused exponential backoff on every missing chunk. NOT the
        // SDK default either: Aip194Strict treats Cancelled as permanent, so a
        // dropped connection failed a 2,800-object push with zero retries on
        // 20260826-ga28. ControlPlaneRetry is permanent on NOT_FOUND and
        // transient on transport failures. See its doc comment.
        let mut control_builder = StorageControl::builder()
            .with_retry_policy(ControlPlaneRetry.with_attempt_limit(config.max_retries));

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
        let io_limit = gcs_io_deadline();
        let mut resp = with_io_deadline(
            io_limit,
            "read_object range request",
            self.storage
                .read_object(&bucket_path, key)
                .set_read_range(ReadRange::segment(offset, len))
                .send(),
        )
        .await?
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
        while let Some(chunk) = with_io_deadline(io_limit, "read_object range stream", resp.next())
            .await?
            .transpose()
            .map_err(|e| {
                anyhow::anyhow!(
                    "GCS range stream error for key '{}' [{}+{}]: {}",
                    key,
                    offset,
                    len,
                    e
                )
            })?
        {
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

/// Retry policy for the GCS **control plane** (`exists`/`delete`/`list`).
///
/// Neither stock policy fits this call pattern, and both failures are on
/// record:
///
/// * `AlwaysRetry` retries `NOT_FOUND`.  `exists()` uses `NOT_FOUND` as its
///   fast "absent" answer, so every missing chunk paid a full exponential
///   back-off and `chunks/check` stalled against a fresh bucket.  That is why
///   `AlwaysRetry` was removed from this client.
///
/// * `Aip194Strict` - the SDK default this then fell back to - retries only
///   `Unavailable`, HTTP 503, io errors, and pre-RPC transients.  Everything
///   else is permanent, **including `Cancelled`**.  On 20260826-ga28 a GCS
///   connection dropped mid-push and surfaced as
///   `Cancelled / tonic::transport::Error(hyper::Error(Canceled, "connection
///   closed"))`.  It was classified permanent, retried zero times, and failed
///   a push that had already written 2,800 objects.  local, minio, aws and
///   azure all passed the same drill; only GCS has a control plane on this
///   path.
///
/// So: `NOT_FOUND` is permanent (that is an answer, not a failure), and
/// transport-level failures are retried.  `Cancelled`, `Aborted`,
/// `DeadlineExceeded` and `Internal` all describe a connection that died
/// rather than a request that was refused, and a `get_object` is a read - safe
/// to repeat.  Anything else defers to `Aip194Strict` so this policy stays a
/// narrow amendment rather than a reimplementation.
///
/// Decorate with `.with_attempt_limit(...)` at the call site; this type does
/// not bound attempts itself.
#[derive(Clone, Debug)]
pub(crate) struct ControlPlaneRetry;

impl ControlPlaneRetry {
    /// Codes that mean "the connection failed", not "the server said no".
    fn is_transport_failure(code: Code) -> bool {
        matches!(
            code,
            Code::Cancelled | Code::Aborted | Code::DeadlineExceeded | Code::Internal
        )
    }
}

impl RetryPolicy for ControlPlaneRetry {
    fn on_error(&self, state: &RetryState, error: GaxError) -> RetryResult {
        if let Some(status) = error.status() {
            // NOT_FOUND is exists()'s answer. Retrying it is the bug this
            // policy exists to avoid re-introducing.
            if status.code == Code::NotFound {
                return RetryResult::Permanent(error);
            }
            if Self::is_transport_failure(status.code) {
                return RetryResult::Continue(error);
            }
        }
        if error.http_status_code() == Some(404) {
            return RetryResult::Permanent(error);
        }
        use google_cloud_gax::retry_policy::Aip194Strict;
        Aip194Strict.on_error(state, error)
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

/// Part size for a GCS XML-API multipart upload.
///
/// GCS's XML API is S3-compatible here, so the limits and the reasoning are the
/// same as `mpu_part_size_s3`: at most 10,000 parts, 5 MiB minimum except for
/// the last one, and a 16 MiB floor so media-sized objects do not turn into
/// thousands of tiny round trips. Kept as its own function rather than shared
/// with s3.rs so a future divergence in GCS's limits has somewhere to land.
fn mpu_part_size_gcs(total_size: u64) -> u64 {
    const MIN_PART: u64 = 5 * 1024 * 1024;
    const MAX_PART: u64 = 5 * 1024 * 1024 * 1024;
    const MAX_PARTS: u64 = 10_000;
    const TARGET_PARTS: u64 = 96;
    const FLOOR: u64 = 16 * 1024 * 1024;

    if let Some(v) = std::env::var("MEDIAGIT_MPU_PART_SIZE")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|&n| (MIN_PART..=MAX_PART).contains(&n))
    {
        return v;
    }
    let by_count = total_size.div_ceil(MAX_PARTS).max(MIN_PART);
    let by_target = total_size.div_ceil(TARGET_PARTS).max(MIN_PART);
    FLOOR.max(by_count).max(by_target).min(MAX_PART)
}

/// Pull `<UploadId>` out of an `InitiateMultipartUploadResult`.
///
/// Deliberately a string scan rather than an XML parser: the response is a
/// fixed four-element document defined by the XML API, this reads exactly one
/// element from it, and an XML dependency for that is not worth carrying. It
/// returns `None` rather than guessing if the element is absent, so a changed
/// response shape surfaces as a clean fallback to single PUT instead of an
/// upload against an empty id.
fn parse_upload_id(xml: &str) -> Option<String> {
    let start = xml.find("<UploadId>")? + "<UploadId>".len();
    let end = xml[start..].find("</UploadId>")? + start;
    let id = xml[start..end].trim();
    if id.is_empty() {
        None
    } else {
        Some(id.to_string())
    }
}

/// Build the `CompleteMultipartUpload` document.
///
/// ETags are echoed back exactly as the part PUTs returned them, quotes and
/// all; GCS compares them verbatim. XML-escaping the ETag matters because it is
/// server-supplied text going into a document, even though in practice it is a
/// quoted hex digest.
fn complete_mpu_xml(parts: &[crate::MpuCompletedPart]) -> String {
    let mut out = String::from("<CompleteMultipartUpload>");
    for p in parts {
        let etag = p.etag.replace('&', "&amp;").replace('<', "&lt;");
        out.push_str(&format!(
            "<Part><PartNumber>{}</PartNumber><ETag>{}</ETag></Part>",
            p.part_number, etag
        ));
    }
    out.push_str("</CompleteMultipartUpload>");
    out
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

        let req = self.storage.read_object(&bucket_path, key);
        let io_limit = gcs_io_deadline();
        let mut resp = with_io_deadline(io_limit, "read_object request", req.send())
            .await?
            .map_err(|e| {
                if Self::is_not_found(&e) {
                    anyhow::anyhow!("object not found: {}", key)
                } else {
                    anyhow::anyhow!("GCS read_object error: {}", e)
                }
            })?;

        let mut buf = Vec::new();
        while let Some(chunk) = with_io_deadline(io_limit, "read_object stream", resp.next())
            .await?
            .transpose()
            .map_err(|e| anyhow::anyhow!("GCS read_object stream error for key '{}': {}", key, e))?
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

        let io_limit = gcs_io_deadline();
        let resp = with_io_deadline(
            io_limit,
            "get_streaming_range request",
            storage
                .read_object(&bucket_path, &prefixed)
                .set_read_range(ReadRange::segment(range.start, len))
                .send(),
        )
        .await?
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

        let stream = futures::stream::unfold(resp, move |mut r| async move {
            match with_io_deadline(io_limit, "get_streaming_range stream", r.next()).await {
                Err(e) => Some((Err(e), r)),
                Ok(step) => match step {
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
                },
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

        match with_io_deadline(
            gcs_control_deadline(),
            "get_object (exists)",
            self.control
                .get_object()
                .set_bucket(&bucket_path)
                .set_object(key)
                .send(),
        )
        .await?
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

        match with_io_deadline(
            gcs_control_deadline(),
            "get_object (size)",
            self.control
                .get_object()
                .set_bucket(&bucket_path)
                .set_object(key)
                .send(),
        )
        .await?
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

        match with_io_deadline(
            gcs_control_deadline(),
            "delete_object",
            self.control
                .delete_object()
                .set_bucket(&bucket_path)
                .set_object(key)
                .send(),
        )
        .await?
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

            let response = with_io_deadline(gcs_control_deadline(), "list_objects", builder.send())
                .await?
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

    /// Presigned multipart upload over the GCS **XML API** (item 29).
    ///
    /// WHY THE XML API. The v1 SDK's native upload is *resumable*: one sequential
    /// session writing to a growing object, with no parallel byte-range upload
    /// and no per-part retry. S3-style multipart only exists on the XML API, and
    /// that API is what `SignedUrlBuilder` already signs -- it emits
    /// `https://storage.googleapis.com/<bucket>/<key>`, which is the XML
    /// endpoint.
    ///
    /// Without this, every GCS pack upload was one all-or-nothing PUT: a
    /// transport failure at 63 of 64 MiB re-sent all 64, and the client had to
    /// hold the whole pack in memory to be able to retry at all. Measured on a
    /// 10.03 GB push: 1,075.7 MB peak client working set on GCS against 365.2 MB
    /// on S3, which had this path. See FUTURE_TODOS item 33.
    ///
    /// Initiate, complete and abort are themselves presigned and then called
    /// unauthenticated, rather than carrying an OAuth token through this
    /// backend: it reuses the one signing path `presign_put` already uses, so
    /// there is a single place where GCS credentials turn into requests.
    ///
    /// `with_query_param("uploads", "")` serializes as `?uploads=`, not bare
    /// `?uploads`, because the signer runs every parameter through
    /// `form_urlencoded::append_pair`. Verified against the live XML API that
    /// both forms return an UploadId, so the signed form is accepted.
    async fn create_presigned_mpu(
        &self,
        key: &str,
        total_size: u64,
        ttl: std::time::Duration,
    ) -> anyhow::Result<Option<crate::PresignedMpu>> {
        const MAX_GCS_TTL: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 3600);
        if ttl > MAX_GCS_TTL || std::env::var_os("MEDIAGIT_GCS_DISABLE_PRESIGN").is_some() {
            return Ok(None);
        }
        if std::env::var_os("MEDIAGIT_GCS_DISABLE_MPU").is_some() {
            return Ok(None);
        }
        let Some(signer) = self.signer.as_ref() else {
            return Ok(None);
        };
        let key = crate::prefixed_key(&self.config.prefix, key);

        let init_url = SignedUrlBuilder::for_object(self.bucket_path(), &key)
            .with_method(http::Method::POST)
            .with_expiration(ttl)
            .with_query_param("uploads", "")
            .sign_with(signer)
            .await
            .map_err(|e| anyhow::anyhow!("GCS presign mpu initiate: {e}"))?;

        // Every failure below returns Ok(None), never Err: the caller treats
        // that as "this backend has no MPU" and falls back to a single PUT,
        // which still works. Turning a transient initiate failure into a hard
        // error would fail a push that had a working path available.
        let resp = match reqwest::Client::new()
            .post(&init_url)
            .header(reqwest::header::CONTENT_LENGTH, 0)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!(target: "mediagit_storage::gcs", error = %e, "GCS MPU initiate failed; falling back to single PUT");
                return Ok(None);
            }
        };
        if !resp.status().is_success() {
            warn!(target: "mediagit_storage::gcs", status = resp.status().as_u16(), "GCS MPU initiate non-2xx; falling back to single PUT");
            return Ok(None);
        }
        let body = resp.text().await.unwrap_or_default();
        let Some(upload_id) = parse_upload_id(&body) else {
            warn!(target: "mediagit_storage::gcs", "GCS MPU initiate returned no UploadId; falling back to single PUT");
            return Ok(None);
        };

        let part_size = mpu_part_size_gcs(total_size);
        let num_parts = total_size.div_ceil(part_size).max(1) as i32;
        let mut parts = Vec::with_capacity(num_parts as usize);
        for part_number in 1..=num_parts {
            let url = SignedUrlBuilder::for_object(self.bucket_path(), &key)
                .with_method(http::Method::PUT)
                .with_expiration(ttl)
                .with_query_param("partNumber", part_number.to_string())
                .with_query_param("uploadId", upload_id.clone())
                .sign_with(signer)
                .await
                .map_err(|e| anyhow::anyhow!("GCS presign mpu part {part_number}: {e}"))?;
            parts.push(crate::PresignedMpuPart { part_number, url });
        }

        debug!(
            target: "mediagit_storage::gcs",
            key = %key, parts = num_parts, part_size,
            "GCS presigned MPU created"
        );
        Ok(Some(crate::PresignedMpu {
            upload_id,
            parts,
            part_size,
        }))
    }

    async fn complete_presigned_mpu(
        &self,
        key: &str,
        upload_id: &str,
        parts: Vec<crate::MpuCompletedPart>,
    ) -> anyhow::Result<()> {
        let Some(signer) = self.signer.as_ref() else {
            anyhow::bail!("GCS complete_presigned_mpu called without a signer");
        };
        let key = crate::prefixed_key(&self.config.prefix, key);
        let url = SignedUrlBuilder::for_object(self.bucket_path(), &key)
            .with_method(http::Method::POST)
            .with_expiration(std::time::Duration::from_secs(3600))
            .with_query_param("uploadId", upload_id.to_string())
            .sign_with(signer)
            .await
            .map_err(|e| anyhow::anyhow!("GCS presign mpu complete: {e}"))?;

        let xml = complete_mpu_xml(&parts);
        let resp = reqwest::Client::new()
            .post(&url)
            .header(reqwest::header::CONTENT_TYPE, "application/xml")
            .body(xml)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("GCS mpu complete: {e}"))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("GCS mpu complete returned {status}: {}", body.trim());
        }
        // The XML API can answer 200 and still report failure in the body --
        // it keeps the connection open while finalizing. Treating that as
        // success would register a pack that does not exist.
        let body = resp.text().await.unwrap_or_default();
        if body.contains("<Error>") {
            anyhow::bail!("GCS mpu complete reported an error body: {}", body.trim());
        }
        Ok(())
    }

    async fn abort_presigned_mpu(&self, key: &str, upload_id: &str) -> anyhow::Result<()> {
        // Best effort, like s3.rs: an orphaned upload costs storage until the
        // bucket lifecycle reaps it, but failing the caller over a cleanup it
        // cannot act on is worse.
        let Some(signer) = self.signer.as_ref() else {
            return Ok(());
        };
        let key = crate::prefixed_key(&self.config.prefix, key);
        if let Ok(url) = SignedUrlBuilder::for_object(self.bucket_path(), &key)
            .with_method(http::Method::DELETE)
            .with_expiration(std::time::Duration::from_secs(3600))
            .with_query_param("uploadId", upload_id.to_string())
            .sign_with(signer)
            .await
        {
            let _ = reqwest::Client::new().delete(&url).send().await;
        }
        Ok(())
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

/// Per-IO deadline for GCS data-plane transfers, in seconds.
///
/// Generous by default because the failure mode it prevents is a hung read of
/// a whole repository, while the cost of being too generous is a genuine hang
/// taking longer to surface.
fn gcs_io_timeout_secs() -> u64 {
    parse_io_timeout_secs(std::env::var("MEDIAGIT_GCS_IO_TIMEOUT_SECS").ok())
}

/// Split from the env lookup so it is assertable: `#![forbid(unsafe_code)]` plus
/// edition 2024 make `set_var` an `unsafe` call, so a test that drove the real
/// variable could not be written without punching a hole in that. Same shape as
/// `azure::parse_io_timeout_secs`.
fn parse_io_timeout_secs(raw: Option<String>) -> u64 {
    const DEFAULT_IO_TIMEOUT_SECS: u64 = 120;
    raw.and_then(|v| v.trim().parse::<u64>().ok())
        // 0 would mean "deadline already passed" and fail every read instantly.
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_IO_TIMEOUT_SECS)
}

/// Await one GCS network step under the per-IO deadline.
///
/// Wraps BOTH the request `send()` and each `next()` on the response body,
/// because they hang independently: `send()` covers "the response never
/// starts", the stream covers "the response starts and then stalls mid-body".
/// The observed failure left an Established socket with zero bytes moving, so
/// bounding only one of the two would have left the other still able to wedge.
///
/// `secs` is a parameter rather than read from the env here so the deadline
/// behaviour is testable without mutating process environment.
async fn with_io_deadline<F, T>(limit: std::time::Duration, what: &str, fut: F) -> anyhow::Result<T>
where
    F: std::future::Future<Output = T>,
{
    tokio::time::timeout(limit, fut).await.map_err(|_| {
        let secs = limit.as_secs();
        anyhow::anyhow!(
            "GCS {what} stalled: no progress for {secs}s \
             (raise MEDIAGIT_GCS_IO_TIMEOUT_SECS if this link is legitimately slower)"
        )
    })
}

/// Deadline for GCS *control-plane* calls (`get_object`, `delete_object`,
/// `list_objects`), as distinct from the data-plane transfer deadline.
///
/// These are metadata round-trips that normally finish in well under a second,
/// so they get a tighter bound than a multi-GB transfer does: waiting the full
/// transfer deadline to discover one is wedged is dead time, and `exists` runs
/// once per chunk on the push path.
///
/// Not tighter than this, though — see the note at `upload_semaphore` about
/// concurrent uploads exhausting GCS TCP connections and producing ~20-25s
/// transport timeouts. A bound near that band would convert congested-but-
/// recoverable calls into hard errors. Fixed rather than env-tunable until
/// something demonstrates it needs to move.
fn gcs_control_deadline() -> std::time::Duration {
    std::time::Duration::from_secs(60)
}

/// The per-IO deadline as a `Duration`, ready to hand to [`with_io_deadline`].
fn gcs_io_deadline() -> std::time::Duration {
    std::time::Duration::from_secs(gcs_io_timeout_secs())
}

#[cfg(test)]
mod tests {

    // Regression guard for the 20260819-gagate11 hang: a GCS read that never
    // produces data must fail on a deadline rather than wedge forever. Without
    // `with_io_deadline` this test hangs instead of failing, which is exactly
    // what the stuck campaign did.
    #[tokio::test]
    async fn a_stalled_gcs_read_fails_on_the_deadline_instead_of_hanging() {
        let stalled = std::future::pending::<()>();
        let err =
            super::with_io_deadline(std::time::Duration::from_millis(10), "test read", stalled)
                .await
                .expect_err("a future that never resolves must hit the deadline");
        let msg = err.to_string();
        assert!(msg.contains("stalled"), "unhelpful message: {msg}");
        assert!(
            msg.contains("MEDIAGIT_GCS_IO_TIMEOUT_SECS"),
            "the message must name the knob that fixes it: {msg}"
        );
    }

    #[tokio::test]
    async fn a_read_that_completes_passes_its_value_through_untouched() {
        let got =
            super::with_io_deadline(std::time::Duration::from_secs(120), "test read", async {
                7u32
            })
            .await
            .expect("a ready future must not be timed out");
        assert_eq!(got, 7);
    }

    #[test]
    fn io_timeout_defaults_are_generous_enough_for_a_wan() {
        assert_eq!(super::parse_io_timeout_secs(None), 120);
        assert_eq!(
            super::parse_io_timeout_secs(Some("not-a-number".into())),
            120
        );
        assert!(super::parse_io_timeout_secs(None) > 10);
    }

    #[test]
    fn io_timeout_rejects_zero_and_honours_valid_overrides() {
        // 0 would make every read fail instantly rather than mean "no limit".
        assert_eq!(super::parse_io_timeout_secs(Some("0".into())), 120);
        assert_eq!(super::parse_io_timeout_secs(Some("45".into())), 45);
        assert_eq!(super::parse_io_timeout_secs(Some("  90  ".into())), 90);
    }

    use super::*;

    // ---- ControlPlaneRetry -------------------------------------------------
    //
    // BOTH HALVES ARE ASSERTED HERE ON PURPOSE. This policy exists because two
    // previous policies each got exactly one half right: `AlwaysRetry` retried
    // NOT_FOUND and stalled chunks/check, and `Aip194Strict` refused to retry a
    // dropped connection and failed a 2,800-object push. A test that only
    // proved "Cancelled retries" would have passed for `AlwaysRetry` too, and
    // would not have caught the regression that motivated removing it.
    mod control_plane_retry {
        use super::*;
        use google_cloud_gax::error::Error as GaxError;
        use google_cloud_gax::error::rpc::{Code, Status};
        use google_cloud_gax::retry_policy::RetryPolicy;
        use google_cloud_gax::retry_state::RetryState;

        fn verdict(code: Code) -> RetryResult {
            // idempotent = true: every control-plane call this policy guards
            // (get_object/exists, list) is a read.
            ControlPlaneRetry.on_error(
                &RetryState::new(true),
                GaxError::service(Status::default().set_code(code)),
            )
        }

        /// The half that `AlwaysRetry` got wrong. NOT_FOUND is `exists()`
        /// answering "absent" - retrying it bought exponential back-off on
        /// every missing chunk.
        #[test]
        fn not_found_is_permanent() {
            assert!(verdict(Code::NotFound).is_permanent());
        }

        /// The half that `Aip194Strict` got wrong, and the exact code seen on
        /// 20260826-ga28: a dropped gRPC connection surfaces as `Cancelled`,
        /// which AIP-194 classifies permanent.
        #[test]
        fn cancelled_is_retried() {
            assert!(matches!(verdict(Code::Cancelled), RetryResult::Continue(_)));
        }

        /// The other codes that describe a dead connection rather than a
        /// refused request.
        #[test]
        fn transport_failures_are_retried() {
            for code in [Code::Aborted, Code::DeadlineExceeded, Code::Internal] {
                assert!(
                    matches!(verdict(code), RetryResult::Continue(_)),
                    "{code:?} should be retried"
                );
            }
        }

        /// Unchanged from the SDK default - this policy is an amendment, not a
        /// replacement, so what AIP-194 already retried must keep retrying.
        #[test]
        fn unavailable_still_retried_via_aip194() {
            assert!(matches!(
                verdict(Code::Unavailable),
                RetryResult::Continue(_)
            ));
        }

        /// A genuine refusal must NOT be retried, or a misconfigured
        /// credential turns into `max_retries` rounds of back-off per object.
        #[test]
        fn permission_denied_is_permanent() {
            assert!(verdict(Code::PermissionDenied).is_permanent());
            assert!(verdict(Code::InvalidArgument).is_permanent());
        }
    }

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

#[cfg(test)]
mod gcs_mpu_tests {
    use super::{complete_mpu_xml, mpu_part_size_gcs, parse_upload_id};

    /// The real shape GCS returns from `POST ?uploads=`, captured from the live
    /// XML API rather than invented, so a changed response format shows up here.
    const REAL_INITIATE: &str = concat!(
        r#"<?xml version='1.0' encoding='UTF-8'?>"#,
        "<InitiateMultipartUploadResult xmlns=\"http://doc.s3.amazonaws.com/2006-03-01\">",
        "<Bucket>mediagit-dev2-storage</Bucket>",
        "<Key>probe/object.bin</Key>",
        "<UploadId>ABPnzm7tEXAMPLEuploadIDvalue</UploadId>",
        "</InitiateMultipartUploadResult>"
    );

    #[test]
    fn upload_id_is_read_from_a_real_initiate_response() {
        assert_eq!(
            parse_upload_id(REAL_INITIATE).as_deref(),
            Some("ABPnzm7tEXAMPLEuploadIDvalue")
        );
    }

    /// Absent or empty must be `None`, never `Some("")`. An empty upload id
    /// would be signed into part URLs and every part would fail against an
    /// upload that does not exist -- far worse than declining MPU and taking
    /// the single-PUT fallback, which is what `None` causes.
    #[test]
    fn a_missing_or_empty_upload_id_declines_rather_than_guesses() {
        assert_eq!(
            parse_upload_id("<Error><Code>AccessDenied</Code></Error>"),
            None
        );
        assert_eq!(parse_upload_id("<UploadId></UploadId>"), None);
        assert_eq!(parse_upload_id("<UploadId>   </UploadId>"), None);
        assert_eq!(parse_upload_id(""), None);
        // Truncated: opening tag with no close must not panic or slice wrongly.
        assert_eq!(parse_upload_id("<UploadId>abc"), None);
    }

    #[test]
    fn complete_document_lists_parts_in_order_with_etags() {
        let parts = vec![
            crate::MpuCompletedPart {
                part_number: 1,
                etag: "\"aaa\"".into(),
            },
            crate::MpuCompletedPart {
                part_number: 2,
                etag: "\"bbb\"".into(),
            },
        ];
        let xml = complete_mpu_xml(&parts);
        assert!(xml.starts_with("<CompleteMultipartUpload>"));
        assert!(xml.ends_with("</CompleteMultipartUpload>"));
        assert!(xml.contains("<PartNumber>1</PartNumber><ETag>\"aaa\"</ETag>"));
        assert!(xml.contains("<PartNumber>2</PartNumber><ETag>\"bbb\"</ETag>"));
        assert!(
            xml.find("<PartNumber>1<") < xml.find("<PartNumber>2<"),
            "parts must stay in ascending order"
        );
    }

    /// The ETag is server-supplied text going into a document. In practice it is
    /// a quoted hex digest, but escaping it is the difference between a
    /// well-formed request and a malformed one if that ever stops being true.
    #[test]
    fn etag_is_xml_escaped() {
        let parts = vec![crate::MpuCompletedPart {
            part_number: 1,
            etag: "a&b<c".into(),
        }];
        let xml = complete_mpu_xml(&parts);
        assert!(xml.contains("a&amp;b&lt;c"));
    }

    /// Part sizing has to satisfy GCS's XML-API limits, which match S3's: no
    /// more than 10,000 parts, and at least 5 MiB per part except the last.
    #[test]
    fn part_size_respects_the_limits_at_every_scale() {
        for total in [
            1u64,
            64 * 1024 * 1024,
            10 * 1024 * 1024 * 1024,
            5 * 1024u64.pow(4),
        ] {
            let ps = mpu_part_size_gcs(total);
            assert!(ps >= 5 * 1024 * 1024, "part {ps} under the 5 MiB minimum");
            assert!(
                ps <= 5 * 1024 * 1024 * 1024,
                "part {ps} over the 5 GiB maximum"
            );
            assert!(
                total.div_ceil(ps) <= 10_000,
                "total {total} needs more than 10,000 parts at {ps}"
            );
        }
    }

    /// A 64 MiB pack is the payload this exists for; it must land on the 16 MiB
    /// floor, i.e. 4 parts, not one part or sixty-four.
    #[test]
    fn a_64mib_pack_uses_the_16mib_floor() {
        let ps = mpu_part_size_gcs(64 * 1024 * 1024);
        assert_eq!(ps, 16 * 1024 * 1024);
    }
}

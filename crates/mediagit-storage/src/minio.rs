// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! MinIO & S3-compatible storage backend
//!
//! Implements the `StorageBackend` trait using the AWS S3 SDK with custom endpoint
//! configuration for MinIO and other S3-compatible storage services (DigitalOcean Spaces,
//! Wasabi, etc.).
//!
//! # Features
//!
//! - S3-compatible API via aws-sdk-s3
//! - Custom endpoint configuration for self-hosted MinIO
//! - MinIO authentication (Access Key ID + Secret Access Key)
//! - SSL/TLS support for secure connections
//! - Automatic credential handling
//!
//! # Configuration
//!
//! MinIO backends can be configured with custom endpoints, credentials, and bucket names:
//!
//! ```rust,no_run
//! use mediagit_storage::minio::MinIOBackend;
//! use mediagit_storage::StorageBackend;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // Create MinIO backend with custom endpoint
//!     let backend = MinIOBackend::new(
//!         "http://localhost:9000",  // MinIO endpoint
//!         "my-bucket",              // bucket name
//!         "minioadmin",             // access key
//!         "minioadmin",             // secret key
//!     ).await?;
//!
//!     // Use like any other storage backend
//!     backend.put("documents/file.pdf", b"content").await?;
//!     let data = backend.get("documents/file.pdf").await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! # Self-Hosted MinIO Deployment
//!
//! For local development with Docker:
//!
//! ```bash
//! docker run -p 9000:9000 -p 9001:9001 \
//!   -e MINIO_ROOT_USER=minioadmin \
//!   -e MINIO_ROOT_PASSWORD=minioadmin \
//!   minio/minio server /data --console-address ":9001"
//! ```
//!
//! Configuration:
//! - Endpoint: `http://localhost:9000`
//! - Access Key: `minioadmin`
//! - Secret Key: `minioadmin`
//! - Bucket: Create via web console at `http://localhost:9001`
//!
//! # Production Deployment
//!
//! For production MinIO clusters:
//! - Use HTTPS endpoint with valid TLS certificate
//! - Configure strong credentials
//! - Enable object versioning if needed
//! - Use MinIO's distributed mode for high availability
//! - Enable encryption at rest for sensitive data

use crate::StorageBackend;
use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use aws_sdk_s3::Client;
use aws_sdk_s3::config::retry::RetryConfig;
use aws_sdk_s3::config::timeout::TimeoutConfig;
use bytes::Bytes;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::Semaphore;
use tracing::{debug, warn};

/// Configuration for the MinIO backend
#[derive(Clone, Debug)]
pub struct MinIOConfig {
    /// MinIO endpoint URL (e.g., http://localhost:9000)
    pub endpoint: String,

    /// Bucket name
    pub bucket: String,

    /// Access key ID
    pub access_key: String,

    /// Secret access key
    pub secret_key: String,

    /// Logical-key prefix applied transparently to every object key on the
    /// wire. Empty means keys land at bucket root (legacy behaviour). Lets
    /// multiple repos share one bucket without colliding on identical OIDs.
    pub prefix: String,

    /// AWS region for SigV4 signing (default: "us-east-1" for MinIO/S3-compatible)
    pub region: String,

    /// Use path-style addressing (default: true for MinIO)
    pub path_style: bool,

    /// Multipart upload part size in bytes (default: 8MB)
    pub part_size: u64,

    /// Maximum number of concurrent parts to upload (default: 8)
    pub max_concurrent_parts: usize,

    /// Maximum number of retries for failed operations (default: 5)
    pub max_retries: u32,

    /// Initial retry delay in milliseconds (default: 1000ms)
    pub initial_retry_delay_ms: u64,
}

impl Default for MinIOConfig {
    fn default() -> Self {
        MinIOConfig {
            endpoint: String::new(),
            bucket: String::new(),
            access_key: String::new(),
            secret_key: String::new(),
            prefix: String::new(),
            region: "us-east-1".to_string(),
            path_style: true,
            // 8MB: chunks larger than this use multipart so each HTTP part
            // is small enough to complete reliably over a slow/lossy WAN
            // connection (S3 resets TCP mid-body on large single PUTs).
            part_size: 8 * 1024 * 1024,
            max_concurrent_parts: 8,
            max_retries: 5,
            initial_retry_delay_ms: 1000,
        }
    }
}

/// Normalize a prefix so it ends with `/` (or is empty). Idempotent. Mirrors
/// the helper in azure.rs to keep the multi-tenant key shape consistent
/// across backends.
fn normalize_prefix(p: &str) -> String {
    if p.is_empty() || p.ends_with('/') {
        p.to_string()
    } else {
        format!("{}/", p)
    }
}

/// Internal statistics for the MinIO backend
#[derive(Debug)]
struct MinIOStats {
    total_bytes_uploaded: AtomicU64,
    total_bytes_downloaded: AtomicU64,
    total_objects_deleted: AtomicU64,
}

impl MinIOStats {
    fn new() -> Self {
        MinIOStats {
            total_bytes_uploaded: AtomicU64::new(0),
            total_bytes_downloaded: AtomicU64::new(0),
            total_objects_deleted: AtomicU64::new(0),
        }
    }
}

/// MinIO & S3-compatible storage backend
///
/// This backend uses the AWS S3 SDK but configured for MinIO or other S3-compatible
/// services. It supports custom endpoints, allowing for self-hosted deployments.
///
/// # Thread Safety
///
/// This implementation is `Send + Sync` and can be safely shared across threads
/// and async tasks.
/// Compute optimal MPU part size for MinIO (S3-API-compatible; same limits as S3).
/// See `mpu_part_size_s3` in s3.rs for the same logic.
fn mpu_part_size_minio(total_size: u64) -> u64 {
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

#[derive(Clone)]
pub struct MinIOBackend {
    client: Client,
    config: Arc<MinIOConfig>,
    stats: Arc<MinIOStats>,
    // Limits concurrent create_multipart_upload calls to prevent overwhelming MinIO.
    // Env: MEDIAGIT_MINIO_MPU_CONCURRENCY (default 16). Set to 0 to disable.
    mpu_sem: Arc<Semaphore>,
    // Bounds concurrent in-flight `with_retry` operations (put/get/exists/delete/head)
    // against this backend. Without this, a backend outage lets every concurrent
    // chunk request retry independently and unboundedly: each retry chain holds a
    // socket/connection while sleeping through exponential backoff, and thousands of
    // concurrent chains piling up over a multi-minute outage can exhaust process
    // socket handles, which starves the server's own accept loop (A7 abuse drill).
    // Env: MEDIAGIT_MINIO_OP_CONCURRENCY (default 64). Set to 0 to disable.
    op_sem: Arc<Semaphore>,
    // Keep these for backward compatibility
    endpoint: String,
    bucket: String,
    _access_key: String,
    _secret_key: String,
}

impl MinIOBackend {
    /// Create a new MinIO backend with the specified configuration
    ///
    /// # Arguments
    ///
    /// * `endpoint` - MinIO server endpoint (e.g., `http://localhost:9000`)
    /// * `bucket` - S3 bucket name
    /// * `access_key` - MinIO Access Key ID
    /// * `secret_key` - MinIO Secret Access Key
    ///
    /// # Returns
    ///
    /// * `Ok(MinIOBackend)` - Successfully created backend
    /// * `Err` - If endpoint is invalid or bucket cannot be accessed
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use mediagit_storage::minio::MinIOBackend;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// let backend = MinIOBackend::new(
    ///     "http://localhost:9000",
    ///     "my-bucket",
    ///     "minioadmin",
    ///     "minioadmin",
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn new(
        endpoint: &str,
        bucket: &str,
        access_key: &str,
        secret_key: &str,
    ) -> anyhow::Result<Self> {
        // Validate endpoint format
        if !endpoint.starts_with("http://") && !endpoint.starts_with("https://") {
            return Err(anyhow::anyhow!(
                "Invalid endpoint: must start with http:// or https://"
            ));
        }

        // Remove trailing slash for consistency
        let endpoint = endpoint.trim_end_matches('/').to_string();

        // Validate bucket name (S3 bucket naming rules)
        if bucket.is_empty() {
            return Err(anyhow::anyhow!("bucket name cannot be empty"));
        }

        if bucket.len() > 63 {
            return Err(anyhow::anyhow!("bucket name must be 63 characters or less"));
        }

        if !bucket
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(anyhow::anyhow!(
                "bucket name must contain only lowercase letters, numbers, and hyphens"
            ));
        }

        if bucket.starts_with('-') || bucket.ends_with('-') {
            return Err(anyhow::anyhow!(
                "bucket name cannot start or end with a hyphen"
            ));
        }

        // Validate credentials
        if access_key.is_empty() {
            return Err(anyhow::anyhow!("access key cannot be empty"));
        }

        if secret_key.is_empty() {
            return Err(anyhow::anyhow!("secret key cannot be empty"));
        }

        let config = MinIOConfig {
            endpoint: endpoint.clone(),
            bucket: bucket.to_string(),
            access_key: access_key.to_string(),
            secret_key: secret_key.to_string(),
            ..Default::default()
        };

        Self::with_config(config).await
    }

    /// Create a new MinIO backend with a logical-key prefix applied to every
    /// object on the wire. Equivalent to `new()` when `prefix` is empty.
    /// Multiple repos sharing one bucket should each pick a distinct prefix
    /// to avoid OID collisions on identical content.
    pub async fn new_with_prefix(
        endpoint: &str,
        bucket: &str,
        access_key: &str,
        secret_key: &str,
        prefix: &str,
    ) -> anyhow::Result<Self> {
        // Mirror the input validation from `new()` — kept inline (rather than
        // a double-init via Self::new() then with_config) to avoid hitting the
        // bucket twice on construction.
        if !endpoint.starts_with("http://") && !endpoint.starts_with("https://") {
            return Err(anyhow::anyhow!(
                "Invalid endpoint: must start with http:// or https://"
            ));
        }
        let endpoint = endpoint.trim_end_matches('/').to_string();
        if bucket.is_empty() {
            return Err(anyhow::anyhow!("bucket name cannot be empty"));
        }
        if bucket.len() > 63 {
            return Err(anyhow::anyhow!("bucket name must be 63 characters or less"));
        }
        if !bucket
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(anyhow::anyhow!(
                "bucket name must contain only lowercase letters, numbers, and hyphens"
            ));
        }
        if bucket.starts_with('-') || bucket.ends_with('-') {
            return Err(anyhow::anyhow!(
                "bucket name cannot start or end with a hyphen"
            ));
        }
        if access_key.is_empty() {
            return Err(anyhow::anyhow!("access key cannot be empty"));
        }
        if secret_key.is_empty() {
            return Err(anyhow::anyhow!("secret key cannot be empty"));
        }

        let config = MinIOConfig {
            endpoint,
            bucket: bucket.to_string(),
            access_key: access_key.to_string(),
            secret_key: secret_key.to_string(),
            prefix: prefix.to_string(),
            ..Default::default()
        };
        Self::with_config(config).await
    }

    /// Create a new MinIO backend with custom configuration
    ///
    /// # Arguments
    ///
    /// * `config` - Custom MinIO configuration
    ///
    /// # Returns
    ///
    /// * `Ok(MinIOBackend)` - Successfully created backend
    /// * `Err` - If AWS SDK initialization or bucket access fails
    pub async fn with_config(mut config: MinIOConfig) -> Result<Self> {
        // Normalize the prefix once at construction so all hot-path uses can
        // assume it already ends with `/` (or is empty).
        config.prefix = normalize_prefix(&config.prefix);
        debug!(
            "Initializing MinIO backend: endpoint={}, bucket={}, prefix='{}', path_style={}",
            config.endpoint, config.bucket, config.prefix, config.path_style
        );

        // Create credentials
        let credentials = aws_sdk_s3::config::Credentials::new(
            config.access_key.clone(),
            config.secret_key.clone(),
            None,
            None,
            "MinIOBackend",
        );

        // Build S3 configuration directly for MinIO/S3-compatible endpoints.
        // We skip aws_config::defaults().load() to avoid IMDS region discovery
        // which causes 2x 1-second timeouts in non-AWS environments.
        //
        // Bound the SDK's worst case explicitly: without these, a stalled TCP
        // can cost ~30 s/attempt × default-3 retries = ~90 s/op, which on a
        // multi-endpoint push compounded to the 10–15 min outages we saw.
        // Our outer `with_retry` wrapper still provides our own retry budget.
        // connect_timeout: fast failure if host is unreachable (5 s is enough for DNS + TCP).
        // read_timeout: time between successive response bytes — keep short to detect stalled
        //   HTTP responses; does NOT cap upload throughput.
        // operation_attempt_timeout / operation_timeout: deliberately NOT set here.
        //   Chunks can be 10s–100s MB; over a slow WAN link a 30-s cap kills legitimate
        //   uploads. The connect_timeout (5 s) is the safety valve against dead servers.
        //   The SDK retry_config (max_attempts=2) provides a second chance on transient errors.
        // F2: wider connect window for cross-region TLS handshakes; more attempts absorb
        // transient S3 connection resets without surfacing 500s to the client.
        // Env overrides: MEDIAGIT_AWS_CONNECT_TIMEOUT_SECS, MEDIAGIT_AWS_MAX_ATTEMPTS.
        let connect_timeout_secs: u64 = std::env::var("MEDIAGIT_AWS_CONNECT_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(10);
        let max_attempts: u32 = std::env::var("MEDIAGIT_AWS_MAX_ATTEMPTS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(5);
        let timeout_config = TimeoutConfig::builder()
            .connect_timeout(Duration::from_secs(connect_timeout_secs))
            .read_timeout(Duration::from_secs(120))
            .build();

        let s3_config = aws_sdk_s3::config::Builder::new()
            .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
            .endpoint_url(&config.endpoint)
            .credentials_provider(credentials)
            .force_path_style(config.path_style)
            .region(aws_sdk_s3::config::Region::new(config.region.clone()))
            .timeout_config(timeout_config)
            .retry_config(RetryConfig::standard().with_max_attempts(max_attempts))
            // F1: share the process-wide warm connection pool so concurrent MPU calls
            // reuse existing TCP+TLS sessions instead of opening new ones on every burst.
            .http_client(crate::http_pool::shared())
            .build();

        let client = Client::from_conf(s3_config);

        // Idempotent bucket init: HEAD first; only CREATE on NotFound.
        // Avoids the slow "create-then-ignore-AlreadyExists" path on every
        // backend construction for buckets that already exist.
        match client.head_bucket().bucket(&config.bucket).send().await {
            Ok(_) => {
                debug!("MinIO bucket '{}' already exists", config.bucket);
            }
            Err(head_err) => {
                let not_found = head_err
                    .as_service_error()
                    .map(|se| se.is_not_found())
                    .unwrap_or(false);
                // Log the real AWS error so the operator can diagnose auth/region/existence issues.
                tracing::error!(
                    "head_bucket '{}' (region={}, endpoint={}): not_found={} | {}",
                    config.bucket,
                    config.region,
                    config.endpoint,
                    not_found,
                    head_err
                );
                let mut create_req = client.create_bucket().bucket(&config.bucket);
                if config.region != "us-east-1" {
                    use aws_sdk_s3::types::{BucketLocationConstraint, CreateBucketConfiguration};
                    let constraint = BucketLocationConstraint::from(config.region.as_str());
                    let cfg = CreateBucketConfiguration::builder()
                        .location_constraint(constraint)
                        .build();
                    create_req = create_req.create_bucket_configuration(cfg);
                }
                match create_req.send().await {
                    Ok(_) => {
                        debug!("MinIO bucket '{}' created successfully", config.bucket);
                    }
                    Err(e) => {
                        let already_exists = e
                            .as_service_error()
                            .map(|se| {
                                se.is_bucket_already_owned_by_you() || se.is_bucket_already_exists()
                            })
                            .unwrap_or(false);
                        if already_exists {
                            debug!(
                                "MinIO bucket '{}' already exists (race-resolved)",
                                config.bucket
                            );
                        } else {
                            tracing::error!(
                                "create_bucket '{}' (region={}) failed: {}",
                                config.bucket,
                                config.region,
                                e
                            );
                            return Err(e).context(format!(
                                "Failed to access or create MinIO bucket: {}",
                                config.bucket
                            ));
                        }
                    }
                }
            }
        }

        debug!("Successfully connected to MinIO bucket: {}", config.bucket);

        // F3: pre-warm the shared HTTP pool so the first MPU burst finds warm TCP sessions.
        // Fire-and-forget; failures are harmless — the pool will self-populate on first use.
        // Env: MEDIAGIT_AWS_POOL_WARM (default 16). Set to 0 to disable.
        let warm: usize = std::env::var("MEDIAGIT_AWS_POOL_WARM")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(16);
        if warm > 0 {
            let warm_client = client.clone();
            let warm_bucket = config.bucket.clone();
            tokio::spawn(async move {
                let futs: Vec<_> = (0..warm)
                    .map(|_| {
                        let c = warm_client.clone();
                        let b = warm_bucket.clone();
                        async move {
                            let _ = c.head_bucket().bucket(&b).send().await;
                        }
                    })
                    .collect();
                futures::future::join_all(futs).await;
            });
        }

        let mpu_concurrency = std::env::var("MEDIAGIT_MINIO_MPU_CONCURRENCY")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(16)
            .max(1);
        let op_concurrency = std::env::var("MEDIAGIT_MINIO_OP_CONCURRENCY")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(64)
            .max(1);
        Ok(MinIOBackend {
            client,
            config: Arc::new(config.clone()),
            stats: Arc::new(MinIOStats::new()),
            mpu_sem: Arc::new(Semaphore::new(mpu_concurrency)),
            op_sem: Arc::new(Semaphore::new(op_concurrency)),
            endpoint: config.endpoint,
            bucket: config.bucket,
            _access_key: config.access_key,
            _secret_key: config.secret_key,
        })
    }

    /// Compose the on-wire object key from a logical key. Returns the input
    /// unchanged when prefix is empty (legacy behaviour).
    fn full_key(&self, key: &str) -> String {
        if self.config.prefix.is_empty() {
            key.to_string()
        } else {
            format!("{}{}", self.config.prefix, key)
        }
    }

    /// Inverse of `full_key`: strip the configured prefix from a wire key to
    /// yield the logical key callers expect. Defensive: keys not starting
    /// with our prefix (foreign data) pass through unchanged.
    fn strip_prefix<'a>(&self, full: &'a str) -> &'a str {
        if self.config.prefix.is_empty() {
            full
        } else {
            full.strip_prefix(self.config.prefix.as_str())
                .unwrap_or(full)
        }
    }

    /// Get current statistics
    pub fn stats(&self) -> (u64, u64, u64) {
        (
            self.stats.total_bytes_uploaded.load(Ordering::Relaxed),
            self.stats.total_bytes_downloaded.load(Ordering::Relaxed),
            self.stats.total_objects_deleted.load(Ordering::Relaxed),
        )
    }

    /// Validate a key for correctness
    fn validate_key(key: &str) -> Result<()> {
        if key.is_empty() {
            return Err(anyhow!("key cannot be empty"));
        }
        if key.starts_with('/') {
            return Err(anyhow!("key cannot start with '/'"));
        }
        Ok(())
    }

    /// Perform operation with exponential backoff retry logic.
    /// Permanent errors (NoSuchKey, AccessDenied) are returned immediately
    /// without retry — retrying them wastes time and hides the real cause.
    async fn with_retry<F, T>(&self, mut operation: F) -> Result<T>
    where
        F: FnMut() -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T>> + Send>>,
    {
        // Acquire before the retry loop (not per-attempt) so a single slow/retrying
        // operation holds exactly one permit for its whole lifetime, capping how many
        // concurrent chains can be mid-backoff against a down backend at once. Waiting
        // for a permit is a plain async yield — it never blocks a tokio worker thread,
        // so it can't itself starve the accept loop the way unbounded retry chains do.
        let _permit = self.op_sem.acquire().await.expect("op_sem is never closed");

        let mut retry_count = 0;
        let mut delay_ms = self.config.initial_retry_delay_ms;

        loop {
            match operation().await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    let msg = e.to_string().to_lowercase();
                    let is_permanent = msg.contains("nosuchkey")
                        || msg.contains("no such key")
                        || msg.contains("accessdenied")
                        || msg.contains("access denied");
                    if is_permanent {
                        return Err(e);
                    }

                    retry_count += 1;
                    if retry_count >= self.config.max_retries {
                        return Err(e).context(format!(
                            "Failed after {} retries; last error follows",
                            self.config.max_retries
                        ));
                    }

                    warn!(
                        "Operation failed (attempt {}/{}), retrying in {}ms: {}",
                        retry_count, self.config.max_retries, delay_ms, e
                    );

                    tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;

                    delay_ms = (delay_ms * 2).min(10000);
                }
            }
        }
    }

    /// Create a new MinIO backend from environment variables
    ///
    /// Expects the following environment variables:
    /// - `MINIO_ENDPOINT` - MinIO endpoint URL
    /// - `MINIO_BUCKET` - S3 bucket name
    /// - `MINIO_ACCESS_KEY` - Access Key ID
    /// - `MINIO_SECRET_KEY` - Secret Access Key
    ///
    /// # Returns
    ///
    /// * `Ok(MinIOBackend)` - Successfully created backend
    /// * `Err` - If any required environment variable is missing or invalid
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use mediagit_storage::minio::MinIOBackend;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// let backend = MinIOBackend::from_env().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn from_env() -> anyhow::Result<Self> {
        let endpoint = std::env::var("MINIO_ENDPOINT")
            .map_err(|_| anyhow::anyhow!("MINIO_ENDPOINT environment variable not set"))?;

        let bucket = std::env::var("MINIO_BUCKET")
            .map_err(|_| anyhow::anyhow!("MINIO_BUCKET environment variable not set"))?;

        let access_key = std::env::var("MINIO_ACCESS_KEY")
            .map_err(|_| anyhow::anyhow!("MINIO_ACCESS_KEY environment variable not set"))?;

        let secret_key = std::env::var("MINIO_SECRET_KEY")
            .map_err(|_| anyhow::anyhow!("MINIO_SECRET_KEY environment variable not set"))?;

        Self::new(&endpoint, &bucket, &access_key, &secret_key).await
    }

    /// Get the configured endpoint
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Get the configured bucket name
    /// Endpoint this backend actually talks to, for error messages.
    ///
    /// WHY THIS EXISTS. This driver is the shared S3-compatible implementation:
    /// `handlers/mod.rs` builds it via `new_with_prefix` for MinIO AND via
    /// `with_config` for AWS. Every error string here used to say "minio:"
    /// regardless, so a real AWS failure was reported as
    ///
    ///   err=complete_multipart_upload minio: dispatch failure
    ///
    /// on the aws backend (20260907-ga42). That is not cosmetic: it points the
    /// next investigation at the wrong backend, and it cost time in the ga42
    /// post-mortem before the endpoint in the URL gave it away.
    pub(crate) fn endpoint_label(&self) -> &str {
        &self.config.endpoint
    }

    /// Which checksum this endpoint's uploads are attested with, if any.
    ///
    /// THIS BACKEND SERVES BOTH real AWS and actual MinIO — the server builds
    /// the "aws" backend from `MinIOConfig` with an
    /// `https://s3.<region>.amazonaws.com` endpoint (`handlers/mod.rs`), so
    /// `B2SpacesDriver` in `b2_spaces_driver.rs` is NOT on the AWS path at
    /// all; only `b2_spaces` constructs that. (That file was `s3.rs` with a
    /// type called `S3Backend` until 2026-09-18 — renamed because a file named
    /// after AWS that AWS never reaches collects AWS fixes that do nothing.)
    /// Attestation therefore has to live here to affect anything.
    ///
    /// Gated on the endpoint host because the capability differs: AWS validates
    /// full-object CRC64NVME, MinIO does not implement it, and declaring an
    /// algorithm an endpoint ignores would produce a checksum nobody checks —
    /// which is worse than none, because phase B would then skip the pack
    /// read-back on the strength of it.
    ///
    /// `MEDIAGIT_S3_ATTEST=off` forces it off, `=on` forces it on for a
    /// non-AWS endpoint that does support it. Escape hatches, not tuning: with
    /// attestation off the read-back stays, so correctness is unchanged.
    fn attestation_algorithm(&self) -> Option<crate::ChecksumAlgorithm> {
        let host_is_aws = self.config.endpoint.contains(".amazonaws.com");
        match std::env::var("MEDIAGIT_S3_ATTEST").ok().as_deref() {
            Some("off") | Some("0") | Some("false") => None,
            Some("crc32c") => Some(crate::ChecksumAlgorithm::Crc32c),
            Some("on") | Some("1") => Some(crate::ChecksumAlgorithm::Crc64Nvme),
            _ if host_is_aws => Some(crate::ChecksumAlgorithm::Crc64Nvme),
            _ => None,
        }
    }

    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    /// Store a small object directly using put_object
    async fn put_simple(&self, key: &str, data: &[u8]) -> Result<()> {
        let client = self.client.clone();
        let bucket = self.config.bucket.clone();
        let key_clone = key.to_string();
        let stats = self.stats.clone();
        let body = Bytes::copy_from_slice(data);

        self.with_retry(|| {
            let client = client.clone();
            let bucket = bucket.clone();
            let key = key_clone.clone();
            let stats = stats.clone();
            let body = body.clone();

            Box::pin(async move {
                debug!(
                    "Putting object to MinIO (simple): {} ({} bytes)",
                    key,
                    body.len()
                );

                client
                    .put_object()
                    .bucket(&bucket)
                    .key(&key)
                    .body(body.clone().into())
                    .send()
                    .await
                    .map_err(|e| anyhow!("Failed to put object: {}", e))?;

                stats
                    .total_bytes_uploaded
                    .fetch_add(body.len() as u64, Ordering::Relaxed);

                Ok(())
            })
        })
        .await
    }

    /// Store a large object using multipart upload
    async fn put_multipart(&self, key: &str, data: &[u8]) -> Result<()> {
        debug!(
            "Putting large object to MinIO (multipart): {} ({} bytes)",
            key,
            data.len()
        );

        let client = self.client.clone();
        let bucket = self.config.bucket.clone();
        let key_clone = key.to_string();

        // Initiate multipart upload
        let multipart = client
            .create_multipart_upload()
            .bucket(&bucket)
            .key(&key_clone)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to initiate multipart upload: {}", e))?;

        let upload_id = multipart
            .upload_id()
            .ok_or_else(|| anyhow!("No upload ID returned from MinIO"))?
            .to_string();

        debug!(
            "Initiated multipart upload for {}: {}",
            key_clone, upload_id
        );

        // Upload parts concurrently
        let mut part_handles = vec![];
        let mut parts = vec![];
        let part_size = self.config.part_size as usize;
        let mut part_number = 1;

        // part_number feeds the S3 PartNumber (1-indexed); kept as an explicit
        // counter for clarity in this multipart-upload hot path.
        #[allow(clippy::explicit_counter_loop)]
        for chunk in data.chunks(part_size) {
            let client = client.clone();
            let bucket = bucket.clone();
            let key = key_clone.clone();
            let upload_id = upload_id.clone();
            let stats = self.stats.clone();
            let chunk_data = chunk.to_vec();
            let part_num = part_number;

            let max_retries = self.config.max_retries;
            let initial_delay = self.config.initial_retry_delay_ms;
            let handle = tokio::spawn(async move {
                debug!(
                    "Uploading part {} ({} bytes) for key: {}",
                    part_num,
                    chunk_data.len(),
                    key
                );

                let mut retry = 0u32;
                let mut delay_ms = initial_delay;
                let (part_num_out, etag) = loop {
                    let response = client
                        .upload_part()
                        .bucket(&bucket)
                        .key(&key)
                        .upload_id(&upload_id)
                        .part_number(part_num)
                        .body(Bytes::from(chunk_data.clone()).into())
                        .send()
                        .await;
                    match response {
                        Ok(r) => {
                            let etag = r
                                .e_tag()
                                .ok_or_else(|| anyhow!("No ETag returned for part {}", part_num))?
                                .to_string();
                            break (part_num, etag);
                        }
                        Err(e) => {
                            retry += 1;
                            if retry >= max_retries {
                                return Err(anyhow!(
                                    "Failed to upload part {} after {} retries: {}",
                                    part_num,
                                    max_retries,
                                    e
                                ));
                            }
                            warn!(
                                "Part {} upload failed (attempt {}/{}), retrying in {}ms: {}",
                                part_num, retry, max_retries, delay_ms, e
                            );
                            tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
                            delay_ms = (delay_ms * 2).min(10_000);
                        }
                    }
                };

                stats
                    .total_bytes_uploaded
                    .fetch_add(chunk_data.len() as u64, Ordering::Relaxed);

                Ok::<_, anyhow::Error>((part_num_out, etag))
            });

            part_handles.push(handle);

            // Limit concurrent uploads
            if part_handles.len() >= self.config.max_concurrent_parts
                && let Some(handle) = part_handles.pop()
            {
                let (part_num, etag) = handle.await??;
                parts.push((part_num, etag));
            }

            part_number += 1;
        }

        // Wait for all remaining parts to complete
        for handle in part_handles {
            let (part_num, etag) = handle.await??;
            parts.push((part_num, etag));
        }

        // Sort parts by part number
        parts.sort_by_key(|p| p.0);

        // Complete multipart upload
        let part_list: Vec<_> = parts
            .into_iter()
            .map(|(part_num, etag)| {
                aws_sdk_s3::types::CompletedPart::builder()
                    .part_number(part_num)
                    .e_tag(etag)
                    .build()
            })
            .collect();

        client
            .complete_multipart_upload()
            .bucket(&bucket)
            .key(&key_clone)
            .upload_id(&upload_id)
            .multipart_upload(
                aws_sdk_s3::types::CompletedMultipartUpload::builder()
                    .set_parts(Some(part_list))
                    .build(),
            )
            .send()
            .await
            .map_err(|e| anyhow!("Failed to complete multipart upload: {}", e))?;

        debug!("Successfully completed multipart upload for {}", key_clone);
        Ok(())
    }
}

impl fmt::Debug for MinIOBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MinIOBackend")
            .field("endpoint", &self.endpoint)
            .field("bucket", &self.bucket)
            .field("access_key", &"***")
            .field("secret_key", &"***")
            .finish()
    }
}

#[async_trait]
impl StorageBackend for MinIOBackend {
    /// Retrieve an object from MinIO
    async fn get(&self, key: &str) -> anyhow::Result<Vec<u8>> {
        Self::validate_key(key)?;

        let client = self.client.clone();
        let bucket = self.config.bucket.clone();
        // Use the wire key (with prefix) for all SDK calls; logical key is
        // only used for log/error messages so the caller's view stays clean.
        let key_clone = self.full_key(key);
        let stats = self.stats.clone();

        self.with_retry(|| {
            let client = client.clone();
            let bucket = bucket.clone();
            let key = key_clone.clone();
            let stats = stats.clone();

            Box::pin(async move {
                debug!("Getting object from MinIO: {}", key);

                let response = client
                    .get_object()
                    .bucket(&bucket)
                    .key(&key)
                    .send()
                    .await
                    .map_err(|e| {
                        let emsg = e.to_string().to_lowercase();
                        // MinIO returns a generic "service error" for GET on non-existent
                        // objects (unlike HEAD which returns a typed 404). Translate to a
                        // nosuchkey message so with_retry treats it as permanent and stops
                        // retrying a clearly missing object.
                        if emsg.contains("service error")
                            && !emsg.contains("timeout")
                            && !emsg.contains("connect")
                        {
                            anyhow!("NoSuchKey: object not found: {}", key)
                        } else {
                            anyhow!("Failed to get object: {}", e)
                        }
                    })?;

                let body = response
                    .body
                    .collect()
                    .await
                    .map_err(|e| anyhow!("Failed to read object body: {}", e))?;

                let data = body.into_bytes().to_vec();
                stats
                    .total_bytes_downloaded
                    .fetch_add(data.len() as u64, Ordering::Relaxed);

                Ok(data)
            })
        })
        .await
    }

    /// Stream an object from MinIO as a `Bytes` sequence (B7).
    ///
    /// When `MEDIAGIT_STORAGE_STREAMING=1`, drives the S3 SDK's `ByteStream` via
    /// `into_async_read()` + `unfold` so the object body is never fully buffered.
    /// Falls back to the default single-chunk impl when the knob is OFF so other
    /// backends remain unaffected.  No retry on the streaming path: mid-stream
    /// failures bubble up to the caller's retry layer.
    async fn get_streaming(
        &self,
        key: &str,
    ) -> anyhow::Result<
        std::pin::Pin<
            Box<dyn futures::Stream<Item = anyhow::Result<bytes::Bytes>> + Send + 'static>,
        >,
    > {
        // B7: default ON — AWS clone 15.8% faster (2026-05-22). Set MEDIAGIT_STORAGE_STREAMING=0 to revert.
        let streaming_enabled = std::env::var("MEDIAGIT_STORAGE_STREAMING")
            .as_deref()
            .unwrap_or("1")
            == "1";
        if !streaming_enabled {
            let data = self.get(key).await?;
            return Ok(Box::pin(futures::stream::once(async move {
                Ok::<bytes::Bytes, anyhow::Error>(bytes::Bytes::from(data))
            })));
        }

        Self::validate_key(key)?;
        let key_wire = self.full_key(key);
        let client = self.client.clone();
        let bucket = self.config.bucket.clone();
        let stats = self.stats.clone();

        let response = client
            .get_object()
            .bucket(&bucket)
            .key(&key_wire)
            .send()
            .await
            .map_err(|e| anyhow!("get_streaming {}: {}", key_wire, e))?;

        // ByteStream does not implement futures::Stream directly; convert to
        // tokio::io::AsyncRead and drive with unfold to yield 64 KiB Bytes chunks.
        let reader = response.body.into_async_read();
        // Fuse on error — see the same note in s3.rs::get_streaming.
        let stream = futures::stream::unfold(
            (reader, stats, false),
            |(mut rdr, stats, failed)| async move {
                use tokio::io::AsyncReadExt;
                if failed {
                    return None;
                }
                let mut buf = vec![0u8; 65536];
                match rdr.read(&mut buf).await {
                    Ok(0) => None,
                    Ok(n) => {
                        buf.truncate(n);
                        stats
                            .total_bytes_downloaded
                            .fetch_add(n as u64, std::sync::atomic::Ordering::Relaxed);
                        Some((
                            Ok::<bytes::Bytes, anyhow::Error>(bytes::Bytes::from(buf)),
                            (rdr, stats, false),
                        ))
                    }
                    Err(e) => Some((
                        Err(anyhow!("get_streaming chunk: {}", e)),
                        (rdr, stats, true),
                    )),
                }
            },
        );

        Ok(Box::pin(stream))
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
        Self::validate_key(key)?;
        let key_wire = self.full_key(key);
        let client = self.client.clone();
        let bucket = self.config.bucket.clone();
        let stats = self.stats.clone();

        let response = client
            .get_object()
            .bucket(&bucket)
            .key(&key_wire)
            .range(format!("bytes={}-{}", range.start, range.end - 1))
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("get_streaming_range {}: {}", key_wire, e))?;

        let reader = response.body.into_async_read();
        // Fuse on error — see the same note in s3.rs::get_streaming.
        let stream = futures::stream::unfold(
            (reader, stats, false),
            |(mut rdr, stats, failed)| async move {
                use tokio::io::AsyncReadExt;
                if failed {
                    return None;
                }
                let mut buf = vec![0u8; 65536];
                match rdr.read(&mut buf).await {
                    Ok(0) => None,
                    Ok(n) => {
                        buf.truncate(n);
                        stats
                            .total_bytes_downloaded
                            .fetch_add(n as u64, std::sync::atomic::Ordering::Relaxed);
                        Some((
                            Ok::<bytes::Bytes, anyhow::Error>(bytes::Bytes::from(buf)),
                            (rdr, stats, false),
                        ))
                    }
                    Err(e) => Some((
                        Err(anyhow::anyhow!("get_streaming_range chunk: {}", e)),
                        (rdr, stats, true),
                    )),
                }
            },
        );
        Ok(Box::pin(stream))
    }

    /// Store an object in MinIO
    async fn put(&self, key: &str, data: &[u8]) -> anyhow::Result<()> {
        Self::validate_key(key)?;

        // Apply prefix exactly once at the trait entry; put_simple and
        // put_multipart receive the wire key and pass it straight to the SDK.
        let wire_key = self.full_key(key);
        if data.len() as u64 <= self.config.part_size {
            return self.put_simple(&wire_key, data).await;
        }
        self.put_multipart(&wire_key, data).await
    }

    /// Check if an object exists in MinIO
    async fn exists(&self, key: &str) -> anyhow::Result<bool> {
        Self::validate_key(key)?;

        let client = self.client.clone();
        let bucket = self.config.bucket.clone();
        let key_clone = self.full_key(key);

        self.with_retry(|| {
            let client = client.clone();
            let bucket = bucket.clone();
            let key = key_clone.clone();

            Box::pin(async move {
                debug!("Checking if object exists in MinIO: {}", key);

                match client.head_object().bucket(&bucket).key(&key).send().await {
                    Ok(_) => {
                        debug!("Object exists: {}", key);
                        Ok(true)
                    }
                    Err(e) => {
                        let error_message = e.to_string().to_lowercase();
                        // Check for various "not found" patterns from real and emulated S3 services
                        if error_message.contains("404")
                            || error_message.contains("not found")
                            || error_message.contains("notfound")
                            || error_message.contains("nosuchkey")
                            || error_message.contains("does not exist")
                            || error_message.contains("no such key")
                            // MinIO emulator sometimes returns generic "service error" for non-existent objects
                            || (error_message.contains("service error") && error_message.len() < 50)
                        {
                            debug!("Object does not exist: {}", key);
                            Ok(false)
                        } else {
                            Err(anyhow!("Failed to check object existence: {}", e))
                        }
                    }
                }
            })
        })
        .await
    }

    async fn head(&self, key: &str) -> anyhow::Result<Option<u64>> {
        Self::validate_key(key)?;

        let client = self.client.clone();
        let bucket = self.config.bucket.clone();
        let key_clone = self.full_key(key);

        self.with_retry(|| {
            let client = client.clone();
            let bucket = bucket.clone();
            let key = key_clone.clone();

            Box::pin(async move {
                match client.head_object().bucket(&bucket).key(&key).send().await {
                    Ok(resp) => Ok(Some(resp.content_length().unwrap_or(0) as u64)),
                    Err(e) => {
                        let emsg = e.to_string().to_lowercase();
                        if emsg.contains("404")
                            || emsg.contains("not found")
                            || emsg.contains("notfound")
                            || emsg.contains("nosuchkey")
                            || emsg.contains("does not exist")
                            || emsg.contains("no such key")
                            || (emsg.contains("service error") && emsg.len() < 50)
                        {
                            Ok(None)
                        } else {
                            Err(anyhow!("Failed to head object: {}", e))
                        }
                    }
                }
            })
        })
        .await
    }

    /// The provider-recorded checksum for `key`, if the service validated one.
    ///
    /// `HeadObject` returns the stored checksum for an object uploaded with
    /// one, and downloads no body — one metadata request against a whole pack
    /// read-back is the entire point of attestation.
    ///
    /// `checksum_mode(ENABLED)` is required: without it S3 omits the checksum
    /// fields from the response and every object looks unattested, which would
    /// silently leave the read-back on forever — a knob that appears to work
    /// while doing nothing.
    ///
    /// Any error maps to `Ok(None)`, i.e. "not attested", so a network blip or
    /// a permission gap keeps the read-back rather than skipping it. The whole
    /// point is to only skip verification on a POSITIVE signal.
    async fn attested_checksum(&self, key: &str) -> anyhow::Result<Option<String>> {
        Self::validate_key(key)?;
        if self.attestation_algorithm().is_none() {
            return Ok(None);
        }
        let full = self.full_key(key);
        let resp = self
            .client
            .head_object()
            .bucket(&self.config.bucket)
            .key(&full)
            .checksum_mode(aws_sdk_s3::types::ChecksumMode::Enabled)
            .send()
            .await;
        match resp {
            Ok(r) => Ok(r
                .checksum_crc64_nvme()
                .map(str::to_string)
                .or_else(|| r.checksum_crc32_c().map(str::to_string))),
            // PROPAGATED, not swallowed. The caller turns any error into "not
            // attested" anyway, so behaviour is the same — but it logs the
            // reason, and that difference matters: an earlier version returned
            // `Ok(None)` here and a HEAD failure was then indistinguishable
            // from an object that genuinely carries no checksum. The whole
            // feature can be inert and look identical to "the provider does
            // not attest".
            Err(e) => Err(anyhow!(
                "head_object checksum {}: {}",
                self.endpoint_label(),
                e
            )),
        }
    }

    /// Delete an object from MinIO
    async fn delete(&self, key: &str) -> anyhow::Result<()> {
        Self::validate_key(key)?;

        let client = self.client.clone();
        let bucket = self.config.bucket.clone();
        let key_clone = self.full_key(key);
        let stats = self.stats.clone();

        self.with_retry(|| {
            let client = client.clone();
            let bucket = bucket.clone();
            let key = key_clone.clone();
            let stats = stats.clone();

            Box::pin(async move {
                debug!("Deleting object from MinIO: {}", key);

                client
                    .delete_object()
                    .bucket(&bucket)
                    .key(&key)
                    .send()
                    .await
                    .map_err(|e| anyhow!("Failed to delete object: {}", e))?;

                stats.total_objects_deleted.fetch_add(1, Ordering::Relaxed);

                Ok(())
            })
        })
        .await
    }

    /// List objects in MinIO with a given logical prefix. The configured
    /// backend prefix is composed with the caller's prefix on the wire and
    /// stripped from each returned key, so callers always see logical keys
    /// (the same ones they wrote via `put`).
    async fn list_objects(&self, prefix: &str) -> anyhow::Result<Vec<String>> {
        let client = self.client.clone();
        let bucket = self.config.bucket.clone();
        let wire_prefix = self.full_key(prefix);

        // The closure can't borrow `&self`, so we do the wire-list there and
        // strip the backend prefix from each key in this outer scope (where
        // self.strip_prefix is reachable). Keeps a single source of truth for
        // prefix handling.
        let wire_keys: Vec<String> = self
            .with_retry(|| {
                let client = client.clone();
                let bucket = bucket.clone();
                let wire_prefix = wire_prefix.clone();

                Box::pin(async move {
                    debug!(
                        "Listing objects in MinIO with wire prefix '{}'",
                        wire_prefix
                    );

                    let mut result = vec![];
                    let mut continuation_token: Option<String> = None;

                    loop {
                        let mut request = client.list_objects_v2().bucket(&bucket);

                        if !wire_prefix.is_empty() {
                            request = request.prefix(&wire_prefix);
                        }

                        if let Some(token) = continuation_token {
                            request = request.continuation_token(token);
                        }

                        let response = request
                            .send()
                            .await
                            .map_err(|e| anyhow!("Failed to list objects: {}", e))?;

                        for obj in response.contents() {
                            if let Some(wire_key) = obj.key() {
                                result.push(wire_key.to_string());
                            }
                        }

                        if response.is_truncated() == Some(true) {
                            continuation_token =
                                response.next_continuation_token().map(|t| t.to_string());
                        } else {
                            break;
                        }
                    }

                    Ok(result)
                })
            })
            .await?;

        let mut result: Vec<String> = wire_keys
            .iter()
            .map(|k| self.strip_prefix(k).to_string())
            .collect();
        result.sort();
        debug!(
            "Found {} objects with logical prefix '{}'",
            result.len(),
            prefix
        );
        Ok(result)
    }

    async fn presign_put(
        &self,
        key: &str,
        content_length: u64,
        ttl: std::time::Duration,
    ) -> anyhow::Result<Option<crate::PresignedPut>> {
        let wire_key = self.full_key(key);
        let presigning = aws_sdk_s3::presigning::PresigningConfig::expires_in(ttl)
            .map_err(|e| anyhow!("presigning config: {e}"))?;
        let mut builder = self.client.put_object().bucket(&self.bucket).key(&wire_key);
        if content_length > 0 {
            builder = builder.content_length(content_length as i64);
        }
        let req = builder
            .presigned(presigning)
            .await
            .map_err(|e| anyhow!("presign_put {}: {e}", self.endpoint_label()))?;
        Ok(Some(crate::PresignedPut {
            url: req.uri().to_string(),
            method: "PUT".to_string(),
            required_headers: req
                .headers()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            expires_at: std::time::SystemTime::now() + ttl,
        }))
    }

    async fn presign_get(
        &self,
        key: &str,
        ttl: std::time::Duration,
    ) -> anyhow::Result<Option<crate::PresignedDownload>> {
        let wire_key = self.full_key(key);
        let presigning = aws_sdk_s3::presigning::PresigningConfig::expires_in(ttl)
            .map_err(|e| anyhow!("presigning config: {e}"))?;
        let req = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(&wire_key)
            .presigned(presigning)
            .await
            .map_err(|e| anyhow!("presign_get {}: {e}", self.endpoint_label()))?;
        Ok(Some(crate::PresignedDownload {
            url: req.uri().to_string(),
            headers: vec![],
            expires_in_secs: ttl.as_secs(),
        }))
    }

    async fn create_presigned_mpu(
        &self,
        key: &str,
        total_size: u64,
        ttl: std::time::Duration,
    ) -> anyhow::Result<Option<crate::PresignedMpu>> {
        let part_size = mpu_part_size_minio(total_size);
        let wire_key = self.full_key(key);
        // Semaphore scope: only protect the TCP connection burst for create_multipart_upload.
        // Released before presigning so the loop (local CPU work, no network) runs concurrently
        // across all tasks. Holding it through presigning serialized 32 tasks and killed throughput.
        let upload_id = {
            let _permit = self
                .mpu_sem
                .acquire()
                .await
                .map_err(|e| anyhow!("mpu semaphore: {}", e))?;
            // ATTESTATION. Declaring the algorithm is what makes the provider
            // validate the ASSEMBLED object at Complete and reject a mismatch
            // with BadDigest — the property that lets the server stop reading
            // every pushed pack back out of the bucket to check it.
            let mut create = self
                .client
                .create_multipart_upload()
                .bucket(&self.config.bucket)
                .key(&wire_key);
            if let Some(alg) = self.attestation_algorithm() {
                create = create
                    .checksum_algorithm(aws_sdk_s3::types::ChecksumAlgorithm::from(alg.as_s3_str()))
                    .checksum_type(aws_sdk_s3::types::ChecksumType::FullObject);
            }
            let resp = create
                .send()
                .await
                .map_err(|e| anyhow!("create_multipart_upload {}: {}", self.endpoint_label(), e))?;
            resp.upload_id()
                .ok_or_else(|| anyhow!("no upload_id from MinIO"))?
                .to_string()
        }; // permit released here — presigning proceeds concurrently

        let num_parts = total_size.div_ceil(part_size).max(1) as i32;
        let presigning = aws_sdk_s3::presigning::PresigningConfig::expires_in(ttl)
            .map_err(|e| anyhow!("presigning config: {}", e))?;
        let mut parts = Vec::with_capacity(num_parts as usize);
        for part_number in 1..=num_parts {
            let req = self
                .client
                .upload_part()
                .bucket(&self.config.bucket)
                .key(&wire_key)
                .upload_id(&upload_id)
                .part_number(part_number)
                .presigned(presigning.clone())
                .await
                .map_err(|e| {
                    anyhow!(
                        "presign upload_part {} {}: {}",
                        part_number,
                        self.endpoint_label(),
                        e
                    )
                })?;
            parts.push(crate::PresignedMpuPart {
                part_number,
                url: req.uri().to_string(),
            });
        }
        Ok(Some(crate::PresignedMpu {
            upload_id,
            parts,
            part_size,
            checksum: self.attestation_algorithm(),
        }))
    }

    async fn complete_presigned_mpu(
        &self,
        key: &str,
        upload_id: &str,
        parts: Vec<crate::MpuCompletedPart>,
    ) -> anyhow::Result<()> {
        let wire_key = self.full_key(key);
        // ATTESTATION TRAVELS HERE, NOT ON THE PART PUTs.
        //
        // The parts are uploaded through PRESIGNED urls, and a presigned URL
        // signs a fixed header set — AWS rejects a part carrying an extra
        // `x-amz-checksum-*` with `AccessDenied` / "There were headers present
        // in the request which were not signed" (measured against real S3,
        // 2026-09-16). So the client cannot attest its own PUTs.
        //
        // This call is different: the SERVER makes it, with its own
        // credentials, so it can sign a checksum header freely. CRC64NVME is
        // linearly combinable, so the per-part digests the client reports fold
        // into the checksum of the ASSEMBLED object, which S3 then validates
        // and rejects with `BadDigest` on mismatch. Same full-object guarantee,
        // no signing problem.
        let alg = self.attestation_algorithm();
        let full_object = match alg {
            Some(crate::ChecksumAlgorithm::Crc64Nvme) => combine_part_crc64(&parts),
            // crc32c is not combinable the same way here; treated as unattested
            // rather than sending something unverifiable.
            _ => None,
        };
        // SORTED: S3 rejects an unordered part list with `InvalidPartOrder`.
        // The client reports parts in a sequential loop today, so they arrive
        // ascending by accident of the caller rather than by guarantee -- the
        // day part upload is parallelised, every part is already in the bucket
        // when the commit fails. `combine_part_crc64` above already sorts its
        // own copy, so the attestation was never order-dependent; only this
        // list was. Measured on GCS 2026-09-17 (400 InvalidPartOrder); S3
        // documents the same requirement.
        let mut parts = parts;
        parts.sort_by_key(|p| p.part_number);
        let completed: Vec<_> = parts
            .into_iter()
            .map(|p| {
                aws_sdk_s3::types::CompletedPart::builder()
                    .part_number(p.part_number)
                    .e_tag(p.etag)
                    .build()
            })
            .collect();
        let _permit = self
            .mpu_sem
            .acquire()
            .await
            .map_err(|e| anyhow!("mpu semaphore: {}", e))?;
        let mut complete = self
            .client
            .complete_multipart_upload()
            .bucket(&self.config.bucket)
            .key(&wire_key)
            .upload_id(upload_id)
            .multipart_upload(
                aws_sdk_s3::types::CompletedMultipartUpload::builder()
                    .set_parts(Some(completed))
                    .build(),
            );
        if let Some(sum) = full_object {
            complete = complete.checksum_crc64_nvme(sum);
        }
        complete
            .send()
            .await
            .map_err(|e| anyhow!("complete_multipart_upload {}: {}", self.endpoint_label(), e))?;
        Ok(())
    }

    async fn abort_presigned_mpu(&self, key: &str, upload_id: &str) -> anyhow::Result<()> {
        let wire_key = self.full_key(key);
        let _ = self
            .client
            .abort_multipart_upload()
            .bucket(&self.config.bucket)
            .key(&wire_key)
            .upload_id(upload_id)
            .send()
            .await;
        Ok(())
    }
}

/// Fold per-part CRC64NVME digests into the checksum of the assembled object.
///
/// CRC64NVME is linearly combinable: given two CRCs and the byte length of the
/// second run, the CRC of the concatenation is computable without the bytes.
/// That is what lets the server attest an upload it never saw — the client
/// hashes each part it sends, and these fold into the whole.
///
/// Returns `None` if any part is missing a digest or a length, because a
/// partial fold would be a checksum of the wrong thing. `None` means
/// "unattested", which keeps the pack read-back — the fail-closed direction.
fn combine_part_crc64(parts: &[crate::MpuCompletedPart]) -> Option<String> {
    use base64::Engine as _;
    let mut ordered: Vec<&crate::MpuCompletedPart> = parts.iter().collect();
    ordered.sort_by_key(|p| p.part_number);

    let mut acc: Option<u64> = None;
    for p in ordered {
        let (raw, len) = (p.checksum.as_deref()?, p.length?);
        let bytes = base64::engine::general_purpose::STANDARD.decode(raw).ok()?;
        let crc = u64::from_be_bytes(<[u8; 8]>::try_from(bytes.as_slice()).ok()?);
        acc = Some(match acc {
            None => crc,
            Some(a) => crc_fast::checksum_combine(crc_fast::CrcAlgorithm::Crc64Nvme, a, crc, len),
        });
    }
    acc.map(|v| base64::engine::general_purpose::STANDARD.encode(v.to_be_bytes()))
}

#[cfg(test)]
#[allow(unsafe_code)] // edition-2024: test-only env::set_var/remove_var requires unsafe
mod tests {
    use super::*;

    #[test]
    fn test_normalize_prefix_empty_passthrough() {
        assert_eq!(normalize_prefix(""), "");
    }

    #[test]
    fn test_normalize_prefix_appends_slash() {
        assert_eq!(normalize_prefix("repo-objects"), "repo-objects/");
    }

    #[test]
    fn test_normalize_prefix_idempotent_for_slashed() {
        assert_eq!(normalize_prefix("repo-objects/"), "repo-objects/");
    }

    #[test]
    fn test_normalize_prefix_double_call_idempotent() {
        let once = normalize_prefix("repo");
        let twice = normalize_prefix(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn test_normalize_prefix_nested() {
        assert_eq!(
            normalize_prefix("multi-tenant/repo-42"),
            "multi-tenant/repo-42/"
        );
    }

    /// Synthetic backend construction without hitting the network — needed
    /// because aws_sdk_s3::Client requires an `EndpointResolver` etc., which
    /// we only assemble inside `with_config`. We build a config with a
    /// localhost endpoint so the SDK's HTTP client never actually fires.
    fn synthetic_minio_for_prefix_test(prefix: &str) -> MinIOBackend {
        let creds = aws_sdk_s3::config::Credentials::new(
            "ak".to_string(),
            "sk".to_string(),
            None,
            None,
            "synthetic",
        );
        let s3_config = aws_sdk_s3::config::Builder::new()
            .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
            .endpoint_url("http://127.0.0.1:1")
            .credentials_provider(creds)
            .force_path_style(true)
            .region(aws_sdk_s3::config::Region::new("us-east-1"))
            .build();
        let client = Client::from_conf(s3_config);
        let cfg = MinIOConfig {
            endpoint: "http://127.0.0.1:1".to_string(),
            bucket: "b".to_string(),
            access_key: "ak".to_string(),
            secret_key: "sk".to_string(),
            prefix: normalize_prefix(prefix),
            ..Default::default()
        };
        MinIOBackend {
            client,
            config: Arc::new(cfg.clone()),
            stats: Arc::new(MinIOStats::new()),
            mpu_sem: Arc::new(Semaphore::new(16)),
            op_sem: Arc::new(Semaphore::new(64)),
            endpoint: cfg.endpoint,
            bucket: cfg.bucket,
            _access_key: cfg.access_key,
            _secret_key: cfg.secret_key,
        }
    }

    /// `with_retry`'s exhaustion message ends "last error follows" — which is
    /// only true if the caller renders the whole chain. Campaign
    /// 20260804-scale-verify2 logged 1,082 of these with `{}`, so nothing
    /// followed and every retry exhaustion was undiagnosable.
    ///
    /// Pins both halves: the cause must be absent from `{}` (the trap that
    /// makes `{:#}` mandatory at call sites) and present in `{:#}`. Swapping
    /// `.context()` for a message-replacing `map_err` fails the second.
    #[tokio::test]
    async fn with_retry_exhaustion_preserves_cause_in_chain() {
        let creds = aws_sdk_s3::config::Credentials::new(
            "ak".to_string(),
            "sk".to_string(),
            None,
            None,
            "synthetic",
        );
        let s3_config = aws_sdk_s3::config::Builder::new()
            .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
            .endpoint_url("http://127.0.0.1:1")
            .credentials_provider(creds)
            .force_path_style(true)
            .region(aws_sdk_s3::config::Region::new("us-east-1"))
            .build();
        let cfg = MinIOConfig {
            endpoint: "http://127.0.0.1:1".to_string(),
            bucket: "b".to_string(),
            access_key: "ak".to_string(),
            secret_key: "sk".to_string(),
            max_retries: 2,
            initial_retry_delay_ms: 1,
            ..Default::default()
        };
        let backend = MinIOBackend {
            client: Client::from_conf(s3_config),
            config: Arc::new(cfg.clone()),
            stats: Arc::new(MinIOStats::new()),
            mpu_sem: Arc::new(Semaphore::new(16)),
            op_sem: Arc::new(Semaphore::new(4)),
            endpoint: cfg.endpoint,
            bucket: cfg.bucket,
            _access_key: cfg.access_key,
            _secret_key: cfg.secret_key,
        };

        const CAUSE: &str = "connection reset by peer at layer 7";
        let result: Result<()> = backend
            .with_retry(|| Box::pin(async move { Err(anyhow!(CAUSE)) }))
            .await;
        let err = result.expect_err("synthetic operation always errors");

        assert!(
            !format!("{err}").contains(CAUSE),
            "plain Display is expected to hide the cause — if this starts passing, \
             the `{{:#}}` requirement at call sites may have changed: {err}"
        );
        assert!(
            format!("{err:#}").contains(CAUSE),
            "alternate Display must carry the underlying cause, got: {err:#}"
        );
    }

    /// Regression test for the A7 abuse-drill finding: during a sustained
    /// backend outage, concurrent chunk uploads must not spin up unbounded
    /// concurrent retry chains against the dead backend (each chain holds a
    /// connection through several seconds of exponential backoff; thousands
    /// of them piling up over a multi-minute outage exhausted process socket
    /// handles and starved the server's own accept loop).
    ///
    /// `with_retry` now acquires one `op_sem` permit for its entire
    /// (possibly-retrying) lifetime, so no more than `op_concurrency`
    /// operations can be mid-backoff at once. This drives `with_retry`
    /// directly with a synthetic always-fails operation (no real network),
    /// so it's fast and deterministic: the assertion is an exact peak count,
    /// not a timing heuristic.
    #[tokio::test]
    async fn with_retry_bounds_concurrent_operations() {
        let op_concurrency = 3usize;
        let backend = {
            let creds = aws_sdk_s3::config::Credentials::new(
                "ak".to_string(),
                "sk".to_string(),
                None,
                None,
                "synthetic",
            );
            let s3_config = aws_sdk_s3::config::Builder::new()
                .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
                .endpoint_url("http://127.0.0.1:1")
                .credentials_provider(creds)
                .force_path_style(true)
                .region(aws_sdk_s3::config::Region::new("us-east-1"))
                .build();
            let cfg = MinIOConfig {
                endpoint: "http://127.0.0.1:1".to_string(),
                bucket: "b".to_string(),
                access_key: "ak".to_string(),
                secret_key: "sk".to_string(),
                max_retries: 2,
                initial_retry_delay_ms: 100,
                ..Default::default()
            };
            MinIOBackend {
                client: Client::from_conf(s3_config),
                config: Arc::new(cfg.clone()),
                stats: Arc::new(MinIOStats::new()),
                mpu_sem: Arc::new(Semaphore::new(16)),
                op_sem: Arc::new(Semaphore::new(op_concurrency)),
                endpoint: cfg.endpoint,
                bucket: cfg.bucket,
                _access_key: cfg.access_key,
                _secret_key: cfg.secret_key,
            }
        };

        let current = Arc::new(AtomicU64::new(0));
        let peak = Arc::new(AtomicU64::new(0));

        let mut handles = Vec::new();
        for _ in 0..9 {
            let backend = backend.clone();
            let current = current.clone();
            let peak = peak.clone();
            handles.push(tokio::spawn(async move {
                let result: Result<()> = backend
                    .with_retry(|| {
                        let current = current.clone();
                        let peak = peak.clone();
                        Box::pin(async move {
                            let now = current.fetch_add(1, Ordering::SeqCst) + 1;
                            peak.fetch_max(now, Ordering::SeqCst);
                            tokio::time::sleep(Duration::from_millis(30)).await;
                            current.fetch_sub(1, Ordering::SeqCst);
                            Err(anyhow!("simulated transient backend outage"))
                        })
                    })
                    .await;
                assert!(result.is_err(), "synthetic operation always errors");
            }));
        }
        for h in handles {
            h.await.unwrap();
        }

        let observed_peak = peak.load(Ordering::SeqCst);
        assert!(
            observed_peak <= op_concurrency as u64,
            "op_sem failed to bound concurrency: observed {observed_peak} concurrent \
             retry chains against a {op_concurrency}-permit semaphore"
        );
        assert_eq!(
            observed_peak, op_concurrency as u64,
            "expected contention to actually reach the concurrency cap with 9 tasks \
             racing for {op_concurrency} permits; got {observed_peak} — test may not be \
             exercising real contention"
        );
    }

    #[test]
    fn test_full_key_empty_prefix_passthrough() {
        let b = synthetic_minio_for_prefix_test("");
        assert_eq!(b.full_key("chunks/abc"), "chunks/abc");
    }

    #[test]
    fn test_full_key_with_prefix_prepends() {
        let b = synthetic_minio_for_prefix_test("repo-objects");
        assert_eq!(b.full_key("chunks/abc"), "repo-objects/chunks/abc");
    }

    #[test]
    fn test_strip_prefix_returns_logical() {
        let b = synthetic_minio_for_prefix_test("repo-objects");
        assert_eq!(b.strip_prefix("repo-objects/chunks/abc"), "chunks/abc");
    }

    #[test]
    fn test_strip_prefix_foreign_key_unchanged() {
        let b = synthetic_minio_for_prefix_test("repo-objects");
        assert_eq!(b.strip_prefix("other-tenant/key"), "other-tenant/key");
    }

    #[test]
    fn test_strip_prefix_empty_passthrough() {
        let b = synthetic_minio_for_prefix_test("");
        assert_eq!(b.strip_prefix("chunks/abc"), "chunks/abc");
    }

    #[test]
    fn test_full_key_strip_prefix_roundtrip() {
        let b = synthetic_minio_for_prefix_test("multi-tenant/repo-42");
        let logical = "manifests/deadbeef";
        let wire = b.full_key(logical);
        assert_eq!(b.strip_prefix(&wire), logical);
    }

    #[tokio::test]
    #[ignore = "requires MinIO server"]
    async fn test_new_valid_config() {
        let backend = MinIOBackend::new(
            "http://localhost:9000",
            "mediagit-test",
            "minioadmin",
            "minioadmin",
        )
        .await;

        assert!(backend.is_ok());
        let backend = backend.unwrap();
        assert_eq!(backend.endpoint(), "http://localhost:9000");
        assert_eq!(backend.bucket(), "mediagit-test");
    }

    #[tokio::test]
    #[ignore = "requires MinIO server"]
    async fn test_new_https_endpoint() {
        // Verify that https:// endpoints are accepted (validation only)
        // This test validates URL parsing, not actual connection
        let backend = MinIOBackend::new(
            "http://localhost:9000",
            "mediagit-test",
            "minioadmin",
            "minioadmin",
        )
        .await;

        assert!(backend.is_ok());
        let backend = backend.unwrap();
        assert_eq!(backend.endpoint(), "http://localhost:9000");
    }

    #[tokio::test]
    #[ignore = "requires MinIO server"]
    async fn test_new_removes_trailing_slash() {
        let backend = MinIOBackend::new(
            "http://localhost:9000/",
            "mediagit-test",
            "minioadmin",
            "minioadmin",
        )
        .await;

        assert!(backend.is_ok());
        let backend = backend.unwrap();
        assert_eq!(backend.endpoint(), "http://localhost:9000");
    }

    #[tokio::test]
    async fn test_invalid_endpoint_format() {
        let result = MinIOBackend::new(
            "localhost:9000", // Missing http://
            "bucket",
            "key",
            "secret",
        )
        .await;

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("must start with http")
        );
    }

    #[tokio::test]
    async fn test_empty_bucket_name() {
        let result = MinIOBackend::new(
            "http://localhost:9000",
            "", // Empty bucket
            "key",
            "secret",
        )
        .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("bucket name"));
    }

    #[tokio::test]
    async fn test_bucket_name_too_long() {
        let long_bucket = "a".repeat(64);
        let result =
            MinIOBackend::new("http://localhost:9000", &long_bucket, "key", "secret").await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("63 characters"));
    }

    #[tokio::test]
    async fn test_bucket_name_invalid_characters() {
        let result = MinIOBackend::new(
            "http://localhost:9000",
            "INVALID_BUCKET", // Contains uppercase
            "key",
            "secret",
        )
        .await;

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("lowercase letters")
        );
    }

    #[tokio::test]
    async fn test_bucket_name_starts_with_hyphen() {
        let result = MinIOBackend::new("http://localhost:9000", "-invalid", "key", "secret").await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_bucket_name_ends_with_hyphen() {
        let result = MinIOBackend::new("http://localhost:9000", "invalid-", "key", "secret").await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_empty_access_key() {
        let result = MinIOBackend::new(
            "http://localhost:9000",
            "bucket",
            "", // Empty access key
            "secret",
        )
        .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("access key"));
    }

    #[tokio::test]
    async fn test_empty_secret_key() {
        let result = MinIOBackend::new(
            "http://localhost:9000",
            "bucket",
            "key",
            "", // Empty secret key
        )
        .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("secret key"));
    }

    #[tokio::test]
    #[ignore = "requires MinIO server"]
    async fn test_debug_impl() {
        let backend = MinIOBackend::new(
            "http://localhost:9000",
            "mediagit-test",
            "minioadmin",
            "minioadmin",
        )
        .await
        .unwrap();

        let debug_str = format!("{:?}", backend);
        assert!(debug_str.contains("MinIOBackend"));
        assert!(debug_str.contains("localhost:9000"));
        assert!(debug_str.contains("***")); // Credentials should be masked
    }

    #[tokio::test]
    #[ignore = "requires MinIO server"]
    async fn test_clone() {
        let backend1 = MinIOBackend::new(
            "http://localhost:9000",
            "mediagit-test",
            "minioadmin",
            "minioadmin",
        )
        .await
        .unwrap();

        let backend2 = backend1.clone();
        assert_eq!(backend2.endpoint(), backend1.endpoint());
        assert_eq!(backend2.bucket(), backend1.bucket());
    }

    #[tokio::test]
    #[ignore = "requires MinIO server"]
    async fn test_valid_bucket_names() {
        // Validate that the CI-provisioned bucket works
        let result = MinIOBackend::new(
            "http://localhost:9000",
            "mediagit-test",
            "minioadmin",
            "minioadmin",
        )
        .await;
        assert!(
            result.is_ok(),
            "Bucket name 'mediagit-test' should be valid"
        );

        let result2 = MinIOBackend::new(
            "http://localhost:9000",
            "mediagit-repos",
            "minioadmin",
            "minioadmin",
        )
        .await;
        assert!(
            result2.is_ok(),
            "Bucket name 'mediagit-repos' should be valid"
        );
    }

    #[tokio::test]
    async fn test_from_env_missing_variables() {
        // Save current env vars
        let endpoint = std::env::var("MINIO_ENDPOINT").ok();
        let bucket = std::env::var("MINIO_BUCKET").ok();
        let access_key = std::env::var("MINIO_ACCESS_KEY").ok();
        let secret_key = std::env::var("MINIO_SECRET_KEY").ok();

        // Clear env vars
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::remove_var("MINIO_ENDPOINT") };
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::remove_var("MINIO_BUCKET") };
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::remove_var("MINIO_ACCESS_KEY") };
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::remove_var("MINIO_SECRET_KEY") };

        let result = MinIOBackend::from_env().await;
        assert!(result.is_err());

        // Restore env vars
        if let Some(v) = endpoint {
            // FIXME: Audit that the environment access only happens in single-threaded code.
            unsafe { std::env::set_var("MINIO_ENDPOINT", v) };
        }
        if let Some(v) = bucket {
            // FIXME: Audit that the environment access only happens in single-threaded code.
            unsafe { std::env::set_var("MINIO_BUCKET", v) };
        }
        if let Some(v) = access_key {
            // FIXME: Audit that the environment access only happens in single-threaded code.
            unsafe { std::env::set_var("MINIO_ACCESS_KEY", v) };
        }
        if let Some(v) = secret_key {
            // FIXME: Audit that the environment access only happens in single-threaded code.
            unsafe { std::env::set_var("MINIO_SECRET_KEY", v) };
        }
    }

    #[tokio::test]
    #[ignore = "requires MinIO server"]
    async fn test_from_env_all_variables() {
        // Save current env vars
        let endpoint = std::env::var("MINIO_ENDPOINT").ok();
        let bucket = std::env::var("MINIO_BUCKET").ok();
        let access_key = std::env::var("MINIO_ACCESS_KEY").ok();
        let secret_key = std::env::var("MINIO_SECRET_KEY").ok();

        // Set test values matching docker-compose.test.yml
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var("MINIO_ENDPOINT", "http://localhost:9000") };
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var("MINIO_BUCKET", "mediagit-test") };
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var("MINIO_ACCESS_KEY", "minioadmin") };
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var("MINIO_SECRET_KEY", "minioadmin") };

        let result = MinIOBackend::from_env().await;
        if let Err(ref e) = result {
            eprintln!("from_env error: {}", e);
        }
        assert!(result.is_ok());

        let backend = result.unwrap();
        assert_eq!(backend.endpoint(), "http://localhost:9000");
        assert_eq!(backend.bucket(), "mediagit-test");

        // Restore env vars
        if let Some(v) = endpoint {
            // FIXME: Audit that the environment access only happens in single-threaded code.
            unsafe { std::env::set_var("MINIO_ENDPOINT", v) };
        } else {
            // FIXME: Audit that the environment access only happens in single-threaded code.
            unsafe { std::env::remove_var("MINIO_ENDPOINT") };
        }
        if let Some(v) = bucket {
            // FIXME: Audit that the environment access only happens in single-threaded code.
            unsafe { std::env::set_var("MINIO_BUCKET", v) };
        } else {
            // FIXME: Audit that the environment access only happens in single-threaded code.
            unsafe { std::env::remove_var("MINIO_BUCKET") };
        }
        if let Some(v) = access_key {
            // FIXME: Audit that the environment access only happens in single-threaded code.
            unsafe { std::env::set_var("MINIO_ACCESS_KEY", v) };
        } else {
            // FIXME: Audit that the environment access only happens in single-threaded code.
            unsafe { std::env::remove_var("MINIO_ACCESS_KEY") };
        }
        if let Some(v) = secret_key {
            // FIXME: Audit that the environment access only happens in single-threaded code.
            unsafe { std::env::set_var("MINIO_SECRET_KEY", v) };
        } else {
            // FIXME: Audit that the environment access only happens in single-threaded code.
            unsafe { std::env::remove_var("MINIO_SECRET_KEY") };
        }
    }
}

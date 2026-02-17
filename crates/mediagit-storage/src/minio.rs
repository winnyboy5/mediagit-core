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
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use aws_sdk_s3::Client;
use bytes::Bytes;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
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

    /// Use path-style addressing (default: true for MinIO)
    pub path_style: bool,

    /// Multipart upload part size in bytes (default: 100MB)
    pub part_size: u64,

    /// Maximum number of concurrent parts to upload (default: 8)
    pub max_concurrent_parts: usize,

    /// Maximum number of retries for failed operations (default: 3)
    pub max_retries: u32,

    /// Initial retry delay in milliseconds (default: 100ms)
    pub initial_retry_delay_ms: u64,
}

impl Default for MinIOConfig {
    fn default() -> Self {
        MinIOConfig {
            endpoint: String::new(),
            bucket: String::new(),
            access_key: String::new(),
            secret_key: String::new(),
            path_style: true,
            part_size: 100 * 1024 * 1024, // 100MB default
            max_concurrent_parts: 8,
            max_retries: 3,
            initial_retry_delay_ms: 100,
        }
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
#[derive(Clone)]
pub struct MinIOBackend {
    client: Client,
    config: Arc<MinIOConfig>,
    stats: Arc<MinIOStats>,
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
    pub async fn with_config(config: MinIOConfig) -> Result<Self> {
        debug!(
            "Initializing MinIO backend: endpoint={}, bucket={}, path_style={}",
            config.endpoint, config.bucket, config.path_style
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
        let s3_config = aws_sdk_s3::config::Builder::new()
            .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
            .endpoint_url(&config.endpoint)
            .credentials_provider(credentials)
            .force_path_style(config.path_style)
            .region(aws_sdk_s3::config::Region::new("us-east-1"))
            .build();

        let client = Client::from_conf(s3_config);

        // Verify bucket access
        client
            .head_bucket()
            .bucket(&config.bucket)
            .send()
            .await
            .context(format!(
                "Failed to verify MinIO bucket access: {}",
                config.bucket
            ))?;

        debug!(
            "Successfully connected to MinIO bucket: {}",
            config.bucket
        );

        Ok(MinIOBackend {
            client,
            config: Arc::new(config.clone()),
            stats: Arc::new(MinIOStats::new()),
            endpoint: config.endpoint,
            bucket: config.bucket,
            _access_key: config.access_key,
            _secret_key: config.secret_key,
        })
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

    /// Perform operation with exponential backoff retry logic
    async fn with_retry<F, T>(&self, mut operation: F) -> Result<T>
    where
        F: FnMut() -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<T>> + Send>,
        >,
    {
        let mut retry_count = 0;
        let mut delay_ms = self.config.initial_retry_delay_ms;

        loop {
            match operation().await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    retry_count += 1;
                    if retry_count >= self.config.max_retries {
                        return Err(e).context(format!(
                            "Failed after {} retries",
                            self.config.max_retries
                        ));
                    }

                    warn!(
                        "Operation failed (attempt {}/{}), retrying in {}ms: {}",
                        retry_count, self.config.max_retries, delay_ms, e
                    );

                    tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;

                    // Exponential backoff with jitter
                    delay_ms = (delay_ms * 2).min(10000); // Cap at 10 seconds
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
                debug!("Putting object to MinIO (simple): {} ({} bytes)", key, body.len());

                client
                    .put_object()
                    .bucket(&bucket)
                    .key(&key)
                    .body(body.clone().into())
                    .send()
                    .await
                    .map_err(|e| anyhow!("Failed to put object: {}", e))?;

                stats.total_bytes_uploaded.fetch_add(body.len() as u64, Ordering::Relaxed);

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
        let part_size = self.config.part_size as usize;
        let mut part_number = 1;

        for chunk in data.chunks(part_size) {
            let client = client.clone();
            let bucket = bucket.clone();
            let key = key_clone.clone();
            let upload_id = upload_id.clone();
            let stats = self.stats.clone();
            let chunk_data = chunk.to_vec();
            let part_num = part_number;

            let handle = tokio::spawn(async move {
                debug!(
                    "Uploading part {} ({} bytes) for key: {}",
                    part_num,
                    chunk_data.len(),
                    key
                );

                let response = client
                    .upload_part()
                    .bucket(&bucket)
                    .key(&key)
                    .upload_id(&upload_id)
                    .part_number(part_num as i32)
                    .body(Bytes::from(chunk_data.clone()).into())
                    .send()
                    .await
                    .map_err(|e| anyhow!("Failed to upload part {}: {}", part_num, e))?;

                let etag = response
                    .e_tag()
                    .ok_or_else(|| anyhow!("No ETag returned for part {}", part_num))?
                    .to_string();

                stats.total_bytes_uploaded.fetch_add(chunk_data.len() as u64, Ordering::Relaxed);

                Ok::<_, anyhow::Error>((part_num, etag))
            });

            part_handles.push(handle);

            // Limit concurrent uploads
            if part_handles.len() >= self.config.max_concurrent_parts {
                // Wait for one to complete before starting more
                if let Some(handle) = part_handles.pop() {
                    let _ = handle.await??;
                }
            }

            part_number += 1;
        }

        // Wait for all remaining parts to complete
        let mut parts = vec![];
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
                    .part_number(part_num as i32)
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
        let key_clone = key.to_string();
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
                    .map_err(|e| anyhow!("Failed to get object: {}", e))?;

                let body = response
                    .body
                    .collect()
                    .await
                    .map_err(|e| anyhow!("Failed to read object body: {}", e))?;

                let data = body.into_bytes().to_vec();
                stats.total_bytes_downloaded.fetch_add(data.len() as u64, Ordering::Relaxed);

                Ok(data)
            })
        })
        .await
    }

    /// Store an object in MinIO
    async fn put(&self, key: &str, data: &[u8]) -> anyhow::Result<()> {
        Self::validate_key(key)?;

        // For small objects, use simple put_object
        if data.len() as u64 <= self.config.part_size {
            return self.put_simple(key, data).await;
        }

        // For large objects, use multipart upload
        self.put_multipart(key, data).await
    }

    /// Check if an object exists in MinIO
    async fn exists(&self, key: &str) -> anyhow::Result<bool> {
        Self::validate_key(key)?;

        let client = self.client.clone();
        let bucket = self.config.bucket.clone();
        let key_clone = key.to_string();

        self.with_retry(|| {
            let client = client.clone();
            let bucket = bucket.clone();
            let key = key_clone.clone();

            Box::pin(async move {
                debug!("Checking if object exists in MinIO: {}", key);

                match client
                    .head_object()
                    .bucket(&bucket)
                    .key(&key)
                    .send()
                    .await
                {
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

    /// Delete an object from MinIO
    async fn delete(&self, key: &str) -> anyhow::Result<()> {
        Self::validate_key(key)?;

        let client = self.client.clone();
        let bucket = self.config.bucket.clone();
        let key_clone = key.to_string();
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

    /// List objects in MinIO with a given prefix
    async fn list_objects(&self, prefix: &str) -> anyhow::Result<Vec<String>> {
        let client = self.client.clone();
        let bucket = self.config.bucket.clone();
        let prefix_clone = prefix.to_string();

        self.with_retry(|| {
            let client = client.clone();
            let bucket = bucket.clone();
            let prefix = prefix_clone.clone();

            Box::pin(async move {
                debug!("Listing objects in MinIO with prefix: '{}'", prefix);

                let mut result = vec![];
                let mut continuation_token: Option<String> = None;

                loop {
                    let mut request = client
                        .list_objects_v2()
                        .bucket(&bucket);

                    if !prefix.is_empty() {
                        request = request.prefix(&prefix);
                    }

                    if let Some(token) = continuation_token {
                        request = request.continuation_token(token);
                    }

                    let response = request
                        .send()
                        .await
                        .map_err(|e| anyhow!("Failed to list objects: {}", e))?;

                    // Collect keys from this page
                    for obj in response.contents() {
                        if let Some(key) = obj.key() {
                            result.push(key.to_string());
                        }
                    }

                    // Check if there are more results
                    if response.is_truncated() == Some(true) {
                        continuation_token = response.next_continuation_token().map(|t| t.to_string());
                    } else {
                        break;
                    }
                }

                // Sort for consistency
                result.sort();

                debug!("Found {} objects with prefix: '{}'", result.len(), prefix);
                Ok(result)
            })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires MinIO server"]
    async fn test_new_valid_config() {
        let backend = MinIOBackend::new(
            "http://localhost:9000",
            "test-bucket",
            "minioadmin",
            "minioadmin",
        )
        .await;

        assert!(backend.is_ok());
        let backend = backend.unwrap();
        assert_eq!(backend.endpoint(), "http://localhost:9000");
        assert_eq!(backend.bucket(), "test-bucket");
    }

    #[tokio::test]
    #[ignore = "requires MinIO server"]
    async fn test_new_https_endpoint() {
        let backend = MinIOBackend::new(
            "https://minio.example.com",
            "my-bucket",
            "admin",
            "secure-password",
        )
        .await;

        assert!(backend.is_ok());
        let backend = backend.unwrap();
        assert_eq!(backend.endpoint(), "https://minio.example.com");
    }

    #[tokio::test]
    #[ignore = "requires MinIO server"]
    async fn test_new_removes_trailing_slash() {
        let backend = MinIOBackend::new(
            "http://localhost:9000/",
            "bucket",
            "key",
            "secret",
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
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("must start with http"));
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
        let result = MinIOBackend::new(
            "http://localhost:9000",
            &long_bucket,
            "key",
            "secret",
        )
        .await;

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
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("lowercase letters"));
    }

    #[tokio::test]
    async fn test_bucket_name_starts_with_hyphen() {
        let result = MinIOBackend::new(
            "http://localhost:9000",
            "-invalid",
            "key",
            "secret",
        )
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_bucket_name_ends_with_hyphen() {
        let result = MinIOBackend::new(
            "http://localhost:9000",
            "invalid-",
            "key",
            "secret",
        )
        .await;

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
            "bucket",
            "key",
            "secret",
        )
        .await
        .unwrap();

        let debug_str = format!("{:?}", backend);
        assert!(debug_str.contains("MinIOBackend"));
        assert!(debug_str.contains("localhost:9000"));
        assert!(debug_str.contains("***")); // Credentials should be masked
        assert!(!debug_str.contains("minioadmin"));
    }

    #[tokio::test]
    #[ignore = "requires MinIO server"]
    async fn test_clone() {
        let backend1 = MinIOBackend::new(
            "http://localhost:9000",
            "bucket",
            "key",
            "secret",
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
        let valid_names = vec![
            "my-bucket",
            "bucket123",
            "a",
            "my-bucket-123",
            "1234567890",
        ];

        for name in valid_names {
            let result = MinIOBackend::new(
                "http://localhost:9000",
                name,
                "key",
                "secret",
            )
            .await;

            assert!(
                result.is_ok(),
                "Bucket name '{}' should be valid",
                name
            );
        }
    }

    #[tokio::test]
    async fn test_from_env_missing_variables() {
        // Save current env vars
        let endpoint = std::env::var("MINIO_ENDPOINT").ok();
        let bucket = std::env::var("MINIO_BUCKET").ok();
        let access_key = std::env::var("MINIO_ACCESS_KEY").ok();
        let secret_key = std::env::var("MINIO_SECRET_KEY").ok();

        // Clear env vars
        std::env::remove_var("MINIO_ENDPOINT");
        std::env::remove_var("MINIO_BUCKET");
        std::env::remove_var("MINIO_ACCESS_KEY");
        std::env::remove_var("MINIO_SECRET_KEY");

        let result = MinIOBackend::from_env().await;
        assert!(result.is_err());

        // Restore env vars
        if let Some(v) = endpoint {
            std::env::set_var("MINIO_ENDPOINT", v);
        }
        if let Some(v) = bucket {
            std::env::set_var("MINIO_BUCKET", v);
        }
        if let Some(v) = access_key {
            std::env::set_var("MINIO_ACCESS_KEY", v);
        }
        if let Some(v) = secret_key {
            std::env::set_var("MINIO_SECRET_KEY", v);
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

        // Set test values
        std::env::set_var("MINIO_ENDPOINT", "http://localhost:9000");
        std::env::set_var("MINIO_BUCKET", "test-bucket");
        std::env::set_var("MINIO_ACCESS_KEY", "testkey");
        std::env::set_var("MINIO_SECRET_KEY", "testsecret");

        let result = MinIOBackend::from_env().await;
        if let Err(ref e) = result {
            eprintln!("from_env error: {}", e);
        }
        assert!(result.is_ok());

        let backend = result.unwrap();
        assert_eq!(backend.endpoint(), "http://localhost:9000");
        assert_eq!(backend.bucket(), "test-bucket");

        // Restore env vars
        if let Some(v) = endpoint {
            std::env::set_var("MINIO_ENDPOINT", v);
        } else {
            std::env::remove_var("MINIO_ENDPOINT");
        }
        if let Some(v) = bucket {
            std::env::set_var("MINIO_BUCKET", v);
        } else {
            std::env::remove_var("MINIO_BUCKET");
        }
        if let Some(v) = access_key {
            std::env::set_var("MINIO_ACCESS_KEY", v);
        } else {
            std::env::remove_var("MINIO_ACCESS_KEY");
        }
        if let Some(v) = secret_key {
            std::env::set_var("MINIO_SECRET_KEY", v);
        } else {
            std::env::remove_var("MINIO_SECRET_KEY");
        }
    }
}

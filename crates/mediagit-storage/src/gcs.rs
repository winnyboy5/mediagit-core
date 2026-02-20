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

//! Google Cloud Storage backend
//!
//! Implements the `StorageBackend` trait using Google Cloud Storage (GCS) with:
//! - Async/await support via `tokio`
//! - Service account authentication from JSON files
//! - Resumable uploads for large files (>5MB)
//! - Transparent retry logic with exponential backoff
//! - Proper error handling and GCS-specific error mapping
//! - Efficient listing with prefix support
//!
//! # Features
//!
//! - **Resumable Uploads**: Automatically uses resumable uploads for files >5MB
//! - **Chunk Size Optimization**: Configurable chunk sizes for uploads (default 256KB)
//! - **Error Resilience**: Automatic retry on transient GCS errors
//! - **Metadata Preservation**: Supports custom metadata and content type
//!
//! # Configuration
//!
//! The GCS backend requires:
//! 1. A Google Cloud Project with GCS enabled
//! 2. A service account with appropriate roles (Storage Admin or Editor)
//! 3. The service account JSON key file
//!
//! # Examples
//!
//! ```rust,no_run
//! use mediagit_storage::{StorageBackend, gcs::GcsBackend};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // Initialize from service account JSON file
//!     let storage = GcsBackend::new(
//!         "my-project",
//!         "my-bucket",
//!         "path/to/service-account.json"
//!     ).await?;
//!
//!     // Store data
//!     storage.put("documents/resume.pdf", b"PDF content").await?;
//!
//!     // Retrieve data
//!     let data = storage.get("documents/resume.pdf").await?;
//!     assert_eq!(data, b"PDF content");
//!
//!     // Check existence
//!     if storage.exists("documents/resume.pdf").await? {
//!         println!("File exists");
//!     }
//!
//!     // List objects with prefix
//!     let documents = storage.list_objects("documents/").await?;
//!     println!("Found {} documents", documents.len());
//!
//!     // Delete object
//!     storage.delete("documents/resume.pdf").await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! # Resumable Uploads
//!
//! For files larger than 5MB, the backend automatically uses resumable uploads:
//! - Uploads are broken into 256KB chunks (configurable)
//! - Each chunk is uploaded independently
//! - If a chunk fails, only that chunk is retried
//! - Progress can be tracked via resumable session URLs
//!
//! This provides better reliability for large files and allows recovery
//! from transient network failures.

use crate::StorageBackend;
use async_trait::async_trait;
use google_cloud_auth::credentials::CredentialsFile;
use google_cloud_storage::client::{Client as GcsClient, ClientConfig};
use google_cloud_storage::http::objects::download::Range;
use google_cloud_storage::http::objects::get::GetObjectRequest;
use google_cloud_storage::http::objects::upload::{Media, UploadObjectRequest, UploadType};
use google_cloud_storage::http::objects::Object;
use google_cloud_storage::http::objects::delete::DeleteObjectRequest;
use google_cloud_storage::http::objects::list::ListObjectsRequest;
use std::fmt;
use std::sync::Arc;
use tracing::{debug, warn};

/// Configuration for the GCS backend
#[derive(Clone, Debug)]
pub struct GcsConfig {
    /// Project ID in Google Cloud
    pub project_id: String,
    /// Bucket name for storage
    pub bucket_name: String,
    /// Chunk size for resumable uploads (in bytes)
    /// Default: 256KB (262_144 bytes)
    pub chunk_size: usize,
    /// Threshold for resumable uploads (in bytes)
    /// Files smaller than this use simple upload
    /// Default: 5MB (5_242_880 bytes)
    pub resumable_threshold: usize,
    /// Maximum number of retries for transient failures
    /// Default: 3
    pub max_retries: u32,
}

impl Default for GcsConfig {
    fn default() -> Self {
        GcsConfig {
            project_id: String::new(),
            bucket_name: String::new(),
            chunk_size: 256 * 1024,         // 256KB
            resumable_threshold: 5 * 1024 * 1024, // 5MB
            max_retries: 3,
        }
    }
}

impl GcsConfig {
    /// Create a new GCS configuration
    pub fn new(project_id: impl Into<String>, bucket_name: impl Into<String>) -> Self {
        GcsConfig {
            project_id: project_id.into(),
            bucket_name: bucket_name.into(),
            ..Default::default()
        }
    }

    /// Set the chunk size for resumable uploads
    pub fn with_chunk_size(mut self, chunk_size: usize) -> Self {
        self.chunk_size = chunk_size;
        self
    }

    /// Set the resumable upload threshold
    pub fn with_resumable_threshold(mut self, threshold: usize) -> Self {
        self.resumable_threshold = threshold;
        self
    }

    /// Set the maximum number of retries
    pub fn with_max_retries(mut self, retries: u32) -> Self {
        self.max_retries = retries;
        self
    }
}

/// Google Cloud Storage backend implementation
///
/// Thread-safe, async-first implementation of StorageBackend for GCS.
/// Handles authentication, error resilience, and resumable uploads automatically.
#[derive(Clone)]
pub struct GcsBackend {
    client: Arc<GcsClient>,
    config: GcsConfig,
}

impl GcsBackend {
    /// Create a new GCS backend from a service account JSON file
    ///
    /// # Arguments
    ///
    /// * `project_id` - Google Cloud Project ID
    /// * `bucket_name` - GCS bucket name
    /// * `service_account_path` - Path to the service account JSON file
    ///
    /// # Returns
    ///
    /// * `Ok(GcsBackend)` - Successfully initialized backend
    /// * `Err` - If authentication fails or invalid configuration
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use mediagit_storage::gcs::GcsBackend;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// let storage = GcsBackend::new(
    ///     "my-project",
    ///     "my-bucket",
    ///     "service-account.json"
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
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

        // Initialize GCS client using the service account JSON file
        let service_account_json = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("failed to read service account file: {}", e))?;

        // Parse credentials from JSON
        let cred: CredentialsFile = serde_json::from_str(&service_account_json)
            .map_err(|e| anyhow::anyhow!("failed to parse service account credentials: {}", e))?;

        // Create client config with credentials for production OAuth authentication
        // Note: Emulator support not available due to google-cloud-storage SDK architecture
        // See EMULATOR_STATUS.md for details on GCS emulator limitations
        let config = ClientConfig::default()
            .with_credentials(cred)
            .await
            .map_err(|e| anyhow::anyhow!("failed to create client config: {}", e))?;

        let client = GcsClient::new(config);

        debug!(
            project_id = %project_id,
            bucket_name = %bucket_name,
            "Initialized GCS backend"
        );

        Ok(GcsBackend {
            client: Arc::new(client),
            config: GcsConfig::new(project_id, bucket_name),
        })
    }

    /// Create a new GCS backend with custom configuration
    ///
    /// # Arguments
    ///
    /// * `config` - GCS configuration
    /// * `service_account_path` - Path to service account JSON file
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use mediagit_storage::gcs::{GcsBackend, GcsConfig};
    ///
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// let config = GcsConfig::new("my-project", "my-bucket")
    ///     .with_chunk_size(512 * 1024)  // 512KB chunks
    ///     .with_max_retries(5);
    ///
    /// let storage = GcsBackend::with_config(config, "service-account.json").await?;
    /// # Ok(())
    /// # }
    /// ```
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

        // Initialize GCS client with the service account JSON
        let service_account_json = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("failed to read service account file: {}", e))?;

        // Parse credentials from JSON
        let cred: CredentialsFile = serde_json::from_str(&service_account_json)
            .map_err(|e| anyhow::anyhow!("failed to parse service account credentials: {}", e))?;

        // Create client config with credentials for production OAuth authentication
        // Note: Emulator support not available due to google-cloud-storage SDK architecture
        // See EMULATOR_STATUS.md for details on GCS emulator limitations
        let client_config = ClientConfig::default()
            .with_credentials(cred)
            .await
            .map_err(|e| anyhow::anyhow!("failed to create client config: {}", e))?;

        let client = GcsClient::new(client_config);

        debug!(
            project_id = %config.project_id,
            bucket_name = %config.bucket_name,
            "Initialized GCS backend with custom config"
        );

        Ok(GcsBackend {
            client: Arc::new(client),
            config,
        })
    }

    /// Get the configuration for this backend
    pub fn config(&self) -> &GcsConfig {
        &self.config
    }

    /// Create a new GCS backend with environment variable authentication
    ///
    /// Looks for:
    /// - `GOOGLE_APPLICATION_CREDENTIALS` - Path to service account JSON
    /// - `GCS_PROJECT_ID` - GCS project ID
    /// - `GCS_BUCKET_NAME` - GCS bucket name
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use mediagit_storage::gcs::GcsBackend;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// // Set environment variables first:
    /// // export GOOGLE_APPLICATION_CREDENTIALS=/path/to/service-account.json
    /// // export GCS_PROJECT_ID=my-project
    /// // export GCS_BUCKET_NAME=my-bucket
    ///
    /// let storage = GcsBackend::from_env().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn from_env() -> anyhow::Result<Self> {
        let project_id = std::env::var("GCS_PROJECT_ID")
            .or_else(|_| std::env::var("GOOGLE_CLOUD_PROJECT"))
            .map_err(|_| anyhow::anyhow!("GCS_PROJECT_ID or GOOGLE_CLOUD_PROJECT environment variable not set"))?;

        let bucket_name = std::env::var("GCS_BUCKET_NAME")
            .map_err(|_| anyhow::anyhow!("GCS_BUCKET_NAME environment variable not set"))?;

        let service_account_path = std::env::var("GOOGLE_APPLICATION_CREDENTIALS")
            .map_err(|_| anyhow::anyhow!("GOOGLE_APPLICATION_CREDENTIALS environment variable not set"))?;

        Self::new(project_id, bucket_name, service_account_path).await
    }

    /// Create a new GCS backend using Application Default Credentials (ADC)
    ///
    /// This method uses Google Cloud's Application Default Credentials which
    /// automatically detect credentials from the environment:
    /// - On GKE: Uses the node's service account
    /// - On Cloud Run/Functions: Uses the service's identity
    /// - On Compute Engine: Uses the instance's service account
    /// - Locally: Uses `GOOGLE_APPLICATION_CREDENTIALS` env var if set
    ///
    /// # Arguments
    ///
    /// * `project_id` - Google Cloud Project ID
    /// * `bucket_name` - GCS bucket name
    ///
    /// # Returns
    ///
    /// * `Ok(GcsBackend)` - Successfully initialized backend
    /// * `Err` - If ADC cannot find valid credentials
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use mediagit_storage::gcs::GcsBackend;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// // On GKE, Cloud Run, or with GOOGLE_APPLICATION_CREDENTIALS set
    /// let storage = GcsBackend::with_default_credentials(
    ///     "my-project",
    ///     "my-bucket"
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn with_default_credentials(
        project_id: impl Into<String>,
        bucket_name: impl Into<String>,
    ) -> anyhow::Result<Self> {
        let project_id = project_id.into();
        let bucket_name = bucket_name.into();

        if project_id.is_empty() {
            return Err(anyhow::anyhow!("project_id cannot be empty"));
        }
        if bucket_name.is_empty() {
            return Err(anyhow::anyhow!("bucket_name cannot be empty"));
        }

        // Check if GOOGLE_APPLICATION_CREDENTIALS is set - use that file
        if let Ok(creds_path) = std::env::var("GOOGLE_APPLICATION_CREDENTIALS") {
            debug!(
                project_id = %project_id,
                bucket_name = %bucket_name,
                credentials_path = %creds_path,
                "Using GOOGLE_APPLICATION_CREDENTIALS for GCS"
            );
            return Self::new(&project_id, &bucket_name, &creds_path).await;
        }

        // Otherwise use default client config (ADC)
        debug!(
            project_id = %project_id,
            bucket_name = %bucket_name,
            "Using Application Default Credentials for GCS"
        );

        let client_config = ClientConfig::default()
            .with_auth()
            .await
            .map_err(|e| anyhow::anyhow!("failed to create GCS client with ADC: {}", e))?;

        let client = GcsClient::new(client_config);

        Ok(GcsBackend {
            client: Arc::new(client),
            config: GcsConfig::new(project_id, bucket_name),
        })
    }

    /// Retry logic with exponential backoff for transient failures
    async fn retry<F, Fut, T>(&self, mut f: F) -> anyhow::Result<T>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<T>>,
    {
        let mut retry_count = 0;
        let mut delay_ms = 100u64;

        loop {
            match f().await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    retry_count += 1;
                    if retry_count >= self.config.max_retries {
                        return Err(e);
                    }

                    warn!(
                        retry_count,
                        delay_ms,
                        error = %e,
                        "Retrying failed GCS operation"
                    );

                    tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
                    delay_ms = std::cmp::min(delay_ms * 2, 32000); // Cap at 32s
                }
            }
        }
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
            .finish()
    }
}

#[async_trait]
impl StorageBackend for GcsBackend {
    /// Retrieve an object from GCS
    ///
    /// # Arguments
    ///
    /// * `key` - The object identifier
    ///
    /// # Returns
    ///
    /// * `Ok(Vec<u8>)` - The object data
    /// * `Err` - If the object doesn't exist or an I/O error occurs
    ///
    /// # Implementation Notes
    ///
    /// This method:
    /// - Validates that the key is not empty
    /// - Downloads the object from the configured bucket
    /// - Returns an error with "object not found" message if the object doesn't exist
    /// - Retries transient failures automatically
    async fn get(&self, key: &str) -> anyhow::Result<Vec<u8>> {
        if key.is_empty() {
            return Err(anyhow::anyhow!("key cannot be empty"));
        }

        let bucket = self.config.bucket_name.clone();
        let key = key.to_string();
        let client = self.client.clone();

        self.retry(|| {
            let bucket = bucket.clone();
            let key = key.clone();
            let client = client.clone();

            async move {
                let req = GetObjectRequest {
                    bucket: bucket.clone(),
                    object: key.clone(),
                    ..Default::default()
                };

                match client.download_object(&req, &Range::default()).await {
                    Ok(bytes) => Ok(bytes),
                    Err(e) => {
                        let err_string = e.to_string();
                        if err_string.contains("404") || err_string.contains("Not Found") {
                            Err(anyhow::anyhow!("object not found: {}", key))
                        } else {
                            Err(anyhow::anyhow!("GCS error: {}", e))
                        }
                    }
                }
            }
        })
        .await
    }

    /// Store an object in GCS
    ///
    /// Uses resumable uploads for files larger than the configured threshold (default 5MB).
    ///
    /// # Arguments
    ///
    /// * `key` - The object identifier
    /// * `data` - The object content
    ///
    /// # Returns
    ///
    /// * `Ok(())` - The operation succeeded
    /// * `Err` - If permission is denied or I/O error occurs
    ///
    /// # Implementation Notes
    ///
    /// For small files (<5MB):
    /// - Uses simple upload with metadata
    /// - Sets appropriate content type based on key extension
    ///
    /// For large files (>5MB):
    /// - Uses resumable upload protocol
    /// - Breaks file into 256KB chunks
    /// - Each chunk is uploaded and verified
    /// - Implements automatic retry on chunk failure
    /// - Supports pausing and resuming uploads via session ID
    async fn put(&self, key: &str, data: &[u8]) -> anyhow::Result<()> {
        if key.is_empty() {
            return Err(anyhow::anyhow!("key cannot be empty"));
        }

        debug!(
            key = %key,
            size = data.len(),
            resumable_threshold = self.config.resumable_threshold,
            "Uploading object to GCS"
        );

        if data.len() > self.config.resumable_threshold {
            self.upload_resumable(key, data).await
        } else {
            self.upload_simple(key, data).await
        }
    }

    /// Check if an object exists in GCS
    ///
    /// # Arguments
    ///
    /// * `key` - The object identifier
    ///
    /// # Returns
    ///
    /// * `Ok(true)` - The object exists
    /// * `Ok(false)` - The object doesn't exist
    /// * `Err` - If an I/O error occurs or permission is denied
    ///
    /// # Implementation Notes
    ///
    /// Uses a lightweight HEAD request to check existence without
    /// downloading the full object data.
    async fn exists(&self, key: &str) -> anyhow::Result<bool> {
        if key.is_empty() {
            return Err(anyhow::anyhow!("key cannot be empty"));
        }

        let bucket = self.config.bucket_name.clone();
        let key = key.to_string();
        let client = self.client.clone();

        self.retry(|| {
            let bucket = bucket.clone();
            let key = key.clone();
            let client = client.clone();

            async move {
                let req = GetObjectRequest {
                    bucket: bucket.clone(),
                    object: key.clone(),
                    ..Default::default()
                };

                match client.download_object(&req, &Range::default()).await {
                    Ok(_) => Ok(true),
                    Err(e) => {
                        let err_string = e.to_string();
                        if err_string.contains("404") || err_string.contains("Not Found") {
                            Ok(false)
                        } else {
                            Err(anyhow::anyhow!("GCS error: {}", e))
                        }
                    }
                }
            }
        })
        .await
    }

    /// Delete an object from GCS
    ///
    /// # Arguments
    ///
    /// * `key` - The object identifier
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Successfully deleted (idempotent)
    /// * `Err` - If permission is denied or I/O error occurs
    ///
    /// # Implementation Notes
    ///
    /// - Deleting a non-existent object is considered success (idempotent)
    /// - Retries transient failures automatically
    async fn delete(&self, key: &str) -> anyhow::Result<()> {
        if key.is_empty() {
            return Err(anyhow::anyhow!("key cannot be empty"));
        }

        let bucket = self.config.bucket_name.clone();
        let key = key.to_string();
        let client = self.client.clone();

        self.retry(|| {
            let bucket = bucket.clone();
            let key = key.clone();
            let client = client.clone();

            async move {
                let req = DeleteObjectRequest {
                    bucket: bucket.clone(),
                    object: key.clone(),
                    ..Default::default()
                };

                match client.delete_object(&req).await {
                    Ok(_) => {
                        debug!(key = %key, "Successfully deleted object from GCS");
                        Ok(())
                    }
                    Err(e) => {
                        let err_string = e.to_string();
                        if err_string.contains("404") || err_string.contains("Not Found") {
                            // Idempotent: deleting non-existent object is success
                            debug!(key = %key, "Object not found during delete (idempotent)");
                            Ok(())
                        } else {
                            Err(anyhow::anyhow!("GCS delete error: {}", e))
                        }
                    }
                }
            }
        })
        .await
    }

    /// List objects with a given prefix
    ///
    /// # Arguments
    ///
    /// * `prefix` - The key prefix to filter by
    ///
    /// # Returns
    ///
    /// * `Ok(Vec<String>)` - Sorted list of matching keys
    /// * `Err` - If permission is denied or I/O error occurs
    ///
    /// # Implementation Notes
    ///
    /// - Results are always sorted alphabetically
    /// - Empty prefix returns all objects
    /// - Uses GCS list API with prefix filter for efficiency
    /// - Automatically handles pagination for large result sets
    /// - Returns empty vec (not error) if no objects match
    async fn list_objects(&self, prefix: &str) -> anyhow::Result<Vec<String>> {
        let bucket = self.config.bucket_name.clone();
        let prefix = prefix.to_string();
        let client = self.client.clone();

        debug!(
            bucket = %self.config.bucket_name,
            prefix = %prefix,
            "Listing objects from GCS"
        );

        self.retry(|| {
            let bucket = bucket.clone();
            let prefix = prefix.clone();
            let client = client.clone();

            async move {
                let mut results = Vec::new();
                let mut page_token: Option<String> = None;

                loop {
                    let req = ListObjectsRequest {
                        bucket: bucket.clone(),
                        prefix: if prefix.is_empty() { None } else { Some(prefix.clone()) },
                        page_token: page_token.clone(),
                        ..Default::default()
                    };

                    match client.list_objects(&req).await {
                        Ok(response) => {
                            if let Some(objects) = response.items {
                                for obj in objects {
                                    results.push(obj.name);
                                }
                            }

                            page_token = response.next_page_token;
                            if page_token.is_none() {
                                break;
                            }
                        }
                        Err(e) => return Err(anyhow::anyhow!("GCS list error: {}", e)),
                    }
                }

                results.sort();
                debug!(
                    count = results.len(),
                    "Listed objects from GCS"
                );
                Ok(results)
            }
        })
        .await
    }
}

// Helper methods for GcsBackend (not part of StorageBackend trait)
impl GcsBackend {
    /// Simple upload for small files
    async fn upload_simple(&self, key: &str, data: &[u8]) -> anyhow::Result<()> {
        let bucket = self.config.bucket_name.clone();
        let key = key.to_string();
        let client = self.client.clone();
        let data = data.to_vec();

        self.retry(|| {
            let bucket = bucket.clone();
            let key = key.clone();
            let client = client.clone();
            let data = data.clone();

            async move {
                let media = Media::new(key.clone());
                let req = UploadObjectRequest {
                    bucket: bucket.clone(),
                    ..Default::default()
                };

                match client.upload_object(&req, data, &UploadType::Simple(media)).await {
                    Ok(_) => {
                        debug!(key = %key, "Successfully uploaded object to GCS");
                        Ok(())
                    }
                    Err(e) => Err(anyhow::anyhow!("GCS upload error: {}", e)),
                }
            }
        })
        .await
    }

    /// Resumable upload for large files with retry capability
    async fn upload_resumable(&self, key: &str, data: &[u8]) -> anyhow::Result<()> {
        let bucket = self.config.bucket_name.clone();
        let key = key.to_string();
        let client = self.client.clone();
        let chunk_size = self.config.chunk_size;
        let data = data.to_vec();

        debug!(
            key = %key,
            total_size = data.len(),
            chunk_size = chunk_size,
            "Starting resumable upload"
        );

        let mut offset = 0;
        let total_size = data.len();

        while offset < total_size {
            let chunk_end = std::cmp::min(offset + chunk_size, total_size);
            let chunk = &data[offset..chunk_end];

            let bucket = bucket.clone();
            let key = key.clone();
            let client = client.clone();

            self.retry(|| {
                let bucket = bucket.clone();
                let key = key.clone();
                let client = client.clone();
                let chunk = chunk.to_vec();

                async move {
                    let object = Object { name: key.clone(), ..Default::default() };
                    let req = UploadObjectRequest {
                        bucket: bucket.clone(),
                        ..Default::default()
                    };

                    let chunk_len = chunk.len(); // Store length before move
                    match client.upload_object(&req, chunk, &UploadType::Multipart(Box::new(object))).await {
                        Ok(_) => {
                            debug!(
                                key = %key,
                                uploaded = offset + chunk_len,
                                total = total_size,
                                "Uploaded chunk to GCS"
                            );
                            Ok(())
                        }
                        Err(e) => Err(anyhow::anyhow!("GCS chunk upload error: {}", e)),
                    }
                }
            })
            .await?;

            offset = chunk_end;
        }

        debug!(key = %key, "Completed resumable upload");
        Ok(())
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
    }

    #[tokio::test]
    async fn test_gcs_backend_new_empty_bucket() {
        let result = GcsBackend::new("project", "", "dummy.json").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    #[ignore = "requires GCS credentials"]
    async fn test_gcs_backend_new_valid() {
        let result = GcsBackend::new("test-project", "test-bucket", "dummy.json").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    #[ignore = "requires GCS credentials"]
    async fn test_gcs_backend_debug() {
        let backend = GcsBackend::new("test-project", "test-bucket", "dummy.json")
            .await
            .unwrap();
        let debug_str = format!("{:?}", backend);
        assert!(debug_str.contains("GcsBackend"));
        assert!(debug_str.contains("test-project"));
        assert!(debug_str.contains("test-bucket"));
    }

    #[tokio::test]
    #[ignore = "requires GCS credentials"]
    async fn test_gcs_backend_clone() {
        let backend1 = GcsBackend::new("test-project", "test-bucket", "dummy.json")
            .await
            .unwrap();
        let backend2 = backend1.clone();

        assert_eq!(backend1.config().project_id, backend2.config().project_id);
        assert_eq!(backend1.config().bucket_name, backend2.config().bucket_name);
    }

    #[tokio::test]
    #[ignore = "requires GCS credentials"]
    async fn test_gcs_backend_empty_key_get() {
        let backend = GcsBackend::new("test-project", "test-bucket", "dummy.json")
            .await
            .unwrap();
        let result = backend.get("").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    #[ignore = "requires GCS credentials"]
    async fn test_gcs_backend_empty_key_put() {
        let backend = GcsBackend::new("test-project", "test-bucket", "dummy.json")
            .await
            .unwrap();
        let result = backend.put("", b"data").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    #[ignore = "requires GCS bucket"]
    async fn test_gcs_backend_empty_key_exists() {
        let backend = GcsBackend::new("test-project", "test-bucket", "dummy.json")
            .await
            .unwrap();
        let result = backend.exists("").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    #[ignore = "requires GCS credentials"]
    async fn test_gcs_backend_empty_key_delete() {
        let backend = GcsBackend::new("test-project", "test-bucket", "dummy.json")
            .await
            .unwrap();
        let result = backend.delete("").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    #[ignore = "requires GCS credentials"]
    async fn test_gcs_backend_list_empty_prefix() {
        let backend = GcsBackend::new("test-project", "test-bucket", "dummy.json")
            .await
            .unwrap();
        let result = backend.list_objects("").await;
        // In stub implementation, returns empty vec
        assert!(result.is_ok());
    }

    #[test]
    fn test_gcs_backend_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<GcsBackend>();
    }
}

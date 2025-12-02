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

//! Azure Blob Storage backend implementation
//!
//! Implements the `StorageBackend` trait using Azure Blob Storage with:
//! - Support for both SAS token and account key authentication
//! - Chunked uploads for large files (streaming multipart uploads)
//! - Proper error handling with Azure-specific error mapping
//! - Connection pooling and efficient resource management
//!
//! # Authentication Methods
//!
//! The Azure backend supports three authentication approaches:
//!
//! ## 1. SAS Token Authentication (Recommended for temporary access)
//!
//! ```rust,no_run
//! use mediagit_storage::azure::AzureBackend;
//!
//! # #[tokio::main]
//! # async fn main() -> anyhow::Result<()> {
//! let backend = AzureBackend::with_sas_token(
//!     "myaccount",
//!     "mycontainer",
//!     "sv=2021-06-08&ss=bfqt&srt=sco&sp=rwdlacupitfx&..."
//! ).await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## 2. Account Key Authentication (Default, more flexible)
//!
//! ```rust,no_run
//! use mediagit_storage::azure::AzureBackend;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let backend = AzureBackend::with_account_key(
//!         "myaccount",
//!         "mycontainer",
//!         "DefaultEndpointsProtocol=https;AccountName=myaccount;AccountKey=...;EndpointSuffix=core.windows.net"
//!     ).await?;
//!     Ok(())
//! }
//! ```
//!
//! ## 3. Connection String
//!
//! ```rust,no_run
//! use mediagit_storage::azure::AzureBackend;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let backend = AzureBackend::with_connection_string(
//!         "mycontainer",
//!         "DefaultEndpointsProtocol=https;..."
//!     ).await?;
//!     Ok(())
//! }
//! ```
//!
//! # Chunked Upload Support
//!
//! Large files are automatically uploaded in chunks (4 MB default) for efficient
//! memory usage and resumable uploads:
//!
//! ```rust,no_run
//! use mediagit_storage::{StorageBackend, azure::AzureBackend};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//! # let backend = AzureBackend::with_account_key("", "", "").await?;
//!     // Large file is automatically chunked
//!     let large_data = vec![0u8; 100_000_000]; // 100 MB
//!     backend.put("large_file.bin", &large_data).await?;
//!     Ok(())
//! }
//! ```
//!
//! # Examples
//!
//! ```rust,no_run
//! use mediagit_storage::{StorageBackend, azure::AzureBackend};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // Create backend with account key
//!     let storage = AzureBackend::with_account_key(
//!         "myaccount",
//!         "mycontainer",
//!         "DefaultEndpointsProtocol=https;..."
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
//! # Testing with Azurite
//!
//! For local development and testing, use [Azurite](https://github.com/Azure/Azurite),
//! the Azure Storage emulator:
//!
//! ```bash
//! # Install and run Azurite
//! npm install -g azurite
//! azurite
//!
//! # Connect to local emulator
//! export AZURE_STORAGE_ACCOUNT_NAME=devstoreaccount1
//! export AZURE_STORAGE_CONNECTION_STRING="DefaultEndpointsProtocol=http;AccountName=devstoreaccount1;AccountKey=Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/KBHBeksoGMGw==;BlobEndpoint=http://127.0.0.1:10000/devstoreaccount1;"
//! ```

use crate::StorageBackend;
use async_trait::async_trait;
use azure_storage::prelude::*;
use azure_storage::CloudLocation;
use azure_storage_blobs::prelude::*;
use futures::TryStreamExt;
use std::fmt;
use std::sync::Arc;

/// Chunk size for multipart uploads (4 MB)
/// This provides a good balance between memory usage and upload efficiency
const CHUNK_SIZE: usize = 4 * 1024 * 1024; // 4 MB

/// Block size for Azure block blob operations
const AZURE_BLOCK_SIZE: usize = 4 * 1024 * 1024; // 4 MB, Azure maximum is 4GB

/// Azure Blob Storage backend
///
/// Thread-safe implementation of `StorageBackend` using Azure Blob Storage.
/// Supports both SAS token and account key authentication.
///
/// # Thread Safety
///
/// This implementation is `Send + Sync` and can be safely shared across threads
/// and async tasks. Connection pooling is handled by the underlying Azure SDK.
#[derive(Clone)]
pub struct AzureBackend {
    account_name: String,
    container_name: String,
    /// The actual Azure SDK client for blob operations
    client: Arc<ContainerClient>,
}

impl fmt::Debug for AzureBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AzureBackend")
            .field("account_name", &self.account_name)
            .field("container_name", &self.container_name)
            .finish()
    }
}

impl AzureBackend {
    /// Create a new Azure Blob Storage backend using SAS token authentication
    ///
    /// SAS (Shared Access Signature) tokens are recommended for temporary access
    /// or when you want to limit permissions to specific operations.
    ///
    /// # Arguments
    ///
    /// * `account_name` - The Azure storage account name (e.g., "myaccount")
    /// * `container_name` - The blob container name (e.g., "mycontainer")
    /// * `sas_token` - The SAS token for authentication (e.g., "sv=2021-06-08&...")
    ///
    /// # Errors
    ///
    /// Returns an error if the container cannot be accessed or created.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use mediagit_storage::azure::AzureBackend;
    ///
    /// #[tokio::main]
    /// async fn main() -> anyhow::Result<()> {
    ///     let backend = AzureBackend::with_sas_token(
    ///         "myaccount",
    ///         "mycontainer",
    ///         "sv=2021-06-08&ss=bfqt&srt=sco&sp=rwdlacupitfx&..."
    ///     ).await?;
    ///
    ///     backend.put("file.bin", b"content").await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn with_sas_token(
        account_name: impl Into<String>,
        container_name: impl Into<String>,
        sas_token: impl Into<String>,
    ) -> anyhow::Result<Self> {
        let account_name = account_name.into();
        let container_name = container_name.into();
        let sas_token = sas_token.into();

        // Validate inputs
        if account_name.is_empty() {
            return Err(anyhow::anyhow!("account_name cannot be empty"));
        }
        if container_name.is_empty() {
            return Err(anyhow::anyhow!("container_name cannot be empty"));
        }
        if sas_token.is_empty() {
            return Err(anyhow::anyhow!("sas_token cannot be empty"));
        }

        // Create the container client with SAS token
        let storage_credentials = StorageCredentials::sas_token(sas_token)?;
        let container_client = ClientBuilder::new(account_name.clone(), storage_credentials)
            .container_client(container_name.clone());

        tracing::info!(
            "Created Azure Blob Storage backend with SAS token for {}/{}",
            account_name,
            container_name
        );

        let backend = AzureBackend {
            account_name: account_name.clone(),
            container_name: container_name.clone(),
            client: Arc::new(container_client),
        };

        // Ensure container exists
        backend.ensure_container_exists().await?;

        Ok(backend)
    }

    /// Create a new Azure Blob Storage backend using account key authentication
    ///
    /// Account key authentication uses the primary or secondary account key and is
    /// suitable for backend-to-backend communication with full storage permissions.
    ///
    /// # Arguments
    ///
    /// * `account_name` - The Azure storage account name (e.g., "myaccount")
    /// * `container_name` - The blob container name (e.g., "mycontainer")
    /// * `account_key` - The account key (base64-encoded, typically 88 characters)
    ///
    /// # Errors
    ///
    /// Returns an error if the container cannot be accessed.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use mediagit_storage::azure::AzureBackend;
    ///
    /// #[tokio::main]
    /// async fn main() -> anyhow::Result<()> {
    ///     let backend = AzureBackend::with_account_key(
    ///         "myaccount",
    ///         "mycontainer",
    ///         "DefaultEndpointsProtocol=https;AccountName=myaccount;AccountKey=...;EndpointSuffix=core.windows.net"
    ///     ).await?;
    ///
    ///     backend.put("file.bin", b"content").await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn with_account_key(
        account_name: impl Into<String>,
        container_name: impl Into<String>,
        account_key: impl Into<String>,
    ) -> anyhow::Result<Self> {
        let account_name = account_name.into();
        let container_name = container_name.into();
        let account_key = account_key.into();

        // Validate inputs
        if account_name.is_empty() {
            return Err(anyhow::anyhow!("account_name cannot be empty"));
        }
        if container_name.is_empty() {
            return Err(anyhow::anyhow!("container_name cannot be empty"));
        }
        if account_key.is_empty() {
            return Err(anyhow::anyhow!("account_key cannot be empty"));
        }

        // Create the container client with account key
        let storage_credentials = StorageCredentials::access_key(account_name.clone(), account_key);
        let container_client = ClientBuilder::new(account_name.clone(), storage_credentials)
            .container_client(container_name.clone());

        tracing::info!(
            "Created Azure Blob Storage backend with account key for {}/{}",
            account_name,
            container_name
        );

        let backend = AzureBackend {
            account_name: account_name.clone(),
            container_name: container_name.clone(),
            client: Arc::new(container_client),
        };

        // Ensure container exists
        backend.ensure_container_exists().await?;

        Ok(backend)
    }

    /// Create a new Azure Blob Storage backend using a connection string
    ///
    /// Connection strings can include either account keys or SAS tokens.
    /// This is a convenient way to configure the backend from environment variables.
    ///
    /// # Arguments
    ///
    /// * `container_name` - The blob container name (e.g., "mycontainer")
    /// * `connection_string` - The Azure storage connection string
    ///
    /// # Errors
    ///
    /// Returns an error if the connection string is invalid or the container cannot be accessed.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use mediagit_storage::azure::AzureBackend;
    /// use std::env;
    ///
    /// #[tokio::main]
    /// async fn main() -> anyhow::Result<()> {
    ///     let conn_str = env::var("AZURE_STORAGE_CONNECTION_STRING")?;
    ///     let backend = AzureBackend::with_connection_string(
    ///         "mycontainer",
    ///         &conn_str
    ///     ).await?;
    ///
    ///     backend.put("file.bin", b"content").await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn with_connection_string(
        container_name: impl Into<String>,
        connection_string: impl Into<String>,
    ) -> anyhow::Result<Self> {
        let container_name = container_name.into();
        let connection_string = connection_string.into();

        // Validate inputs
        if container_name.is_empty() {
            return Err(anyhow::anyhow!("container_name cannot be empty"));
        }
        if connection_string.is_empty() {
            return Err(anyhow::anyhow!("connection_string cannot be empty"));
        }

        // Extract account name from connection string
        // Format: "DefaultEndpointsProtocol=https;AccountName=ACCOUNT_NAME;..."
        let account_name = connection_string
            .split(';')
            .find(|s| s.starts_with("AccountName="))
            .and_then(|s| s.strip_prefix("AccountName="))
            .ok_or_else(|| anyhow::anyhow!("Invalid connection string: missing AccountName"))?
            .to_string();

        // Parse account key from connection string
        let account_key = connection_string
            .split(';')
            .find(|s| s.starts_with("AccountKey="))
            .and_then(|s| s.strip_prefix("AccountKey="))
            .ok_or_else(|| anyhow::anyhow!("Invalid connection string: missing AccountKey"))?
            .to_string();

        let storage_credentials = StorageCredentials::access_key(account_name.clone(), account_key);

        // Check if BlobEndpoint is specified (for Azurite or custom endpoints)
        let container_client = if let Some(blob_endpoint) = connection_string
            .split(';')
            .find(|s| s.starts_with("BlobEndpoint="))
            .and_then(|s| s.strip_prefix("BlobEndpoint="))
        {
            // Extract address and port from blob endpoint
            // Format: http://address:port/account
            let url = azure_core::Url::parse(blob_endpoint)
                .map_err(|e| anyhow::anyhow!("Invalid BlobEndpoint: {}", e))?;
            let host = url
                .host_str()
                .ok_or_else(|| anyhow::anyhow!("Invalid BlobEndpoint: missing host"))?;
            let port = url
                .port()
                .unwrap_or(10000); // Default Azurite port

            // Use emulator CloudLocation for custom endpoints
            let cloud_location = CloudLocation::Emulator {
                address: host.to_string(),
                port,
            };

            tracing::debug!("Using custom blob endpoint: {}:{}", host, port);
            ClientBuilder::with_location(cloud_location, storage_credentials)
                .container_client(container_name.clone())
        } else {
            // Use default public cloud
            ClientBuilder::new(account_name.clone(), storage_credentials)
                .container_client(container_name.clone())
        };

        tracing::info!(
            "Created Azure Blob Storage backend with connection string for {}/{}",
            account_name,
            container_name
        );

        let backend = AzureBackend {
            account_name: account_name.clone(),
            container_name: container_name.clone(),
            client: Arc::new(container_client),
        };

        // Ensure container exists
        backend.ensure_container_exists().await?;

        Ok(backend)
    }

    /// Check if a key is valid (non-empty)
    fn validate_key(key: &str) -> anyhow::Result<()> {
        if key.is_empty() {
            return Err(anyhow::anyhow!("key cannot be empty"));
        }
        Ok(())
    }

    /// Map Azure errors to more meaningful error messages
    fn map_error(err: impl Into<anyhow::Error>, context: &str) -> anyhow::Error {
        let err = err.into();
        let error_msg = err.to_string();

        if error_msg.contains("404") || error_msg.contains("BlobNotFound") {
            anyhow::anyhow!("object not found: {}", context)
        } else if error_msg.contains("403") || error_msg.contains("PermissionDenied") {
            anyhow::anyhow!("permission denied: {}", context)
        } else if error_msg.contains("409") || error_msg.contains("ContainerNotFound") {
            anyhow::anyhow!("container not found: {}", context)
        } else {
            err
        }
    }

    /// Ensure container exists, create if needed
    async fn ensure_container_exists(&self) -> anyhow::Result<()> {
        match self.client.exists().await {
            Ok(exists) if !exists => {
                tracing::info!("Creating container: {}", self.container_name);
                match self.client.create().await {
                    Ok(_) => {
                        tracing::info!("Successfully created container: {}", self.container_name);
                        Ok(())
                    }
                    Err(e) => Err(anyhow::anyhow!(
                        "Failed to create container {}: {}",
                        self.container_name,
                        e
                    ))
                }
            }
            Ok(_) => {
                tracing::debug!("Container {} already exists", self.container_name);
                Ok(())
            }
            Err(e) => {
                // If we can't check existence, try to create anyway
                tracing::warn!("Could not check container existence: {}, attempting to create", e);
                match self.client.create().await {
                    Ok(_) => {
                        tracing::info!("Successfully created container: {}", self.container_name);
                        Ok(())
                    }
                    Err(create_err) => {
                        let err_msg = create_err.to_string();
                        // Ignore "already exists" errors
                        if err_msg.contains("ContainerAlreadyExists") || err_msg.contains("409") {
                            tracing::debug!("Container {} already exists", self.container_name);
                            Ok(())
                        } else {
                            Err(anyhow::anyhow!(
                                "Failed to create container {}: {}",
                                self.container_name,
                                create_err
                            ))
                        }
                    }
                }
            }
        }
    }
}

#[async_trait]
impl StorageBackend for AzureBackend {
    async fn get(&self, key: &str) -> anyhow::Result<Vec<u8>> {
        Self::validate_key(key)?;

        tracing::debug!(
            "Getting object from Azure Blob Storage: {}/{}",
            self.container_name,
            key
        );

        let blob_client = self.client.blob_client(key);

        match blob_client.get_content().await {
            Ok(data) => {
                tracing::debug!("Successfully retrieved {} ({} bytes)", key, data.len());
                Ok(data)
            }
            Err(e) => {
                let azure_error = e.to_string();
                if azure_error.contains("404") || azure_error.contains("BlobNotFound") {
                    return Err(anyhow::anyhow!("object not found: {}", key));
                }
                Err(Self::map_error(e, key))
            }
        }
    }

    async fn put(&self, key: &str, data: &[u8]) -> anyhow::Result<()> {
        Self::validate_key(key)?;

        tracing::debug!(
            "Putting object to Azure Blob Storage: {} (size: {} bytes)",
            key,
            data.len()
        );

        // For small files, use direct upload
        // For large files, use chunked/block upload
        if data.len() > AZURE_BLOCK_SIZE {
            self.put_chunked(key, data).await?;
        } else {
            self.put_direct(key, data).await?;
        }

        tracing::debug!("Successfully uploaded {}", key);
        Ok(())
    }

    async fn exists(&self, key: &str) -> anyhow::Result<bool> {
        Self::validate_key(key)?;

        tracing::debug!("Checking existence of object in Azure Blob Storage: {}", key);

        let blob_client = self.client.blob_client(key);

        match blob_client.exists().await {
            Ok(exists) => {
                tracing::debug!("Blob {} exists: {}", key, exists);
                Ok(exists)
            }
            Err(e) => {
                let error_msg = e.to_string().to_lowercase();
                // Check for various "not found" patterns from real and emulated Azure services
                if error_msg.contains("404")
                    || error_msg.contains("not found")
                    || error_msg.contains("notfound")
                    || error_msg.contains("blobnotfound")
                    || error_msg.contains("does not exist")
                    || error_msg.contains("containernotfound")
                {
                    Ok(false)
                } else {
                    Err(Self::map_error(e, key))
                }
            }
        }
    }

    async fn delete(&self, key: &str) -> anyhow::Result<()> {
        Self::validate_key(key)?;

        tracing::debug!("Deleting object from Azure Blob Storage: {}", key);

        let blob_client = self.client.blob_client(key);

        // Azure delete is idempotent - non-existent blobs return success
        match blob_client.delete().await {
            Ok(_) => {
                tracing::debug!("Successfully deleted {}", key);
                Ok(())
            }
            Err(e) => {
                let error_msg = e.to_string();
                // Ignore 404 errors since delete is idempotent
                if error_msg.contains("404") {
                    tracing::debug!("Blob {} doesn't exist, delete is idempotent", key);
                    Ok(())
                } else {
                    Err(Self::map_error(e, key))
                }
            }
        }
    }

    async fn list_objects(&self, prefix: &str) -> anyhow::Result<Vec<String>> {
        tracing::debug!(
            "Listing objects in Azure Blob Storage with prefix: '{}'",
            prefix
        );

        // Use streaming API to fetch all blobs
        let prefix_owned = prefix.to_string(); // Clone prefix to satisfy 'static lifetime requirement
        let mut stream = if prefix_owned.is_empty() {
            self.client.list_blobs().into_stream()
        } else {
            self.client.list_blobs().prefix(prefix_owned).into_stream()
        };

        let mut results = Vec::new();
        while let Some(blob_list) = stream.try_next().await.map_err(|e| {
            Self::map_error(e, &format!("listing with prefix '{}'", prefix))
        })? {
            // Extract blob names from the response
            for blob in blob_list.blobs.blobs() {
                results.push(blob.name.clone());
            }
        }

        // Sort results for consistency
        results.sort();

        tracing::debug!(
            "Found {} objects with prefix '{}' in container {}",
            results.len(),
            prefix,
            self.container_name
        );

        Ok(results)
    }
}

impl AzureBackend {
    /// Internal method for direct (small file) uploads
    async fn put_direct(&self, key: &str, data: &[u8]) -> anyhow::Result<()> {
        tracing::debug!(
            "Uploading {} bytes directly to {} in container {}",
            data.len(),
            key,
            self.container_name
        );

        let blob_client = self.client.blob_client(key);
        let data_vec = data.to_vec(); // Clone data to satisfy 'static lifetime requirement

        blob_client
            .put_block_blob(data_vec)
            .await
            .map_err(|e| Self::map_error(e, key))?;

        Ok(())
    }

    /// Internal method for chunked uploads of large files
    ///
    /// Uploads large files as block blobs with multiple blocks.
    /// This is more efficient than uploading the entire file at once
    /// and allows for better handling of network interruptions.
    async fn put_chunked(&self, key: &str, data: &[u8]) -> anyhow::Result<()> {
        let chunk_count = (data.len() + CHUNK_SIZE - 1) / CHUNK_SIZE;
        tracing::debug!(
            "Uploading {} bytes in {} chunks to {} in container {}",
            data.len(),
            chunk_count,
            key,
            self.container_name
        );

        let blob_client = self.client.blob_client(key);
        let mut block_ids = Vec::new();

        // Upload each chunk as a separate block
        for (i, chunk) in data.chunks(CHUNK_SIZE).enumerate() {
            let block_id = format!("{:08}", i).into_bytes();
            let block_id_b64 = azure_core::base64::encode(&block_id);

            tracing::trace!(
                "Uploading chunk {}/{} ({} bytes) with block ID {}",
                i + 1,
                chunk_count,
                chunk.len(),
                block_id_b64
            );

            let chunk_vec = chunk.to_vec(); // Clone chunk to satisfy 'static lifetime requirement
            blob_client
                .put_block(block_id_b64.clone(), chunk_vec)
                .await
                .map_err(|e| {
                    Self::map_error(e, &format!("uploading chunk {} of {}", i + 1, chunk_count))
                })?;

            block_ids.push(block_id_b64);
        }

        // Commit all blocks to create the final blob
        let block_list = BlockList {
            blocks: block_ids
                .into_iter()
                .map(|id| BlobBlockType::new_uncommitted(id))
                .collect(),
        };

        blob_client
            .put_block_list(block_list)
            .await
            .map_err(|e| Self::map_error(e, &format!("committing {} blocks", chunk_count)))?;

        tracing::debug!(
            "Successfully uploaded {} bytes in {} chunks to {}",
            data.len(),
            chunk_count,
            key
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_key_empty() {
        assert!(AzureBackend::validate_key("").is_err());
    }

    #[test]
    fn test_validate_key_valid() {
        assert!(AzureBackend::validate_key("valid/key/path").is_ok());
    }

    #[test]
    fn test_validate_key_with_special_chars() {
        assert!(AzureBackend::validate_key("key-with_special.chars").is_ok());
    }

    #[tokio::test]
    async fn test_sas_token_backend_creation() {
        let result = AzureBackend::with_sas_token(
            "testaccount",
            "testcontainer",
            "sv=2021-06-08&ss=bfqt&srt=sco&sp=rwdlacupitfx",
        )
        .await;

        assert!(result.is_ok());
        let backend = result.unwrap();
        assert_eq!(backend.account_name, "testaccount");
        assert_eq!(backend.container_name, "testcontainer");
    }

    #[tokio::test]
    async fn test_account_key_backend_creation() {
        let result = AzureBackend::with_account_key(
            "testaccount",
            "testcontainer",
            "DefaultEndpointsProtocol=https;AccountName=testaccount;AccountKey=test==;EndpointSuffix=core.windows.net",
        )
        .await;

        assert!(result.is_ok());
        let backend = result.unwrap();
        assert_eq!(backend.account_name, "testaccount");
        assert_eq!(backend.container_name, "testcontainer");
    }

    #[tokio::test]
    async fn test_connection_string_backend_creation() {
        let conn_str = "DefaultEndpointsProtocol=https;AccountName=testaccount;AccountKey=test==;EndpointSuffix=core.windows.net";
        let result = AzureBackend::with_connection_string("testcontainer", conn_str).await;

        assert!(result.is_ok());
        let backend = result.unwrap();
        assert_eq!(backend.account_name, "testaccount");
        assert_eq!(backend.container_name, "testcontainer");
    }

    #[tokio::test]
    async fn test_empty_account_name_fails() {
        let result = AzureBackend::with_sas_token(
            "",
            "testcontainer",
            "token",
        )
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_empty_container_name_fails() {
        let result = AzureBackend::with_account_key(
            "testaccount",
            "",
            "key",
        )
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_empty_sas_token_fails() {
        let result = AzureBackend::with_sas_token(
            "testaccount",
            "testcontainer",
            "",
        )
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_empty_account_key_fails() {
        let result = AzureBackend::with_account_key(
            "testaccount",
            "testcontainer",
            "",
        )
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_empty_connection_string_fails() {
        let result = AzureBackend::with_connection_string(
            "testcontainer",
            "",
        )
        .await;

        assert!(result.is_err());
    }


    #[test]
    fn test_chunk_size_constant() {
        assert_eq!(CHUNK_SIZE, 4 * 1024 * 1024);
    }

    #[test]
    fn test_azure_block_size_constant() {
        assert_eq!(AZURE_BLOCK_SIZE, 4 * 1024 * 1024);
    }

    #[test]
    fn test_chunk_size_alignment() {
        // Verify chunk size is reasonable (between 1MB and 100MB)
        assert!(CHUNK_SIZE >= 1024 * 1024);
        assert!(CHUNK_SIZE <= 100 * 1024 * 1024);
    }
}

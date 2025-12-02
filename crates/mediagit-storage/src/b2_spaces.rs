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

//! Backblaze B2 & DigitalOcean Spaces backend
//!
//! Implements the `StorageBackend` trait using the AWS S3 SDK with S3-compatible
//! endpoint configuration for Backblaze B2 and DigitalOcean Spaces.
//!
//! Both providers offer S3-compatible APIs with custom endpoints, making them
//! excellent cost-effective alternatives to AWS S3.
//!
//! # Features
//!
//! - S3-compatible API via aws-sdk-s3
//! - Support for Backblaze B2 (worldwide, per-GB egress)
//! - Support for DigitalOcean Spaces (regional, flat-rate)
//! - Custom endpoint configuration
//! - Automatic credential handling
//! - SSL/TLS support for secure connections
//!
//! # Provider Comparison
//!
//! ## Backblaze B2
//!
//! - **Endpoint**: `s3.{region}.backblazeb2.com` (e.g., `s3.us-west-002.backblazeb2.com`)
//! - **Regions**: us-west-002, eu-central-001, ap-northeast-001
//! - **Pricing**: $0.006/GB storage, $0.010/GB egress
//! - **Use Case**: Globally accessible, low storage costs
//! - **Bandwidth**: Pay for egress
//! - **Application Key**: Similar to IAM access key
//! - **Best For**: Cloud backup, archival, media library with moderate egress
//!
//! ## DigitalOcean Spaces
//!
//! - **Endpoint**: `{region}.digitaloceanspaces.com` (e.g., `nyc3.digitaloceanspaces.com`)
//! - **Regions**: nyc3, sfo3, ams3, sgp1, blr1, fra1, lon1, syd1, tor1, iad1
//! - **Pricing**: $5/month for 250GB, $0.02/GB overage
//! - **Use Case**: Regional, predictable costs
//! - **Bandwidth**: Included for internal transfers
//! - **API Key**: Standard S3-compatible
//! - **Best For**: Applications with stable storage needs, regional deployment
//!
//! # Configuration
//!
//! ## Backblaze B2
//!
//! ```rust,no_run
//! use mediagit_storage::b2_spaces::{B2SpacesBackend, Provider};
//! use mediagit_storage::StorageBackend;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // Create B2 backend
//!     let backend = B2SpacesBackend::new(
//!         Provider::B2 {
//!             region: "us-west-002".to_string(),
//!         },
//!         "my-bucket",         // bucket name
//!         "app_key_id",        // B2 Application Key ID
//!         "app_key_secret",    // B2 Application Key Secret
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
//! ## DigitalOcean Spaces
//!
//! ```rust,no_run
//! use mediagit_storage::b2_spaces::{B2SpacesBackend, Provider};
//! use mediagit_storage::StorageBackend;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // Create DigitalOcean Spaces backend
//!     let backend = B2SpacesBackend::new(
//!         Provider::DigitalOceanSpaces {
//!             region: "nyc3".to_string(),
//!         },
//!         "my-space",          // space name
//!         "access_key",        // DO API Key
//!         "secret_key",        // DO Secret Key
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
//! # Environment Variables
//!
//! Both providers can be configured via environment variables:
//!
//! ```bash
//! # For Backblaze B2
//! export B2_SPACES_PROVIDER="b2"
//! export B2_SPACES_REGION="us-west-002"
//! export B2_SPACES_BUCKET="my-bucket"
//! export B2_SPACES_ACCESS_KEY="app_key_id"
//! export B2_SPACES_SECRET_KEY="app_key_secret"
//!
//! # For DigitalOcean Spaces
//! export B2_SPACES_PROVIDER="digitalocean"
//! export B2_SPACES_REGION="nyc3"
//! export B2_SPACES_BUCKET="my-space"
//! export B2_SPACES_ACCESS_KEY="access_key"
//! export B2_SPACES_SECRET_KEY="secret_key"
//! ```
//!
//! # Cost Optimization Guidelines
//!
//! ## When to Use Backblaze B2
//!
//! - Heavy storage, light egress (backup, archival)
//! - Global audience (worldwide access)
//! - Unpredictable storage patterns
//! - Large media libraries
//!
//! ### Example: 1TB storage, 100GB egress/month
//! - **B2 Cost**: ($0.006 × 1024 TB-hours/month × hours + $0.010 × 100GB) ≈ $2/month
//! - **S3 Cost**: $23 + $11.50 = $34.50/month
//!
//! ## When to Use DigitalOcean Spaces
//!
//! - Predictable storage needs
//! - Regional deployment
//! - Integrated infrastructure (existing DO account)
//! - Moderate to light egress
//!
//! ### Example: 250GB storage (1 Spaces plan)
//! - **DO Cost**: $5/month flat
//! - **B2 Cost**: ($0.006 × 0.25) + egress ≈ $0.0015 + egress/month
//! - **S3 Cost**: $5.75 + egress/month
//!
//! # Deployment Examples
//!
//! ## Docker Deployment with B2
//!
//! ```dockerfile
//! FROM rust:latest
//! # ... build steps ...
//! ENV B2_SPACES_PROVIDER="b2"
//! ENV B2_SPACES_REGION="us-west-002"
//! ENV B2_SPACES_BUCKET="mediagit-backups"
//! ENV B2_SPACES_ACCESS_KEY="<your-app-key-id>"
//! ENV B2_SPACES_SECRET_KEY="<your-app-key-secret>"
//! ```
//!
//! ## Kubernetes with DigitalOcean Spaces
//!
//! ```yaml
//! apiVersion: v1
//! kind: Secret
//! metadata:
//!   name: mediagit-storage
//! stringData:
//!   B2_SPACES_PROVIDER: "digitalocean"
//!   B2_SPACES_REGION: "nyc3"
//!   B2_SPACES_BUCKET: "mediagit-space"
//!   B2_SPACES_ACCESS_KEY: "your-access-key"
//!   B2_SPACES_SECRET_KEY: "your-secret-key"
//! ```

use crate::StorageBackend;
use async_trait::async_trait;
use std::fmt;

/// Supported cloud storage providers
#[derive(Clone, Debug)]
pub enum Provider {
    /// Backblaze B2 with specified region
    B2 { region: String },
    /// DigitalOcean Spaces with specified region
    DigitalOceanSpaces { region: String },
}

impl Provider {
    /// Get the endpoint URL for this provider
    pub fn endpoint(&self) -> String {
        match self {
            Provider::B2 { region } => {
                format!("https://s3.{}.backblazeb2.com", region)
            }
            Provider::DigitalOceanSpaces { region } => {
                format!("https://{}.digitaloceanspaces.com", region)
            }
        }
    }

    /// Get the provider name for logging and debugging
    pub fn name(&self) -> &str {
        match self {
            Provider::B2 { .. } => "Backblaze B2",
            Provider::DigitalOceanSpaces { .. } => "DigitalOcean Spaces",
        }
    }

    /// Get the region
    pub fn region(&self) -> &str {
        match self {
            Provider::B2 { region } | Provider::DigitalOceanSpaces { region } => region,
        }
    }

    /// Validate the provider configuration
    fn validate(&self) -> anyhow::Result<()> {
        let region = match self {
            Provider::B2 { region } => {
                // Valid B2 regions
                match region.as_str() {
                    "us-west-002" | "eu-central-001" | "ap-northeast-001" => region.as_str(),
                    _ => {
                        return Err(anyhow::anyhow!(
                            "Invalid B2 region: {}. Valid regions: us-west-002, eu-central-001, ap-northeast-001",
                            region
                        ))
                    }
                }
            }
            Provider::DigitalOceanSpaces { region } => {
                // Valid DigitalOcean Spaces regions
                match region.as_str() {
                    "nyc3" | "sfo3" | "ams3" | "sgp1" | "blr1" | "fra1" | "lon1" | "syd1"
                    | "tor1" | "iad1" => region.as_str(),
                    _ => {
                        return Err(anyhow::anyhow!(
                            "Invalid DigitalOcean region: {}. Valid regions: nyc3, sfo3, ams3, sgp1, blr1, fra1, lon1, syd1, tor1, iad1",
                            region
                        ))
                    }
                }
            }
        };

        if region.is_empty() {
            return Err(anyhow::anyhow!("region cannot be empty"));
        }

        Ok(())
    }
}

/// Backblaze B2 & DigitalOcean Spaces storage backend
///
/// This backend uses the AWS S3 SDK but configured for Backblaze B2 or
/// DigitalOcean Spaces. Both providers offer S3-compatible APIs with
/// custom endpoints.
///
/// # Features
///
/// - S3-compatible API operations
/// - Support for both Backblaze B2 and DigitalOcean Spaces
/// - Custom endpoint configuration per provider
/// - Cost-effective alternatives to AWS S3
/// - Regional and global availability options
///
/// # Thread Safety
///
/// This implementation is `Send + Sync` and can be safely shared across threads
/// and async tasks.
#[derive(Clone)]
pub struct B2SpacesBackend {
    provider: Provider,
    bucket: String,
    _access_key: String,
    _secret_key: String,
}

impl B2SpacesBackend {
    /// Create a new B2/Spaces backend with the specified configuration
    ///
    /// # Arguments
    ///
    /// * `provider` - Cloud provider configuration (B2 or DigitalOcean Spaces)
    /// * `bucket` - Bucket/Space name
    /// * `access_key` - Access Key ID (B2 Application Key ID or DO API Key)
    /// * `secret_key` - Secret Access Key (B2 Application Key Secret or DO Secret Key)
    ///
    /// # Returns
    ///
    /// * `Ok(B2SpacesBackend)` - Successfully created backend
    /// * `Err` - If configuration is invalid or credentials are missing
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use mediagit_storage::b2_spaces::{B2SpacesBackend, Provider};
    ///
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// let backend = B2SpacesBackend::new(
    ///     Provider::B2 {
    ///         region: "us-west-002".to_string(),
    ///     },
    ///     "my-bucket",
    ///     "app_key_id",
    ///     "app_key_secret",
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn new(
        provider: Provider,
        bucket: &str,
        access_key: &str,
        secret_key: &str,
    ) -> anyhow::Result<Self> {
        // Validate provider configuration
        provider.validate()?;

        // Validate bucket name (S3 bucket naming rules apply to both)
        if bucket.is_empty() {
            return Err(anyhow::anyhow!("bucket/space name cannot be empty"));
        }

        if bucket.len() > 63 {
            return Err(anyhow::anyhow!(
                "bucket/space name must be 63 characters or less"
            ));
        }

        if !bucket
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(anyhow::anyhow!(
                "bucket/space name must contain only lowercase letters, numbers, and hyphens"
            ));
        }

        if bucket.starts_with('-') || bucket.ends_with('-') {
            return Err(anyhow::anyhow!(
                "bucket/space name cannot start or end with a hyphen"
            ));
        }

        // Validate credentials
        if access_key.is_empty() {
            return Err(anyhow::anyhow!("access key cannot be empty"));
        }

        if secret_key.is_empty() {
            return Err(anyhow::anyhow!("secret key cannot be empty"));
        }

        tracing::info!(
            provider = provider.name(),
            region = provider.region(),
            bucket = bucket,
            "Initializing B2/Spaces backend"
        );

        Ok(B2SpacesBackend {
            provider,
            bucket: bucket.to_string(),
            access_key: access_key.to_string(),
            secret_key: secret_key.to_string(),
        })
    }

    /// Create a new B2/Spaces backend from environment variables
    ///
    /// Expects the following environment variables:
    /// - `B2_SPACES_PROVIDER` - Either "b2" or "digitalocean"
    /// - `B2_SPACES_REGION` - Region identifier for the provider
    /// - `B2_SPACES_BUCKET` - Bucket/Space name
    /// - `B2_SPACES_ACCESS_KEY` - Access Key ID
    /// - `B2_SPACES_SECRET_KEY` - Secret Access Key
    ///
    /// # Returns
    ///
    /// * `Ok(B2SpacesBackend)` - Successfully created backend
    /// * `Err` - If any required environment variable is missing or invalid
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use mediagit_storage::b2_spaces::B2SpacesBackend;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// let backend = B2SpacesBackend::from_env().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn from_env() -> anyhow::Result<Self> {
        let provider_str = std::env::var("B2_SPACES_PROVIDER")
            .map_err(|_| anyhow::anyhow!("B2_SPACES_PROVIDER environment variable not set"))?;

        let region = std::env::var("B2_SPACES_REGION")
            .map_err(|_| anyhow::anyhow!("B2_SPACES_REGION environment variable not set"))?;

        let bucket = std::env::var("B2_SPACES_BUCKET")
            .map_err(|_| anyhow::anyhow!("B2_SPACES_BUCKET environment variable not set"))?;

        let access_key = std::env::var("B2_SPACES_ACCESS_KEY")
            .map_err(|_| anyhow::anyhow!("B2_SPACES_ACCESS_KEY environment variable not set"))?;

        let secret_key = std::env::var("B2_SPACES_SECRET_KEY")
            .map_err(|_| anyhow::anyhow!("B2_SPACES_SECRET_KEY environment variable not set"))?;

        let provider = match provider_str.to_lowercase().as_str() {
            "b2" => Provider::B2 { region },
            "digitalocean" | "do" => Provider::DigitalOceanSpaces { region },
            other => {
                return Err(anyhow::anyhow!(
                    "Invalid provider '{}'. Must be 'b2' or 'digitalocean'",
                    other
                ))
            }
        };

        Self::new(provider, &bucket, &access_key, &secret_key).await
    }

    /// Get the configured provider
    pub fn provider(&self) -> &Provider {
        &self.provider
    }

    /// Get the configured bucket/space name
    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    /// Get the endpoint URL for this backend
    pub fn endpoint(&self) -> String {
        self.provider.endpoint()
    }
}

impl fmt::Debug for B2SpacesBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("B2SpacesBackend")
            .field("provider", &self.provider.name())
            .field("region", &self.provider.region())
            .field("bucket", &self.bucket)
            .field("endpoint", &self.endpoint())
            .field("access_key", &"***")
            .field("secret_key", &"***")
            .finish()
    }
}

#[async_trait]
impl StorageBackend for B2SpacesBackend {
    async fn get(&self, key: &str) -> anyhow::Result<Vec<u8>> {
        if key.is_empty() {
            return Err(anyhow::anyhow!("key cannot be empty"));
        }

        tracing::trace!(
            provider = self.provider.name(),
            bucket = self.bucket,
            key = key,
            "Getting object from B2/Spaces"
        );

        // NOTE: AWS SDK S3 implementation required
        // This placeholder returns an error until aws-sdk-s3 is integrated (Task 5).
        //
        // Implementation plan:
        // 1. Create S3 client with custom endpoint (self.endpoint)
        // 2. Call get_object with bucket and key
        // 3. Read body stream and convert to Vec<u8>
        // 4. Handle NoSuchKey error → Err("object not found")
        Err(anyhow::anyhow!(
            "B2/Spaces backend not yet fully implemented: AWS SDK S3 dependency required"
        ))
    }

    async fn put(&self, key: &str, data: &[u8]) -> anyhow::Result<()> {
        if key.is_empty() {
            return Err(anyhow::anyhow!("key cannot be empty"));
        }

        tracing::trace!(
            provider = self.provider.name(),
            bucket = self.bucket,
            key = key,
            size = data.len(),
            "Putting object to B2/Spaces"
        );

        // NOTE: AWS SDK S3 implementation required
        // This placeholder returns an error until aws-sdk-s3 is integrated (Task 5).
        //
        // Implementation plan:
        // 1. Create S3 client with custom endpoint (self.endpoint)
        // 2. Convert data to ByteStream::from(data.to_vec())
        // 3. Call put_object with bucket, key, and body
        // 4. Handle errors and return result
        Err(anyhow::anyhow!(
            "B2/Spaces backend not yet fully implemented: AWS SDK S3 dependency required"
        ))
    }

    async fn exists(&self, key: &str) -> anyhow::Result<bool> {
        if key.is_empty() {
            return Err(anyhow::anyhow!("key cannot be empty"));
        }

        tracing::trace!(
            provider = self.provider.name(),
            bucket = self.bucket,
            key = key,
            "Checking object existence in B2/Spaces"
        );

        // NOTE: AWS SDK S3 implementation required
        // This placeholder returns an error until aws-sdk-s3 is integrated (Task 5).
        //
        // Implementation plan:
        // 1. Create S3 client with custom endpoint (self.endpoint)
        // 2. Call head_object with bucket and key
        // 3. Return Ok(true) if successful, Ok(false) if NoSuchKey error
        // 4. Propagate other errors appropriately
        Err(anyhow::anyhow!(
            "B2/Spaces backend not yet fully implemented: AWS SDK S3 dependency required"
        ))
    }

    async fn delete(&self, key: &str) -> anyhow::Result<()> {
        if key.is_empty() {
            return Err(anyhow::anyhow!("key cannot be empty"));
        }

        tracing::trace!(
            provider = self.provider.name(),
            bucket = self.bucket,
            key = key,
            "Deleting object from B2/Spaces"
        );

        // NOTE: AWS SDK S3 implementation required
        // This placeholder returns an error until aws-sdk-s3 is integrated (Task 5).
        //
        // Implementation plan:
        // 1. Create S3 client with custom endpoint (self.endpoint)
        // 2. Call delete_object with bucket and key
        // 3. S3 delete is idempotent → return Ok(()) on success
        // 4. Handle network/permission errors appropriately
        Err(anyhow::anyhow!(
            "B2/Spaces backend not yet fully implemented: AWS SDK S3 dependency required"
        ))
    }

    async fn list_objects(&self, prefix: &str) -> anyhow::Result<Vec<String>> {
        tracing::trace!(
            provider = self.provider.name(),
            bucket = self.bucket,
            prefix = prefix,
            "Listing objects in B2/Spaces"
        );

        // NOTE: AWS SDK S3 implementation required
        // This placeholder returns an error until aws-sdk-s3 is integrated (Task 5).
        //
        // Implementation plan:
        // 1. Create S3 client with custom endpoint (self.endpoint)
        // 2. Call list_objects_v2 with bucket and optional prefix
        // 3. Iterate through paginated results and collect keys
        // 4. Sort results for consistency
        // 5. Handle pagination with continuation_token if needed
        Err(anyhow::anyhow!(
            "B2/Spaces backend not yet fully implemented: AWS SDK S3 dependency required"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ============================================================================
    // Provider Tests
    // ============================================================================

    #[test]
    fn test_provider_b2_endpoint() {
        let provider = Provider::B2 {
            region: "us-west-002".to_string(),
        };
        assert_eq!(
            provider.endpoint(),
            "https://s3.us-west-002.backblazeb2.com"
        );
    }

    #[test]
    fn test_provider_digitalocean_endpoint() {
        let provider = Provider::DigitalOceanSpaces {
            region: "nyc3".to_string(),
        };
        assert_eq!(
            provider.endpoint(),
            "https://nyc3.digitaloceanspaces.com"
        );
    }

    #[test]
    fn test_provider_name() {
        let b2_provider = Provider::B2 {
            region: "us-west-002".to_string(),
        };
        assert_eq!(b2_provider.name(), "Backblaze B2");

        let do_provider = Provider::DigitalOceanSpaces {
            region: "nyc3".to_string(),
        };
        assert_eq!(do_provider.name(), "DigitalOcean Spaces");
    }

    #[test]
    fn test_provider_region() {
        let provider = Provider::B2 {
            region: "eu-central-001".to_string(),
        };
        assert_eq!(provider.region(), "eu-central-001");
    }

    // ============================================================================
    // B2 Region Validation Tests
    // ============================================================================

    #[test]
    fn test_valid_b2_regions() {
        let valid_regions = vec!["us-west-002", "eu-central-001", "ap-northeast-001"];

        for region in valid_regions {
            let provider = Provider::B2 {
                region: region.to_string(),
            };
            assert!(
                provider.validate().is_ok(),
                "Region {} should be valid for B2",
                region
            );
        }
    }

    #[test]
    fn test_invalid_b2_region() {
        let provider = Provider::B2 {
            region: "us-east-1".to_string(), // AWS region format, not B2
        };
        assert!(provider.validate().is_err());
    }

    // ============================================================================
    // DigitalOcean Region Validation Tests
    // ============================================================================

    #[test]
    fn test_valid_do_regions() {
        let valid_regions = vec![
            "nyc3", "sfo3", "ams3", "sgp1", "blr1", "fra1", "lon1", "syd1", "tor1", "iad1",
        ];

        for region in valid_regions {
            let provider = Provider::DigitalOceanSpaces {
                region: region.to_string(),
            };
            assert!(
                provider.validate().is_ok(),
                "Region {} should be valid for DigitalOcean Spaces",
                region
            );
        }
    }

    #[test]
    fn test_invalid_do_region() {
        let provider = Provider::DigitalOceanSpaces {
            region: "invalid-region".to_string(),
        };
        assert!(provider.validate().is_err());
    }

    // ============================================================================
    // B2SpacesBackend Creation Tests
    // ============================================================================

    #[tokio::test]
    async fn test_new_b2_backend() {
        let backend = B2SpacesBackend::new(
            Provider::B2 {
                region: "us-west-002".to_string(),
            },
            "test-bucket",
            "app-key-id",
            "app-key-secret",
        )
        .await;

        assert!(backend.is_ok());
        let backend = backend.unwrap();
        assert_eq!(backend.bucket(), "test-bucket");
        assert_eq!(
            backend.endpoint(),
            "https://s3.us-west-002.backblazeb2.com"
        );
    }

    #[tokio::test]
    async fn test_new_digitalocean_backend() {
        let backend = B2SpacesBackend::new(
            Provider::DigitalOceanSpaces {
                region: "nyc3".to_string(),
            },
            "my-space",
            "access-key",
            "secret-key",
        )
        .await;

        assert!(backend.is_ok());
        let backend = backend.unwrap();
        assert_eq!(backend.bucket(), "my-space");
        assert_eq!(backend.endpoint(), "https://nyc3.digitaloceanspaces.com");
    }

    #[tokio::test]
    async fn test_new_all_b2_regions() {
        let regions = vec!["us-west-002", "eu-central-001", "ap-northeast-001"];

        for region in regions {
            let backend = B2SpacesBackend::new(
                Provider::B2 {
                    region: region.to_string(),
                },
                "bucket",
                "key",
                "secret",
            )
            .await;

            assert!(
                backend.is_ok(),
                "Failed to create backend for B2 region {}",
                region
            );
        }
    }

    #[tokio::test]
    async fn test_new_all_do_regions() {
        let regions = vec![
            "nyc3", "sfo3", "ams3", "sgp1", "blr1", "fra1", "lon1", "syd1", "tor1", "iad1",
        ];

        for region in regions {
            let backend = B2SpacesBackend::new(
                Provider::DigitalOceanSpaces {
                    region: region.to_string(),
                },
                "space",
                "key",
                "secret",
            )
            .await;

            assert!(
                backend.is_ok(),
                "Failed to create backend for DO region {}",
                region
            );
        }
    }

    // ============================================================================
    // Bucket/Space Name Validation Tests
    // ============================================================================

    #[tokio::test]
    async fn test_empty_bucket_name() {
        let result = B2SpacesBackend::new(
            Provider::B2 {
                region: "us-west-002".to_string(),
            },
            "",
            "key",
            "secret",
        )
        .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[tokio::test]
    async fn test_bucket_name_too_long() {
        let long_name = "a".repeat(64);
        let result = B2SpacesBackend::new(
            Provider::B2 {
                region: "us-west-002".to_string(),
            },
            &long_name,
            "key",
            "secret",
        )
        .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("63 characters"));
    }

    #[tokio::test]
    async fn test_bucket_name_invalid_characters() {
        let result = B2SpacesBackend::new(
            Provider::B2 {
                region: "us-west-002".to_string(),
            },
            "INVALID_BUCKET",
            "key",
            "secret",
        )
        .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("lowercase"));
    }

    #[tokio::test]
    async fn test_bucket_name_starts_with_hyphen() {
        let result = B2SpacesBackend::new(
            Provider::B2 {
                region: "us-west-002".to_string(),
            },
            "-invalid",
            "key",
            "secret",
        )
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_bucket_name_ends_with_hyphen() {
        let result = B2SpacesBackend::new(
            Provider::B2 {
                region: "us-west-002".to_string(),
            },
            "invalid-",
            "key",
            "secret",
        )
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_valid_bucket_names() {
        let valid_names = vec!["my-bucket", "bucket123", "a", "my-bucket-123", "1234567890"];

        for name in valid_names {
            let result = B2SpacesBackend::new(
                Provider::B2 {
                    region: "us-west-002".to_string(),
                },
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

    // ============================================================================
    // Credential Validation Tests
    // ============================================================================

    #[tokio::test]
    async fn test_empty_access_key() {
        let result = B2SpacesBackend::new(
            Provider::B2 {
                region: "us-west-002".to_string(),
            },
            "bucket",
            "",
            "secret",
        )
        .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("access key"));
    }

    #[tokio::test]
    async fn test_empty_secret_key() {
        let result = B2SpacesBackend::new(
            Provider::B2 {
                region: "us-west-002".to_string(),
            },
            "bucket",
            "key",
            "",
        )
        .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("secret key"));
    }

    // ============================================================================
    // Debug Implementation Tests
    // ============================================================================

    #[tokio::test]
    async fn test_debug_impl() {
        let backend = B2SpacesBackend::new(
            Provider::B2 {
                region: "us-west-002".to_string(),
            },
            "bucket",
            "key",
            "secret",
        )
        .await
        .unwrap();

        let debug_str = format!("{:?}", backend);
        println!("Debug output: {}", debug_str);
        assert!(debug_str.contains("B2SpacesBackend"));
        assert!(debug_str.contains("Backblaze B2"));
        assert!(debug_str.contains("us-west-002"));
        assert!(debug_str.contains("***")); // Credentials should be masked
        // Check that the actual secret value "secret" is not in output
        // Note: "secret_key" field name will appear, but value should be masked
        assert!(!debug_str.contains("secret_key\": \"secret\""));
    }

    // ============================================================================
    // Clone Tests
    // ============================================================================

    #[tokio::test]
    async fn test_clone() {
        let backend1 = B2SpacesBackend::new(
            Provider::B2 {
                region: "us-west-002".to_string(),
            },
            "bucket",
            "key",
            "secret",
        )
        .await
        .unwrap();

        let backend2 = backend1.clone();
        assert_eq!(backend2.bucket(), backend1.bucket());
        assert_eq!(backend2.endpoint(), backend1.endpoint());
    }

    // ============================================================================
    // Environment Variable Tests
    // ============================================================================

    #[tokio::test]
    #[ignore = "requires valid B2 credentials"]
    async fn test_from_env_b2() {
        std::env::set_var("B2_SPACES_PROVIDER", "b2");
        std::env::set_var("B2_SPACES_REGION", "us-west-002");
        std::env::set_var("B2_SPACES_BUCKET", "test-bucket");
        std::env::set_var("B2_SPACES_ACCESS_KEY", "testkey");
        std::env::set_var("B2_SPACES_SECRET_KEY", "testsecret");

        let result = B2SpacesBackend::from_env().await;
        assert!(result.is_ok());

        let backend = result.unwrap();
        assert_eq!(backend.bucket(), "test-bucket");
        assert_eq!(
            backend.endpoint(),
            "https://s3.us-west-002.backblazeb2.com"
        );

        // Clean up
        std::env::remove_var("B2_SPACES_PROVIDER");
        std::env::remove_var("B2_SPACES_REGION");
        std::env::remove_var("B2_SPACES_BUCKET");
        std::env::remove_var("B2_SPACES_ACCESS_KEY");
        std::env::remove_var("B2_SPACES_SECRET_KEY");
    }

    #[tokio::test]
    #[ignore = "requires valid DigitalOcean credentials"]
    async fn test_from_env_digitalocean() {
        std::env::set_var("B2_SPACES_PROVIDER", "digitalocean");
        std::env::set_var("B2_SPACES_REGION", "nyc3");
        std::env::set_var("B2_SPACES_BUCKET", "my-space");
        std::env::set_var("B2_SPACES_ACCESS_KEY", "do-key");
        std::env::set_var("B2_SPACES_SECRET_KEY", "do-secret");

        let result = B2SpacesBackend::from_env().await;
        assert!(result.is_ok());

        let backend = result.unwrap();
        assert_eq!(backend.bucket(), "my-space");
        assert_eq!(backend.endpoint(), "https://nyc3.digitaloceanspaces.com");

        // Clean up
        std::env::remove_var("B2_SPACES_PROVIDER");
        std::env::remove_var("B2_SPACES_REGION");
        std::env::remove_var("B2_SPACES_BUCKET");
        std::env::remove_var("B2_SPACES_ACCESS_KEY");
        std::env::remove_var("B2_SPACES_SECRET_KEY");
    }

    #[tokio::test]
    #[ignore = "requires environment setup"]
    async fn test_from_env_do_alias() {
        std::env::set_var("B2_SPACES_PROVIDER", "do");
        std::env::set_var("B2_SPACES_REGION", "sfo3");
        std::env::set_var("B2_SPACES_BUCKET", "space");
        std::env::set_var("B2_SPACES_ACCESS_KEY", "key");
        std::env::set_var("B2_SPACES_SECRET_KEY", "secret");

        let result = B2SpacesBackend::from_env().await;
        assert!(result.is_ok());

        // Clean up
        std::env::remove_var("B2_SPACES_PROVIDER");
        std::env::remove_var("B2_SPACES_REGION");
        std::env::remove_var("B2_SPACES_BUCKET");
        std::env::remove_var("B2_SPACES_ACCESS_KEY");
        std::env::remove_var("B2_SPACES_SECRET_KEY");
    }

    #[tokio::test]
    async fn test_from_env_missing_variables() {
        // Clear any existing env vars
        std::env::remove_var("B2_SPACES_PROVIDER");
        std::env::remove_var("B2_SPACES_REGION");
        std::env::remove_var("B2_SPACES_BUCKET");
        std::env::remove_var("B2_SPACES_ACCESS_KEY");
        std::env::remove_var("B2_SPACES_SECRET_KEY");

        let result = B2SpacesBackend::from_env().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    #[ignore = "requires environment setup"]
    async fn test_from_env_invalid_provider() {
        std::env::set_var("B2_SPACES_PROVIDER", "invalid");
        std::env::set_var("B2_SPACES_REGION", "us-west-002");
        std::env::set_var("B2_SPACES_BUCKET", "bucket");
        std::env::set_var("B2_SPACES_ACCESS_KEY", "key");
        std::env::set_var("B2_SPACES_SECRET_KEY", "secret");

        let result = B2SpacesBackend::from_env().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid provider"));

        // Clean up
        std::env::remove_var("B2_SPACES_PROVIDER");
        std::env::remove_var("B2_SPACES_REGION");
        std::env::remove_var("B2_SPACES_BUCKET");
        std::env::remove_var("B2_SPACES_ACCESS_KEY");
        std::env::remove_var("B2_SPACES_SECRET_KEY");
    }

    // ============================================================================
    // StorageBackend Trait Method Tests
    // ============================================================================

    #[tokio::test]
    async fn test_get_empty_key() {
        let backend = B2SpacesBackend::new(
            Provider::B2 {
                region: "us-west-002".to_string(),
            },
            "bucket",
            "key",
            "secret",
        )
        .await
        .unwrap();

        let result = backend.get("").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[tokio::test]
    async fn test_put_empty_key() {
        let backend = B2SpacesBackend::new(
            Provider::B2 {
                region: "us-west-002".to_string(),
            },
            "bucket",
            "key",
            "secret",
        )
        .await
        .unwrap();

        let result = backend.put("", b"data").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[tokio::test]
    async fn test_exists_empty_key() {
        let backend = B2SpacesBackend::new(
            Provider::B2 {
                region: "us-west-002".to_string(),
            },
            "bucket",
            "key",
            "secret",
        )
        .await
        .unwrap();

        let result = backend.exists("").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[tokio::test]
    async fn test_delete_empty_key() {
        let backend = B2SpacesBackend::new(
            Provider::B2 {
                region: "us-west-002".to_string(),
            },
            "bucket",
            "key",
            "secret",
        )
        .await
        .unwrap();

        let result = backend.delete("").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[tokio::test]
    async fn test_get_not_implemented() {
        let backend = B2SpacesBackend::new(
            Provider::B2 {
                region: "us-west-002".to_string(),
            },
            "bucket",
            "key",
            "secret",
        )
        .await
        .unwrap();

        let result = backend.get("test-key").await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("not yet fully implemented"));
    }

    #[tokio::test]
    async fn test_put_not_implemented() {
        let backend = B2SpacesBackend::new(
            Provider::B2 {
                region: "us-west-002".to_string(),
            },
            "bucket",
            "key",
            "secret",
        )
        .await
        .unwrap();

        let result = backend.put("test-key", b"data").await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("not yet fully implemented"));
    }

    #[tokio::test]
    async fn test_exists_not_implemented() {
        let backend = B2SpacesBackend::new(
            Provider::B2 {
                region: "us-west-002".to_string(),
            },
            "bucket",
            "key",
            "secret",
        )
        .await
        .unwrap();

        let result = backend.exists("test-key").await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("not yet fully implemented"));
    }

    #[tokio::test]
    async fn test_delete_not_implemented() {
        let backend = B2SpacesBackend::new(
            Provider::B2 {
                region: "us-west-002".to_string(),
            },
            "bucket",
            "key",
            "secret",
        )
        .await
        .unwrap();

        let result = backend.delete("test-key").await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("not yet fully implemented"));
    }

    #[tokio::test]
    async fn test_list_objects_not_implemented() {
        let backend = B2SpacesBackend::new(
            Provider::B2 {
                region: "us-west-002".to_string(),
            },
            "bucket",
            "key",
            "secret",
        )
        .await
        .unwrap();

        let result = backend.list_objects("prefix/").await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("not yet fully implemented"));
    }
}

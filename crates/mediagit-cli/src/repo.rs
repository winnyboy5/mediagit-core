//! Repository utilities for MediaGit CLI
//!
//! Shared utilities for repository discovery, path handling, and storage backend creation.

use anyhow::{Context, Result};
use mediagit_storage::StorageBackend;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Find the root of the MediaGit repository by walking up from current directory.
///
/// # Returns
/// - `Ok(PathBuf)` - Path to repository root (directory containing `.mediagit`)
/// - `Err` - If not inside a MediaGit repository
///
/// # Example
/// ```no_run
/// use mediagit_cli::repo::find_repo_root;
///
/// let root = find_repo_root()?;
/// println!("Repository at: {}", root.display());
/// # Ok::<(), anyhow::Error>(())
/// ```
pub fn find_repo_root() -> Result<PathBuf> {
    let mut current = std::env::current_dir()?;

    loop {
        if current.join(".mediagit").exists() {
            return Ok(current);
        }

        if !current.pop() {
            anyhow::bail!("Not a mediagit repository (or any parent up to mount point)");
        }
    }
}

/// Find repository root from a specific starting path.
///
/// # Arguments
/// * `start` - Starting directory to search from
///
/// # Returns
/// - `Ok(PathBuf)` - Path to repository root
/// - `Err` - If not inside a MediaGit repository
#[allow(dead_code)] // Reserved for path-based command operations
pub fn find_repo_root_from(start: &std::path::Path) -> Result<PathBuf> {
    let mut current = start.to_path_buf();

    loop {
        if current.join(".mediagit").exists() {
            return Ok(current);
        }

        if !current.pop() {
            anyhow::bail!("Not a mediagit repository (or any parent up to mount point)");
        }
    }
}

/// Get the .mediagit directory path for the current repository.
///
/// # Returns
/// - `Ok(PathBuf)` - Path to `.mediagit` directory
/// - `Err` - If not inside a MediaGit repository
#[allow(dead_code)] // Reserved for direct .mediagit access patterns
pub fn get_mediagit_dir() -> Result<PathBuf> {
    Ok(find_repo_root()?.join(".mediagit"))
}

/// Create the appropriate storage backend based on repository config.
///
/// Reads `.mediagit/config.toml` to determine backend type (filesystem, S3, Azure, GCS).
/// Falls back to local filesystem if config is missing or uses default storage.
///
/// # Arguments
/// * `repo_root` - Root of the mediagit repository (parent of .mediagit/)
///
/// # Returns
/// An `Arc<dyn StorageBackend>` configured per the repository's config.toml
pub async fn create_storage_backend(repo_root: &Path) -> Result<Arc<dyn StorageBackend>> {
    let mediagit_dir = repo_root.join(".mediagit");

    // Load config (returns default if config.toml doesn't exist)
    let config = mediagit_config::Config::load(repo_root)
        .await
        .unwrap_or_default();

    match &config.storage {
        mediagit_config::StorageConfig::FileSystem(fs_config) => {
            let storage_path = if std::path::Path::new(&fs_config.base_path).is_absolute() {
                PathBuf::from(&fs_config.base_path)
            } else if fs_config.base_path == "./data" {
                // Default config value - use .mediagit
                mediagit_dir.clone()
            } else {
                repo_root.join(&fs_config.base_path)
            };
            let storage = mediagit_storage::LocalBackend::new(&storage_path)
                .await
                .context("Failed to initialize filesystem storage backend")?;
            Ok(Arc::new(storage))
        }
        mediagit_config::StorageConfig::S3(s3_config) => {
            if let Some(endpoint) = &s3_config.endpoint {
                // S3-compatible (MinIO, DigitalOcean Spaces, etc.)
                let storage = mediagit_storage::MinIOBackend::new(
                    endpoint,
                    &s3_config.bucket,
                    s3_config.access_key_id.as_deref().unwrap_or(""),
                    s3_config.secret_access_key.as_deref().unwrap_or(""),
                )
                .await
                .context("Failed to initialize S3-compatible storage backend")?;
                Ok(Arc::new(storage))
            } else {
                // AWS S3
                let aws_endpoint = format!("https://s3.{}.amazonaws.com", s3_config.region);
                let storage = mediagit_storage::MinIOBackend::new(
                    &aws_endpoint,
                    &s3_config.bucket,
                    s3_config.access_key_id.as_deref().unwrap_or(""),
                    s3_config.secret_access_key.as_deref().unwrap_or(""),
                )
                .await
                .context("Failed to initialize AWS S3 storage backend")?;
                Ok(Arc::new(storage))
            }
        }
        mediagit_config::StorageConfig::Azure(azure_config) => {
            let storage = if let Some(conn_str) = &azure_config.connection_string {
                mediagit_storage::AzureBackend::with_connection_string(
                    &azure_config.container,
                    conn_str,
                )
                .await
                .context("Failed to initialize Azure storage backend")?
            } else if let Some(account_key) = &azure_config.account_key {
                mediagit_storage::AzureBackend::with_account_key(
                    &azure_config.account_name,
                    &azure_config.container,
                    account_key,
                )
                .await
                .context("Failed to initialize Azure storage backend")?
            } else {
                anyhow::bail!("Azure backend requires either connection_string or account_key");
            };
            Ok(Arc::new(storage))
        }
        mediagit_config::StorageConfig::GCS(gcs_config) => {
            let credentials_path = gcs_config
                .credentials_path
                .as_deref()
                .unwrap_or("");

            let storage = if credentials_path.is_empty() {
                mediagit_storage::GcsBackend::with_default_credentials(
                    &gcs_config.project_id,
                    &gcs_config.bucket,
                )
                .await
                .context("Failed to initialize GCS storage backend")?
            } else {
                mediagit_storage::GcsBackend::new(
                    &gcs_config.project_id,
                    &gcs_config.bucket,
                    credentials_path,
                )
                .await
                .context("Failed to initialize GCS storage backend")?
            };
            Ok(Arc::new(storage))
        }
        mediagit_config::StorageConfig::Multi(_) => {
            anyhow::bail!("Multi-backend storage is not yet implemented");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_find_repo_root_from() {
        let temp = TempDir::new().unwrap();
        let repo_root = temp.path();

        // Create .mediagit directory
        std::fs::create_dir(repo_root.join(".mediagit")).unwrap();

        // Create nested directory
        let nested = repo_root.join("src").join("commands");
        std::fs::create_dir_all(&nested).unwrap();

        // Should find root from nested path
        let found = find_repo_root_from(&nested).unwrap();
        assert_eq!(found, repo_root);
    }

    #[test]
    fn test_find_repo_root_from_not_found() {
        let temp = TempDir::new().unwrap();
        // No .mediagit directory
        let result = find_repo_root_from(temp.path());
        assert!(result.is_err());
    }
}

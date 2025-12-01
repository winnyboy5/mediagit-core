// SPDX-License-Identifier: AGPL-3.0
// Copyright (C) 2025 MediaGit Contributors

//! Git filter driver implementation
//!
//! This module implements the Git filter driver protocol, providing clean and
//! smudge filters for MediaGit pointer files.
//!
//! ## Filter Operations
//!
//! - **Clean**: Converts media files to pointer files when staging (`git add`)
//! - **Smudge**: Restores pointer files to media files when checking out (`git checkout`)
//!
//! ## Usage
//!
//! ```rust,no_run
//! use mediagit_git::{FilterDriver, FilterConfig};
//! use std::path::Path;
//!
//! let config = FilterConfig::default();
//! let driver = FilterDriver::new(config)?;
//!
//! // Install filter driver in repository
//! driver.install(Path::new("/path/to/repo"))?;
//!
//! // Configure file patterns
//! driver.track_pattern(Path::new("/path/to/repo"), "*.psd")?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use crate::error::{GitError, GitResult};
use crate::pointer::PointerFile;
use git2::Repository;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;
use tracing::{debug, info, warn};

/// Filter driver name used in Git configuration
pub const FILTER_DRIVER_NAME: &str = "mediagit";

/// Minimum file size threshold for using MediaGit (bytes)
/// Files smaller than this will be stored in Git normally
pub const MIN_FILE_SIZE_THRESHOLD: u64 = 1024 * 1024; // 1 MB

/// Configuration for the filter driver
#[derive(Debug, Clone)]
pub struct FilterConfig {
    /// Minimum file size to use MediaGit storage (default: 1MB)
    pub min_file_size: u64,

    /// Path to MediaGit object storage
    pub storage_path: Option<String>,

    /// Whether to skip binary detection
    pub skip_binary_check: bool,
}

impl Default for FilterConfig {
    fn default() -> Self {
        Self {
            min_file_size: MIN_FILE_SIZE_THRESHOLD,
            storage_path: None,
            skip_binary_check: false,
        }
    }
}

/// Git filter driver implementation for MediaGit
pub struct FilterDriver {
    config: FilterConfig,
}

impl FilterDriver {
    /// Creates a new filter driver with the given configuration
    pub fn new(config: FilterConfig) -> GitResult<Self> {
        Ok(Self { config })
    }

    /// Returns a reference to the filter configuration
    pub fn config(&self) -> &FilterConfig {
        &self.config
    }

    /// Installs the filter driver in a Git repository
    ///
    /// This configures the Git repository to use MediaGit's clean and smudge filters.
    ///
    /// # Arguments
    ///
    /// * `repo_path` - Path to the Git repository
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use mediagit_git::{FilterDriver, FilterConfig};
    /// use std::path::Path;
    ///
    /// let driver = FilterDriver::new(FilterConfig::default())?;
    /// driver.install(Path::new("/path/to/repo"))?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn install(&self, repo_path: &Path) -> GitResult<()> {
        info!("Installing MediaGit filter driver in repository: {:?}", repo_path);

        let repo = Repository::open(repo_path).map_err(|e| {
            GitError::RepositoryNotFound(format!("{}: {}", repo_path.display(), e))
        })?;

        let mut config = repo.config()?;

        // Configure clean filter
        config.set_str(
            &format!("filter.{}.clean", FILTER_DRIVER_NAME),
            "mediagit filter-clean %f",
        )?;

        // Configure smudge filter
        config.set_str(
            &format!("filter.{}.smudge", FILTER_DRIVER_NAME),
            "mediagit filter-smudge %f",
        )?;

        // Mark as required (Git will abort if filter fails)
        config.set_bool(&format!("filter.{}.required", FILTER_DRIVER_NAME), true)?;

        info!("Filter driver installed successfully");
        Ok(())
    }

    /// Configures .gitattributes to track a file pattern
    ///
    /// # Arguments
    ///
    /// * `repo_path` - Path to the Git repository
    /// * `pattern` - File pattern to track (e.g., "*.psd", "*.mp4")
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use mediagit_git::{FilterDriver, FilterConfig};
    /// use std::path::Path;
    ///
    /// let driver = FilterDriver::new(FilterConfig::default())?;
    /// driver.track_pattern(Path::new("/path/to/repo"), "*.psd")?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn track_pattern(&self, repo_path: &Path, pattern: &str) -> GitResult<()> {
        info!("Tracking pattern: {}", pattern);

        let gitattributes_path = repo_path.join(".gitattributes");

        // Read existing content
        let mut content = if gitattributes_path.exists() {
            fs::read_to_string(&gitattributes_path)
                .map_err(|e| GitError::GitattributesConfig(e.to_string()))?
        } else {
            String::new()
        };

        // Check if pattern already exists
        let filter_line = format!("{} filter={}", pattern, FILTER_DRIVER_NAME);
        if content.contains(&filter_line) {
            debug!("Pattern {} already tracked", pattern);
            return Ok(());
        }

        // Add new pattern
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(&filter_line);
        content.push('\n');

        // Write updated content
        fs::write(&gitattributes_path, content)
            .map_err(|e| GitError::GitattributesConfig(e.to_string()))?;

        info!("Pattern {} added to .gitattributes", pattern);
        Ok(())
    }

    /// Removes a file pattern from .gitattributes
    ///
    /// # Arguments
    ///
    /// * `repo_path` - Path to the Git repository
    /// * `pattern` - File pattern to untrack
    pub fn untrack_pattern(&self, repo_path: &Path, pattern: &str) -> GitResult<()> {
        info!("Untracking pattern: {}", pattern);

        let gitattributes_path = repo_path.join(".gitattributes");

        if !gitattributes_path.exists() {
            debug!(".gitattributes does not exist");
            return Ok(());
        }

        let content = fs::read_to_string(&gitattributes_path)
            .map_err(|e| GitError::GitattributesConfig(e.to_string()))?;

        let filter_line = format!("{} filter={}", pattern, FILTER_DRIVER_NAME);
        let new_content: String = content
            .lines()
            .filter(|line| !line.contains(&filter_line))
            .collect::<Vec<_>>()
            .join("\n");

        fs::write(&gitattributes_path, new_content)
            .map_err(|e| GitError::GitattributesConfig(e.to_string()))?;

        info!("Pattern {} removed from .gitattributes", pattern);
        Ok(())
    }

    /// Executes the clean filter (file → pointer)
    ///
    /// Reads file content from stdin, computes hash, stores in object database,
    /// and outputs pointer file to stdout.
    ///
    /// # Arguments
    ///
    /// * `file_path` - Path to the file being cleaned (for logging)
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use mediagit_git::{FilterDriver, FilterConfig};
    ///
    /// let driver = FilterDriver::new(FilterConfig::default())?;
    /// driver.clean(Some("path/to/file.psd"))?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn clean(&self, file_path: Option<&str>) -> GitResult<()> {
        let path_info = file_path.unwrap_or("<stdin>");
        debug!("Running clean filter for: {}", path_info);

        // Read file content from stdin
        let mut content = Vec::new();
        io::stdin()
            .read_to_end(&mut content)
            .map_err(|e| GitError::FilterFailed(format!("Failed to read stdin: {}", e)))?;

        let file_size = content.len() as u64;

        // Check if file is too small to use MediaGit
        if file_size < self.config.min_file_size {
            debug!(
                "File {} is {} bytes, below threshold {}. Passing through.",
                path_info, file_size, self.config.min_file_size
            );
            io::stdout()
                .write_all(&content)
                .map_err(|e| GitError::FilterFailed(format!("Failed to write stdout: {}", e)))?;
            return Ok(());
        }

        // Compute SHA-256 hash
        let mut hasher = Sha256::new();
        hasher.update(&content);
        let hash = hasher.finalize();
        let oid = hex::encode(hash);

        debug!("Computed OID for {}: {}", path_info, oid);

        // NOTE: Object storage integration pending
        // The actual file content should be stored in the object database using mediagit-storage.
        // Current implementation creates pointer files without storage integration.
        // Integration requires: StorageBackend trait implementation + ODB write operations.

        // Create and output pointer file
        let pointer = PointerFile::new(oid, file_size);
        let pointer_content = pointer.to_bytes();

        io::stdout()
            .write_all(&pointer_content)
            .map_err(|e| GitError::FilterFailed(format!("Failed to write pointer: {}", e)))?;

        info!("Clean filter completed for {}: {} bytes → pointer", path_info, file_size);
        Ok(())
    }

    /// Executes the smudge filter (pointer → file)
    ///
    /// Reads pointer file from stdin, retrieves actual content from object database,
    /// and outputs file content to stdout.
    ///
    /// # Arguments
    ///
    /// * `file_path` - Path to the file being smudged (for logging)
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use mediagit_git::{FilterDriver, FilterConfig};
    ///
    /// let driver = FilterDriver::new(FilterConfig::default())?;
    /// driver.smudge(Some("path/to/file.psd"))?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn smudge(&self, file_path: Option<&str>) -> GitResult<()> {
        let path_info = file_path.unwrap_or("<stdin>");
        debug!("Running smudge filter for: {}", path_info);

        // Read input from stdin
        let mut input = String::new();
        io::stdin()
            .read_to_string(&mut input)
            .map_err(|e| GitError::FilterFailed(format!("Failed to read stdin: {}", e)))?;

        // Check if input is a pointer file
        if !PointerFile::is_pointer(&input) {
            debug!("Input is not a pointer file, passing through");
            io::stdout()
                .write_all(input.as_bytes())
                .map_err(|e| GitError::FilterFailed(format!("Failed to write stdout: {}", e)))?;
            return Ok(());
        }

        // Parse pointer file
        let pointer = PointerFile::parse(&input)?;
        debug!("Parsed pointer for {}: OID={}, size={}", path_info, pointer.oid, pointer.size);

        // NOTE: Object retrieval integration pending
        // The actual file content should be retrieved from the object database using mediagit-storage.
        // Current implementation passes through pointer files without retrieval.
        // Integration requires: StorageBackend trait implementation + ODB read operations.

        warn!("Object retrieval not yet implemented, outputting pointer file");
        io::stdout()
            .write_all(input.as_bytes())
            .map_err(|e| GitError::FilterFailed(format!("Failed to write stdout: {}", e)))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_filter_config_default() {
        let config = FilterConfig::default();
        assert_eq!(config.min_file_size, MIN_FILE_SIZE_THRESHOLD);
        assert!(config.storage_path.is_none());
        assert!(!config.skip_binary_check);
    }

    #[test]
    fn test_filter_driver_new() {
        let config = FilterConfig::default();
        let driver = FilterDriver::new(config);
        assert!(driver.is_ok());
    }

    #[test]
    fn test_install_in_nonexistent_repo() {
        let driver = FilterDriver::new(FilterConfig::default()).unwrap();
        let temp_dir = TempDir::new().unwrap();
        let result = driver.install(temp_dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_track_pattern() {
        let driver = FilterDriver::new(FilterConfig::default()).unwrap();
        let temp_dir = TempDir::new().unwrap();

        // Track a pattern
        driver.track_pattern(temp_dir.path(), "*.psd").unwrap();

        // Check .gitattributes was created
        let gitattributes_path = temp_dir.path().join(".gitattributes");
        assert!(gitattributes_path.exists());

        // Check content
        let content = fs::read_to_string(&gitattributes_path).unwrap();
        assert!(content.contains("*.psd filter=mediagit"));
    }

    #[test]
    fn test_track_pattern_duplicate() {
        let driver = FilterDriver::new(FilterConfig::default()).unwrap();
        let temp_dir = TempDir::new().unwrap();

        // Track same pattern twice
        driver.track_pattern(temp_dir.path(), "*.psd").unwrap();
        driver.track_pattern(temp_dir.path(), "*.psd").unwrap();

        // Check .gitattributes has only one entry
        let gitattributes_path = temp_dir.path().join(".gitattributes");
        let content = fs::read_to_string(&gitattributes_path).unwrap();
        let count = content.matches("*.psd filter=mediagit").count();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_untrack_pattern() {
        let driver = FilterDriver::new(FilterConfig::default()).unwrap();
        let temp_dir = TempDir::new().unwrap();

        // Track and then untrack
        driver.track_pattern(temp_dir.path(), "*.psd").unwrap();
        driver.untrack_pattern(temp_dir.path(), "*.psd").unwrap();

        // Check content doesn't contain the pattern
        let gitattributes_path = temp_dir.path().join(".gitattributes");
        let content = fs::read_to_string(&gitattributes_path).unwrap();
        assert!(!content.contains("*.psd filter=mediagit"));
    }

    #[test]
    fn test_untrack_nonexistent_pattern() {
        let driver = FilterDriver::new(FilterConfig::default()).unwrap();
        let temp_dir = TempDir::new().unwrap();

        // Untrack pattern that was never tracked
        let result = driver.untrack_pattern(temp_dir.path(), "*.psd");
        assert!(result.is_ok());
    }
}

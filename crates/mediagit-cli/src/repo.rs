//! Repository utilities for MediaGit CLI
//!
//! Shared utilities for repository discovery and path handling.

use anyhow::Result;
use std::path::PathBuf;

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

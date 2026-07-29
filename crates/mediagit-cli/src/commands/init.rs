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

// ! Initialize a new MediaGit repository.
//!
//! The `init` command creates a new MediaGit repository with the required
//! directory structure and configuration files.

use anyhow::{Context, Result};
use clap::Parser;
use mediagit_config::{Config, FileSystemStorage, StorageConfig};
use mediagit_storage::LocalBackend;
use mediagit_versioning::{Ref, RefDatabase};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::info;

/// Initialize a new MediaGit repository
///
/// Creates a new MediaGit repository with the required directory structure,
/// configuration files, and initial branch setup.
#[derive(Parser, Debug)]
#[command(after_help = "EXAMPLES:
    # Initialize repository in current directory
    mediagit init

    # Initialize repository in a specific path
    mediagit init my-project

    # Initialize with custom initial branch name
    mediagit init --initial-branch develop

SEE ALSO:
    mediagit-remote(1), mediagit-clone(1)")]
pub struct InitCmd {
    /// Path to initialize (defaults to current directory)
    #[arg(value_name = "PATH")]
    pub path: Option<String>,

    /// Initial branch name (default: main)
    #[arg(long, value_name = "BRANCH")]
    pub initial_branch: Option<String>,

    /// Compatibility alias: MediaGit repositories always use the .mediagit
    /// layout, so this creates the same structure as a plain init. Commonly
    /// used when seeding a server-side repository directory.
    #[arg(long)]
    pub bare: bool,

    /// Quiet mode - minimal output
    #[arg(short, long)]
    pub quiet: bool,
}

impl InitCmd {
    pub async fn execute(&self) -> Result<()> {
        use crate::output;

        // Determine repository path
        let repo_path = self.get_repo_path()?;

        if !self.quiet {
            output::header(&format!(
                "Initializing MediaGit repository in {}",
                repo_path.display()
            ));
        }

        // Check if already initialized
        if repo_path.join(".mediagit").exists() {
            anyhow::bail!("Repository already initialized at {}", repo_path.display());
        }

        // Create repository structure
        self.create_directory_structure(&repo_path)
            .context("Failed to create repository structure")?;

        // Initialize storage backend (local for now)
        // LocalBackend will create the "objects" directory automatically
        let storage_path = repo_path.join(".mediagit");
        let _storage: Arc<dyn mediagit_storage::StorageBackend> =
            Arc::new(LocalBackend::new(&storage_path).await?);

        // Initialize reference database (uses direct filesystem, not StorageBackend)
        let refdb = RefDatabase::new(&storage_path);

        // Create initial branch
        let initial_branch = self.initial_branch.as_deref().unwrap_or("main");
        validate_branch_name(initial_branch)?;
        let branch_ref_name = format!("refs/heads/{}", initial_branch);

        // Create HEAD pointing to initial branch (symbolic ref)
        let head = Ref::new_symbolic("HEAD".to_string(), branch_ref_name.clone());
        refdb
            .write(&head)
            .await
            .context("Failed to create HEAD reference")?;

        // Create default configuration
        self.create_default_config(&repo_path, initial_branch)?;

        // Write the layout-v2 LAYOUT marker under the repo's namespace. Must
        // happen after config.toml exists (it carries repo_namespace and
        // repo_id), so we re-derive the (now-namespaced) storage backend via
        // the same factory every other command uses rather than reusing the
        // raw, unwrapped `_storage` constructed above.
        // `create_storage_backend` performs the marker check/write itself
        // (it's one of the two production wrap-points), so no separate call
        // is needed here.
        crate::repo::create_storage_backend(&repo_path)
            .await
            .context("Failed to write LAYOUT marker")?;

        if !self.quiet {
            output::success(&format!(
                "Initialized empty MediaGit repository in {}",
                repo_path.join(".mediagit").display()
            ));
            output::detail("Initial branch", initial_branch);
        }

        Ok(())
    }

    fn get_repo_path(&self) -> Result<PathBuf> {
        let path = match &self.path {
            Some(p) => PathBuf::from(p),
            None => std::env::current_dir().context("Failed to get current directory")?,
        };

        fs::create_dir_all(&path)
            .context(format!("Failed to create directory: {}", path.display()))?;
        // Use dunce::canonicalize for cross-platform compatibility
        // This avoids Windows \\?\ prefix in display paths
        dunce::canonicalize(&path).context("Failed to canonicalize path")
    }

    fn create_directory_structure(&self, repo_path: &Path) -> Result<()> {
        info!("Creating .mediagit directory structure");

        let mediagit_dir = repo_path.join(".mediagit");

        // Create main directories
        fs::create_dir(&mediagit_dir).context("Failed to create .mediagit directory")?;

        fs::create_dir(mediagit_dir.join("objects"))
            .context("Failed to create objects directory")?;

        fs::create_dir(mediagit_dir.join("refs")).context("Failed to create refs directory")?;

        fs::create_dir(mediagit_dir.join("refs/heads"))
            .context("Failed to create refs/heads directory")?;

        fs::create_dir(mediagit_dir.join("refs/tags"))
            .context("Failed to create refs/tags directory")?;

        fs::create_dir(mediagit_dir.join("refs/remotes"))
            .context("Failed to create refs/remotes directory")?;

        Ok(())
    }

    fn create_default_config(&self, repo_path: &Path, _initial_branch: &str) -> Result<()> {
        info!("Creating default configuration");

        // Layout v2: default namespace = sanitized basename of the repo root,
        // computed once and persisted so it survives the repo being moved
        // or MEDIAGIT_REPO_NAMESPACE not being set on a later invocation.
        let namespace = repo_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "repo".to_string());

        // Configure filesystem storage
        let config = Config {
            storage: StorageConfig::FileSystem(FileSystemStorage {
                base_path: repo_path.join(".mediagit/objects").display().to_string(),
                create_dirs: true,
                sync: false,
                file_permissions: "0644".to_string(),
            }),
            cdc_seed: generate_cdc_seed(),
            repo_namespace: Some(mediagit_storage::sanitize_namespace(&namespace)),
            layout_version: mediagit_config::CURRENT_LAYOUT_VERSION,
            repo_id: Some(mediagit_storage::generate_repo_id()),
            ..Config::default()
        };

        let config_path = repo_path.join(".mediagit/config.toml");
        let config_content =
            toml::to_string_pretty(&config).context("Failed to serialize config")?;

        fs::write(&config_path, config_content).context("Failed to write config file")?;

        Ok(())
    }
}

/// Validate `--initial-branch` before it's written into HEAD as a symbolic
/// ref. An empty or malformed name would leave HEAD as e.g.
/// `ref: refs/heads/` — a repo that's born broken.
fn validate_branch_name(name: &str) -> Result<()> {
    if name.trim().is_empty() {
        anyhow::bail!("initial branch name must not be empty");
    }
    if name.chars().any(char::is_whitespace) {
        anyhow::bail!(
            "initial branch name must not contain whitespace: {:?}",
            name
        );
    }
    if name.starts_with('/') || name.ends_with('/') || name.starts_with('.') || name.ends_with('.')
    {
        anyhow::bail!(
            "initial branch name must not start or end with '/' or '.': {:?}",
            name
        );
    }
    if name.contains("..") || name.contains("@{") || name.ends_with(".lock") {
        anyhow::bail!(
            "initial branch name contains an invalid sequence: {:?}",
            name
        );
    }
    const FORBIDDEN: &[char] = &['~', '^', ':', '?', '*', '[', '`'];
    if name.chars().any(|c| FORBIDDEN.contains(&c)) {
        anyhow::bail!(
            "initial branch name contains an invalid character: {:?}",
            name
        );
    }
    Ok(())
}

/// Generate a fresh per-repo CDC seed for new repositories.
///
/// Draws 32 bytes from the OS RNG and derives the seed via BLAKE3's key
/// derivation function, then discards the random bytes themselves — only the
/// derived seed is persisted. Storing just the seed (vs. seed + secret) is
/// one config field instead of two and leaks nothing extra: the seed only
/// need be unpredictable enough to avoid two independently-created repos
/// colliding, not cryptographically secret.
fn generate_cdc_seed() -> u64 {
    let mut secret = [0u8; 32];
    if getrandom::fill(&mut secret).is_err() {
        // OS RNG unavailable: fall back to legacy unseeded behavior rather
        // than failing `init` entirely.
        return 0;
    }
    let derived = blake3::derive_key("mediagit cdc seed v1", &secret);
    u64::from_le_bytes(derived[..8].try_into().expect("8 bytes"))
}

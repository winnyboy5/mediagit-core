// ! Initialize a new MediaGit repository.
//!
//! The `init` command creates a new MediaGit repository with the required
//! directory structure and configuration files.

use anyhow::{Context, Result};
use clap::Parser;
use mediagit_config::{Config, FileSystemStorage, StorageConfig};
use mediagit_storage::LocalBackend;
use mediagit_versioning::{ObjectDatabase, Ref, RefDatabase};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::info;

/// Initialize a new MediaGit repository
#[derive(Parser, Debug)]
pub struct InitCmd {
    /// Path to initialize (defaults to current directory)
    #[arg(value_name = "PATH")]
    pub path: Option<String>,

    /// Don't create initial branch
    #[arg(long)]
    pub bare: bool,

    /// Initial branch name (default: main)
    #[arg(long, value_name = "BRANCH")]
    pub initial_branch: Option<String>,

    /// Template directory
    #[arg(long, value_name = "PATH")]
    pub template: Option<String>,

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
            output::header(&format!("Initializing MediaGit repository in {}", repo_path.display()));
        }

        // Check if already initialized
        if repo_path.join(".mediagit").exists() {
            anyhow::bail!(
                "Repository already initialized at {}",
                repo_path.display()
            );
        }

        // Create repository structure
        self.create_directory_structure(&repo_path)
            .context("Failed to create repository structure")?;

        // Initialize storage backend (local for now)
        let storage_path = repo_path.join(".mediagit/objects");
        let storage: Arc<dyn mediagit_storage::StorageBackend> = Arc::new(LocalBackend::new(&storage_path).await?);

        // Initialize object database
        let _odb = ObjectDatabase::new(storage.clone(), 1000);

        // Initialize reference database
        let refdb = RefDatabase::new(storage);

        // Create initial branch
        let initial_branch = self.initial_branch.as_deref().unwrap_or("main");
        let branch_ref_name = format!("refs/heads/{}", initial_branch);

        // Create HEAD pointing to initial branch (symbolic ref)
        let head = Ref::new_symbolic("HEAD".to_string(), branch_ref_name.clone());
        refdb
            .write(&head)
            .await
            .context("Failed to create HEAD reference")?;

        // Create default configuration
        self.create_default_config(&repo_path, initial_branch)?;

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

        path.canonicalize()
            .context("Failed to canonicalize path")
    }

    fn create_directory_structure(&self, repo_path: &Path) -> Result<()> {
        info!("Creating .mediagit directory structure");

        let mediagit_dir = repo_path.join(".mediagit");

        // Create main directories
        fs::create_dir(&mediagit_dir)
            .context("Failed to create .mediagit directory")?;

        fs::create_dir(mediagit_dir.join("objects"))
            .context("Failed to create objects directory")?;

        fs::create_dir(mediagit_dir.join("refs"))
            .context("Failed to create refs directory")?;

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

        let mut config = Config::default();

        // Configure filesystem storage
        config.storage = StorageConfig::FileSystem(FileSystemStorage {
            base_path: repo_path.join(".mediagit/objects").display().to_string(),
            create_dirs: true,
            sync: false,
            file_permissions: "0644".to_string(),
        });

        let config_path = repo_path.join(".mediagit/config.toml");
        let config_content = toml::to_string_pretty(&config)
            .context("Failed to serialize config")?;

        fs::write(&config_path, config_content)
            .context("Failed to write config file")?;

        Ok(())
    }
}

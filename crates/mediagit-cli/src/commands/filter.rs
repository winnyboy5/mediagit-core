// SPDX-License-Identifier: AGPL-3.0
// Copyright (C) 2025 MediaGit Contributors

//! Git filter driver commands (clean and smudge)

use anyhow::{Context, Result};
use clap::Subcommand;
use mediagit_git::{FilterConfig, FilterDriver};
use std::path::PathBuf;

#[derive(Debug, Subcommand)]
pub enum FilterCmd {
    /// Clean filter: convert file to pointer (git add)
    Clean {
        /// File path being cleaned
        #[arg(value_name = "FILE")]
        file_path: Option<String>,
    },

    /// Smudge filter: restore pointer to file (git checkout)
    Smudge {
        /// File path being smudged
        #[arg(value_name = "FILE")]
        file_path: Option<String>,
    },
}

impl FilterCmd {
    pub fn execute(self) -> Result<()> {
        // Find repository root to get storage path
        let repo_root = Self::find_repo_root()?;
        let storage_path = repo_root.join(".mediagit");

        // Create filter configuration
        let config = FilterConfig {
            min_file_size: 1024 * 1024, // 1 MB
            storage_path: Some(storage_path.to_string_lossy().to_string()),
            skip_binary_check: false,
        };

        // Create filter driver
        let driver = FilterDriver::new(config)
            .context("Failed to create filter driver")?;

        match self {
            FilterCmd::Clean { file_path } => {
                driver.clean(file_path.as_deref())
                    .context("Clean filter operation failed")?;
                Ok(())
            }
            FilterCmd::Smudge { file_path } => {
                driver.smudge(file_path.as_deref())
                    .context("Smudge filter operation failed")?;
                Ok(())
            }
        }
    }

    fn find_repo_root() -> Result<PathBuf> {
        let mut current = std::env::current_dir()?;

        loop {
            if current.join(".mediagit").exists() || current.join(".git").exists() {
                return Ok(current);
            }

            if !current.pop() {
                anyhow::bail!("Not in a git or mediagit repository");
            }
        }
    }
}

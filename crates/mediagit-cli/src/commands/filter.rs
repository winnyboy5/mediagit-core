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
        let driver = FilterDriver::new(config).context("Failed to create filter driver")?;

        match self {
            FilterCmd::Clean { file_path } => {
                driver
                    .clean(file_path.as_deref())
                    .context("Clean filter operation failed")?;
                Ok(())
            }
            FilterCmd::Smudge { file_path } => {
                driver
                    .smudge(file_path.as_deref())
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

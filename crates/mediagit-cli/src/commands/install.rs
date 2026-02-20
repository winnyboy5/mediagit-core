// Copyright (C) 2026  winnyboy5
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
// SPDX-License-Identifier: AGPL-3.0
// Copyright (C) 2025 MediaGit Contributors

//! Install MediaGit Git filter driver

use anyhow::{Context, Result};
use clap::Args;
use console::style;
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Args)]
pub struct InstallCmd {
    /// Force reinstallation even if already installed
    #[arg(short, long)]
    pub force: bool,

    /// Repository path (defaults to current directory)
    #[arg(short, long)]
    pub repo: Option<String>,

    /// Install globally for all repositories
    #[arg(short, long)]
    pub global: bool,
}

impl InstallCmd {
    pub fn execute(self) -> Result<()> {
        let repo_path = if let Some(ref path) = self.repo {
            PathBuf::from(path)
        } else {
            std::env::current_dir().context("Failed to get current directory")?
        };

        println!(
            "{} Installing MediaGit filter driver...",
            style("🔧").cyan().bold()
        );

        if self.global {
            return self.install_global();
        }

        // Verify it's a git repository
        if !repo_path.join(".git").exists() && !repo_path.join(".mediagit").exists() {
            anyhow::bail!(
                "Not a git repository: {}\nRun 'git init' or 'mediagit init' first",
                repo_path.display()
            );
        }

        // Check if already installed
        if !self.force {
            let check = Command::new("git")
                .args(&["config", "--get", "filter.mediagit.clean"])
                .current_dir(&repo_path)
                .output();

            if let Ok(output) = check {
                if output.status.success() && !output.stdout.is_empty() {
                    println!("{} MediaGit filter already installed", style("ℹ").blue());
                    println!("  Use --force to reinstall");
                    return Ok(());
                }
            }
        }

        // Get mediagit binary path
        let mediagit_path = std::env::current_exe()
            .context("Failed to get mediagit executable path")?;

        // Install filter driver configuration
        self.run_git_config(
            &repo_path,
            "filter.mediagit.clean",
            &format!("{} filter clean", mediagit_path.display()),
        )?;

        self.run_git_config(
            &repo_path,
            "filter.mediagit.smudge",
            &format!("{} filter smudge", mediagit_path.display()),
        )?;

        self.run_git_config(
            &repo_path,
            "filter.mediagit.required",
            "true",
        )?;

        // Install diff driver
        self.run_git_config(
            &repo_path,
            "diff.mediagit.command",
            &format!("{} diff", mediagit_path.display()),
        )?;

        // Install merge driver
        self.run_git_config(
            &repo_path,
            "merge.mediagit.name",
            "MediaGit merge driver",
        )?;

        self.run_git_config(
            &repo_path,
            "merge.mediagit.driver",
            &format!("{} merge %O %A %B", mediagit_path.display()),
        )?;

        println!(
            "{} Successfully installed MediaGit filter driver",
            style("✓").green().bold()
        );
        println!();
        println!("Configuration added:");
        println!("  • Clean filter: mediagit filter clean");
        println!("  • Smudge filter: mediagit filter smudge");
        println!("  • Diff driver: mediagit diff");
        println!("  • Merge driver: mediagit merge");
        println!();
        println!("Next steps:");
        println!("  1. Use 'mediagit track <PATTERN>' to specify which files to manage");
        println!("  2. Example: mediagit track '*.psd'");
        println!("  3. Commit your changes as normal");

        Ok(())
    }

    fn install_global(&self) -> Result<()> {
        let mediagit_path = std::env::current_exe()
            .context("Failed to get mediagit executable path")?;

        println!("{} Installing globally for all repositories", style("🌍").cyan());

        // Install global configuration
        self.run_git_config_global(
            "filter.mediagit.clean",
            &format!("{} filter clean", mediagit_path.display()),
        )?;

        self.run_git_config_global(
            "filter.mediagit.smudge",
            &format!("{} filter smudge", mediagit_path.display()),
        )?;

        self.run_git_config_global(
            "filter.mediagit.required",
            "true",
        )?;

        self.run_git_config_global(
            "diff.mediagit.command",
            &format!("{} diff", mediagit_path.display()),
        )?;

        self.run_git_config_global(
            "merge.mediagit.name",
            "MediaGit merge driver",
        )?;

        self.run_git_config_global(
            "merge.mediagit.driver",
            &format!("{} merge %O %A %B", mediagit_path.display()),
        )?;

        println!(
            "{} Successfully installed MediaGit filter driver globally",
            style("✓").green().bold()
        );
        println!();
        println!("The filter is now available in all repositories.");
        println!("Use 'mediagit track <PATTERN>' in each repository to enable it.");

        Ok(())
    }

    fn run_git_config(&self, repo_path: &PathBuf, key: &str, value: &str) -> Result<()> {
        let output = Command::new("git")
            .args(&["config", key, value])
            .current_dir(repo_path)
            .output()
            .context(format!("Failed to run: git config {} {}", key, value))?;

        if !output.status.success() {
            let error = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Git config failed: {}", error);
        }

        Ok(())
    }

    fn run_git_config_global(&self, key: &str, value: &str) -> Result<()> {
        let output = Command::new("git")
            .args(&["config", "--global", key, value])
            .output()
            .context(format!("Failed to run: git config --global {} {}", key, value))?;

        if !output.status.success() {
            let error = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Git config failed: {}", error);
        }

        Ok(())
    }
}

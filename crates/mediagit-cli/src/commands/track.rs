// SPDX-License-Identifier: AGPL-3.0
// Copyright (C) 2025 MediaGit Contributors

//! Track/untrack file patterns with MediaGit

use anyhow::{Context, Result};
use clap::Args;
use console::style;
use std::fs;
use std::path::PathBuf;
use super::super::repo::find_repo_root;

#[derive(Debug, Args)]
pub struct TrackCmd {
    /// File pattern to track (e.g., "*.psd", "*.mp4")
    #[arg(value_name = "PATTERN")]
    pub pattern: Option<String>,

    /// Show tracked patterns without adding new ones
    #[arg(short, long)]
    pub list: bool,
}

impl TrackCmd {
    pub fn execute(self) -> Result<()> {
        let repo_root = find_repo_root()?;
        let gitattributes_path = repo_root.join(".gitattributes");

        if self.list {
            return self.list_tracked_patterns(&gitattributes_path);
        }

        let pattern = self.pattern.clone().context(
            "Pattern required. Use --list to see tracked patterns or provide a pattern to track."
        )?;

        self.add_tracked_pattern(&gitattributes_path, &pattern)
    }

    fn list_tracked_patterns(&self, gitattributes_path: &PathBuf) -> Result<()> {
        if !gitattributes_path.exists() {
            println!("{} No tracked patterns found", style("ℹ").blue());
            println!("  Use 'mediagit track <PATTERN>' to start tracking media files");
            return Ok(());
        }

        let content = fs::read_to_string(gitattributes_path)
            .context("Failed to read .gitattributes")?;

        let tracked: Vec<&str> = content
            .lines()
            .filter(|line| line.contains("filter=mediagit"))
            .collect();

        if tracked.is_empty() {
            println!("{} No tracked patterns found", style("ℹ").blue());
        } else {
            println!("{} Tracked patterns:", style("📋").cyan().bold());
            for line in tracked {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if let Some(pattern) = parts.first() {
                    println!("  {}", style(pattern).yellow());
                }
            }
        }

        Ok(())
    }

    fn add_tracked_pattern(&self, gitattributes_path: &PathBuf, pattern: &str) -> Result<()> {
        // Read existing content
        let mut content = if gitattributes_path.exists() {
            fs::read_to_string(gitattributes_path)
                .context("Failed to read .gitattributes")?
        } else {
            String::new()
        };

        // Check if pattern already tracked
        if content.lines().any(|line| {
            line.starts_with(pattern) && line.contains("filter=mediagit")
        }) {
            println!(
                "{} Pattern already tracked: {}",
                style("ℹ").blue(),
                style(pattern).yellow()
            );
            return Ok(());
        }

        // Add new tracking line
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(&format!(
            "{} filter=mediagit diff=mediagit merge=mediagit -text\n",
            pattern
        ));

        // Write updated content
        fs::write(gitattributes_path, content)
            .context("Failed to write .gitattributes")?;

        println!(
            "{} Now tracking: {}",
            style("✓").green().bold(),
            style(pattern).yellow()
        );
        println!("  Added to .gitattributes with MediaGit filter");
        println!();
        println!("  Note: Run 'mediagit install' to set up the filter driver if not already done");

        Ok(())
    }

}

#[derive(Debug, Args)]
pub struct UntrackCmd {
    /// File pattern to untrack
    #[arg(value_name = "PATTERN")]
    pub pattern: String,
}

impl UntrackCmd {
    pub fn execute(self) -> Result<()> {
        let repo_root = find_repo_root()?;
        let gitattributes_path = repo_root.join(".gitattributes");

        if !gitattributes_path.exists() {
            println!("{} No .gitattributes file found", style("ℹ").blue());
            return Ok(());
        }

        let content = fs::read_to_string(&gitattributes_path)
            .context("Failed to read .gitattributes")?;

        let original_lines: Vec<&str> = content.lines().collect();
        let filtered: Vec<&str> = original_lines
            .iter()
            .filter(|line| {
                !line.starts_with(&self.pattern) || !line.contains("filter=mediagit")
            })
            .copied()
            .collect();

        if original_lines.len() == filtered.len() {
            println!(
                "{} Pattern not tracked: {}",
                style("ℹ").blue(),
                style(&self.pattern).yellow()
            );
            return Ok(());
        }

        // Write updated content
        let new_content = filtered.join("\n");
        let final_content = if !new_content.is_empty() {
            format!("{}\n", new_content)
        } else {
            new_content
        };

        fs::write(&gitattributes_path, final_content)
            .context("Failed to write .gitattributes")?;

        println!(
            "{} No longer tracking: {}",
            style("✓").green().bold(),
            style(&self.pattern).yellow()
        );

        Ok(())
    }

}

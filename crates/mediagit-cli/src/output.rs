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

//! Shared output formatting utilities for CLI commands.
//!
//! This module provides consistent, colored output formatting with emoji indicators
//! for all MediaGit CLI commands. It ensures a unified user experience across the
//! entire application.
//!
//! # Examples
//!
//! ```rust
//! use mediagit_cli::output;
//!
//! // Success message
//! output::success("Repository initialized successfully");
//!
//! // Error message
//! output::error("Failed to read configuration file");
//!
//! // Info message
//! output::info("Analyzing repository structure...");
//!
//! // Detail line (key-value pair)
//! output::detail("Branch", "main");
//! output::detail("Commits", "42");
//! ```

#![allow(dead_code)] // Functions used by commands, not all implemented yet

use console::style;

/// Print a success message with green checkmark emoji.
///
/// # Examples
///
/// ```rust
/// output::success("Repository initialized successfully");
/// // Output: ✅ Repository initialized successfully
/// ```
pub fn success(msg: &str) {
    println!("{} {}", style("✅").green().bold(), msg);
}

/// Print an error message to stderr with red X emoji.
///
/// # Examples
///
/// ```rust
/// output::error("Failed to connect to remote");
/// // Output (stderr): ❌ Failed to connect to remote
/// ```
pub fn error(msg: &str) {
    eprintln!("{} {}", style("❌").red().bold(), msg);
}

/// Print an informational message with cyan info emoji.
///
/// # Examples
///
/// ```rust
/// output::info("Scanning for media files...");
/// // Output: ℹ️  Scanning for media files...
/// ```
pub fn info(msg: &str) {
    println!("{} {}", style("ℹ️").cyan(), msg);
}

/// Print a warning message with yellow warning emoji.
///
/// # Examples
///
/// ```rust
/// output::warning("Large file detected (>100MB)");
/// // Output: ⚠️  Large file detected (>100MB)
/// ```
pub fn warning(msg: &str) {
    println!("{} {}", style("⚠️").yellow(), msg);
}

/// Print a detail line with key-value formatting.
///
/// The key is displayed in regular text, and the value is highlighted in cyan.
/// This is useful for displaying configuration details or status information.
///
/// # Examples
///
/// ```rust
/// output::detail("Repository path", "/home/user/project");
/// output::detail("Initial branch", "main");
/// output::detail("Storage backend", "local");
/// // Output:
/// //   Repository path: /home/user/project
/// //   Initial branch: main
/// //   Storage backend: local
/// ```
pub fn detail(key: &str, value: &str) {
    println!("  {}: {}", key, style(value).cyan());
}

/// Print a header message with film emoji (for MediaGit branding).
///
/// # Examples
///
/// ```rust
/// output::header("Initializing MediaGit repository...");
/// // Output: 🎬 Initializing MediaGit repository...
/// ```
pub fn header(msg: &str) {
    println!("{} {}", style("🎬").green().bold(), msg);
}

/// Print a progress indicator message.
///
/// # Examples
///
/// ```rust
/// output::progress("Compressing media files...");
/// // Output: 🔄 Compressing media files...
/// ```
pub fn progress(msg: &str) {
    println!("{} {}", style("🔄").cyan(), msg);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_output_functions_compile() {
        // Compile-time verification that all output functions exist
        // These don't actually test output, just ensure the API is correct
        let _ = success;
        let _ = error;
        let _ = info;
        let _ = warning;
        let _ = detail;
        let _ = header;
        let _ = progress;
    }
}

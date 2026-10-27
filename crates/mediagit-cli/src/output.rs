// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Shared output formatting utilities for CLI commands.
//!
//! This module provides consistent, colored output formatting with emoji indicators
//! for all MediaGit CLI commands. It ensures a unified user experience across the
//! entire application.
//!
//! # Examples
//!
//! ```rust,ignore
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

use console::style;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};

/// Version of the `--machine` stdout envelope. Bump only on incompatible change.
pub const MACHINE_CONTRACT_VERSION: u32 = 1;

static MACHINE_MODE: AtomicBool = AtomicBool::new(false);

pub fn set_machine_mode(on: bool) {
    MACHINE_MODE.store(on, Ordering::Relaxed);
}

/// `--machine`: stdout carries exactly one JSON envelope, human output is silent.
pub fn machine_mode() -> bool {
    MACHINE_MODE.load(Ordering::Relaxed)
}

/// A prompt was reached in machine mode: the caller (a GUI) must collect the
/// input itself and re-run the command with it supplied.
#[derive(Debug)]
pub struct NeedsInput(pub String);

impl std::fmt::Display for NeedsInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "input required: {}", self.0)
    }
}

impl std::error::Error for NeedsInput {}

/// Call before any interactive prompt; errors with `NeedsInput` in machine mode.
pub fn ensure_interactive(what: &str) -> anyhow::Result<()> {
    if machine_mode() {
        return Err(NeedsInput(what.to_string()).into());
    }
    Ok(())
}

/// A subcommand reached in machine mode that has no machine output yet.
#[derive(Debug)]
pub struct JsonUnsupported(pub String);

impl std::fmt::Display for JsonUnsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} has no machine output yet", self.0)
    }
}

impl std::error::Error for JsonUnsupported {}

/// An error that carries its own envelope code (e.g. `invalid_args`).
#[derive(Debug)]
pub struct CodedError {
    pub code: &'static str,
    pub message: String,
}

impl CodedError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for CodedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CodedError {}

/// Guard for subcommands without machine output: errors in machine mode.
pub fn ensure_machine_supported(what: &str) -> anyhow::Result<()> {
    if machine_mode() {
        return Err(JsonUnsupported(what.to_string()).into());
    }
    Ok(())
}

/// Envelope error code for a failed command.
pub fn error_code(err: &anyhow::Error) -> &'static str {
    if err.chain().any(|c| c.is::<NeedsInput>()) {
        "needs_input"
    } else if err.chain().any(|c| c.is::<JsonUnsupported>()) {
        "json_unsupported"
    } else if let Some(c) = err.chain().find_map(|c| c.downcast_ref::<CodedError>()) {
        c.code
    } else {
        "error"
    }
}

#[derive(Serialize)]
struct OkEnvelope<'a, T: Serialize> {
    v: u32,
    ok: bool,
    data: &'a T,
}

#[derive(Serialize)]
struct ErrBody<'a> {
    code: &'a str,
    message: &'a str,
}

#[derive(Serialize)]
struct ErrEnvelope<'a> {
    v: u32,
    ok: bool,
    error: ErrBody<'a>,
}

/// Print the success envelope as one stdout line.
pub fn emit_ok<T: Serialize>(data: &T) -> anyhow::Result<()> {
    let line = serde_json::to_string(&OkEnvelope {
        v: MACHINE_CONTRACT_VERSION,
        ok: true,
        data,
    })?;
    println!("{line}");
    Ok(())
}

/// Print the failure envelope as one stdout line.
pub fn emit_err(code: &str, message: &str) {
    let line = serde_json::to_string(&ErrEnvelope {
        v: MACHINE_CONTRACT_VERSION,
        ok: false,
        error: ErrBody { code, message },
    })
    .expect("error envelope serializes");
    println!("{line}");
}

/// Print a success message with green checkmark emoji.
///
/// # Examples
///
/// ```rust,ignore
/// output::success("Repository initialized successfully");
/// // Output: ✅ Repository initialized successfully
/// ```
pub fn success(msg: &str) {
    if machine_mode() {
        return;
    }
    println!("{} {}", style("✅").green().bold(), msg);
}

/// Print an error message to stderr with red X emoji.
///
/// # Examples
///
/// ```rust,ignore
/// output::error("Failed to connect to remote");
/// // Output (stderr): ❌ Failed to connect to remote
/// ```
pub fn error(msg: &str) {
    if machine_mode() {
        return;
    }
    eprintln!("{} {}", style("❌").red().bold(), msg);
}

/// Print an informational message with cyan info emoji.
///
/// # Examples
///
/// ```rust,ignore
/// output::info("Scanning for media files...");
/// // Output: ℹ️  Scanning for media files...
/// ```
pub fn info(msg: &str) {
    if machine_mode() {
        return;
    }
    println!("{} {}", style("ℹ️").cyan(), msg);
}

/// Print a warning message with yellow warning emoji.
///
/// # Examples
///
/// ```rust,ignore
/// output::warning("Large file detected (>100MB)");
/// // Output: ⚠️  Large file detected (>100MB)
/// ```
pub fn warning(msg: &str) {
    if machine_mode() {
        return;
    }
    println!("{} {}", style("⚠️").yellow(), msg);
}

/// Print a detail line with key-value formatting.
///
/// The key is displayed in regular text, and the value is highlighted in cyan.
/// This is useful for displaying configuration details or status information.
///
/// # Examples
///
/// ```rust,ignore
/// output::detail("Repository path", "/home/user/project");
/// output::detail("Initial branch", "main");
/// output::detail("Storage backend", "local");
/// // Output:
/// //   Repository path: /home/user/project
/// //   Initial branch: main
/// //   Storage backend: local
/// ```
pub fn detail(key: &str, value: &str) {
    if machine_mode() {
        return;
    }
    println!("  {}: {}", key, style(value).cyan());
}

/// Print a header message with film emoji (for MediaGit branding).
///
/// # Examples
///
/// ```rust,ignore
/// output::header("Initializing MediaGit repository...");
/// // Output: 🎬 Initializing MediaGit repository...
/// ```
pub fn header(msg: &str) {
    if machine_mode() {
        return;
    }
    println!("{} {}", style("🎬").green().bold(), msg);
}

/// Print a progress indicator message.
///
/// # Examples
///
/// ```rust,ignore
/// output::progress("Compressing media files...");
/// // Output: 🔄 Compressing media files...
/// ```
pub fn progress(msg: &str) {
    if machine_mode() {
        return;
    }
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

    #[test]
    fn needs_input_maps_to_its_code_even_with_context() {
        let plain = anyhow::anyhow!("boom");
        assert_eq!(error_code(&plain), "error");
        let needs = anyhow::Error::from(NeedsInput("passphrase".into())).context("unlocking");
        assert_eq!(error_code(&needs), "needs_input");
    }
}

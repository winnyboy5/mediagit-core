// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Configuration for structured logging and tracing.
//!
//! This module provides types and utilities for configuring the logging
//! behavior of the application, including output formats, log levels, and
//! filter configurations.

use std::io;
use thiserror::Error;

/// Errors that can occur during logging configuration
#[derive(Error, Debug)]
pub enum LogError {
    #[error("Invalid log level: {0}")]
    InvalidLogLevel(String),

    #[error("IO error: {0}")]
    IoError(#[from] io::Error),

    #[error("Configuration error: {0}")]
    ConfigError(String),
}

/// Output format for logs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogFormat {
    /// Pretty-printed logs with colors and human-readable formatting
    #[default]
    Pretty,

    /// `tracing-subscriber`'s own default single-line format.
    ///
    /// Added 2026-09-18 when the server was wired to this crate. The server
    /// had been building a bare `fmt::layer()`, which is this format and is
    /// **not** `Pretty` — `Pretty` is the multi-line `.pretty()` renderer.
    /// Without this variant, "wire the server up" would have silently changed
    /// the shape of every line the QA harness parses out of the server log,
    /// which is a behaviour change dressed as a refactor.
    Full,

    /// Compact single-line format
    Compact,

    /// JSON format for machine-readable logs
    Json,
}

impl LogFormat {
    /// Parse a format string into a LogFormat
    pub fn parse(s: &str) -> Result<Self, LogError> {
        match s.to_lowercase().as_str() {
            "pretty" => Ok(LogFormat::Pretty),
            "full" => Ok(LogFormat::Full),
            "compact" => Ok(LogFormat::Compact),
            "json" => Ok(LogFormat::Json),
            _ => Err(LogError::InvalidLogLevel(format!(
                "Unknown format: {}. Expected one of: pretty, full, compact, json",
                s
            ))),
        }
    }
}

/// Configuration for logging
#[derive(Debug, Clone)]
pub struct LogConfig {
    /// Output format for logs
    pub format: LogFormat,

    /// Log level filter (e.g., "info", "debug", "trace")
    /// If None, will be determined from RUST_LOG environment variable
    pub level: Option<String>,

    /// Whether to use colored output (only for Pretty format)
    pub use_color: bool,

    /// Whether to include timestamps in output
    pub use_timestamps: bool,

    /// Whether to include thread IDs in output
    pub include_thread_ids: bool,

    /// Whether to include target module names
    pub include_targets: bool,

    /// Output destination (stderr by default)
    pub output: LogOutput,
}

/// Log output destination
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogOutput {
    /// Write to standard error
    Stderr,

    /// Write to standard output
    Stdout,
}

impl Default for LogConfig {
    fn default() -> Self {
        LogConfig {
            format: LogFormat::Pretty,
            level: None,
            use_color: true,
            use_timestamps: true,
            include_thread_ids: false,
            include_targets: true,
            output: LogOutput::Stderr,
        }
    }
}

impl LogConfig {
    /// Create a new default configuration
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the output format
    pub fn with_format(mut self, format: LogFormat) -> Self {
        self.format = format;
        self
    }

    /// Set the log level
    pub fn with_level(mut self, level: impl Into<String>) -> Self {
        self.level = Some(level.into());
        self
    }

    /// Enable or disable color output
    pub fn with_color(mut self, use_color: bool) -> Self {
        self.use_color = use_color;
        self
    }

    /// Enable or disable timestamps
    pub fn with_timestamps(mut self, use_timestamps: bool) -> Self {
        self.use_timestamps = use_timestamps;
        self
    }

    /// Enable or disable thread IDs
    pub fn with_thread_ids(mut self, include_thread_ids: bool) -> Self {
        self.include_thread_ids = include_thread_ids;
        self
    }

    /// Enable or disable target module names
    pub fn with_targets(mut self, include_targets: bool) -> Self {
        self.include_targets = include_targets;
        self
    }

    /// Set the output destination
    pub fn with_output(mut self, output: LogOutput) -> Self {
        self.output = output;
        self
    }

    /// Get the effective log level from config or environment
    pub fn get_effective_level(&self) -> String {
        self.level
            .clone()
            .or_else(|| std::env::var("RUST_LOG").ok())
            .unwrap_or_else(|| "info".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_format_parsing() {
        assert_eq!(LogFormat::parse("pretty").unwrap(), LogFormat::Pretty);
        assert_eq!(LogFormat::parse("full").unwrap(), LogFormat::Full);
        assert_eq!(LogFormat::parse("compact").unwrap(), LogFormat::Compact);
        assert_eq!(LogFormat::parse("json").unwrap(), LogFormat::Json);
        assert!(LogFormat::parse("invalid").is_err());
    }

    /// `Full` and `Pretty` are different renderers and must stay distinct.
    /// Collapsing them would silently reformat every `mediagit-server` log
    /// line, which is what `Full` was added to prevent.
    #[test]
    fn full_and_pretty_are_not_the_same_format() {
        assert_ne!(LogFormat::Full, LogFormat::Pretty);
        assert_ne!(LogFormat::parse("full").unwrap(), LogFormat::Pretty);
    }

    /// The error has to name the accepted set, because the caller is usually
    /// someone who typed `jsn` into a config file.
    #[test]
    fn an_unknown_format_error_lists_what_is_accepted() {
        let msg = LogFormat::parse("jsn").unwrap_err().to_string();
        for expected in ["pretty", "full", "compact", "json"] {
            assert!(msg.contains(expected), "{expected} missing from: {msg}");
        }
    }

    #[test]
    fn test_log_format_case_insensitive() {
        assert_eq!(LogFormat::parse("PRETTY").unwrap(), LogFormat::Pretty);
        assert_eq!(LogFormat::parse("JSON").unwrap(), LogFormat::Json);
    }

    #[test]
    fn test_log_config_builder() {
        let config = LogConfig::new()
            .with_format(LogFormat::Json)
            .with_level("debug")
            .with_color(false)
            .with_timestamps(false);

        assert_eq!(config.format, LogFormat::Json);
        assert_eq!(config.level, Some("debug".to_string()));
        assert!(!config.use_color);
        assert!(!config.use_timestamps);
    }

    #[test]
    fn test_effective_level_from_config() {
        let config = LogConfig::new().with_level("debug");
        assert_eq!(config.get_effective_level(), "debug");
    }

    #[test]
    fn test_log_output_default() {
        let config = LogConfig::default();
        assert_eq!(config.output, LogOutput::Stderr);
    }
}

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    /// Pretty-printed logs with colors and human-readable formatting
    Pretty,

    /// Compact single-line format
    Compact,

    /// JSON format for machine-readable logs
    Json,
}

impl Default for LogFormat {
    fn default() -> Self {
        LogFormat::Pretty
    }
}

impl LogFormat {
    /// Parse a format string into a LogFormat
    pub fn from_str(s: &str) -> Result<Self, LogError> {
        match s.to_lowercase().as_str() {
            "pretty" => Ok(LogFormat::Pretty),
            "compact" => Ok(LogFormat::Compact),
            "json" => Ok(LogFormat::Json),
            _ => Err(LogError::InvalidLogLevel(format!(
                "Unknown format: {}. Expected one of: pretty, compact, json",
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
        assert_eq!(LogFormat::from_str("pretty").unwrap(), LogFormat::Pretty);
        assert_eq!(LogFormat::from_str("compact").unwrap(), LogFormat::Compact);
        assert_eq!(LogFormat::from_str("json").unwrap(), LogFormat::Json);
        assert!(LogFormat::from_str("invalid").is_err());
    }

    #[test]
    fn test_log_format_case_insensitive() {
        assert_eq!(LogFormat::from_str("PRETTY").unwrap(), LogFormat::Pretty);
        assert_eq!(LogFormat::from_str("JSON").unwrap(), LogFormat::Json);
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

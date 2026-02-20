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
//! Logging initialization and setup.
//!
//! This module provides functions to initialize the tracing system with
//! different configurations and output formats.

use crate::config::{LogConfig, LogError, LogFormat, LogOutput};
use std::io;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter, Registry};

/// Initialize tracing with the specified format and optional log level.
///
/// This is a convenience function that uses default configuration except
/// for the format and log level.
///
/// # Arguments
///
/// * `format` - The output format for logs
/// * `level` - Optional log level (e.g., "info", "debug"). If None, uses RUST_LOG env var
///
/// # Example
///
/// ```ignore
/// use mediagit_observability::{init_tracing, LogFormat};
///
/// #[tokio::main]
/// async fn main() {
///     init_tracing(LogFormat::Pretty, Some("debug")).unwrap();
///     tracing::info!("Application started");
/// }
/// ```
pub fn init_tracing(format: LogFormat, level: Option<&str>) -> Result<(), LogError> {
    let config = LogConfig::new()
        .with_format(format)
        .with_level(level.unwrap_or("info"));
    init_tracing_with_config(config)
}

/// Initialize tracing with a detailed configuration.
///
/// # Arguments
///
/// * `config` - The logging configuration
///
/// # Example
///
/// ```ignore
/// use mediagit_observability::{init_tracing_with_config, LogConfig, LogFormat};
///
/// #[tokio::main]
/// async fn main() {
///     let config = LogConfig::new()
///         .with_format(LogFormat::Json)
///         .with_level("debug")
///         .with_timestamps(true);
///
///     init_tracing_with_config(config).unwrap();
///     tracing::info!("Application started");
/// }
/// ```
pub fn init_tracing_with_config(config: LogConfig) -> Result<(), LogError> {
    let env_filter = build_env_filter(&config)?;
    let registry = Registry::default().with(env_filter);

    match config.format {
        LogFormat::Pretty => {
            let layer = fmt::layer()
                .with_writer(get_writer(&config.output))
                .with_target(config.include_targets)
                .with_thread_ids(config.include_thread_ids)
                .with_thread_names(true)
                .with_span_events(FmtSpan::ACTIVE)
                .pretty();

            if config.use_timestamps && config.use_color {
                registry.with(layer.with_timer(fmt::time::SystemTime).with_ansi(true)).init();
            } else if config.use_timestamps {
                registry.with(layer.with_timer(fmt::time::SystemTime).with_ansi(false)).init();
            } else if config.use_color {
                registry.with(layer.without_time().with_ansi(true)).init();
            } else {
                registry.with(layer.without_time().with_ansi(false)).init();
            }
        }
        LogFormat::Compact => {
            let layer = fmt::layer()
                .with_writer(get_writer(&config.output))
                .with_target(config.include_targets)
                .with_thread_ids(config.include_thread_ids)
                .with_thread_names(false)
                .with_span_events(FmtSpan::CLOSE)
                .compact();

            if config.use_timestamps && config.use_color {
                registry.with(layer.with_timer(fmt::time::SystemTime).with_ansi(true)).init();
            } else if config.use_timestamps {
                registry.with(layer.with_timer(fmt::time::SystemTime).with_ansi(false)).init();
            } else if config.use_color {
                registry.with(layer.without_time().with_ansi(true)).init();
            } else {
                registry.with(layer.without_time().with_ansi(false)).init();
            }
        }
        LogFormat::Json => {
            let layer = fmt::layer()
                .with_writer(get_writer(&config.output))
                .json()
                .with_target(config.include_targets)
                .with_thread_ids(config.include_thread_ids)
                .with_thread_names(true)
                .with_span_events(FmtSpan::FULL);

            if config.use_timestamps {
                registry.with(layer.with_timer(fmt::time::SystemTime)).init();
            } else {
                registry.with(layer.without_time()).init();
            }
        }
    }

    Ok(())
}

/// Get the writer for the specified output
fn get_writer(output: &LogOutput) -> fn() -> Box<dyn io::Write + Send> {
    match output {
        LogOutput::Stderr => || Box::new(io::stderr()),
        LogOutput::Stdout => || Box::new(io::stdout()),
    }
}

/// Build an environment filter for the given configuration
fn build_env_filter(config: &LogConfig) -> Result<EnvFilter, LogError> {
    let level_str = config.get_effective_level();

    EnvFilter::try_new(&level_str).map_err(|e| {
        LogError::ConfigError(format!("Failed to parse log filter '{}': {}", level_str, e))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: Tests that initialize the global subscriber are not included here
    // because once a global default is set, it cannot be changed in tests.
    // See integration_tests.rs for those tests.

    #[test]
    fn test_env_filter_parsing() {
        // Test just the filter parsing logic
        let result = build_env_filter(&LogConfig::new().with_level("debug"));
        assert!(result.is_ok());
    }

    #[test]
    fn test_valid_trace_level() {
        let result = build_env_filter(&LogConfig::new().with_level("trace"));
        assert!(result.is_ok());
    }
}

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
//! Integration tests for logging system
//!
//! Tests the logging system with different configurations and output formats.
//!
//! NOTE: We test configuration building but not global subscriber initialization
//! because the global subscriber can only be set once per process lifetime.

use mediagit_observability::{LogConfig, LogFormat, LogOutput};

// Configuration building tests - these don't require global initialization

#[test]
fn test_config_builder_chaining() {
    let config = LogConfig::new()
        .with_format(LogFormat::Json)
        .with_level("debug")
        .with_timestamps(false)
        .with_color(false)
        .with_thread_ids(true)
        .with_targets(false)
        .with_output(LogOutput::Stdout);

    assert_eq!(config.format, LogFormat::Json);
    assert_eq!(config.level, Some("debug".to_string()));
    assert!(!config.use_timestamps);
    assert!(!config.use_color);
    assert!(config.include_thread_ids);
    assert!(!config.include_targets);
    assert_eq!(config.output, LogOutput::Stdout);
}

#[test]
fn test_default_config() {
    let config = LogConfig::default();
    assert_eq!(config.format, LogFormat::Pretty);
    assert_eq!(config.output, LogOutput::Stderr);
    assert!(config.use_color);
    assert!(config.use_timestamps);
}

#[test]
fn test_environment_variable_fallback() {
    std::env::set_var("RUST_LOG", "trace");
    let config = LogConfig::new().with_format(LogFormat::Compact);
    assert_eq!(config.get_effective_level(), "trace");
}

#[test]
fn test_explicit_level_overrides_env() {
    std::env::set_var("RUST_LOG", "trace");
    let config = LogConfig::new()
        .with_format(LogFormat::Compact)
        .with_level("warn");
    assert_eq!(config.get_effective_level(), "warn");
}

// Note: Tests that initialize the global subscriber are not included here
// because the global default subscriber can only be set once per process.
// The examples/ directory contains working examples of logging initialization.

// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

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
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("RUST_LOG", "trace") };
    let config = LogConfig::new().with_format(LogFormat::Compact);
    assert_eq!(config.get_effective_level(), "trace");
}

#[test]
fn test_explicit_level_overrides_env() {
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("RUST_LOG", "trace") };
    let config = LogConfig::new()
        .with_format(LogFormat::Compact)
        .with_level("warn");
    assert_eq!(config.get_effective_level(), "warn");
}

// Note: Tests that initialize the global subscriber are not included here
// because the global default subscriber can only be set once per process.
// The examples/ directory contains working examples of logging initialization.

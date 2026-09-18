// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Integration tests for logging system
//!
//! Tests the logging system with different configurations and output formats.
//!
//! NOTE: We test configuration building but not global subscriber
//! initialization, because the global subscriber can only be set once per
//! process lifetime.
//!
//! **That note is also this file's blind spot, and it cost something.** Every
//! test below asserts the shape of a `LogConfig`; none produces a log line. So
//! when it turned out (2026-09-18) that neither shipping binary could select
//! `LogFormat::Json` — the CLI hardcoded `Pretty`, the server did not depend on
//! this crate at all — every test here stayed green throughout. A test that a
//! variant EXISTS cannot fail when nothing can reach it.
//!
//! The end-to-end check lives where it can actually run: the real binary is
//! executed and its output parsed, in
//! `mediagit-cli/tests/cli_log_output_format_test.rs`.

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

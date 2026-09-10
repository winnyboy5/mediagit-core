// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

#![allow(missing_docs)] // internal test helper crate — not part of public API

//! # MediaGit Test Utilities
//!
//! Shared test utilities for MediaGit crates providing:
//! - CLI command helpers for testing mediagit commands
//! - Repository setup and management for integration tests
//! - Cross-platform path utilities
//! - Test fixtures and data management
//! - Custom assertions for common test patterns

pub mod assertions;
pub mod cli;
pub mod fixtures;
pub mod platform;
pub mod repo;

// Re-export commonly used items at crate root
pub use assertions::*;
pub use cli::{MediagitCommand, mediagit};
pub use fixtures::TestFixtures;
pub use platform::TestPaths;
pub use repo::TestRepo;

/// In-memory [`StorageBackend`](mediagit_storage::StorageBackend) for unit
/// tests that need a storage backend without touching disk or network.
/// Re-exported from `mediagit-storage` rather than reimplemented here.
pub use mediagit_storage::mock::MockBackend as MockStorage;

/// Test-only safe wrappers around edition-2024's now-`unsafe` env mutators.
/// The `unsafe` is contained here so `forbid(unsafe_code)` crates can set env
/// in tests. Callers MUST still serialize env access (process-global); this
/// only relocates the unsafe, it does not make concurrent env mutation sound.
#[allow(unsafe_code)]
pub fn set_var<K: AsRef<std::ffi::OsStr>, V: AsRef<std::ffi::OsStr>>(key: K, val: V) {
    unsafe { std::env::set_var(key, val) }
}
#[allow(unsafe_code)]
pub fn remove_var<K: AsRef<std::ffi::OsStr>>(key: K) {
    unsafe { std::env::remove_var(key) }
}

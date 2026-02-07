// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2025 MediaGit Contributors

//! # MediaGit Test Utilities
//!
//! Shared test utilities for MediaGit crates providing:
//! - CLI command helpers for testing mediagit commands
//! - Repository setup and management for integration tests
//! - Cross-platform path utilities
//! - Test fixtures and data management
//! - Custom assertions for common test patterns

pub mod cli;
pub mod repo;
pub mod platform;
pub mod fixtures;
pub mod assertions;

// Re-export commonly used items at crate root
pub use cli::{mediagit, MediagitCommand};
pub use repo::TestRepo;
pub use platform::TestPaths;
pub use fixtures::TestFixtures;
pub use assertions::*;

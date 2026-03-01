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
pub use cli::{mediagit, MediagitCommand};
pub use fixtures::TestFixtures;
pub use platform::TestPaths;
pub use repo::TestRepo;

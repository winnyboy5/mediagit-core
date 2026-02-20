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

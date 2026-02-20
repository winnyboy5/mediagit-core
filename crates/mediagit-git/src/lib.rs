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
// SPDX-License-Identifier: AGPL-3.0
// Copyright (C) 2025 MediaGit Contributors

//! # MediaGit Git Integration Layer
//!
//! This crate provides Git integration capabilities for MediaGit, implementing
//! Git filter drivers (clean/smudge) to enable seamless integration with
//! standard Git workflows.
//!
//! ## Architecture
//!
//! The Git integration layer consists of:
//!
//! - **Pointer Files**: Lightweight text files stored in Git that reference
//!   actual media content stored in MediaGit's object database
//! - **Clean Filter**: Converts media files to pointer files when staging
//! - **Smudge Filter**: Restores pointer files to actual media files when checking out
//! - **Filter Driver**: Git filter driver registration and configuration
//!
//! ## Pointer File Format
//!
//! MediaGit pointer files follow a simple text format:
//!
//! ```text
//! version https://mediagit.dev/spec/v1
//! oid sha256:4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393
//! size 12345
//! ```
//!
//! ## Usage
//!
//! ```rust,no_run
//! use mediagit_git::{PointerFile, FilterDriver};
//!
//! // Parse a pointer file
//! let content = "version https://mediagit.dev/spec/v1\noid sha256:abc123...\nsize 12345\n";
//! let pointer = PointerFile::parse(content)?;
//!
//! // Generate a pointer file
//! let pointer = PointerFile::new("abc123...".to_string(), 12345);
//! let content = pointer.to_string();
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod error;
pub mod filter;
pub mod pointer;

pub use error::{GitError, GitResult};
pub use filter::{FilterDriver, FilterConfig};
pub use pointer::PointerFile;

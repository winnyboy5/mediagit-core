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
//! MediaGit Observability Module
//!
//! Provides structured logging and tracing capabilities for MediaGit-Core.
//!
//! # Features
//!
//! - **Multiple Output Formats**: Pretty, JSON, and compact output formats
//! - **Environment-based Filtering**: Dynamic log level control via `RUST_LOG`
//! - **Async Context Propagation**: Proper span context in async/tokio runtime
//! - **Structured Logging**: JSON output for machine-readable logs
//!
//! # Example
//!
//! ```ignore
//! use mediagit_observability::{init_tracing, LogFormat};
//!
//! #[tokio::main]
//! async fn main() {
//!     // Initialize with default format (pretty)
//!     init_tracing(LogFormat::Pretty, None)?;
//!
//!     // Use tracing macros for logging
//!     tracing::info!("Application started");
//! }
//! ```

pub mod config;
pub mod initialization;
pub mod macros;

pub use config::{LogFormat, LogConfig, LogOutput};
pub use initialization::{init_tracing, init_tracing_with_config};

/// Tracing re-exports for convenience
pub use tracing::{debug, error, info, warn, trace, span, Level};

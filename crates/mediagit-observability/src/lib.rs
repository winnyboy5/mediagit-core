// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

#![allow(missing_docs)]
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

pub use config::{LogConfig, LogFormat, LogOutput};
pub use initialization::{init_tracing, init_tracing_with_config};

/// Tracing re-exports for convenience
pub use tracing::{Level, debug, error, info, span, trace, warn};

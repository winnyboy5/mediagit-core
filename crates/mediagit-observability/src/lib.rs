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

// `macros.rs` was here: SIX macros -- `log_info!`, `log_debug!`, `log_warn!`,
// `log_error!`, `trace_span!` and `instrument_async!`. Zero callers anywhere in
// the workspace, and each was an alias of the `tracing::` macro of the same
// name -- including the "structured fields" arm, which expanded to the field
// syntax `tracing` already provides natively. The crate's own
// `examples/async_tracing.rs` reaches past them to `tracing::trace_span!`
// directly, which is the whole argument in one line.
// Deleted 2026-09-18 rather than kept as a wrapper nobody used over a macro
// everybody used directly.

pub use config::{LogConfig, LogFormat, LogOutput};
pub use initialization::{init_tracing, init_tracing_with_config};

/// Tracing re-exports for convenience
pub use tracing::{Level, debug, error, info, span, trace, warn};

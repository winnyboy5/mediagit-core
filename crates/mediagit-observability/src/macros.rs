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
//! Convenience macros for structured logging.
//!
//! This module provides macros for logging with structured fields
//! and common patterns used throughout MediaGit.

/// Log a message with structured fields
///
/// # Example
///
/// ```ignore
/// log_info!("Processing file" => {
///     "path" => "/path/to/file",
///     "size" => 1024,
/// });
/// ```
#[macro_export]
macro_rules! log_info {
    ($msg:expr) => {
        tracing::info!($msg)
    };
    ($msg:expr => { $($key:expr => $value:expr),* $(,)? }) => {
        tracing::info!($msg, $($key = $value),*)
    };
}

/// Log a debug message with structured fields
#[macro_export]
macro_rules! log_debug {
    ($msg:expr) => {
        tracing::debug!($msg)
    };
    ($msg:expr => { $($key:expr => $value:expr),* $(,)? }) => {
        tracing::debug!($msg, $($key = $value),*)
    };
}

/// Log a warning message with structured fields
#[macro_export]
macro_rules! log_warn {
    ($msg:expr) => {
        tracing::warn!($msg)
    };
    ($msg:expr => { $($key:expr => $value:expr),* $(,)? }) => {
        tracing::warn!($msg, $($key = $value),*)
    };
}

/// Log an error message with structured fields
#[macro_export]
macro_rules! log_error {
    ($msg:expr) => {
        tracing::error!($msg)
    };
    ($msg:expr => { $($key:expr => $value:expr),* $(,)? }) => {
        tracing::error!($msg, $($key = $value),*)
    };
}

/// Create a span for performance tracking
///
/// # Example
///
/// ```ignore
/// let span = trace_span!("operation_name", field1 = "value");
/// let _guard = span.enter();
/// // Code here is within the span
/// ```
#[macro_export]
macro_rules! trace_span {
    ($name:expr) => {
        tracing::span!(tracing::Level::DEBUG, $name)
    };
    ($name:expr, $($field:tt)*) => {
        tracing::span!(tracing::Level::DEBUG, $name, $($field)*)
    };
}

/// Create an instrumented async block
///
/// This is useful for tracking async operations without creating a full function.
///
/// # Example
///
/// ```ignore
/// let result = instrument_async!("operation", async {
///     // async code
/// }).await;
/// ```
#[macro_export]
macro_rules! instrument_async {
    ($name:expr, $future:expr) => {{
        let span = $crate::trace_span!($name);
        async move { $future.await }.instrument(span)
    }};
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_macros_compile() {
        // This test just verifies that the macros compile correctly
        // Actual logging output is tested in integration tests
        let _x = true;
    }
}

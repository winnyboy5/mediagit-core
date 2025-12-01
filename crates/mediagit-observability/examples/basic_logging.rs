//! Basic logging example demonstrating different output formats.
//!
//! Run with: RUST_LOG=debug cargo run --example basic_logging -- <format>
//! Where <format> is one of: pretty, compact, json

use mediagit_observability::{init_tracing, LogFormat};
use std::env;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = env::args().collect();
    let format_str = args.get(1).map(|s| s.as_str()).unwrap_or("pretty");

    let format = LogFormat::from_str(format_str).unwrap_or(LogFormat::Pretty);

    println!("Initializing with format: {:?}", format);
    init_tracing(format, Some("debug"))?;

    tracing::info!("Application started");

    // Simulate some operations with different log levels
    tracing::debug!("This is a debug message");
    tracing::info!("This is an info message");
    tracing::warn!("This is a warning message");

    // Structured logging with fields
    tracing::info!(
        request_id = "abc123",
        duration_ms = 42,
        "Processing request"
    );

    // Async operation with span
    let result = process_file("/path/to/file").await;
    tracing::info!("File processing result: {:?}", result);

    tracing::debug!("Application shutting down");

    Ok(())
}

async fn process_file(path: &str) -> anyhow::Result<String> {
    let span = tracing::debug_span!("process_file", ?path);
    let _guard = span.enter();

    tracing::debug!("Starting file processing");

    // Simulate some work
    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

    let result = format!("Processed: {}", path);
    tracing::debug!(result = %result, "File processing complete");

    Ok(result)
}

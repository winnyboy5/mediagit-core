//! Async context propagation example
//!
//! Demonstrates how tracing context is automatically propagated
//! across async tasks in a tokio runtime.
//!
//! Run with: RUST_LOG=debug cargo run --example async_tracing

use mediagit_observability::{init_tracing, LogFormat};
use std::time::Duration;
use tracing::Instrument;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing(LogFormat::Pretty, Some("debug"))?;

    tracing::info!("Starting async tracing example");

    // Create a root span for the entire operation
    let root_span = tracing::info_span!("main_operation", request_id = "req-001");

    root_span.in_scope(|| {
        tracing::info!("Inside root span");
    });

    // Spawn multiple async tasks with automatic span inheritance
    let handle1 = tokio::spawn(
        async {
            process_batch("batch-1", 3).await
        }
        .instrument(tracing::info_span!("batch_processor", batch_id = "batch-1")),
    );

    let handle2 = tokio::spawn(
        async {
            process_batch("batch-2", 2).await
        }
        .instrument(tracing::info_span!("batch_processor", batch_id = "batch-2")),
    );

    // Wait for both to complete
    let _ = tokio::join!(handle1, handle2);

    tracing::info!("All operations complete");

    Ok(())
}

async fn process_batch(batch_name: &str, count: usize) {
    let span = tracing::debug_span!("process_items", batch_name, count);
    let _guard = span.enter();

    for i in 0..count {
        tracing::debug!(item_index = i, "Processing item");
        process_item(i).await;
    }

    tracing::info!("Batch processing complete");
}

async fn process_item(index: usize) {
    let span = tracing::trace_span!("process_item", index);
    let _guard = span.enter();

    tracing::trace!("Starting item processing");

    // Simulate async work
    tokio::time::sleep(Duration::from_millis(10)).await;

    tracing::trace!(status = "completed", "Item processing finished");
}

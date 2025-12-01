# MediaGit Observability

Structured logging and tracing capabilities for MediaGit-Core.

## Features

- **Multiple Output Formats**: Pretty-printed, compact, and JSON formats
- **Environment-based Filtering**: Dynamic log level control via `RUST_LOG` environment variable
- **Async Context Propagation**: Proper span context propagation in async/tokio runtime
- **Structured Logging**: JSON output for machine-readable logs with rich context
- **Zero-copy Spans**: Efficient tracing with minimal overhead

## Quick Start

### Basic Usage

```rust
use mediagit_observability::{init_tracing, LogFormat};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize with default settings
    init_tracing(LogFormat::Pretty, Some("debug"))?;

    tracing::info!("Application started");
    Ok(())
}
```

### With Configuration

```rust
use mediagit_observability::{init_tracing_with_config, LogConfig, LogFormat};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = LogConfig::new()
        .with_format(LogFormat::Json)
        .with_level("debug")
        .with_timestamps(true)
        .with_color(false);

    init_tracing_with_config(config)?;

    tracing::info!("Application started");
    Ok(())
}
```

## Output Formats

### Pretty Format
Human-readable output with colors and formatting:
```
2025-11-08T18:06:05.169610Z  INFO integration_tests: Task started task_id=1
  at crates/mediagit-observability/examples/async_tracing.rs:45
```

### Compact Format
Single-line, concise output:
```
2025-11-08T18:06:05.169610Z INFO mediagit_observability: Processing request request_id=abc123 duration_ms=42
```

### JSON Format
Machine-readable JSON output for log aggregation systems:
```json
{
  "timestamp": "2025-11-08T18:06:05.169610Z",
  "level": "INFO",
  "message": "Processing request",
  "request_id": "abc123",
  "duration_ms": 42
}
```

## Log Level Control

Set log levels via environment variable:

```bash
# Enable debug logging
RUST_LOG=debug cargo run

# Enable trace logging for specific modules
RUST_LOG=mediagit=trace,tokio=debug cargo run

# JSON output with debug level
RUST_LOG=debug cargo run -- --format json
```

## Structured Logging

Add context to logs with structured fields:

```rust
tracing::info!(
    request_id = "abc123",
    duration_ms = 42,
    user_id = 100,
    "Processing request"
);
```

## Async Context Propagation

Spans are automatically inherited by spawned tasks:

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing(LogFormat::Pretty, Some("debug"))?;

    let span = tracing::info_span!("operation", request_id = "req-001");

    tokio::spawn(
        async {
            // This log will include request_id from parent span
            tracing::info!("Processing in background");
        }
        .instrument(span)
    );

    Ok(())
}
```

## Configuration Options

### LogConfig Builder

```rust
let config = LogConfig::new()
    .with_format(LogFormat::Pretty)      // Output format
    .with_level("debug")                 // Log level
    .with_timestamps(true)               // Include timestamps
    .with_color(true)                    // Use ANSI colors
    .with_thread_ids(false)              // Include thread IDs
    .with_targets(true)                  // Include module targets
    .with_output(LogOutput::Stderr);     // Output destination
```

## Examples

The `examples/` directory contains:

- `basic_logging.rs` - Basic logging with different formats
- `async_tracing.rs` - Async context propagation and concurrent logging

Run examples:

```bash
# Pretty format
cargo run --example basic_logging -- pretty

# Compact format
RUST_LOG=debug cargo run --example basic_logging -- compact

# JSON format with trace logging
RUST_LOG=trace cargo run --example basic_logging -- json

# Async context example
RUST_LOG=debug cargo run --example async_tracing
```

## Integration with MediaGit

The CLI is already configured to use this logging system:

```rust
// In main.rs
let config = LogConfig::new()
    .with_format(format)
    .with_level(level)
    .with_color(use_colors);

init_tracing_with_config(config)?;
```

## Performance Considerations

- Lazy evaluation: logs at disabled levels have minimal overhead
- JSON serialization: offloaded to async background thread
- Span context: zero-copy propagation in async/await
- No heap allocations: for disabled log levels

## Common Patterns

### Function-level Instrumentation

```rust
async fn process_file(path: &str) -> Result<String> {
    let span = tracing::debug_span!("process_file", ?path);
    let _guard = span.enter();

    tracing::debug!("Starting processing");
    // ... do work ...
    tracing::debug!("Processing complete");

    Ok(result)
}
```

### Performance Tracking

```rust
let start = std::time::Instant::now();
// ... do work ...
let duration = start.elapsed().as_secs_f64();

tracing::info!(
    duration_secs = duration,
    "Operation completed"
);
```

### Error Context

```rust
match risky_operation().await {
    Ok(result) => {
        tracing::info!("Operation succeeded");
        Ok(result)
    }
    Err(e) => {
        tracing::error!(error = %e, "Operation failed");
        Err(e)
    }
}
```

## Testing

Run tests:

```bash
# Unit tests
cargo test -p mediagit-observability --lib

# Integration tests
cargo test -p mediagit-observability --test integration_tests

# All tests with logging
RUST_LOG=debug cargo test -p mediagit-observability -- --nocapture
```

## Troubleshooting

### Global Subscriber Already Set

```
thread panicked: failed to set global default subscriber:
SetGlobalDefaultError("a global default trace dispatcher has already been set")
```

This occurs when `init_tracing` or `init_tracing_with_config` is called more than once.
The global subscriber can only be initialized once per process lifetime.

**Solution**: Initialize once at application startup, before spawning any tasks.

### No Log Output

Ensure `RUST_LOG` environment variable is set:

```bash
RUST_LOG=info cargo run
```

### JSON Output is Malformed

Ensure JSON format is selected and valid filter is set:

```rust
let config = LogConfig::new()
    .with_format(LogFormat::Json)
    .with_level("info");
```

## Dependencies

- `tracing 0.1.41`: Core tracing framework
- `tracing-subscriber 0.3.19`: Tracing subscriber with fmt, json, env-filter support
- `tokio 1.48`: Async runtime (for context propagation)

## License

AGPL-3.0

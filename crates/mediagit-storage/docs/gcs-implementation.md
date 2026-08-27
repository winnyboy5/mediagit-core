# Google Cloud Storage Backend Implementation

## Overview

The GCS backend (`gcs::GcsBackend`) implements the `StorageBackend` trait for Google Cloud Storage, providing:

- **Async-first operations** using tokio
- **Resumable uploads** for large files (>5MB)
- **Service account authentication** from JSON files
- **Automatic retry logic** with exponential backoff
- **Efficient prefix-based listing**
- **Thread-safe concurrent access** (Send + Sync)

## Architecture

### Core Components

#### GcsConfig
Configuration object for customizing backend behavior:

```rust
pub struct GcsConfig {
    pub project_id: String,
    pub bucket_name: String,
    pub chunk_size: usize,              // Default: 256KB
    pub resumable_threshold: usize,     // Default: 5MB
    pub max_retries: u32,               // Default: 3
}
```

#### GcsBackend
The main backend implementation:

```rust
pub struct GcsBackend {
    config: GcsConfig,
    // In production: authenticated GCS client
}
```

### Resumable Upload Protocol

For files larger than the configured threshold (default 5MB), the backend uses the GCS resumable upload protocol:

#### Upload Phases

1. **Initiation**: Create a resumable session
   - Client sends: Initial request with file metadata
   - GCS responds: Session URI for resumable upload

2. **Uploading**: Send file in chunks
   - Split file into configured chunk size (default 256KB)
   - Upload each chunk with offset information
   - GCS responds: Offset of next expected byte after success

3. **Completion**: Finalize upload
   - Final chunk signals completion
   - GCS responds: Full object metadata

#### Retry Strategy

For transient failures (network errors, 5xx responses):
- Retry with exponential backoff
- Configurable max retries (default 3)
- Log failures for observability

#### Chunk Size Tuning

Configured via `GcsConfig::with_chunk_size()`:

- **Smaller chunks** (e.g., 64KB): Lower memory, more requests
- **Larger chunks** (e.g., 512KB+): Higher memory, fewer requests
- **Default (256KB)**: Balance between memory and request overhead

### Error Handling

The implementation maps GCS errors to appropriate responses:

| GCS Error | Response | Notes |
|-----------|----------|-------|
| 404 Not Found | `Err("object not found: ...")` | For `get()` operations |
| 403 Forbidden | `Err("permission denied")` | Credentials or bucket access issue |
| 500+ Server Error | Retry with backoff | Transient, recoverable |
| Network timeout | Retry or fail | Transient for resumable uploads |

### Implementation Status

#### Completed

- ✅ Configuration management (GcsConfig)
- ✅ Backend initialization (new, with_config, from_env)
- ✅ Input validation (empty key checks)
- ✅ Debug and Clone implementations
- ✅ Thread safety (Send + Sync + Debug)
- ✅ Comprehensive documentation
- ✅ Integration tests for configuration
- ✅ Integration tests for error handling

#### TODO: Core Operations (Requires google-cloud-storage crate)

- ⏳ `get()` - Download object from bucket
- ⏳ `put()` - Upload object (simple or resumable)
- ⏳ `exists()` - Check object existence via metadata
- ⏳ `delete()` - Delete object (idempotent)
- ⏳ `list_objects()` - List objects with prefix and pagination

## Usage Examples

### Basic Setup

```rust
use mediagit_storage::{StorageBackend, GcsBackend};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize with service account JSON
    let storage = GcsBackend::new(
        "my-gcp-project",
        "my-storage-bucket",
        "/path/to/service-account.json"
    ).await?;

    // Use like any StorageBackend
    storage.put("documents/file.pdf", b"content").await?;
    let data = storage.get("documents/file.pdf").await?;

    Ok(())
}
```

### Custom Configuration

```rust
use mediagit_storage::gcs::{GcsBackend, GcsConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = GcsConfig::new("my-project", "my-bucket")
        .with_chunk_size(512 * 1024)        // 512KB chunks
        .with_resumable_threshold(10 * 1024 * 1024)  // 10MB threshold
        .with_max_retries(5);

    let storage = GcsBackend::with_config(config, "service-account.json").await?;

    Ok(())
}
```

### Environment Variable Setup

```bash
export GOOGLE_APPLICATION_CREDENTIALS="/path/to/service-account.json"
export GCS_PROJECT_ID="my-gcp-project"
export GCS_BUCKET_NAME="my-storage-bucket"
```

```rust
use mediagit_storage::GcsBackend;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let storage = GcsBackend::from_env().await?;
    Ok(())
}
```

## Testing

### Unit Tests

Located in `gcs.rs` module tests:
- Configuration creation and validation
- Builder pattern functionality
- Empty key rejection
- Send/Sync trait verification

### Integration Tests

Located in `tests/gcs_integration_tests.rs`:
- Backend initialization
- Error handling for invalid configurations
- Configuration reusability
- Debug output verification

### GCS Emulator Testing (Recommended)

For local testing without GCP credentials:

```bash
# Start GCS emulator
docker run -d -p 4443:4443 fsouza/fake-gcs-server

# Configure endpoint
export STORAGE_EMULATOR_HOST=http://localhost:4443
```

Then run integration tests with real-like scenarios.

## Performance Considerations

### Resumable Upload Benefits

1. **Reliability**: Can recover from network failures mid-upload
2. **Monitorability**: Track upload progress
3. **Efficiency**: Parallel chunk uploads possible
4. **Scalability**: Large files don't load entire contents in memory

### Chunk Size Tuning

For typical use cases:
- **Documents** (< 100MB): 256KB chunks (default)
- **Videos** (100MB - 5GB): 512KB - 1MB chunks
- **Very Large** (> 5GB): 2MB+ chunks with parallel uploads

### Connection Pooling

In production, the GCS client should:
- Reuse HTTP connections
- Implement connection pooling
- Configure timeouts for transient failures

## Security Considerations

### Service Account Authentication

1. **Key Storage**: Keep service account JSON file secure
   - Use environment variables instead of hardcoding
   - Restrict file permissions (0600)
   - Rotate keys regularly

2. **Least Privilege**:
   - Grant only necessary GCS permissions
   - Use custom roles if needed
   - Audit service account usage

3. **Bucket Access Control**:
   - Use uniform bucket-level access when possible
   - Configure Object Lifecycle policies
   - Enable bucket versioning for recovery

### Error Messages

- Avoid exposing sensitive information in error messages
- Log errors with context for debugging
- Use structured logging for security monitoring

## Future Enhancements

### Planned Features

1. **Parallel Uploads**: Split large files into multiple concurrent chunks
2. **Progress Tracking**: Expose upload progress via callbacks
3. **Compression**: Optional compression before upload
4. **Encryption**: Support for customer-managed encryption keys (CMEK)
5. **Signed URLs**: Generate temporary access URLs
6. **Batch Operations**: Bulk delete/copy operations

### Integration Opportunities

1. **Observability**: Prometheus metrics for operation latency/errors
2. **Caching**: Local cache layer for frequently accessed objects
3. **Versioning**: Support GCS object versioning
4. **Notifications**: Cloud Pub/Sub integration for change notifications

## Dependencies

### Required

- `tokio`: Async runtime
- `async-trait`: Async trait support
- `anyhow`: Error handling
- `google-cloud-storage`: GCS client library
- `google-cloud-default`: Default authentication

### Optional (Future)

- `bytes`: Efficient data handling
- `futures`: Advanced async patterns
- `tracing`: Distributed tracing
- `prometheus`: Metrics collection

## References

### Google Cloud Documentation

- [GCS Resumable Upload Protocol](https://cloud.google.com/storage/docs/json_api/v1/how-tos/resumable-upload)
- [GCS Authentication Methods](https://cloud.google.com/docs/authentication)
- [GCS Best Practices](https://cloud.google.com/storage/docs/best-practices)

### Rust Ecosystem

- [async-trait crate](https://docs.rs/async-trait/)
- [google-cloud-storage crate](https://docs.rs/google-cloud-storage/)
- [tokio async runtime](https://tokio.rs/)

## Troubleshooting

### Common Issues

1. **"Project not found"**:
   - Verify project ID in config
   - Check service account has required permissions
   - Ensure GCP billing is enabled

2. **"Bucket not found"**:
   - Verify bucket name is correct
   - Check bucket exists in correct region
   - Verify service account has bucket access

3. **"Permission denied"**:
   - Check service account roles
   - Verify bucket IAM policies
   - Check object ACLs

4. **Resumable upload failures**:
   - Verify chunk size < 5TB
   - Check network connectivity
   - Review GCS quota usage

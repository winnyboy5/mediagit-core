# MediaGit Storage Backend

Unified cloud storage abstraction layer for MediaGit, providing consistent APIs across multiple storage providers.

## Overview

MediaGit Storage provides a trait-based abstraction (`StorageBackend`) for working with different cloud storage providers through a unified interface. This enables seamless storage provider switching, multi-cloud deployments, and local development with emulators.

## Supported Backends

### Production Backends
- **AWS S3** - Industry-standard object storage with multipart upload support
- **Azure Blob Storage** - Microsoft Azure cloud storage with chunked uploads
- **Google Cloud Storage (GCS)** - Google Cloud object storage with resumable uploads
- **MinIO** - Self-hosted S3-compatible storage for on-premise deployments
- **Backblaze B2** - Cost-effective cloud storage with S3-compatible API
- **DigitalOcean Spaces** - S3-compatible object storage for app platforms

### Development Backends
- **Local Filesystem** - File-based storage for development and testing
- **In-Memory Mock** - Ephemeral storage for unit tests
- **Cache** - LRU caching layer for any backend

## Quick Start

### Add Dependency

```toml
[dependencies]
mediagit-storage = { path = "../mediagit-storage" }
tokio = { version = "1", features = ["full"] }
```

### Basic Usage

```rust
use mediagit_storage::{StorageBackend, S3Backend};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create S3 backend
    let storage = S3Backend::new("my-bucket").await?;

    // Store data
    storage.put("media/video.mp4", &video_data).await?;

    // Retrieve data
    let retrieved = storage.get("media/video.mp4").await?;

    // Check existence
    if storage.exists("media/video.mp4").await? {
        println!("Video exists!");
    }

    // List objects
    let objects = storage.list_objects("media/").await?;
    for key in objects {
        println!("Found: {}", key);
    }

    // Delete object
    storage.delete("media/video.mp4").await?;

    Ok(())
}
```

## Backend Configuration

### AWS S3

```rust
use mediagit_storage::S3Backend;

// From environment variables (AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY, AWS_REGION)
let s3 = S3Backend::from_env("my-bucket").await?;

// Explicit configuration
let s3 = S3Backend::new("my-bucket").await?;
```

### Azure Blob Storage

```rust
use mediagit_storage::AzureBackend;

// With connection string
let azure = AzureBackend::with_connection_string(
    "my-container",
    "DefaultEndpointsProtocol=https;AccountName=...;AccountKey=...;"
).await?;

// With account key
let azure = AzureBackend::with_account_key(
    "account-name",
    "my-container",
    "account-key"
).await?;

// With SAS token
let azure = AzureBackend::with_sas_token(
    "account-name",
    "my-container",
    "?sv=2021-06-08&ss=b&srt=sco&sp=..."
).await?;
```

### Google Cloud Storage

```rust
use mediagit_storage::GcsBackend;

// With service account JSON
let gcs = GcsBackend::new(
    "my-project",
    "my-bucket",
    "/path/to/service-account.json"
).await?;

// From environment (GCS_PROJECT_ID, GCS_BUCKET_NAME, GOOGLE_APPLICATION_CREDENTIALS)
let gcs = GcsBackend::from_env().await?;
```

### MinIO

```rust
use mediagit_storage::MinIOBackend;

// Self-hosted MinIO
let minio = MinIOBackend::new(
    "http://localhost:9000",
    "my-bucket",
    "minioadmin",
    "minioadmin"
).await?;

// From environment (MINIO_ENDPOINT, MINIO_BUCKET, MINIO_ACCESS_KEY, MINIO_SECRET_KEY)
let minio = MinIOBackend::from_env().await?;
```

### Backblaze B2 / DigitalOcean Spaces

```rust
use mediagit_storage::{B2SpacesBackend, Provider};

// Backblaze B2
let b2 = B2SpacesBackend::new_b2(
    "us-west-002",
    "my-bucket",
    "application-key-id",
    "application-key"
).await?;

// DigitalOcean Spaces
let spaces = B2SpacesBackend::new_spaces(
    "nyc3",
    "my-bucket",
    "access-key",
    "secret-key"
).await?;
```

### Local Development

```rust
use mediagit_storage::LocalBackend;

// Local filesystem storage
let local = LocalBackend::new("/tmp/mediagit-storage").await?;

// In-memory mock for testing
use mediagit_storage::MockBackend;
let mock = MockBackend::new();
```

## StorageBackend Trait

All backends implement the `StorageBackend` trait:

```rust
#[async_trait]
pub trait StorageBackend: Send + Sync + Debug {
    /// Retrieve an object by key
    async fn get(&self, key: &str) -> anyhow::Result<Vec<u8>>;

    /// Store an object with the given key
    async fn put(&self, key: &str, data: &[u8]) -> anyhow::Result<()>;

    /// Check if an object exists
    async fn exists(&self, key: &str) -> anyhow::Result<bool>;

    /// Delete an object (idempotent)
    async fn delete(&self, key: &str) -> anyhow::Result<()>;

    /// List objects with optional prefix
    async fn list_objects(&self, prefix: &str) -> anyhow::Result<Vec<String>>;
}
```

## Features

### Multipart & Chunked Uploads
- **S3**: Automatic multipart upload for files >100MB
- **Azure**: 4MB chunk uploads for large blobs
- **GCS**: 256KB resumable uploads for files >5MB

### Retry Logic
- Exponential backoff for transient failures
- Configurable retry attempts (default: 3)
- Automatic retry on network errors

### Concurrent Operations
- Thread-safe implementations (Send + Sync)
- Parallel uploads and downloads
- Efficient connection pooling

### Error Handling
- Comprehensive error types with context
- Detailed error messages for debugging
- Idempotent delete operations

## Testing

### Unit Tests (No Dependencies)

```bash
# Run all unit tests
cargo test

# Test specific backend
cargo test --lib s3::tests
cargo test --lib azure::tests
```

### Integration Tests (Requires Emulators)

```bash
# Start all emulators
docker-compose up -d

# Run all integration tests
cargo test --test '*' -- --ignored

# Test specific backend
cargo test --test s3_localstack_tests -- --ignored
cargo test --test azure_azurite_tests -- --ignored
cargo test --test gcs_emulator_tests -- --ignored
cargo test --test minio_docker_tests -- --ignored
```

**See [TESTING.md](TESTING.md) for complete testing guide.**

## Test Coverage

- **Unit Tests**: 62 test cases (configuration, validation, traits)
- **Integration Tests**: 89 test cases (CRUD, concurrent, edge cases)
- **Total**: 151 comprehensive tests

### Integration Test Breakdown
- S3 LocalStack: 21 tests
- Azure Azurite: 21 tests
- GCS Emulator: 24 tests
- MinIO Docker: 23 tests

## Performance

### Throughput Benchmarks (Emulators)
- LocalStack (S3): ~50-100 MB/s
- Azurite (Azure): ~80-150 MB/s
- GCS Emulator: ~30-60 MB/s
- MinIO: ~100-200 MB/s

### Optimization Features
- Connection pooling and reuse
- Automatic multipart uploads
- Configurable chunk sizes
- Parallel operations support

## Architecture

```
mediagit-storage/
├── src/
│   ├── lib.rs              # StorageBackend trait and exports
│   ├── s3.rs               # AWS S3 implementation (781 lines)
│   ├── azure.rs            # Azure Blob Storage (819 lines)
│   ├── gcs.rs              # Google Cloud Storage (894 lines)
│   ├── minio.rs            # MinIO S3-compatible (1,111 lines)
│   ├── b2_spaces.rs        # B2/Spaces unified (1,267 lines)
│   ├── local.rs            # Local filesystem backend
│   ├── mock.rs             # In-memory mock backend
│   ├── cache.rs            # LRU caching layer
│   └── error.rs            # Error types and handling
├── tests/
│   ├── s3_localstack_tests.rs      # S3 integration tests
│   ├── azure_azurite_tests.rs      # Azure integration tests
│   ├── gcs_emulator_tests.rs       # GCS integration tests
│   ├── minio_docker_tests.rs       # MinIO integration tests
│   └── gcs_integration_tests.rs    # GCS unit tests
├── docker-compose.yml      # Emulator orchestration
├── TESTING.md             # Complete testing guide
└── README.md              # This file
```

## Dependencies

### Core Dependencies
- `async-trait` - Async trait definitions
- `tokio` - Async runtime
- `anyhow` - Error handling

### Backend SDKs
- `aws-sdk-s3` - AWS S3 SDK
- `azure_storage_blobs` - Azure SDK
- `google-cloud-storage` - GCS SDK

### Optional Dependencies
- `serde` - Serialization (configuration)
- `tracing` - Logging and instrumentation

## Environment Variables

### AWS S3
- `AWS_ACCESS_KEY_ID` - AWS access key
- `AWS_SECRET_ACCESS_KEY` - AWS secret key
- `AWS_REGION` - AWS region (e.g., us-east-1)
- `AWS_SESSION_TOKEN` - Optional session token

### Azure Blob Storage
- `AZURE_STORAGE_ACCOUNT` - Storage account name
- `AZURE_STORAGE_KEY` - Account key
- `AZURE_STORAGE_CONNECTION_STRING` - Full connection string
- `AZURE_STORAGE_SAS_TOKEN` - SAS token

### Google Cloud Storage
- `GCS_PROJECT_ID` - GCP project ID
- `GCS_BUCKET_NAME` - Bucket name
- `GOOGLE_APPLICATION_CREDENTIALS` - Path to service account JSON

### MinIO
- `MINIO_ENDPOINT` - MinIO endpoint URL
- `MINIO_BUCKET` - Bucket name
- `MINIO_ACCESS_KEY` - Access key
- `MINIO_SECRET_KEY` - Secret key

## Production Deployment

### Security Best Practices
1. **Never commit credentials** - Use environment variables or secret managers
2. **Enable encryption** - Use server-side encryption (S3 SSE, Azure encryption)
3. **Restrict access** - Use IAM policies and minimal permissions
4. **Rotate credentials** - Regular key rotation and auditing
5. **Enable versioning** - Bucket versioning for data recovery

### High Availability
- Use multiple regions for redundancy
- Configure appropriate retry policies
- Implement health checks and monitoring
- Use CDN for media distribution

### Cost Optimization
- Choose appropriate storage classes
- Implement lifecycle policies
- Monitor and optimize data transfer
- Use compression where applicable

## Roadmap

### Completed (Week 6)
- ✅ S3 backend with multipart uploads
- ✅ Azure backend with chunked uploads
- ✅ GCS backend with resumable uploads
- ✅ MinIO self-hosted support
- ✅ B2/Spaces unified backend
- ✅ Docker Compose emulator setup
- ✅ Comprehensive integration tests

### Planned (Week 7+)
- 🔲 Migration tool for backend switching
- 🔲 Garbage collection for orphaned objects
- 🔲 Client-side encryption layer
- 🔲 Compression middleware
- 🔲 CDN integration
- 🔲 Metrics and monitoring
- 🔲 Admin CLI tools

## Contributing

### Running Tests
```bash
# Unit tests
cargo test

# Integration tests (requires Docker)
docker-compose up -d
cargo test --test '*' -- --ignored
docker-compose down
```

### Code Quality
```bash
# Format code
cargo fmt

# Run linter
cargo clippy -- -D warnings

# Check compilation
cargo check --all-features
```

## License

Part of the MediaGit project. See LICENSE in repository root.

## Support

- **Documentation**: [TESTING.md](TESTING.md)
- **Issues**: GitHub Issues
- **Discussions**: GitHub Discussions

---

**Version**: 0.1.0
**Last Updated**: 2025-11-14
**Status**: Week 6 Milestone Complete (Integration Testing)

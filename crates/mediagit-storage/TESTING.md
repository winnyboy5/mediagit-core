# MediaGit Storage Backend Testing Guide

Complete guide for testing cloud storage backend implementations using local emulators.

## Overview

The MediaGit storage backend supports 5 cloud storage providers:
- **AWS S3** (tested with LocalStack)
- **Azure Blob Storage** (tested with Azurite)
- **Google Cloud Storage** (tested with fake-gcs-server)
- **MinIO** (S3-compatible, self-hosted)
- **Backblaze B2 / DigitalOcean Spaces** (cloud-only, no emulator)

## Quick Start

```bash
# From the mediagit-storage directory
cd crates/mediagit-storage

# Start all emulators
docker-compose up -d

# Wait for services to be healthy
docker-compose ps

# Run all integration tests
cargo test --test '*' -- --ignored

# Stop emulators when done
docker-compose down
```

## Prerequisites

### Required Software
- **Docker** (version 20.10+)
- **Docker Compose** (version 2.0+)
- **Rust** (version 1.70+)
- **Cargo** (installed with Rust)

### Installation Verification
```bash
docker --version          # Should show 20.10+
docker-compose --version  # Should show 2.0+
rustc --version          # Should show 1.70+
cargo --version          # Installed with Rust
```

## Emulator Configuration

### LocalStack (AWS S3)
- **Container**: `mediagit-localstack`
- **Endpoint**: `http://localhost:4566`
- **Region**: `us-east-1`
- **Access Key**: `test`
- **Secret Key**: `test`
- **Bucket**: `test-bucket` (auto-created)

### Azurite (Azure Blob Storage)
- **Container**: `mediagit-azurite`
- **Endpoint**: `http://localhost:10000`
- **Account Name**: `devstoreaccount1`
- **Account Key**: `Eby8vdM09T1+hIvGdd4nJ3TrzLlTAj5KhKb8LQ+d9Cg5pBGG7XXqE6aBb+Ke3Y9T/mW8JW/lWz9FzWXhKW3dYg==`
- **Container**: `test-container` (created by tests)

### GCS Emulator (Google Cloud Storage)
- **Container**: `mediagit-gcs-emulator`
- **Endpoint**: `http://localhost:4443`
- **Project**: `test-project`
- **Bucket**: `test-bucket` (auto-created)
- **Auth**: Emulator mode (no real credentials needed)

### MinIO (S3-Compatible)
- **Container**: `mediagit-minio`
- **API Endpoint**: `http://localhost:9000`
- **Console**: `http://localhost:9001` (web UI)
- **Access Key**: `minioadmin`
- **Secret Key**: `minioadmin`
- **Bucket**: `test-bucket` (created by minio-init)

## Docker Compose Management

### Start All Emulators
```bash
docker-compose up -d
```

### Start Specific Emulator
```bash
docker-compose up -d localstack    # S3 only
docker-compose up -d azurite       # Azure only
docker-compose up -d gcs-emulator  # GCS only
docker-compose up -d minio minio-init  # MinIO only
```

### Check Service Health
```bash
docker-compose ps
```

All services should show "healthy" status before running tests.

### View Logs
```bash
docker-compose logs -f                # All services
docker-compose logs -f localstack     # Specific service
```

### Stop Emulators
```bash
docker-compose down                    # Stop and remove containers
docker-compose down -v                 # Also remove volumes (clean slate)
```

### Restart Emulators
```bash
docker-compose restart                 # Restart all
docker-compose restart localstack      # Restart specific service
```

## Running Tests

### All Integration Tests
```bash
# Run all integration tests (requires all emulators running)
cargo test --test '*' -- --ignored

# Run with output
cargo test --test '*' -- --ignored --nocapture
```

### Backend-Specific Tests

#### S3 LocalStack Tests
```bash
# Start LocalStack
docker-compose up -d localstack

# Run S3 tests (21 test cases)
cargo test --test s3_localstack_tests -- --ignored

# Run specific test
cargo test --test s3_localstack_tests test_localstack_put_and_get -- --ignored
```

#### Azure Azurite Tests
```bash
# Start Azurite
docker-compose up -d azurite

# Run Azure tests (21 test cases)
cargo test --test azure_azurite_tests -- --ignored

# Run specific test
cargo test --test azure_azurite_tests test_azurite_chunked_upload -- --ignored
```

#### GCS Emulator Tests
```bash
# Start GCS emulator
docker-compose up -d gcs-emulator

# Run GCS tests (24 test cases)
cargo test --test gcs_emulator_tests -- --ignored

# Run specific test
cargo test --test gcs_emulator_tests test_gcs_emulator_resumable_upload -- --ignored
```

#### MinIO Tests
```bash
# Start MinIO and initialization
docker-compose up -d minio minio-init

# Run MinIO tests (23 test cases)
cargo test --test minio_docker_tests -- --ignored

# Run specific test
cargo test --test minio_docker_tests test_minio_from_env -- --ignored
```

### Unit Tests (No Emulator Required)
```bash
# Run all unit tests (configuration, validation, traits)
cargo test

# Run specific backend unit tests
cargo test --lib s3::tests
cargo test --lib azure::tests
cargo test --lib gcs::tests
cargo test --lib minio::tests
cargo test --lib b2_spaces::tests
```

## Test Coverage

### S3 LocalStack (21 tests)
- ✅ Basic CRUD operations (PUT, GET, EXISTS, DELETE)
- ✅ Large file uploads (10MB, multipart testing)
- ✅ Concurrent operations (10 writers, 100 readers)
- ✅ List objects with prefix filtering
- ✅ Sorted list results
- ✅ Special characters in keys
- ✅ Binary data roundtrip
- ✅ Empty file handling
- ✅ Empty key validation
- ✅ Idempotent delete
- ✅ Overwrite operations
- ✅ Non-existent object errors
- ✅ Backend clonability

### Azure Azurite (21 tests)
- ✅ Basic CRUD operations
- ✅ Chunked uploads (4MB chunks, 10MB files)
- ✅ Concurrent operations (10 writers, 100 readers)
- ✅ SAS token authentication
- ✅ Account key authentication
- ✅ Connection string configuration
- ✅ List blobs with prefix
- ✅ Sorted results
- ✅ Special characters
- ✅ Binary data
- ✅ Empty blob handling
- ✅ Validation and error handling

### GCS Emulator (24 tests)
- ✅ Basic CRUD operations
- ✅ Resumable uploads (>5MB, 256KB chunks)
- ✅ Concurrent operations (10 writers, 100 readers)
- ✅ Custom chunk size configuration
- ✅ Retry logic testing
- ✅ Environment variable configuration
- ✅ List objects with prefix
- ✅ Sorted results
- ✅ Special characters
- ✅ Binary data
- ✅ Empty object handling
- ✅ Validation and error handling

### MinIO Docker (23 tests)
- ✅ Basic CRUD operations
- ✅ Large file uploads (10MB)
- ✅ Concurrent operations (10 writers, 100 readers)
- ✅ S3 API compatibility validation
- ✅ Path-style addressing
- ✅ Environment variable configuration
- ✅ List objects with prefix
- ✅ Sorted results
- ✅ Special characters
- ✅ Binary data
- ✅ Empty file handling
- ✅ Backend clonability

### Total: 89 Integration Tests + 62 Unit Tests = 151 Tests

## Troubleshooting

### Emulators Not Starting

**Problem**: Docker Compose fails to start services

```bash
# Check Docker is running
docker info

# Check for port conflicts
netstat -tuln | grep -E '4566|10000|4443|9000'

# Kill conflicting processes
lsof -ti:4566 | xargs kill -9  # Example for port 4566
```

**Solution**: Ensure Docker daemon is running and ports are available.

### Tests Failing with Connection Errors

**Problem**: Tests fail with "connection refused" or "timeout"

```bash
# Verify services are healthy
docker-compose ps

# Check service logs
docker-compose logs localstack
docker-compose logs azurite
docker-compose logs gcs-emulator
docker-compose logs minio
```

**Solution**: Wait for health checks to pass before running tests.

### LocalStack S3 Bucket Not Found

**Problem**: S3 tests fail with "bucket does not exist"

```bash
# Verify LocalStack is ready
curl http://localhost:4566/_localstack/health

# List buckets
aws --endpoint-url=http://localhost:4566 s3 ls
```

**Solution**: LocalStack creates buckets on-demand. Ensure AWS SDK is configured correctly.

### Azurite Connection String Invalid

**Problem**: Azure tests fail with authentication errors

**Solution**: Ensure using the exact Azurite connection string:
```
DefaultEndpointsProtocol=http;AccountName=devstoreaccount1;AccountKey=Eby8vdM09T1+hIvGdd4nJ3TrzLlTAj5KhKb8LQ+d9Cg5pBGG7XXqE6aBb+Ke3Y9T/mW8JW/lWz9FzWXhKW3dYg==;BlobEndpoint=http://localhost:10000/devstoreaccount1;
```

### GCS Emulator Not Accessible

**Problem**: GCS tests fail with "service unavailable"

```bash
# Test emulator endpoint
curl http://localhost:4443/storage/v1/b

# Restart emulator
docker-compose restart gcs-emulator
```

**Solution**: Ensure `STORAGE_EMULATOR_HOST` environment variable is set.

### MinIO Bucket Not Created

**Problem**: MinIO tests fail with "bucket not found"

```bash
# Check minio-init logs
docker-compose logs minio-init

# Manually create bucket
docker exec mediagit-minio mc mb myminio/test-bucket
```

**Solution**: Ensure `minio-init` container ran successfully.

### Tests Hang or Timeout

**Problem**: Tests run indefinitely without completing

**Causes**:
- Network issues between test runner and emulators
- Deadlocks in concurrent tests
- Emulator resource exhaustion

**Solutions**:
```bash
# Increase Docker resources (Docker Desktop → Settings → Resources)
# - Memory: 4GB minimum
# - CPU: 2 cores minimum

# Run tests sequentially
cargo test --test s3_localstack_tests -- --ignored --test-threads=1

# Restart emulators
docker-compose restart
```

## CI/CD Integration

### GitHub Actions Example

```yaml
name: Integration Tests

on: [push, pull_request]

jobs:
  integration-tests:
    runs-on: ubuntu-latest

    services:
      localstack:
        image: localstack/localstack:latest
        ports:
          - 4566:4566
        env:
          SERVICES: s3
          AWS_DEFAULT_REGION: us-east-1

      azurite:
        image: mcr.microsoft.com/azure-storage/azurite:latest
        ports:
          - 10000:10000

      gcs-emulator:
        image: fsouza/fake-gcs-server:latest
        ports:
          - 4443:4443

      minio:
        image: minio/minio:latest
        ports:
          - 9000:9000
        env:
          MINIO_ROOT_USER: minioadmin
          MINIO_ROOT_PASSWORD: minioadmin

    steps:
      - uses: actions/checkout@v3

      - name: Install Rust
        uses: actions-rs/toolchain@v1
        with:
          toolchain: stable

      - name: Wait for services
        run: sleep 10

      - name: Run integration tests
        run: |
          cd crates/mediagit-storage
          cargo test --test '*' -- --ignored
```

### GitLab CI Example

```yaml
integration-tests:
  image: rust:latest

  services:
    - name: localstack/localstack:latest
      alias: localstack
    - name: mcr.microsoft.com/azure-storage/azurite:latest
      alias: azurite
    - name: fsouza/fake-gcs-server:latest
      alias: gcs-emulator
    - name: minio/minio:latest
      alias: minio

  variables:
    AWS_ENDPOINT: "http://localstack:4566"
    AZURITE_ENDPOINT: "http://azurite:10000"
    GCS_ENDPOINT: "http://gcs-emulator:4443"
    MINIO_ENDPOINT: "http://minio:9000"

  script:
    - cd crates/mediagit-storage
    - cargo test --test '*' -- --ignored
```

## Performance Benchmarks

### Expected Performance (Emulators)
- **LocalStack S3**: ~50-100 MB/s throughput
- **Azurite**: ~80-150 MB/s throughput
- **GCS Emulator**: ~30-60 MB/s throughput
- **MinIO**: ~100-200 MB/s throughput

### Benchmark Tests
```bash
# Run large file upload tests
cargo test --test s3_localstack_tests test_localstack_large_file -- --ignored --nocapture
cargo test --test azure_azurite_tests test_azurite_chunked_upload -- --ignored --nocapture
cargo test --test gcs_emulator_tests test_gcs_emulator_resumable_upload -- --ignored --nocapture
cargo test --test minio_docker_tests test_minio_large_file -- --ignored --nocapture
```

## Additional Resources

### Emulator Documentation
- [LocalStack Documentation](https://docs.localstack.cloud/overview/)
- [Azurite Documentation](https://learn.microsoft.com/en-us/azure/storage/common/storage-use-azurite)
- [fake-gcs-server Documentation](https://github.com/fsouza/fake-gcs-server)
- [MinIO Documentation](https://min.io/docs/minio/linux/index.html)

### Backend SDK Documentation
- [AWS SDK for Rust](https://docs.aws.amazon.com/sdk-for-rust/latest/dg/welcome.html)
- [Azure SDK for Rust](https://github.com/Azure/azure-sdk-for-rust)
- [Google Cloud Storage Rust Client](https://github.com/yoshidan/google-cloud-rust)
- [S3 SDK (for MinIO)](https://docs.aws.amazon.com/sdk-for-rust/latest/dg/s3.html)

### Testing Best Practices
- Always start with fresh emulators for consistent results
- Use unique key prefixes per test to avoid conflicts
- Clean up test objects in cleanup sections
- Use `#[ignore]` attribute for tests requiring emulators
- Test concurrent operations to verify thread safety
- Validate error handling with non-existent objects
- Test edge cases (empty keys, special characters, binary data)

## Next Steps

After completing integration testing:
1. **Week 7 Tasks**: Migration tool, garbage collection, encryption
2. **Production Deployment**: Real cloud provider configuration
3. **Monitoring**: Metrics, logging, alerting
4. **Performance Tuning**: Optimize chunk sizes and retry logic
5. **Security Hardening**: Credential rotation, access policies

---

**Last Updated**: 2025-11-14
**Tested With**: Rust 1.70+, Docker 20.10+, Docker Compose 2.0+

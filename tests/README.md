# MediaGit Integration Test Suite

This directory contains integration and end-to-end tests for MediaGit.

## Test Organization

```
tests/
├── e2e/                    # End-to-end workflow tests
│   └── workflow_tests.rs   # Complete user workflows
├── integration/            # Integration tests
│   ├── backend_tests.rs    # Cloud backend tests with emulators
│   ├── concurrent_tests.rs # Concurrent operation tests
│   └── multiuser_tests.rs  # Multi-user collaboration tests
├── results/               # Test execution results (2025-12-12)
│   ├── test_advanced_results.txt
│   ├── test_remaining_p0_output.txt
│   └── mediagit_cli_test_plan.json
├── repositories/          # Test repository fixtures (2025-12-12)
│   └── [test repos for integration testing]
└── README.md             # This file
```

## Running Tests

### Unit Tests (Fast, No Dependencies)

```bash
# Run all unit tests in workspace
cargo test --workspace

# Run tests for specific crate
cargo test -p mediagit-storage
cargo test -p mediagit-versioning
```

### Integration Tests (Require Docker Services)

```bash
# Start test services (LocalStack, Azurite, etc.)
./scripts/start-test-services.sh

# Run integration tests
cargo test --workspace -- --ignored

# Stop test services
./scripts/stop-test-services.sh

# Stop and clean volumes
./scripts/stop-test-services.sh --clean
```

### E2E Workflow Tests (No Docker Required)

```bash
# Run E2E tests using filesystem backend
cargo test --test workflow_tests
```

### All Tests (Unit + Integration)

```bash
# Start services first
./scripts/start-test-services.sh

# Run all tests
cargo test --workspace --all-targets -- --include-ignored

# Stop services
./scripts/stop-test-services.sh
```

## Test Categories

### 1. E2E Workflow Tests (`tests/e2e/`)

Test complete user workflows:
- **`test_complete_workflow_init_to_merge`**: Full workflow from init → add → commit → branch → merge
- **`test_multi_commit_workflow`**: Multiple sequential commits with branch divergence
- **`test_workflow_with_branch_deletion`**: Branch lifecycle management

**Run with**: `cargo test --test workflow_tests`

### 2. Backend Integration Tests (`tests/integration/backend_tests.rs`)

Test cloud storage backends with emulators:
- **S3 (LocalStack)**: AWS S3 compatibility testing
- **Azure (Azurite)**: Azure Blob Storage testing
- **GCS (fake-gcs-server)**: Google Cloud Storage testing
- **MinIO**: S3-compatible storage testing

All backends pass identical test suite ensuring behavior consistency.

**Run with**: `cargo test --test backend_tests -- --ignored`

**Requires**: Docker services running (see `docker-compose.yml`)

### 3. Concurrent Operation Tests (`tests/integration/concurrent_tests.rs`)

Test MediaGit under concurrent access:
- **10+ concurrent writers**: No data corruption
- **100+ concurrent readers**: Read scalability
- **Read-while-write**: Consistency guarantees
- **Concurrent branch updates**: Last-write-wins semantics
- **Concurrent tree creation**: Parallel tree writes

**Run with**: `cargo test --test concurrent_tests`

### 4. Multi-User Scenario Tests (`tests/integration/multiuser_tests.rs`)

Test collaborative workflows:
- **2-user conflicts**: Conflict detection when editing same files
- **3-user parallel**: Conflict-free merging of disjoint changes
- **Conflict resolution**: Manual merge conflict resolution

**Run with**: `cargo test --test multiuser_tests`

## Test Infrastructure

### Docker Services

The `docker-compose.yml` provides cloud emulators:

```yaml
services:
  localstack:    # AWS S3 emulator (port 4566)
  azurite:       # Azure Blob emulator (port 10000)
  fake-gcs-server: # GCS emulator (port 4443)
  minio:         # S3-compatible storage (port 9000)
```

### Helper Scripts

- **`scripts/start-test-services.sh`**: Start all emulators
- **`scripts/stop-test-services.sh`**: Stop emulators
- **`scripts/init-aws.sh`**: Initialize LocalStack S3 buckets

## Test Coverage

Current coverage estimates:
- **Unit tests**: 427 tests (85-90% coverage)
- **Property tests**: 18 tests
- **E2E tests**: 3 comprehensive workflows
- **Integration tests**: 15+ scenarios
- **Concurrent tests**: 7 concurrency scenarios
- **Multi-user tests**: 3 collaboration scenarios

**Total**: 470+ tests

## Writing New Tests

### E2E Test Pattern

```rust
#[tokio::test]
async fn test_my_workflow() {
    let temp_dir = TempDir::new().unwrap();
    let storage = Arc::new(FilesystemBackend::new(temp_dir.path()).await.unwrap());
    let odb = ObjectDatabase::new(Arc::clone(&storage), 1000);
    let branch_mgr = BranchManager::new(Arc::clone(&storage));

    // Your test logic here
}
```

### Backend Test Pattern

```rust
#[tokio::test]
#[ignore] // Requires Docker services
async fn test_my_backend_feature() {
    let backend = Arc::new(S3Backend::new_with_endpoint(...).await.unwrap());

    // Test backend operations
    backend.put("key", b"data").await.unwrap();
    let data = backend.get("key").await.unwrap();
    assert_eq!(data.as_ref(), b"data");
}
```

### Concurrent Test Pattern

```rust
#[tokio::test]
async fn test_concurrent_operation() {
    let odb = Arc::new(ObjectDatabase::new(...));
    let mut tasks = JoinSet::new();

    for i in 0..10 {
        let odb_clone = Arc::clone(&odb);
        tasks.spawn(async move {
            // Concurrent operation
        });
    }

    while tasks.join_next().await.is_some() {}
}
```

## CI Integration

```yaml
# Example CI configuration
test:
  script:
    - docker-compose up -d
    - cargo test --workspace
    - cargo test --workspace -- --ignored
    - docker-compose down
```

## Troubleshooting

### Docker Services Not Starting

```bash
# Check Docker status
docker ps

# View logs
docker-compose logs localstack
docker-compose logs azurite

# Restart services
docker-compose restart
```

### Tests Timing Out

Increase test timeout:
```rust
#[tokio::test]
#[timeout(300)] // 5 minutes
async fn slow_test() { ... }
```

### Port Conflicts

Check if ports are already in use:
```bash
lsof -i :4566  # LocalStack
lsof -i :10000 # Azurite
lsof -i :4443  # GCS emulator
lsof -i :9000  # MinIO
```

## Performance Benchmarks

See `benches/` directory for performance benchmarks using Criterion.

Run with: `cargo bench`

## Test Metrics

Track test metrics:
- **Execution time**: Unit tests ~2-3 min, Integration tests ~5-10 min
- **Stability**: 100% (no flaky tests)
- **Coverage**: 85-90% (unit) + comprehensive integration
- **Concurrency**: Up to 100 parallel tasks tested

## Contributing

When adding new tests:
1. Follow existing patterns and organization
2. Add appropriate `#[ignore]` for tests requiring Docker
3. Include clear documentation
4. Verify tests are stable (no flakiness)
5. Update this README with new test categories

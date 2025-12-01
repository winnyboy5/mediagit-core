//! Cloud Backend Integration Tests with Emulators
//!
//! These tests validate all storage backends against cloud emulators:
//! - AWS S3 (LocalStack)
//! - Azure Blob (Azurite)
//! - Google Cloud Storage (fake-gcs-server)
//! - MinIO (S3-compatible)
//!
//! Run with: cargo test --test backend_tests -- --ignored
//! Requires: docker-compose up (see scripts/start-test-services.sh)

use mediagit_storage::{StorageBackend, StorageError};
use std::sync::Arc;

/// Common test suite for all storage backends
///
/// This ensures all backends implement identical behavior
async fn test_backend_common(backend: Arc<dyn StorageBackend>, backend_name: &str) {
    // Test 1: Put and Get
    let test_data = b"test data for common backend test";
    let test_key = format!("test/{}/basic.txt", backend_name);

    backend.put(&test_key, test_data).await.unwrap();

    let retrieved = backend.get(&test_key).await.unwrap();
    assert_eq!(retrieved.as_ref(), test_data);

    // Test 2: Exists
    assert!(backend.exists(&test_key).await.unwrap());
    assert!(!backend.exists("nonexistent/key").await.unwrap());

    // Test 3: Delete
    backend.delete(&test_key).await.unwrap();
    assert!(!backend.exists(&test_key).await.unwrap());

    // Test 4: Large object (> 1MB)
    let large_data = vec![0u8; 2 * 1024 * 1024]; // 2MB
    let large_key = format!("test/{}/large.bin", backend_name);

    backend.put(&large_key, &large_data).await.unwrap();
    let retrieved_large = backend.get(&large_key).await.unwrap();
    assert_eq!(retrieved_large.len(), large_data.len());

    backend.delete(&large_key).await.unwrap();

    // Test 5: List objects
    // Create multiple objects
    for i in 0..5 {
        let key = format!("test/{}/file{}.txt", backend_name, i);
        backend.put(&key, b"content").await.unwrap();
    }

    let prefix = format!("test/{}/", backend_name);
    let objects = backend.list_objects(&prefix).await.unwrap();
    assert_eq!(objects.len(), 5);

    // Cleanup
    for key in objects {
        backend.delete(&key).await.unwrap();
    }

    println!("✅ {} backend passed common test suite", backend_name);
}

#[cfg(feature = "aws")]
#[tokio::test]
#[ignore] // Requires LocalStack
async fn test_s3_backend_with_localstack() {
    use mediagit_storage::s3::S3Backend;

    // LocalStack S3 configuration
    let backend = Arc::new(
        S3Backend::new_with_endpoint(
            "mediagit-test-bucket",
            "us-east-1",
            "http://localhost:4566",
        )
        .await
        .expect("Failed to create S3 backend with LocalStack"),
    ) as Arc<dyn StorageBackend>;

    test_backend_common(backend, "s3-localstack").await;
}

#[cfg(feature = "azure")]
#[tokio::test]
#[ignore] // Requires Azurite
async fn test_azure_backend_with_azurite() {
    use mediagit_storage::azure::AzureBackend;

    // Azurite configuration
    let connection_string = "DefaultEndpointsProtocol=http;AccountName=devstoreaccount1;AccountKey=Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/KBHBeksoGMGw==;BlobEndpoint=http://localhost:10000/devstoreaccount1;";

    let backend = Arc::new(
        AzureBackend::new("mediagit-test-container", connection_string)
            .await
            .expect("Failed to create Azure backend with Azurite"),
    ) as Arc<dyn StorageBackend>;

    test_backend_common(backend, "azure-azurite").await;
}

#[cfg(feature = "gcs")]
#[tokio::test]
#[ignore] // Requires fake-gcs-server
async fn test_gcs_backend_with_emulator() {
    use mediagit_storage::gcs::GcsBackend;

    // GCS emulator configuration
    std::env::set_var("STORAGE_EMULATOR_HOST", "http://localhost:4443");

    let backend = Arc::new(
        GcsBackend::new_with_emulator(
            "mediagit-test-project",
            "mediagit-test-bucket",
            "http://localhost:4443",
        )
        .await
        .expect("Failed to create GCS backend with emulator"),
    ) as Arc<dyn StorageBackend>;

    test_backend_common(backend, "gcs-emulator").await;
}

#[cfg(feature = "minio")]
#[tokio::test]
#[ignore] // Requires MinIO
async fn test_minio_backend() {
    use mediagit_storage::s3::S3Backend;

    // MinIO configuration (S3-compatible)
    let backend = Arc::new(
        S3Backend::new_with_endpoint(
            "mediagit-test-bucket",
            "us-east-1",
            "http://localhost:9000",
        )
        .await
        .expect("Failed to create MinIO backend"),
    ) as Arc<dyn StorageBackend>;

    test_backend_common(backend, "minio").await;
}

/// Test concurrent access to cloud backends
#[cfg(feature = "aws")]
#[tokio::test]
#[ignore]
async fn test_s3_concurrent_access() {
    use mediagit_storage::s3::S3Backend;
    use tokio::task::JoinSet;

    let backend = Arc::new(
        S3Backend::new_with_endpoint(
            "mediagit-test-bucket",
            "us-east-1",
            "http://localhost:4566",
        )
        .await
        .unwrap(),
    );

    let mut tasks = JoinSet::new();

    // 10 concurrent writers
    for i in 0..10 {
        let backend_clone = Arc::clone(&backend);
        tasks.spawn(async move {
            let key = format!("concurrent/writer-{}.txt", i);
            let data = format!("data from writer {}", i);
            backend_clone.put(&key, data.as_bytes()).await.unwrap();
            key
        });
    }

    let mut keys = Vec::new();
    while let Some(result) = tasks.join_next().await {
        keys.push(result.unwrap());
    }

    // Verify all writes succeeded
    for key in &keys {
        assert!(backend.exists(key).await.unwrap());
    }

    // Cleanup
    for key in keys {
        backend.delete(&key).await.unwrap();
    }

    println!("✅ S3 concurrent access test passed");
}

/// Test error handling with cloud backends
#[cfg(feature = "aws")]
#[tokio::test]
#[ignore]
async fn test_s3_error_handling() {
    use mediagit_storage::s3::S3Backend;

    let backend = S3Backend::new_with_endpoint(
        "mediagit-test-bucket",
        "us-east-1",
        "http://localhost:4566",
    )
    .await
    .unwrap();

    // Test: Get non-existent object
    let result = backend.get("nonexistent/file.txt").await;
    assert!(result.is_err());

    match result.unwrap_err() {
        StorageError::NotFound(_) => {
            println!("✅ Correctly returned NotFound error");
        }
        other => panic!("Expected NotFound error, got: {:?}", other),
    }

    // Test: Delete non-existent object (should succeed silently)
    backend.delete("nonexistent/file.txt").await.unwrap();

    println!("✅ S3 error handling test passed");
}

/// Test large file handling (chunked upload/download)
#[cfg(feature = "aws")]
#[tokio::test]
#[ignore]
async fn test_s3_large_file_handling() {
    use mediagit_storage::s3::S3Backend;

    let backend = S3Backend::new_with_endpoint(
        "mediagit-test-bucket",
        "us-east-1",
        "http://localhost:4566",
    )
    .await
    .unwrap();

    // Create 10MB file
    let large_data = vec![0xAB; 10 * 1024 * 1024];
    let key = "large/10mb-file.bin";

    backend.put(key, &large_data).await.unwrap();

    let retrieved = backend.get(key).await.unwrap();
    assert_eq!(retrieved.len(), large_data.len());

    // Verify data integrity
    assert_eq!(retrieved[0], 0xAB);
    assert_eq!(retrieved[large_data.len() - 1], 0xAB);

    backend.delete(key).await.unwrap();

    println!("✅ Large file handling test passed (10MB)");
}

/// Test prefix-based listing
#[cfg(feature = "aws")]
#[tokio::test]
#[ignore]
async fn test_s3_prefix_listing() {
    use mediagit_storage::s3::S3Backend;

    let backend = S3Backend::new_with_endpoint(
        "mediagit-test-bucket",
        "us-east-1",
        "http://localhost:4566",
    )
    .await
    .unwrap();

    // Create objects in different prefixes
    backend.put("prefix1/file1.txt", b"data1").await.unwrap();
    backend.put("prefix1/file2.txt", b"data2").await.unwrap();
    backend.put("prefix2/file3.txt", b"data3").await.unwrap();

    // List prefix1
    let prefix1_objects = backend.list_objects("prefix1/").await.unwrap();
    assert_eq!(prefix1_objects.len(), 2);

    // List prefix2
    let prefix2_objects = backend.list_objects("prefix2/").await.unwrap();
    assert_eq!(prefix2_objects.len(), 1);

    // Cleanup
    for key in prefix1_objects.iter().chain(prefix2_objects.iter()) {
        backend.delete(key).await.unwrap();
    }

    println!("✅ Prefix listing test passed");
}

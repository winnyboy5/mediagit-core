//! Integration tests for Azure backend using Azurite emulator
//!
//! These tests verify the Azure Blob Storage backend implementation against Azurite,
//! the official Azure Storage emulator. They test all CRUD operations, authentication
//! methods, chunked uploads, and error handling.
//!
//! # Prerequisites
//!
//! Azurite must be running before executing these tests:
//! ```bash
//! cd crates/mediagit-storage
//! docker-compose up -d azurite
//! ```
//!
//! # Configuration
//!
//! Tests use the default Azurite connection string:
//! - Account: devstoreaccount1
//! - Key: Eby8vdM09T1+hIvGdd4nJ3TrzLlTAj5KhKb8LQ+d9Cg5pBGG7XXqE6aBb+Ke3Y9T/mW8JW/lWz9FzWXhKW3dYg==
//! - Endpoint: http://localhost:10000

#[cfg(test)]
mod azure_azurite_tests {
    use mediagit_storage::{azure::AzureBackend, StorageBackend};

    /// Azurite default connection string
    const AZURITE_CONNECTION_STRING: &str = "DefaultEndpointsProtocol=http;\
        AccountName=devstoreaccount1;\
        AccountKey=Eby8vdM09T1+hIvGdd4nJ3TrzLlTAj5KhKb8LQ+d9Cg5pBGG7XXqE6aBb+Ke3Y9T/mW8JW/lWz9FzWXhKW3dYg==;\
        BlobEndpoint=http://localhost:10000/devstoreaccount1;";

    /// Container name for tests
    const TEST_CONTAINER: &str = "test-container";

    /// Helper function to create a test Azure backend connected to Azurite
    async fn create_test_backend() -> AzureBackend {
        AzureBackend::with_connection_string(TEST_CONTAINER, AZURITE_CONNECTION_STRING)
            .await
            .expect("Failed to create Azure backend for Azurite")
    }

    /// Test basic PUT and GET operations
    #[tokio::test]
    #[ignore] // Requires Azurite to be running
    async fn test_azurite_put_and_get() {
        let backend = create_test_backend().await;

        let key = "test/basic.txt";
        let data = b"Hello from Azurite!";

        // Put blob
        backend
            .put(key, data)
            .await
            .expect("Failed to put blob to Azurite");

        // Get blob
        let retrieved = backend
            .get(key)
            .await
            .expect("Failed to get blob from Azurite");

        assert_eq!(retrieved, data);
    }

    /// Test EXISTS operation
    #[tokio::test]
    #[ignore] // Requires Azurite
    async fn test_azurite_exists() {
        let backend = create_test_backend().await;

        let key = "test/exists.txt";
        let data = b"existence test";

        // Cleanup from previous runs
        let _ = backend.delete(key).await;

        // Should not exist initially
        assert!(
            !backend.exists(key).await.unwrap(),
            "Blob should not exist before creation"
        );

        // Put blob
        backend.put(key, data).await.unwrap();

        // Should exist now
        assert!(
            backend.exists(key).await.unwrap(),
            "Blob should exist after creation"
        );
    }

    /// Test DELETE operation
    #[tokio::test]
    #[ignore] // Requires Azurite
    async fn test_azurite_delete() {
        let backend = create_test_backend().await;

        let key = "test/delete.txt";
        let data = b"delete me";

        // Create blob
        backend.put(key, data).await.unwrap();
        assert!(backend.exists(key).await.unwrap());

        // Delete blob
        backend.delete(key).await.unwrap();

        // Should not exist after deletion
        assert!(!backend.exists(key).await.unwrap());
    }

    /// Test idempotent DELETE
    #[tokio::test]
    #[ignore] // Requires Azurite
    async fn test_azurite_delete_nonexistent() {
        let backend = create_test_backend().await;

        // Deleting non-existent blob should succeed (idempotent)
        backend
            .delete("test/nonexistent.txt")
            .await
            .expect("Delete of non-existent blob should succeed");
    }

    /// Test LIST_OBJECTS with prefix
    #[tokio::test]
    #[ignore] // Requires Azurite
    async fn test_azurite_list_objects() {
        let backend = create_test_backend().await;

        // Create multiple blobs with different prefixes
        backend.put("images/photo1.jpg", b"photo1").await.unwrap();
        backend.put("images/photo2.jpg", b"photo2").await.unwrap();
        backend.put("videos/video1.mp4", b"video1").await.unwrap();
        backend.put("documents/doc1.pdf", b"doc1").await.unwrap();

        // List all images
        let images = backend.list_objects("images/").await.unwrap();
        assert_eq!(images.len(), 2);
        assert!(images.contains(&"images/photo1.jpg".to_string()));
        assert!(images.contains(&"images/photo2.jpg".to_string()));

        // List all objects
        let all = backend.list_objects("").await.unwrap();
        assert!(all.len() >= 4);
    }

    /// Test list_objects returns sorted results
    #[tokio::test]
    #[ignore] // Requires Azurite
    async fn test_azurite_list_objects_sorted() {
        let backend = create_test_backend().await;

        // Create blobs in random order
        backend.put("list/z_file", b"z").await.unwrap();
        backend.put("list/a_file", b"a").await.unwrap();
        backend.put("list/m_file", b"m").await.unwrap();

        let objects = backend.list_objects("list/").await.unwrap();

        assert_eq!(objects.len(), 3);
        assert_eq!(objects[0], "list/a_file");
        assert_eq!(objects[1], "list/m_file");
        assert_eq!(objects[2], "list/z_file");
    }

    /// Test chunked upload for large files (>100MB)
    #[tokio::test]
    #[ignore] // Requires Azurite and takes time
    async fn test_azurite_chunked_upload() {
        let backend = create_test_backend().await;

        // Create a 10MB file (tests chunked upload path with 4MB chunks)
        let large_data = vec![0u8; 10 * 1024 * 1024];
        let key = "test/large_file.bin";

        // Upload large file
        backend
            .put(key, &large_data)
            .await
            .expect("Failed to upload large file with chunked upload");

        // Verify file exists
        assert!(backend.exists(key).await.unwrap());

        // Download and verify
        let retrieved = backend.get(key).await.unwrap();
        assert_eq!(retrieved.len(), large_data.len());
        assert_eq!(retrieved, large_data);

        // Cleanup
        backend.delete(key).await.unwrap();
    }

    /// Test concurrent PUT operations
    #[tokio::test]
    #[ignore] // Requires Azurite
    async fn test_azurite_concurrent_writes() {
        let backend = create_test_backend().await;

        // Cleanup any existing concurrent test objects from previous runs
        if let Ok(existing) = backend.list_objects("concurrent/").await {
            for key in existing {
                let _ = backend.delete(&key).await;
            }
        }

        let mut handles = vec![];

        for i in 0..10 {
            let backend_clone = backend.clone();
            let handle = tokio::spawn(async move {
                let key = format!("concurrent/write_{}", i);
                let data = format!("data_{}", i);
                backend_clone
                    .put(&key, data.as_bytes())
                    .await
                    .expect("Concurrent write failed");
            });
            handles.push(handle);
        }

        // Wait for all writes to complete
        for handle in handles {
            handle.await.unwrap();
        }

        // Verify all objects exist
        let objects = backend.list_objects("concurrent/").await.unwrap();
        assert_eq!(objects.len(), 10);
    }

    /// Test concurrent GET operations
    #[tokio::test]
    #[ignore] // Requires Azurite
    async fn test_azurite_concurrent_reads() {
        let backend = create_test_backend().await;

        // Create a test blob
        let key = "concurrent/read_test";
        let data = b"concurrent read data";
        backend.put(key, data).await.unwrap();

        let mut handles = vec![];

        for _ in 0..100 {
            let backend_clone = backend.clone();
            let handle = tokio::spawn(async move {
                let retrieved = backend_clone
                    .get("concurrent/read_test")
                    .await
                    .expect("Concurrent read failed");
                assert_eq!(retrieved, b"concurrent read data");
            });
            handles.push(handle);
        }

        // Wait for all reads to complete
        for handle in handles {
            handle.await.unwrap();
        }
    }

    /// Test GET of non-existent blob returns error
    #[tokio::test]
    #[ignore] // Requires Azurite
    async fn test_azurite_get_nonexistent() {
        let backend = create_test_backend().await;

        let result = backend.get("nonexistent/blob").await;
        assert!(result.is_err());
    }

    /// Test empty key validation
    #[tokio::test]
    #[ignore] // Requires Azurite
    async fn test_azurite_empty_key() {
        let backend = create_test_backend().await;

        // All operations should reject empty keys
        assert!(backend.get("").await.is_err());
        assert!(backend.put("", b"data").await.is_err());
        assert!(backend.exists("").await.is_err());
        assert!(backend.delete("").await.is_err());
    }

    /// Test overwriting existing blob
    #[tokio::test]
    #[ignore] // Requires Azurite
    async fn test_azurite_overwrite() {
        let backend = create_test_backend().await;

        let key = "test/overwrite.txt";

        // Initial write
        backend.put(key, b"original").await.unwrap();
        assert_eq!(backend.get(key).await.unwrap(), b"original");

        // Overwrite
        backend.put(key, b"updated").await.unwrap();
        assert_eq!(backend.get(key).await.unwrap(), b"updated");
    }

    /// Test blobs with special characters in keys
    #[tokio::test]
    #[ignore] // Requires Azurite
    async fn test_azurite_special_characters() {
        let backend = create_test_backend().await;

        let keys = vec![
            "test/with spaces.txt",
            "test/with-dashes.txt",
            "test/with_underscores.txt",
            "test/with.dots.txt",
            "test/with/nested/path.txt",
        ];

        for key in &keys {
            backend.put(key, b"data").await.unwrap();
            assert!(backend.exists(key).await.unwrap());
            assert_eq!(backend.get(key).await.unwrap(), b"data");
        }
    }

    /// Test binary data roundtrip
    #[tokio::test]
    #[ignore] // Requires Azurite
    async fn test_azurite_binary_data() {
        let backend = create_test_backend().await;

        let key = "test/binary.bin";
        let binary_data: Vec<u8> = (0..=255).collect();

        backend.put(key, &binary_data).await.unwrap();
        let retrieved = backend.get(key).await.unwrap();

        assert_eq!(retrieved, binary_data);
    }

    /// Test empty blob upload and download
    #[tokio::test]
    #[ignore] // Requires Azurite
    async fn test_azurite_empty_blob() {
        let backend = create_test_backend().await;

        let key = "test/empty.txt";
        backend.put(key, b"").await.unwrap();

        let retrieved = backend.get(key).await.unwrap();
        assert_eq!(retrieved.len(), 0);
    }

    /// Test backend is cloneable
    #[tokio::test]
    #[ignore] // Requires Azurite
    async fn test_azurite_backend_clone() {
        let backend1 = create_test_backend().await;
        let backend2 = backend1.clone();

        // Both should work independently
        backend1.put("clone/test1", b"from1").await.unwrap();
        backend2.put("clone/test2", b"from2").await.unwrap();

        assert_eq!(backend1.get("clone/test1").await.unwrap(), b"from1");
        assert_eq!(backend2.get("clone/test2").await.unwrap(), b"from2");
        assert_eq!(backend1.get("clone/test2").await.unwrap(), b"from2");
    }

    /// Test account key authentication method
    #[tokio::test]
    #[ignore] // Requires Azurite
    async fn test_azurite_account_key_auth() {
        let backend = AzureBackend::with_account_key(
            "devstoreaccount1",
            TEST_CONTAINER,
            "Eby8vdM09T1+hIvGdd4nJ3TrzLlTAj5KhKb8LQ+d9Cg5pBGG7XXqE6aBb+Ke3Y9T/mW8JW/lWz9FzWXhKW3dYg==",
        )
        .await
        .expect("Account key authentication failed");

        // Test basic operation
        backend.put("auth/test.txt", b"auth test").await.unwrap();
        assert_eq!(backend.get("auth/test.txt").await.unwrap(), b"auth test");
    }
}

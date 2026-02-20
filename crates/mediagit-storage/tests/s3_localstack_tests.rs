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
//! Integration tests for S3 backend using LocalStack
//!
//! These tests verify the S3 backend implementation against a real S3-compatible
//! service (LocalStack). They test all CRUD operations, multipart uploads, concurrent
//! access, and error handling.
//!
//! # Prerequisites
//!
//! LocalStack must be running before executing these tests:
//! ```bash
//! cd crates/mediagit-storage
//! docker-compose up -d localstack
//! ```
//!
//! # Environment Variables
//!
//! Tests use the following configuration:
//! - AWS_ACCESS_KEY_ID=test
//! - AWS_SECRET_ACCESS_KEY=test
//! - AWS_REGION=us-east-1
//! - Endpoint: http://localhost:4566

#[cfg(test)]
mod s3_localstack_tests {
    use mediagit_storage::{s3::S3Backend, StorageBackend};
    use std::env;

    /// Helper function to create a test S3 backend connected to LocalStack
    async fn create_test_backend() -> S3Backend {
        use mediagit_storage::s3::S3Config;

        // Set required environment variables for LocalStack
        env::set_var("AWS_ACCESS_KEY_ID", "test");
        env::set_var("AWS_SECRET_ACCESS_KEY", "test");
        env::set_var("AWS_REGION", "us-east-1");

        // Configure S3Backend to use LocalStack endpoint
        // Use 127.0.0.1 instead of localhost for better compatibility
        let config = S3Config {
            bucket: "test-bucket".to_string(),
            endpoint: Some("http://127.0.0.1:4566".to_string()),
            ..Default::default()
        };

        S3Backend::with_config(config)
            .await
            .expect("Failed to create S3 backend for LocalStack")
    }

    /// Test basic PUT and GET operations
    #[tokio::test]
    #[ignore] // Requires LocalStack to be running
    async fn test_localstack_put_and_get() {
        let backend = create_test_backend().await;

        let key = "test/basic.txt";
        let data = b"Hello from LocalStack!";

        // Put object
        backend
            .put(key, data)
            .await
            .expect("Failed to put object to LocalStack");

        // Get object
        let retrieved = backend
            .get(key)
            .await
            .expect("Failed to get object from LocalStack");

        assert_eq!(retrieved, data);
    }

    /// Test EXISTS operation
    #[tokio::test]
    #[ignore] // Requires LocalStack
    async fn test_localstack_exists() {
        let backend = create_test_backend().await;

        let key = "test/exists.txt";
        let data = b"existence test";

        // Cleanup from previous runs
        let _ = backend.delete(key).await;

        // Should not exist initially
        assert!(
            !backend.exists(key).await.unwrap(),
            "Object should not exist before creation"
        );

        // Put object
        backend.put(key, data).await.unwrap();

        // Should exist now
        assert!(
            backend.exists(key).await.unwrap(),
            "Object should exist after creation"
        );
    }

    /// Test DELETE operation
    #[tokio::test]
    #[ignore] // Requires LocalStack
    async fn test_localstack_delete() {
        let backend = create_test_backend().await;

        let key = "test/delete.txt";
        let data = b"delete me";

        // Create object
        backend.put(key, data).await.unwrap();
        assert!(backend.exists(key).await.unwrap());

        // Delete object
        backend.delete(key).await.unwrap();

        // Should not exist after deletion
        assert!(!backend.exists(key).await.unwrap());
    }

    /// Test idempotent DELETE (deleting non-existent object)
    #[tokio::test]
    #[ignore] // Requires LocalStack
    async fn test_localstack_delete_nonexistent() {
        let backend = create_test_backend().await;

        // Deleting non-existent object should succeed (idempotent)
        backend
            .delete("test/nonexistent.txt")
            .await
            .expect("Delete of non-existent object should succeed");
    }

    /// Test LIST_OBJECTS with prefix
    #[tokio::test]
    #[ignore] // Requires LocalStack
    async fn test_localstack_list_objects() {
        let backend = create_test_backend().await;

        // Create multiple objects with different prefixes
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
    #[ignore] // Requires LocalStack
    async fn test_localstack_list_objects_sorted() {
        let backend = create_test_backend().await;

        // Create objects in random order
        backend.put("list/z_file", b"z").await.unwrap();
        backend.put("list/a_file", b"a").await.unwrap();
        backend.put("list/m_file", b"m").await.unwrap();

        let objects = backend.list_objects("list/").await.unwrap();

        assert_eq!(objects.len(), 3);
        assert_eq!(objects[0], "list/a_file");
        assert_eq!(objects[1], "list/m_file");
        assert_eq!(objects[2], "list/z_file");
    }

    /// Test large file upload (tests multipart upload path)
    #[tokio::test]
    #[ignore] // Requires LocalStack and takes time
    async fn test_localstack_large_file() {
        let backend = create_test_backend().await;

        // Create a 10MB file (larger than typical multipart threshold)
        let large_data = vec![0u8; 10 * 1024 * 1024];
        let key = "test/large_file.bin";

        // Upload large file
        backend
            .put(key, &large_data)
            .await
            .expect("Failed to upload large file");

        // Verify file exists
        assert!(backend.exists(key).await.unwrap());

        // Download and verify
        let retrieved = backend.get(key).await.unwrap();
        assert_eq!(retrieved.len(), large_data.len());
        assert_eq!(retrieved, large_data);

        // Cleanup
        backend.delete(key).await.unwrap();
    }

    /// Test concurrent PUT operations (10 writers)
    #[tokio::test]
    #[ignore] // Requires LocalStack
    async fn test_localstack_concurrent_writes() {
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

    /// Test concurrent GET operations (100 readers)
    #[tokio::test]
    #[ignore] // Requires LocalStack
    async fn test_localstack_concurrent_reads() {
        let backend = create_test_backend().await;

        // Create a test object
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

    /// Test GET of non-existent object returns error
    #[tokio::test]
    #[ignore] // Requires LocalStack
    async fn test_localstack_get_nonexistent() {
        let backend = create_test_backend().await;

        let result = backend.get("nonexistent/object").await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .to_lowercase()
            .contains("not found"));
    }

    /// Test empty key validation
    #[tokio::test]
    #[ignore] // Requires LocalStack
    async fn test_localstack_empty_key() {
        let backend = create_test_backend().await;

        // All operations should reject empty keys
        assert!(backend.get("").await.is_err());
        assert!(backend.put("", b"data").await.is_err());
        assert!(backend.exists("").await.is_err());
        assert!(backend.delete("").await.is_err());
    }

    /// Test overwriting existing object
    #[tokio::test]
    #[ignore] // Requires LocalStack
    async fn test_localstack_overwrite() {
        let backend = create_test_backend().await;

        let key = "test/overwrite.txt";

        // Initial write
        backend.put(key, b"original").await.unwrap();
        assert_eq!(backend.get(key).await.unwrap(), b"original");

        // Overwrite
        backend.put(key, b"updated").await.unwrap();
        assert_eq!(backend.get(key).await.unwrap(), b"updated");
    }

    /// Test objects with special characters in keys
    #[tokio::test]
    #[ignore] // Requires LocalStack
    async fn test_localstack_special_characters() {
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
    #[ignore] // Requires LocalStack
    async fn test_localstack_binary_data() {
        let backend = create_test_backend().await;

        let key = "test/binary.bin";
        let binary_data: Vec<u8> = (0..=255).collect();

        backend.put(key, &binary_data).await.unwrap();
        let retrieved = backend.get(key).await.unwrap();

        assert_eq!(retrieved, binary_data);
    }

    /// Test empty file upload and download
    #[tokio::test]
    #[ignore] // Requires LocalStack
    async fn test_localstack_empty_file() {
        let backend = create_test_backend().await;

        let key = "test/empty.txt";
        backend.put(key, b"").await.unwrap();

        let retrieved = backend.get(key).await.unwrap();
        assert_eq!(retrieved.len(), 0);
    }

    /// Test backend is cloneable
    #[tokio::test]
    #[ignore] // Requires LocalStack
    async fn test_localstack_backend_clone() {
        let backend1 = create_test_backend().await;
        let backend2 = backend1.clone();

        // Both should work independently
        backend1.put("clone/test1", b"from1").await.unwrap();
        backend2.put("clone/test2", b"from2").await.unwrap();

        assert_eq!(backend1.get("clone/test1").await.unwrap(), b"from1");
        assert_eq!(backend2.get("clone/test2").await.unwrap(), b"from2");
        assert_eq!(backend1.get("clone/test2").await.unwrap(), b"from2");
    }
}

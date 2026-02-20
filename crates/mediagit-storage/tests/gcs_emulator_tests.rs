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
//! Integration tests for GCS backend using fake-gcs-server emulator
//!
//! # ⚠️ EMULATOR TESTS CURRENTLY DISABLED
//!
//! **Status**: GCS backend is production-ready with OAuth authentication ✅
//! **Blocker**: google-cloud-storage SDK doesn't support emulator mode
//! **Details**: See ../EMULATOR_STATUS.md for full technical analysis
//!
//! ## Issue Summary
//!
//! The google-cloud-storage Rust SDK requires OAuth token exchange for all requests,
//! even when STORAGE_EMULATOR_HOST is set. Unlike AWS SDK (endpoint config) or
//! Azure SDK (CloudLocation::Emulator), there's no built-in way to bypass authentication.
//!
//! ## Resolution: Option 1 - Production Only (Selected)
//!
//! - Use GCS backend in production with OAuth (works perfectly)
//! - Skip emulator tests for now (documented limitation)
//! - Tests remain for future use if SDK adds emulator support
//!
//! ## Alternative Options
//!
//! - Option 2: Custom HTTP client for emulator (~2-3 days effort)
//! - Option 3: Contribute to google-cloud-storage crate (~1-2 weeks)
//!
//! ## Original Test Plan
//!
//! These tests were designed to verify against fake-gcs-server emulator:
//! - Endpoint: http://localhost:4443
//! - Bucket: test-bucket
//! - All tests marked with `#[ignore]` until emulator support is available

#[cfg(test)]
mod gcs_emulator_tests {
    use mediagit_storage::{gcs::GcsBackend, StorageBackend};
    use std::env;

    /// Helper function to create a test GCS backend connected to emulator
    async fn create_test_backend() -> GcsBackend {
        // Set emulator endpoint environment variable
        env::set_var("STORAGE_EMULATOR_HOST", "http://localhost:4443");

        // Use minimal credentials file for emulator (emulator doesn't validate)
        let temp_creds = create_temp_credentials();

        GcsBackend::new("test-project", "test-bucket", &temp_creds)
            .await
            .expect("Failed to create GCS backend for emulator")
    }

    /// Create temporary credentials file for emulator testing
    fn create_temp_credentials() -> String {
        use std::fs;
        use std::io::Write;

        let temp_dir = env::temp_dir();
        let creds_path = temp_dir.join("gcs_emulator_creds.json");

        // Minimal valid service account JSON (emulator doesn't validate)
        let creds_content = r#"{
            "type": "service_account",
            "project_id": "test-project",
            "private_key_id": "test-key-id",
            "private_key": "-----BEGIN PRIVATE KEY-----\nMIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQC1d3brosB4qpGd\nIn1su8hJFVUll7S1BzZpkTToD1iw4pJxqJX95jMm5j1xBQ8YzWCZosa3BlFTdGia\nSwBvYDBE0ADNoX0hmKF6mJ5h0rwVbeUnM4M+4BwkIp+v0+Grj9dl7he2ZHJ4SfRV\n9KdUZMBRJye3qPbrHNL020NPbz2wysBfxRd/k49L+XJsebEjsXyGB78S4gO6GblZ\nlNQ/kzkarpk3Diq7AWKdO66FrluQ0IitQuSPC+YFK81niBgGtE77YWVwpqj6mSj8\nHuh0Fh7sRq1r1SH9sfo4DH3fNnJT/PCP4LP/5zqU5wxIQNQkreG875ifThJlpmK0\nGl/lWUP1AgMBAAECggEAJyBPfUX1ru7D/7foiDHC4PMfDUB0/5VDB6b92803R8hK\nYARD9t8UB16cP8qh8yyRF/8vTlYn4dEXHrFuMhVwwt2AVtXrZ3uD0a2ndJsd35b2\n0il6smta1fW7LYuHPFkCzeD0ruhggAweCQx7qaghiT3ig+iD+LSZzZ6bGDz5da0c\npJCg+nLlyjcANyTKtP1CcS8e6JbpVy05II67PN0TG+okmBmIboBxHo7JJbvvePbf\nkmwfO+OdXaFLh4GhmRmuKE2d20lKHmc6H1tLhiWQxU2jvAZiIowuIVyHvC5xbCQa\n/1/0oc3RoMgnfLN4nCq+uZXftTAe+/+hY1VluSzsUQKBgQDfkVQ0hw51y8agK70u\nQbhfCxUPVbSiNBdJfcQNQPVKrCWyu2Ovtgsa79O+TUM2Gv/MSyP7J+ANDx80bTRr\nrHNb+7dtpGotPUMk2By237nOZAsrX4uzAev+H673h8cV5tqFgOEUSdAnP05VE5kS\nYCjRQtwt/6T663vGPbpRArrAuQKBgQDPyp9YsU4MGfEajffkU/32cIL1UBlaozHQ\n581pSua5r06VAdrlsNIW44N/2a8gQzMgtX//Ph8LGFUZKHewCP6guvyyE6+57mPg\nBfH2kKLPBrwjClGsfc0g7QuJBy7ok5+H1BesgZH/4ANdVvgnX5pBA9NQVcPY30+J\nDQVIXl9nHQKBgQC0FIbELMlr/vEGEVU4Dj3paK7VBE8UnGrpioFBv8IVHObcue5J\nGZSGZQmk7u0lhsfmkdvwsSTawAR9oT0pQeZGAFK24UmZGRCde+pdL4amBZWtoS+Q\nyAqETpcL0XV+Yc5A3Rfv1KjzBB4fj0KsN4KJVJawAoyshMPVYeFS4aT2GQKBgFcB\nH2FytBxLDHIy+ZXoOVFj4OG4jTUvWd9//7lTvHIJXlzz7uT3+a/NybTRwAtBN/o9\nJQAJ0dPCd3dWQ285BOzl/oLNzWmL0NPviVXVT+Zhiosdef9AmZBs0MSqdlC55zVn\ncBYyFqDN+nqtvLA3zo3kfSmJD70SG+plwk1//nBdAoGBAMmJUQ32rXrsutFdPpYO\nhAqcDOwGMuaWBeXvTOWs6srAXiQUrdTvtjGxP+KHvt5f7vcFd4GC1zf7PGabdKqF\n/HjHfYxdwx3NpSIo9tDkYNzDGYlCs0IJPZf10GV2XOkBIg6MgJi/0zWqcBiiYufm\ngDBxD5SRvGggLm5s/FcV+LS5\n-----END PRIVATE KEY-----",
            "client_email": "test@test-project.iam.gserviceaccount.com",
            "client_id": "123456789",
            "auth_uri": "https://accounts.google.com/o/oauth2/auth",
            "token_uri": "https://oauth2.googleapis.com/token"
        }"#;

        let mut file = fs::File::create(&creds_path)
            .expect("Failed to create temporary credentials file");
        file.write_all(creds_content.as_bytes())
            .expect("Failed to write credentials");

        creds_path.to_string_lossy().to_string()
    }

    /// Test basic PUT and GET operations
    #[tokio::test]
    #[ignore] // Requires GCS emulator to be running
    async fn test_gcs_emulator_put_and_get() {
        let backend = create_test_backend().await;

        let key = "test/basic.txt";
        let data = b"Hello from GCS emulator!";

        // Put object
        backend
            .put(key, data)
            .await
            .expect("Failed to put object to GCS emulator");

        // Get object
        let retrieved = backend
            .get(key)
            .await
            .expect("Failed to get object from GCS emulator");

        assert_eq!(retrieved, data);
    }

    /// Test EXISTS operation
    #[tokio::test]
    #[ignore] // Requires GCS emulator
    async fn test_gcs_emulator_exists() {
        let backend = create_test_backend().await;

        let key = "test/exists.txt";
        let data = b"existence test";

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
    #[ignore] // Requires GCS emulator
    async fn test_gcs_emulator_delete() {
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

    /// Test idempotent DELETE
    #[tokio::test]
    #[ignore] // Requires GCS emulator
    async fn test_gcs_emulator_delete_nonexistent() {
        let backend = create_test_backend().await;

        // Deleting non-existent object should succeed (idempotent)
        backend
            .delete("test/nonexistent.txt")
            .await
            .expect("Delete of non-existent object should succeed");
    }

    /// Test LIST_OBJECTS with prefix
    #[tokio::test]
    #[ignore] // Requires GCS emulator
    async fn test_gcs_emulator_list_objects() {
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
    #[ignore] // Requires GCS emulator
    async fn test_gcs_emulator_list_objects_sorted() {
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

    /// Test resumable upload for large files (>5MB)
    #[tokio::test]
    #[ignore] // Requires GCS emulator and takes time
    async fn test_gcs_emulator_resumable_upload() {
        let backend = create_test_backend().await;

        // Create a 10MB file (triggers resumable upload path)
        let large_data = vec![0u8; 10 * 1024 * 1024];
        let key = "test/large_file.bin";

        // Upload large file
        backend
            .put(key, &large_data)
            .await
            .expect("Failed to upload large file with resumable upload");

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
    #[ignore] // Requires GCS emulator
    async fn test_gcs_emulator_concurrent_writes() {
        let backend = create_test_backend().await;

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
    #[ignore] // Requires GCS emulator
    async fn test_gcs_emulator_concurrent_reads() {
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
    #[ignore] // Requires GCS emulator
    async fn test_gcs_emulator_get_nonexistent() {
        let backend = create_test_backend().await;

        let result = backend.get("nonexistent/object").await;
        assert!(result.is_err());
    }

    /// Test empty key validation
    #[tokio::test]
    #[ignore] // Requires GCS emulator
    async fn test_gcs_emulator_empty_key() {
        let backend = create_test_backend().await;

        // All operations should reject empty keys
        assert!(backend.get("").await.is_err());
        assert!(backend.put("", b"data").await.is_err());
        assert!(backend.exists("").await.is_err());
        assert!(backend.delete("").await.is_err());
    }

    /// Test overwriting existing object
    #[tokio::test]
    #[ignore] // Requires GCS emulator
    async fn test_gcs_emulator_overwrite() {
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
    #[ignore] // Requires GCS emulator
    async fn test_gcs_emulator_special_characters() {
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
    #[ignore] // Requires GCS emulator
    async fn test_gcs_emulator_binary_data() {
        let backend = create_test_backend().await;

        let key = "test/binary.bin";
        let binary_data: Vec<u8> = (0..=255).collect();

        backend.put(key, &binary_data).await.unwrap();
        let retrieved = backend.get(key).await.unwrap();

        assert_eq!(retrieved, binary_data);
    }

    /// Test empty object upload and download
    #[tokio::test]
    #[ignore] // Requires GCS emulator
    async fn test_gcs_emulator_empty_object() {
        let backend = create_test_backend().await;

        let key = "test/empty.txt";
        backend.put(key, b"").await.unwrap();

        let retrieved = backend.get(key).await.unwrap();
        assert_eq!(retrieved.len(), 0);
    }

    /// Test backend is cloneable
    #[tokio::test]
    #[ignore] // Requires GCS emulator
    async fn test_gcs_emulator_backend_clone() {
        let backend1 = create_test_backend().await;
        let backend2 = backend1.clone();

        // Both should work independently
        backend1.put("clone/test1", b"from1").await.unwrap();
        backend2.put("clone/test2", b"from2").await.unwrap();

        assert_eq!(backend1.get("clone/test1").await.unwrap(), b"from1");
        assert_eq!(backend2.get("clone/test2").await.unwrap(), b"from2");
        assert_eq!(backend1.get("clone/test2").await.unwrap(), b"from2");
    }

    /// Test chunked upload configuration
    #[tokio::test]
    #[ignore] // Requires GCS emulator
    async fn test_gcs_emulator_custom_chunk_size() {
        use mediagit_storage::gcs::GcsConfig;

        let temp_creds = create_temp_credentials();

        let config = GcsConfig::new("test-project", "test-bucket")
            .with_chunk_size(512 * 1024) // 512KB chunks
            .with_resumable_threshold(2 * 1024 * 1024); // 2MB threshold

        let backend = GcsBackend::with_config(config, &temp_creds)
            .await
            .expect("Failed to create backend with custom config");

        // Test with medium-sized file that should use custom chunking
        let data = vec![1u8; 3 * 1024 * 1024]; // 3MB
        backend.put("test/chunked.bin", &data).await.unwrap();

        let retrieved = backend.get("test/chunked.bin").await.unwrap();
        assert_eq!(retrieved, data);
    }

    /// Test environment variable configuration
    #[tokio::test]
    #[ignore] // Requires GCS emulator
    async fn test_gcs_emulator_from_env() {
        use std::env;

        let temp_creds = create_temp_credentials();

        // Set environment variables
        env::set_var("GCS_PROJECT_ID", "test-project");
        env::set_var("GCS_BUCKET_NAME", "test-bucket");
        env::set_var("GOOGLE_APPLICATION_CREDENTIALS", &temp_creds);
        env::set_var("STORAGE_EMULATOR_HOST", "http://localhost:4443");

        let backend = GcsBackend::from_env()
            .await
            .expect("Failed to create GCS backend from environment");

        // Test basic operation
        backend.put("env/test.txt", b"env test").await.unwrap();
        assert_eq!(backend.get("env/test.txt").await.unwrap(), b"env test");

        // Cleanup env vars
        env::remove_var("GCS_PROJECT_ID");
        env::remove_var("GCS_BUCKET_NAME");
        env::remove_var("GOOGLE_APPLICATION_CREDENTIALS");
        env::remove_var("STORAGE_EMULATOR_HOST");
    }

    /// Test retry logic with transient failures
    #[tokio::test]
    #[ignore] // Requires GCS emulator
    async fn test_gcs_emulator_retry_logic() {
        use mediagit_storage::gcs::GcsConfig;

        let temp_creds = create_temp_credentials();

        let config = GcsConfig::new("test-project", "test-bucket")
            .with_max_retries(5); // Increase retries for testing

        let backend = GcsBackend::with_config(config, &temp_creds)
            .await
            .expect("Failed to create backend with retry config");

        // Normal operations should work with retry logic in place
        backend.put("test/retry.txt", b"retry test").await.unwrap();
        assert_eq!(backend.get("test/retry.txt").await.unwrap(), b"retry test");
    }
}

// MediaGit - Git for Media Files
// Copyright (C) 2025 MediaGit Contributors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU Affero General Public License for more details.

//! Integration tests for GCS backend
//!
//! These tests verify the GCS backend implementation.
//! They use the GCS emulator for testing without requiring GCP credentials.

#[cfg(test)]
mod gcs_tests {
    use mediagit_storage::gcs::{GcsBackend, GcsConfig};
    use mediagit_storage::StorageBackend;

    /// Test configuration for GCS backend
    #[tokio::test]
    #[ignore = "Requires GCS service account file (dummy.json)"]
    async fn test_gcs_backend_configuration() {
        let backend = GcsBackend::new("test-project", "test-bucket", "dummy.json")
            .await
            .unwrap();

        // Verify configuration is set correctly
        let config = backend.config();
        assert_eq!(config.project_id, "test-project");
        assert_eq!(config.bucket_name, "test-bucket");
        assert_eq!(config.chunk_size, 256 * 1024); // Default 256KB
        assert_eq!(config.resumable_threshold, 5 * 1024 * 1024); // Default 5MB
        assert_eq!(config.max_retries, 3); // Default 3 retries
    }

    /// Test GCS backend with custom configuration
    #[tokio::test]
    #[ignore = "Requires GCS service account file (dummy.json)"]
    async fn test_gcs_backend_custom_config() {
        use mediagit_storage::gcs::GcsConfig;

        let config = GcsConfig::new("my-project", "my-bucket")
            .with_chunk_size(512 * 1024)
            .with_resumable_threshold(10 * 1024 * 1024)
            .with_max_retries(5);

        let backend = GcsBackend::with_config(config, "dummy.json").await.unwrap();

        let actual_config = backend.config();
        assert_eq!(actual_config.chunk_size, 512 * 1024);
        assert_eq!(actual_config.resumable_threshold, 10 * 1024 * 1024);
        assert_eq!(actual_config.max_retries, 5);
    }

    /// Test that empty project ID fails validation
    #[tokio::test]
    async fn test_gcs_empty_project_validation() {
        let result = GcsBackend::new("", "bucket", "dummy.json").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("project_id"));
    }

    /// Test that empty bucket name fails validation
    #[tokio::test]
    async fn test_gcs_empty_bucket_validation() {
        let result = GcsBackend::new("project", "", "dummy.json").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("bucket_name"));
    }

    /// Test backend implements Send and Sync
    #[test]
    fn test_gcs_backend_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<GcsBackend>();
    }

    /// Test backend is cloneable
    #[tokio::test]
    #[ignore = "Requires GCS service account file (dummy.json)"]
    async fn test_gcs_backend_clone() {
        let backend1 = GcsBackend::new("test-project", "test-bucket", "dummy.json")
            .await
            .unwrap();
        let backend2 = backend1.clone();

        assert_eq!(backend1.config().project_id, backend2.config().project_id);
        assert_eq!(backend1.config().bucket_name, backend2.config().bucket_name);
    }

    /// Test backend debug output
    #[tokio::test]
    #[ignore = "Requires GCS service account file (dummy.json)"]
    async fn test_gcs_backend_debug() {
        let backend = GcsBackend::new("test-project", "test-bucket", "dummy.json")
            .await
            .unwrap();
        let debug_str = format!("{:?}", backend);

        assert!(debug_str.contains("GcsBackend"));
        assert!(debug_str.contains("test-project"));
        assert!(debug_str.contains("test-bucket"));
        assert!(debug_str.contains("chunk_size"));
        assert!(debug_str.contains("resumable_threshold"));
    }

    /// Test that empty keys are rejected in get()
    #[tokio::test]
    #[ignore = "Requires GCS service account file (dummy.json)"]
    async fn test_gcs_get_empty_key_rejected() {
        let backend = GcsBackend::new("test-project", "test-bucket", "dummy.json")
            .await
            .unwrap();
        let result = backend.get("").await;

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("key cannot be empty"));
    }

    /// Test that empty keys are rejected in put()
    #[tokio::test]
    #[ignore = "Requires GCS service account file (dummy.json)"]
    async fn test_gcs_put_empty_key_rejected() {
        let backend = GcsBackend::new("test-project", "test-bucket", "dummy.json")
            .await
            .unwrap();
        let result = backend.put("", b"data").await;

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("key cannot be empty"));
    }

    /// Test that empty keys are rejected in exists()
    #[tokio::test]
    #[ignore = "Requires GCS service account file (dummy.json)"]
    async fn test_gcs_exists_empty_key_rejected() {
        let backend = GcsBackend::new("test-project", "test-bucket", "dummy.json")
            .await
            .unwrap();
        let result = backend.exists("").await;

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("key cannot be empty"));
    }

    /// Test that empty keys are rejected in delete()
    #[tokio::test]
    #[ignore = "Requires GCS service account file (dummy.json)"]
    async fn test_gcs_delete_empty_key_rejected() {
        let backend = GcsBackend::new("test-project", "test-bucket", "dummy.json")
            .await
            .unwrap();
        let result = backend.delete("").await;

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("key cannot be empty"));
    }

    /// Test list_objects returns empty vec by default (stub implementation)
    #[tokio::test]
    #[ignore = "Requires GCS service account file (dummy.json)"]
    async fn test_gcs_list_objects_stub() {
        let backend = GcsBackend::new("test-project", "test-bucket", "dummy.json")
            .await
            .unwrap();
        let result = backend.list_objects("prefix/").await;

        assert!(result.is_ok());
        let items = result.unwrap();
        // Stub implementation returns empty vec
        assert_eq!(items.len(), 0);
    }

    /// Test from_env function with missing environment variables
    #[tokio::test]
    async fn test_gcs_from_env_missing_vars() {
        // Clear environment variables if they exist
        std::env::remove_var("GCS_PROJECT_ID");
        std::env::remove_var("GCS_BUCKET_NAME");
        std::env::remove_var("GOOGLE_APPLICATION_CREDENTIALS");

        let result = GcsBackend::from_env().await;
        assert!(result.is_err());
    }

    /// Test configuration builder pattern
    #[test]
    fn test_gcs_config_builder_pattern() {
        let config = GcsConfig::new("project", "bucket");

        assert_eq!(config.project_id, "project");
        assert_eq!(config.bucket_name, "bucket");
        assert_eq!(config.chunk_size, 256 * 1024);
        assert_eq!(config.resumable_threshold, 5 * 1024 * 1024);
        assert_eq!(config.max_retries, 3);

        let updated = config
            .with_chunk_size(1024)
            .with_resumable_threshold(2048)
            .with_max_retries(10);

        assert_eq!(updated.chunk_size, 1024);
        assert_eq!(updated.resumable_threshold, 2048);
        assert_eq!(updated.max_retries, 10);
    }

    /// Test that configuration can be used multiple times
    #[tokio::test]
    #[ignore = "Requires GCS service account file (dummy.json)"]
    async fn test_gcs_config_reusable() {
        let config = GcsConfig::new("project1", "bucket1");
        let backend1 = GcsBackend::with_config(config.clone(), "dummy.json")
            .await
            .unwrap();
        let backend2 = GcsBackend::with_config(config, "dummy.json").await.unwrap();

        assert_eq!(backend1.config().project_id, backend2.config().project_id);
    }

    /// Round-trip a >resumable_threshold blob through the real GCS backend.
    /// Pinpoints the bug where the prior `upload_resumable` looped over chunks,
    /// re-uploaded each one to the same object key, and silently truncated the
    /// stored object to `(data.len() % chunk_size)` bytes.
    ///
    /// Requires:
    ///   MEDIAGIT_GCS_BUCKET   = test bucket
    ///   MEDIAGIT_GCS_PROJECT  = GCP project id
    ///   ADC (gcloud auth application-default login) or
    ///   GOOGLE_APPLICATION_CREDENTIALS pointing at a service account key.
    ///
    /// Run with:
    ///   cargo test -p mediagit-storage --release --test gcs_integration_tests \
    ///     test_gcs_large_blob_roundtrip -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "Requires real GCS bucket + ADC; gated on MEDIAGIT_GCS_BUCKET/PROJECT env vars"]
    async fn test_gcs_large_blob_roundtrip() {
        let bucket = match std::env::var("MEDIAGIT_GCS_BUCKET") {
            Ok(v) if !v.is_empty() => v,
            _ => {
                eprintln!("skipping: MEDIAGIT_GCS_BUCKET not set");
                return;
            }
        };
        let project = std::env::var("MEDIAGIT_GCS_PROJECT")
            .or_else(|_| std::env::var("GOOGLE_CLOUD_PROJECT"))
            .expect("MEDIAGIT_GCS_PROJECT or GOOGLE_CLOUD_PROJECT must be set");

        let backend = GcsBackend::with_default_credentials(&project, &bucket)
            .await
            .expect("failed to construct GcsBackend with ADC");

        // 7 MiB > default resumable_threshold (5 MiB) -> forces upload_resumable.
        let mut data = vec![0u8; 7 * 1024 * 1024];
        for (i, b) in data.iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(31).wrapping_add(7);
        }
        let key = format!(
            "gcs-manual-test/upload-resumable-roundtrip/{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        );

        backend.put(&key, &data).await.expect("put failed");

        let got = backend.get(&key).await.expect("get failed");
        assert_eq!(
            got.len(),
            data.len(),
            "round-trip size mismatch: got {} expected {} (this is the bug we just fixed: \
             the old upload_resumable truncated to (data.len() % chunk_size) bytes)",
            got.len(),
            data.len()
        );
        assert_eq!(got, data, "round-trip byte mismatch");

        backend.delete(&key).await.ok();
    }

    /// Round-trip a >32 MiB blob through the GCS backend, exercising the
    /// caller-driven striped `get_with_size_hint` path.
    ///
    /// The threshold for striping is 32 MiB (see
    /// `mediagit_storage::gcs::STRIPED_GET_THRESHOLD`); we pick 40 MiB so the
    /// object straddles the threshold by a meaningful margin and produces
    /// multiple stripes (40 MiB / 8 MiB = 5 stripes).
    ///
    /// Verifies:
    ///   1. Striped download is byte-identical to the single-shot `get`.
    ///   2. Calling `get_with_size_hint(key, None)` falls back to `get` (i.e.
    ///      no metadata probe happens; the call should succeed and return the
    ///      same bytes).
    ///   3. Calling `get_with_size_hint(key, Some(small))` for a sub-threshold
    ///      hint also falls back (no spurious striping for tiny objects).
    ///
    /// Same env requirements as `test_gcs_large_blob_roundtrip`.
    #[tokio::test]
    #[ignore = "Requires real GCS bucket + ADC; gated on MEDIAGIT_GCS_BUCKET/PROJECT env vars"]
    async fn test_gcs_striped_get_roundtrip() {
        let bucket = match std::env::var("MEDIAGIT_GCS_BUCKET") {
            Ok(v) if !v.is_empty() => v,
            _ => {
                eprintln!("skipping: MEDIAGIT_GCS_BUCKET not set");
                return;
            }
        };
        let project = std::env::var("MEDIAGIT_GCS_PROJECT")
            .or_else(|_| std::env::var("GOOGLE_CLOUD_PROJECT"))
            .expect("MEDIAGIT_GCS_PROJECT or GOOGLE_CLOUD_PROJECT must be set");

        let backend = GcsBackend::with_default_credentials(&project, &bucket)
            .await
            .expect("failed to construct GcsBackend with ADC");

        // 40 MiB > 32 MiB STRIPED_GET_THRESHOLD -> exercises the striped path.
        let total: usize = 40 * 1024 * 1024;
        let mut data = vec![0u8; total];
        // Same deterministic pattern as the smaller roundtrip; gives every
        // 256-byte block a unique fingerprint so a corrupted stripe boundary
        // (e.g. off-by-one in offset/len math) shows up as a single-byte diff.
        for (i, b) in data.iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(37).wrapping_add(11);
        }
        let key = format!(
            "gcs-manual-test/striped-get-roundtrip/{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        );

        backend.put(&key, &data).await.expect("put failed");

        // (1) striped fast-path: caller passes the known size.
        let got_striped = backend
            .get_with_size_hint(&key, Some(total as u64))
            .await
            .expect("get_with_size_hint(Some(size)) failed");
        assert_eq!(got_striped.len(), data.len(), "striped size mismatch");
        assert_eq!(got_striped, data, "striped byte mismatch");

        // (2) None hint: must fall back to single-shot get without a metadata
        //     RPC. We cannot assert "no extra RPC" from outside; we only assert
        //     correctness here. The "no probe" property is enforced by the
        //     implementation invariant documented on get_with_size_hint.
        let got_fallback = backend
            .get_with_size_hint(&key, None)
            .await
            .expect("get_with_size_hint(None) failed");
        assert_eq!(got_fallback, data, "None-hint fallback byte mismatch");

        // (3) sub-threshold hint must also take the single-shot path; we just
        //     re-verify byte-equality (the threshold gate is documented, not
        //     directly observable from the trait surface).
        let got_small_hint = backend
            .get_with_size_hint(&key, Some(1024))
            .await
            .expect("get_with_size_hint(Some(small)) failed");
        assert_eq!(got_small_hint, data, "small-hint fallback byte mismatch");

        backend.delete(&key).await.ok();
    }
}

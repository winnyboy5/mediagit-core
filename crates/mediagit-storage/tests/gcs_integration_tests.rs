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

        assert_eq!(
            backend1.config().project_id,
            backend2.config().project_id
        );
        assert_eq!(
            backend1.config().bucket_name,
            backend2.config().bucket_name
        );
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
        assert!(result.unwrap_err().to_string().contains("key cannot be empty"));
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
        assert!(result.unwrap_err().to_string().contains("key cannot be empty"));
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
        assert!(result.unwrap_err().to_string().contains("key cannot be empty"));
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
        assert!(result.unwrap_err().to_string().contains("key cannot be empty"));
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
        let backend2 = GcsBackend::with_config(config, "dummy.json")
            .await
            .unwrap();

        assert_eq!(
            backend1.config().project_id,
            backend2.config().project_id
        );
    }
}

// MediaGit - Git for Media Files
// Copyright (C) 2025 MediaGit Contributors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

//! Integrity verification for migrated objects
//!
//! This module provides checksum verification and metadata comparison
//! to ensure data integrity during migration.

use anyhow::{Context, Result};
use mediagit_storage::StorageBackend;
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// Object metadata for verification
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectMetadata {
    /// Object key
    pub key: String,

    /// Object size in bytes
    pub size: usize,

    /// SHA-256 checksum (hex encoded)
    pub checksum: String,
}

/// Verification result for a single object
#[derive(Debug, Clone)]
pub struct VerificationResult {
    /// Object key
    pub key: String,

    /// Whether verification passed
    pub passed: bool,

    /// Error message if verification failed
    pub error: Option<String>,

    /// Source checksum
    pub source_checksum: Option<String>,

    /// Target checksum
    pub target_checksum: Option<String>,

    /// Source size
    pub source_size: Option<usize>,

    /// Target size
    pub target_size: Option<usize>,
}

impl VerificationResult {
    /// Create a successful verification result
    pub fn success(key: String, checksum: String, size: usize) -> Self {
        Self {
            key,
            passed: true,
            error: None,
            source_checksum: Some(checksum.clone()),
            target_checksum: Some(checksum),
            source_size: Some(size),
            target_size: Some(size),
        }
    }

    /// Create a failed verification result
    pub fn failure(
        key: String,
        error: String,
        source_checksum: Option<String>,
        target_checksum: Option<String>,
        source_size: Option<usize>,
        target_size: Option<usize>,
    ) -> Self {
        Self {
            key,
            passed: false,
            error: Some(error),
            source_checksum,
            target_checksum,
            source_size,
            target_size,
        }
    }
}

/// Integrity verifier
pub struct IntegrityVerifier {
    source: Arc<dyn StorageBackend>,
    target: Arc<dyn StorageBackend>,
}

impl IntegrityVerifier {
    /// Create a new integrity verifier
    pub fn new(source: Arc<dyn StorageBackend>, target: Arc<dyn StorageBackend>) -> Self {
        Self { source, target }
    }

    /// Compute SHA-256 checksum of data
    pub fn compute_checksum(data: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(data);
        hex::encode(hasher.finalize())
    }

    /// Get metadata for an object
    pub async fn get_metadata(
        backend: &dyn StorageBackend,
        key: &str,
    ) -> Result<ObjectMetadata> {
        let data = backend.get(key).await
            .with_context(|| format!("Failed to get object: {}", key))?;

        let checksum = Self::compute_checksum(&data);
        let size = data.len();

        Ok(ObjectMetadata {
            key: key.to_string(),
            checksum,
            size,
        })
    }

    /// Verify a single object migration
    pub async fn verify_object(&self, key: &str) -> Result<VerificationResult> {
        // Get source metadata
        let source_meta = match Self::get_metadata(&*self.source, key).await {
            Ok(meta) => meta,
            Err(e) => {
                return Ok(VerificationResult::failure(
                    key.to_string(),
                    format!("Failed to get source object: {}", e),
                    None,
                    None,
                    None,
                    None,
                ));
            }
        };

        // Get target metadata
        let target_meta = match Self::get_metadata(&*self.target, key).await {
            Ok(meta) => meta,
            Err(e) => {
                return Ok(VerificationResult::failure(
                    key.to_string(),
                    format!("Failed to get target object: {}", e),
                    Some(source_meta.checksum),
                    None,
                    Some(source_meta.size),
                    None,
                ));
            }
        };

        // Compare checksums
        if source_meta.checksum != target_meta.checksum {
            return Ok(VerificationResult::failure(
                key.to_string(),
                "Checksum mismatch".to_string(),
                Some(source_meta.checksum),
                Some(target_meta.checksum),
                Some(source_meta.size),
                Some(target_meta.size),
            ));
        }

        // Compare sizes
        if source_meta.size != target_meta.size {
            return Ok(VerificationResult::failure(
                key.to_string(),
                "Size mismatch".to_string(),
                Some(source_meta.checksum.clone()),
                Some(target_meta.checksum),
                Some(source_meta.size),
                Some(target_meta.size),
            ));
        }

        Ok(VerificationResult::success(
            key.to_string(),
            source_meta.checksum,
            source_meta.size,
        ))
    }

    /// Verify all migrated objects
    pub async fn verify_all(&self, keys: &[String]) -> Result<Vec<VerificationResult>> {
        let mut results = Vec::new();

        for key in keys {
            let result = self.verify_object(key).await?;
            results.push(result);
        }

        Ok(results)
    }

    /// Verify migration completeness
    ///
    /// Checks that all objects from source exist in target
    pub async fn verify_completeness(&self, prefix: &str) -> Result<Vec<String>> {
        let source_keys = self.source.list_objects(prefix).await
            .context("Failed to list source objects")?;

        let target_keys = self.target.list_objects(prefix).await
            .context("Failed to list target objects")?;

        let target_set: std::collections::HashSet<_> = target_keys.into_iter().collect();

        let missing: Vec<String> = source_keys
            .into_iter()
            .filter(|key| !target_set.contains(key))
            .collect();

        Ok(missing)
    }
}

/// Verification report
#[derive(Debug)]
pub struct VerificationReport {
    /// Total objects verified
    pub total: usize,

    /// Number of successful verifications
    pub passed: usize,

    /// Number of failed verifications
    pub failed: usize,

    /// Failed verification details
    pub failures: Vec<VerificationResult>,
}

impl VerificationReport {
    /// Create a report from verification results
    pub fn from_results(results: Vec<VerificationResult>) -> Self {
        let total = results.len();
        let passed = results.iter().filter(|r| r.passed).count();
        let failed = total - passed;
        let failures: Vec<_> = results.into_iter().filter(|r| !r.passed).collect();

        Self {
            total,
            passed,
            failed,
            failures,
        }
    }

    /// Check if all verifications passed
    pub fn all_passed(&self) -> bool {
        self.failed == 0
    }

    /// Format the report as a string
    pub fn format(&self) -> String {
        let mut output = String::new();
        output.push_str(&format!(
            "Verification Report\n\
             Total: {}\n\
             Passed: {}\n\
             Failed: {}\n",
            self.total, self.passed, self.failed
        ));

        if !self.failures.is_empty() {
            output.push_str("\nFailures:\n");
            for failure in &self.failures {
                output.push_str(&format!("  - {}: {}\n",
                    failure.key,
                    failure.error.as_deref().unwrap_or("Unknown error")
                ));

                if let (Some(src), Some(tgt)) = (&failure.source_checksum, &failure.target_checksum) {
                    output.push_str(&format!("    Source checksum: {}\n", src));
                    output.push_str(&format!("    Target checksum: {}\n", tgt));
                }

                if let (Some(src), Some(tgt)) = (failure.source_size, failure.target_size) {
                    output.push_str(&format!("    Source size: {} bytes\n", src));
                    output.push_str(&format!("    Target size: {} bytes\n", tgt));
                }
            }
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mediagit_storage::mock::MockBackend;

    #[tokio::test]
    async fn test_checksum_computation() {
        let data = b"Hello, World!";
        let checksum = IntegrityVerifier::compute_checksum(data);

        // Expected SHA-256 hash of "Hello, World!"
        assert_eq!(
            checksum,
            "dffd6021bb2bd5b0af676290809ec3a53191dd81c7f70a4b28688a362182986f"
        );
    }

    #[tokio::test]
    async fn test_verify_object_success() {
        let source = Arc::new(MockBackend::new());
        let target = Arc::new(MockBackend::new());

        let data = b"test data";
        source.put("test_key", data).await.unwrap();
        target.put("test_key", data).await.unwrap();

        let verifier = IntegrityVerifier::new(source, target);
        let result = verifier.verify_object("test_key").await.unwrap();

        assert!(result.passed);
        assert!(result.error.is_none());
    }

    #[tokio::test]
    async fn test_verify_object_checksum_mismatch() {
        let source = Arc::new(MockBackend::new());
        let target = Arc::new(MockBackend::new());

        source.put("test_key", b"source data").await.unwrap();
        target.put("test_key", b"target data").await.unwrap();

        let verifier = IntegrityVerifier::new(source, target);
        let result = verifier.verify_object("test_key").await.unwrap();

        assert!(!result.passed);
        assert!(result.error.as_ref().unwrap().contains("Checksum mismatch"));
    }

    #[tokio::test]
    async fn test_verification_report() {
        let results = vec![
            VerificationResult::success("obj1".to_string(), "hash1".to_string(), 100),
            VerificationResult::success("obj2".to_string(), "hash2".to_string(), 200),
            VerificationResult::failure(
                "obj3".to_string(),
                "Checksum mismatch".to_string(),
                Some("hash3a".to_string()),
                Some("hash3b".to_string()),
                Some(300),
                Some(300),
            ),
        ];

        let report = VerificationReport::from_results(results);

        assert_eq!(report.total, 3);
        assert_eq!(report.passed, 2);
        assert_eq!(report.failed, 1);
        assert!(!report.all_passed());
    }
}

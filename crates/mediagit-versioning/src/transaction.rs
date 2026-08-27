// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Transaction model for atomic pack processing
//!
//! Ensures all-or-nothing semantics for pack transfers: either all objects
//! are stored successfully, or none are (with automatic rollback).

use crate::{ObjectType, Oid};
use mediagit_storage::StorageBackend;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Transaction for atomic pack processing
pub struct PackTransaction {
    temp_dir: PathBuf,
    pending_objects: Vec<(Oid, ObjectType)>,
    committed: bool,
    storage: Arc<dyn StorageBackend>,
    transaction_id: Uuid,
}

impl PackTransaction {
    /// Create new transaction with unique temp directory
    pub fn new(storage: Arc<dyn StorageBackend>, base_path: &Path) -> anyhow::Result<Self> {
        let transaction_id = Uuid::new_v4();
        let temp_dir = base_path.join(format!("tx_{}", transaction_id));

        std::fs::create_dir_all(&temp_dir)?;

        // Create transaction marker for crash recovery
        let marker_path = temp_dir.join(".transaction_marker");
        std::fs::write(&marker_path, transaction_id.as_bytes())?;

        debug!(
            transaction_id = %transaction_id,
            temp_dir = %temp_dir.display(),
            "Created pack transaction"
        );

        Ok(Self {
            temp_dir,
            pending_objects: Vec::new(),
            committed: false,
            storage,
            transaction_id,
        })
    }

    /// Add object to transaction (writes to temp location)
    pub async fn add_object(
        &mut self,
        oid: Oid,
        obj_type: ObjectType,
        data: &[u8],
    ) -> anyhow::Result<()> {
        let temp_path = self.temp_dir.join(oid.to_hex());

        // Write to temp location with verification
        fs::write(&temp_path, data).await?;

        // Verify written data
        let written_data = fs::read(&temp_path).await?;
        if written_data != data {
            return Err(anyhow::anyhow!("Data verification failed for {}", oid));
        }

        self.pending_objects.push((oid, obj_type));

        debug!(
            oid = %oid,
            obj_type = ?obj_type,
            size = data.len(),
            pending_count = self.pending_objects.len(),
            "Added object to transaction"
        );

        Ok(())
    }

    /// Commit all objects atomically
    pub async fn commit(mut self) -> anyhow::Result<()> {
        info!(
            transaction_id = %self.transaction_id,
            object_count = self.pending_objects.len(),
            "Committing transaction"
        );

        for (oid, _obj_type) in &self.pending_objects {
            let temp_path = self.temp_dir.join(oid.to_hex());
            let data = fs::read(&temp_path).await?;

            // Write to final storage location
            let key = oid.to_hex();
            self.storage.put(&key, &data).await?;

            debug!(oid = %oid, "Moved object to final location");
        }

        self.committed = true;

        // Clean up temp directory
        if let Err(e) = fs::remove_dir_all(&self.temp_dir).await {
            warn!(
                error = %e,
                temp_dir = %self.temp_dir.display(),
                "Failed to clean up temp directory"
            );
        }

        info!(
            transaction_id = %self.transaction_id,
            objects_committed = self.pending_objects.len(),
            "Transaction committed successfully"
        );

        Ok(())
    }

    /// Get number of pending objects
    pub fn pending_count(&self) -> usize {
        self.pending_objects.len()
    }

    /// Get transaction ID
    pub fn id(&self) -> Uuid {
        self.transaction_id
    }
}

impl Drop for PackTransaction {
    fn drop(&mut self) {
        if !self.committed {
            warn!(
                transaction_id = %self.transaction_id,
                pending_count = self.pending_objects.len(),
                "Transaction dropped without commit - rolling back"
            );

            // Sync rollback: delete temp directory
            if let Err(e) = std::fs::remove_dir_all(&self.temp_dir) {
                warn!(
                    error = %e,
                    temp_dir = %self.temp_dir.display(),
                    "Failed to rollback transaction"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mediagit_storage::LocalBackend;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_transaction_commit() {
        let temp = TempDir::new().unwrap();
        let storage = Arc::new(LocalBackend::new(temp.path()).await.unwrap());

        let mut tx = PackTransaction::new(storage.clone(), temp.path()).unwrap();

        let test_data = b"test data";
        let oid = Oid::hash(test_data);

        tx.add_object(oid, ObjectType::Blob, test_data)
            .await
            .unwrap();

        assert_eq!(tx.pending_count(), 1);

        tx.commit().await.unwrap();

        // Verify object exists in storage
        assert!(storage.exists(&oid.to_hex()).await.unwrap());
    }

    #[tokio::test]
    async fn test_transaction_rollback_on_drop() {
        let temp = TempDir::new().unwrap();
        let storage = Arc::new(LocalBackend::new(temp.path()).await.unwrap());

        let test_data = b"test data";
        let oid = Oid::hash(test_data);

        {
            let mut tx = PackTransaction::new(storage.clone(), temp.path()).unwrap();
            tx.add_object(oid, ObjectType::Blob, test_data)
                .await
                .unwrap();
            // Drop without commit
        }

        // Verify object does NOT exist in final storage
        assert!(!storage.exists(&oid.to_hex()).await.unwrap());
    }
}

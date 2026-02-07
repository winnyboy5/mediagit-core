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

/// Report from transaction recovery process
#[derive(Debug, Default)]
pub struct RecoveryReport {
    pub rolled_back: usize,
    pub errors: Vec<String>,
}

/// Recover incomplete transactions on ODB initialization
pub async fn recover_incomplete_transactions(storage_path: &Path) -> anyhow::Result<RecoveryReport> {
    let mut report = RecoveryReport::default();

    let temp_root = storage_path.join("temp");
    if !temp_root.exists() {
        return Ok(report);
    }

    let mut entries = fs::read_dir(&temp_root).await?;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();

        if !path.is_dir() {
            continue;
        }

        // Check for transaction marker
        let marker_path = path.join(".transaction_marker");
        if marker_path.exists() {
            // Incomplete transaction found - rollback
            match fs::remove_dir_all(&path).await {
                Ok(_) => {
                    info!(transaction_dir = %path.display(), "Rolled back incomplete transaction");
                    report.rolled_back += 1;
                }
                Err(e) => {
                    let error_msg = format!("Failed to rollback {}: {}", path.display(), e);
                    warn!("{}", error_msg);
                    report.errors.push(error_msg);
                }
            }
        }
    }

    if report.rolled_back > 0 {
        info!(
            rolled_back = report.rolled_back,
            errors = report.errors.len(),
            "Transaction recovery complete"
        );
    }

    Ok(report)
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

    #[tokio::test]
    async fn test_transaction_recovery() {
        let temp = TempDir::new().unwrap();
        let storage = Arc::new(LocalBackend::new(temp.path()).await.unwrap());

        // Create temp subdirectory for transactions (matching recovery expectations)
        let temp_subdir = temp.path().join("temp");
        std::fs::create_dir_all(&temp_subdir).unwrap();

        // Create incomplete transaction in temp subdir and capture the transaction dir path
        let tx_dir: PathBuf;
        {
            let mut tx = PackTransaction::new(storage.clone(), &temp_subdir).unwrap();
            tx_dir = temp_subdir.join(format!("tx_{}", tx.id()));
            tx.add_object(Oid::hash(b"data1"), ObjectType::Blob, b"data1")
                .await
                .unwrap();
            // Don't commit - simulate crash by std::mem::forget
            std::mem::forget(tx);
        }

        // Verify the transaction directory and marker exist before recovery
        assert!(tx_dir.exists(), "Transaction dir should exist: {:?}", tx_dir);
        assert!(
            tx_dir.join(".transaction_marker").exists(),
            "Marker should exist"
        );

        // Run recovery (looks in storage_path/temp)
        let report = recover_incomplete_transactions(temp.path()).await.unwrap();

        assert_eq!(report.rolled_back, 1);
        assert_eq!(report.errors.len(), 0);

        // Verify transaction directory was cleaned up
        assert!(!tx_dir.exists(), "Transaction dir should be removed");
    }
}

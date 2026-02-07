// MediaGit - Git for Media Files
// Copyright (C) 2025 MediaGit Contributors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

//! Migration state management with JSON persistence
//!
//! This module handles tracking migration progress, enabling resume functionality
//! and rollback capability.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tokio::fs;

/// Migration state persisted to disk
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationState {
    /// Source backend type
    pub source_backend: String,

    /// Target backend type
    pub target_backend: String,

    /// Total number of objects to migrate
    pub total_objects: usize,

    /// Objects successfully migrated (keys)
    pub migrated_objects: HashSet<String>,

    /// Objects that failed migration (key -> error message)
    pub failed_objects: Vec<(String, String)>,

    /// Migration start timestamp
    pub started_at: chrono::DateTime<chrono::Utc>,

    /// Last checkpoint timestamp
    pub last_checkpoint: chrono::DateTime<chrono::Utc>,

    /// Migration status
    pub status: MigrationStatus,

    /// Original backend configuration (for rollback)
    pub original_config: serde_json::Value,
}

/// Migration status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MigrationStatus {
    /// Migration in progress
    InProgress,

    /// Migration completed successfully
    Completed,

    /// Migration failed
    Failed,

    /// Migration paused/interrupted
    Paused,

    /// Migration rolled back
    RolledBack,
}

impl MigrationState {
    /// Create a new migration state
    pub fn new(
        source_backend: String,
        target_backend: String,
        total_objects: usize,
        original_config: serde_json::Value,
    ) -> Self {
        let now = chrono::Utc::now();
        Self {
            source_backend,
            target_backend,
            total_objects,
            migrated_objects: HashSet::new(),
            failed_objects: Vec::new(),
            started_at: now,
            last_checkpoint: now,
            status: MigrationStatus::InProgress,
            original_config,
        }
    }

    /// Mark an object as migrated
    pub fn mark_migrated(&mut self, key: String) {
        self.migrated_objects.insert(key);
        self.last_checkpoint = chrono::Utc::now();
    }

    /// Mark an object as failed
    pub fn mark_failed(&mut self, key: String, error: String) {
        self.failed_objects.push((key, error));
        self.last_checkpoint = chrono::Utc::now();
    }

    /// Get migration progress (0.0 to 1.0)
    pub fn progress(&self) -> f64 {
        if self.total_objects == 0 {
            return 1.0;
        }
        self.migrated_objects.len() as f64 / self.total_objects as f64
    }

    /// Check if an object is already migrated
    pub fn is_migrated(&self, key: &str) -> bool {
        self.migrated_objects.contains(key)
    }

    /// Get the number of objects remaining
    pub fn remaining(&self) -> usize {
        self.total_objects.saturating_sub(self.migrated_objects.len())
    }

    /// Save state to file
    pub async fn save(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self)
            .context("Failed to serialize migration state")?;

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await
                .context("Failed to create state directory")?;
        }

        fs::write(path, json).await
            .context("Failed to write migration state")?;

        Ok(())
    }

    /// Load state from file
    pub async fn load(path: &Path) -> Result<Self> {
        let json = fs::read_to_string(path).await
            .context("Failed to read migration state")?;

        let state: Self = serde_json::from_str(&json)
            .context("Failed to deserialize migration state")?;

        Ok(state)
    }

    /// Check if a state file exists
    pub async fn exists(path: &Path) -> bool {
        tokio::fs::metadata(path).await.is_ok()
    }

    /// Delete state file
    pub async fn delete(path: &Path) -> Result<()> {
        if Self::exists(path).await {
            fs::remove_file(path).await
                .context("Failed to delete migration state")?;
        }
        Ok(())
    }
}

/// State file location manager
pub struct StateManager {
    base_dir: PathBuf,
}

impl StateManager {
    /// Create a new state manager
    pub fn new(repo_path: &Path) -> Self {
        let base_dir = repo_path.join(".mediagit").join("migration");
        Self { base_dir }
    }

    /// Get the path to the current migration state file
    pub fn current_state_path(&self) -> PathBuf {
        self.base_dir.join("state.json")
    }

    /// Get the path to a backup state file
    pub fn backup_state_path(&self, timestamp: &str) -> PathBuf {
        self.base_dir.join(format!("state-backup-{}.json", timestamp))
    }

    /// Create a backup of the current state
    pub async fn backup_current_state(&self) -> Result<()> {
        let current = self.current_state_path();
        if !MigrationState::exists(&current).await {
            return Ok(());
        }

        let timestamp = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
        let backup = self.backup_state_path(&timestamp);

        fs::copy(&current, &backup).await
            .context("Failed to backup migration state")?;

        tracing::info!("Created migration state backup at: {}", backup.display());
        Ok(())
    }

    /// Ensure the migration directory exists
    pub async fn ensure_dir(&self) -> Result<()> {
        fs::create_dir_all(&self.base_dir).await
            .context("Failed to create migration directory")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_state_persistence() {
        let dir = tempdir().unwrap();
        let state_path = dir.path().join("state.json");

        let mut state = MigrationState::new(
            "local".to_string(),
            "s3".to_string(),
            100,
            serde_json::json!({"backend": "local"}),
        );

        state.mark_migrated("obj1".to_string());
        state.mark_migrated("obj2".to_string());
        state.mark_failed("obj3".to_string(), "Network error".to_string());

        // Save and reload
        state.save(&state_path).await.unwrap();
        let loaded = MigrationState::load(&state_path).await.unwrap();

        assert_eq!(loaded.source_backend, "local");
        assert_eq!(loaded.target_backend, "s3");
        assert_eq!(loaded.total_objects, 100);
        assert_eq!(loaded.migrated_objects.len(), 2);
        assert_eq!(loaded.failed_objects.len(), 1);
        assert!(loaded.is_migrated("obj1"));
        assert!(!loaded.is_migrated("obj3"));
    }

    #[test]
    fn test_progress_calculation() {
        let mut state = MigrationState::new(
            "local".to_string(),
            "s3".to_string(),
            100,
            serde_json::json!({}),
        );

        assert_eq!(state.progress(), 0.0);

        for i in 0..50 {
            state.mark_migrated(format!("obj{}", i));
        }

        assert_eq!(state.progress(), 0.5);
        assert_eq!(state.remaining(), 50);
    }

    #[tokio::test]
    async fn test_state_manager() {
        let dir = tempdir().unwrap();
        let manager = StateManager::new(dir.path());

        manager.ensure_dir().await.unwrap();
        assert!(manager.base_dir.exists());

        let state_path = manager.current_state_path();
        // Normalize path separators for cross-platform compatibility (Windows uses \)
        let normalized_path = state_path.to_string_lossy().replace('\\', "/");
        assert!(normalized_path.contains("migration/state.json"));
    }
}

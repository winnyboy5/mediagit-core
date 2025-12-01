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

//! In-memory mock storage backend for testing
//!
//! Provides a thread-safe, in-memory implementation of [`StorageBackend`](crate::StorageBackend)
//! using `Arc<RwLock<HashMap>>` for concurrent access.
//!
//! # Examples
//!
//! ```rust,no_run
//! use mediagit_storage::{StorageBackend, mock::MockBackend};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let storage = MockBackend::new();
//!
//!     // Store data
//!     storage.put("test.bin", b"hello world").await?;
//!
//!     // Retrieve data
//!     let data = storage.get("test.bin").await?;
//!     assert_eq!(data, b"hello world");
//!
//!     // Check existence
//!     assert!(storage.exists("test.bin").await?);
//!
//!     // List objects
//!     let objects = storage.list_objects("test").await?;
//!     assert_eq!(objects.len(), 1);
//!
//!     // Delete object
//!     storage.delete("test.bin").await?;
//!     assert!(!storage.exists("test.bin").await?);
//!
//!     Ok(())
//! }
//! ```

use crate::StorageBackend;
use async_trait::async_trait;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use tokio::sync::RwLock;

/// In-memory mock storage backend for testing
///
/// Thread-safe implementation suitable for unit and integration tests.
/// Uses `Arc<RwLock<HashMap>>` to allow concurrent read/write operations.
///
/// # Thread Safety
///
/// This implementation is `Send + Sync` and can be safely shared across threads
/// and async tasks.
#[derive(Clone)]
pub struct MockBackend {
    store: Arc<RwLock<HashMap<String, Vec<u8>>>>,
}

impl MockBackend {
    /// Create a new empty mock storage backend
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use mediagit_storage::mock::MockBackend;
    ///
    /// let storage = MockBackend::new();
    /// ```
    pub fn new() -> Self {
        MockBackend {
            store: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a mock storage backend with initial data
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use mediagit_storage::mock::MockBackend;
    ///
    /// let mut initial_data = std::collections::HashMap::new();
    /// initial_data.insert("key1".to_string(), vec![1, 2, 3]);
    /// initial_data.insert("key2".to_string(), vec![4, 5, 6]);
    ///
    /// let storage = MockBackend::with_data(initial_data);
    /// ```
    pub fn with_data(initial_data: HashMap<String, Vec<u8>>) -> Self {
        MockBackend {
            store: Arc::new(RwLock::new(initial_data)),
        }
    }

    /// Get the current number of objects stored
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use mediagit_storage::mock::MockBackend;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let storage = MockBackend::new();
    /// assert_eq!(storage.len().await, 0);
    /// # }
    /// ```
    pub async fn len(&self) -> usize {
        self.store.read().await.len()
    }

    /// Check if the storage is empty
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use mediagit_storage::mock::MockBackend;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let storage = MockBackend::new();
    /// assert!(storage.is_empty().await);
    /// # }
    /// ```
    pub async fn is_empty(&self) -> bool {
        self.store.read().await.is_empty()
    }

    /// Clear all stored objects
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use mediagit_storage::mock::MockBackend;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let storage = MockBackend::new();
    /// storage.clear().await;
    /// assert!(storage.is_empty().await);
    /// # }
    /// ```
    pub async fn clear(&self) {
        self.store.write().await.clear();
    }

    /// Get a copy of all stored keys
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use mediagit_storage::mock::MockBackend;
    /// # use mediagit_storage::StorageBackend;
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// let storage = MockBackend::new();
    /// storage.put("a", b"data").await?;
    /// storage.put("b", b"data").await?;
    ///
    /// let keys = storage.keys().await;
    /// assert_eq!(keys.len(), 2);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn keys(&self) -> Vec<String> {
        self.store.read().await.keys().cloned().collect()
    }
}

impl Default for MockBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for MockBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MockBackend").finish()
    }
}

#[async_trait]
impl StorageBackend for MockBackend {
    async fn get(&self, key: &str) -> anyhow::Result<Vec<u8>> {
        if key.is_empty() {
            return Err(anyhow::anyhow!("key cannot be empty"));
        }

        let store = self.store.read().await;
        store
            .get(key)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("object not found: {}", key))
    }

    async fn put(&self, key: &str, data: &[u8]) -> anyhow::Result<()> {
        if key.is_empty() {
            return Err(anyhow::anyhow!("key cannot be empty"));
        }

        let mut store = self.store.write().await;
        store.insert(key.to_string(), data.to_vec());
        Ok(())
    }

    async fn exists(&self, key: &str) -> anyhow::Result<bool> {
        if key.is_empty() {
            return Err(anyhow::anyhow!("key cannot be empty"));
        }

        let store = self.store.read().await;
        Ok(store.contains_key(key))
    }

    async fn delete(&self, key: &str) -> anyhow::Result<()> {
        if key.is_empty() {
            return Err(anyhow::anyhow!("key cannot be empty"));
        }

        let mut store = self.store.write().await;
        store.remove(key);
        Ok(())
    }

    async fn list_objects(&self, prefix: &str) -> anyhow::Result<Vec<String>> {
        let store = self.store.read().await;
        let mut results: Vec<String> = store
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect();
        results.sort();
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StorageBackend;

    #[tokio::test]
    async fn test_new() {
        let backend = MockBackend::new();
        assert!(backend.is_empty().await);
        assert_eq!(backend.len().await, 0);
    }

    #[tokio::test]
    async fn test_put_and_get() {
        let backend = MockBackend::new();
        let data = b"test data";

        backend.put("key1", data).await.unwrap();
        assert_eq!(backend.len().await, 1);

        let retrieved = backend.get("key1").await.unwrap();
        assert_eq!(retrieved, data);
    }

    #[tokio::test]
    async fn test_get_nonexistent() {
        let backend = MockBackend::new();
        let result = backend.get("nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_empty_key_operations() {
        let backend = MockBackend::new();

        assert!(backend.put("", b"data").await.is_err());
        assert!(backend.get("").await.is_err());
        assert!(backend.exists("").await.is_err());
        assert!(backend.delete("").await.is_err());
    }

    #[tokio::test]
    async fn test_exists() {
        let backend = MockBackend::new();

        backend.put("key1", b"data").await.unwrap();
        assert!(backend.exists("key1").await.unwrap());
        assert!(!backend.exists("key2").await.unwrap());
    }

    #[tokio::test]
    async fn test_delete() {
        let backend = MockBackend::new();

        backend.put("key1", b"data").await.unwrap();
        assert!(backend.exists("key1").await.unwrap());

        backend.delete("key1").await.unwrap();
        assert!(!backend.exists("key1").await.unwrap());
        assert_eq!(backend.len().await, 0);
    }

    #[tokio::test]
    async fn test_delete_nonexistent() {
        let backend = MockBackend::new();
        // Should not error, just do nothing
        backend.delete("nonexistent").await.unwrap();
    }

    #[tokio::test]
    async fn test_list_objects() {
        let backend = MockBackend::new();

        backend.put("images/photo1.jpg", b"data1").await.unwrap();
        backend.put("images/photo2.jpg", b"data2").await.unwrap();
        backend.put("videos/video1.mp4", b"data3").await.unwrap();
        backend.put("audio/song1.mp3", b"data4").await.unwrap();

        let images = backend.list_objects("images/").await.unwrap();
        assert_eq!(images.len(), 2);
        assert!(images.contains(&"images/photo1.jpg".to_string()));
        assert!(images.contains(&"images/photo2.jpg".to_string()));

        let videos = backend.list_objects("videos/").await.unwrap();
        assert_eq!(videos.len(), 1);

        let empty = backend.list_objects("nonexistent/").await.unwrap();
        assert_eq!(empty.len(), 0);
    }

    #[tokio::test]
    async fn test_list_objects_sorting() {
        let backend = MockBackend::new();

        backend.put("z_file", b"data").await.unwrap();
        backend.put("a_file", b"data").await.unwrap();
        backend.put("m_file", b"data").await.unwrap();

        let objects = backend.list_objects("").await.unwrap();
        assert_eq!(objects, vec!["a_file", "m_file", "z_file"]);
    }

    #[tokio::test]
    async fn test_overwrite() {
        let backend = MockBackend::new();

        backend.put("key1", b"old data").await.unwrap();
        assert_eq!(backend.get("key1").await.unwrap(), b"old data");

        backend.put("key1", b"new data").await.unwrap();
        assert_eq!(backend.get("key1").await.unwrap(), b"new data");
        assert_eq!(backend.len().await, 1);
    }

    #[tokio::test]
    async fn test_with_data() {
        let mut initial = HashMap::new();
        initial.insert("key1".to_string(), vec![1, 2, 3]);
        initial.insert("key2".to_string(), vec![4, 5, 6]);

        let backend = MockBackend::with_data(initial);
        assert_eq!(backend.len().await, 2);
        assert_eq!(backend.get("key1").await.unwrap(), vec![1, 2, 3]);
        assert_eq!(backend.get("key2").await.unwrap(), vec![4, 5, 6]);
    }

    #[tokio::test]
    async fn test_clear() {
        let backend = MockBackend::new();

        backend.put("key1", b"data").await.unwrap();
        backend.put("key2", b"data").await.unwrap();
        assert_eq!(backend.len().await, 2);

        backend.clear().await;
        assert!(backend.is_empty().await);
        assert_eq!(backend.len().await, 0);
    }

    #[tokio::test]
    async fn test_concurrent_access() {
        let backend = MockBackend::new();

        // Simulate concurrent writes
        let backend1 = backend.clone();
        let handle1 = tokio::spawn(async move {
            for i in 0..10 {
                let key = format!("task1_key{}", i);
                backend1.put(&key, b"data").await.unwrap();
            }
        });

        let backend2 = backend.clone();
        let handle2 = tokio::spawn(async move {
            for i in 0..10 {
                let key = format!("task2_key{}", i);
                backend2.put(&key, b"data").await.unwrap();
            }
        });

        handle1.await.unwrap();
        handle2.await.unwrap();

        assert_eq!(backend.len().await, 20);
    }

    #[tokio::test]
    async fn test_clone_shares_state() {
        let backend1 = MockBackend::new();
        backend1.put("key1", b"data").await.unwrap();

        let backend2 = backend1.clone();
        assert_eq!(backend2.len().await, 1);
        assert_eq!(backend2.get("key1").await.unwrap(), b"data");

        backend2.put("key2", b"data").await.unwrap();
        assert_eq!(backend1.len().await, 2);
    }

    #[tokio::test]
    async fn test_keys() {
        let backend = MockBackend::new();

        backend.put("apple", b"data").await.unwrap();
        backend.put("banana", b"data").await.unwrap();
        backend.put("cherry", b"data").await.unwrap();

        let keys = backend.keys().await;
        assert_eq!(keys.len(), 3);
        assert!(keys.contains(&"apple".to_string()));
        assert!(keys.contains(&"banana".to_string()));
        assert!(keys.contains(&"cherry".to_string()));
    }

    #[tokio::test]
    async fn test_debug_impl() {
        let backend = MockBackend::new();
        let debug_str = format!("{:?}", backend);
        assert!(debug_str.contains("MockBackend"));
    }
}

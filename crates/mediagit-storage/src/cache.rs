// Copyright (C) 2025 MediaGit Contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

//! LRU cache implementation for object database
//!
//! Provides thread-safe, async-compatible LRU caching with:
//! - Size-based eviction (configurable max bytes)
//! - Count-based eviction (configurable max entries)
//! - O(1) get/put operations
//! - Concurrent access via tokio RwLock
//!
//! # Examples
//!
//! ```
//! use mediagit_storage::cache::LruCache;
//!
//! #[tokio::main]
//! async fn main() {
//!     let cache = LruCache::new(1024 * 1024); // 1MB cache
//!
//!     cache.put("key1", vec![1, 2, 3]).await;
//!     let value = cache.get("key1").await;
//!     assert_eq!(value, Some(vec![1, 2, 3]));
//! }
//! ```

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Entry in the LRU cache with metadata
#[derive(Debug, Clone)]
struct CacheEntry {
    /// Cached data
    data: Vec<u8>,
    /// Size in bytes
    size: usize,
    /// Access order (higher = more recent)
    access_order: u64,
}

/// LRU cache with size and count limits
///
/// Thread-safe cache using tokio RwLock for concurrent access.
/// Evicts least recently used entries when limits are exceeded.
#[derive(Debug, Clone)]
pub struct LruCache {
    /// Internal storage
    inner: Arc<RwLock<LruCacheInner>>,
}

#[derive(Debug)]
struct LruCacheInner {
    /// Cache storage
    entries: HashMap<String, CacheEntry>,
    /// Access order queue (keys in LRU order)
    access_queue: VecDeque<String>,
    /// Current total size in bytes
    current_size: usize,
    /// Maximum cache size in bytes
    max_size: usize,
    /// Maximum number of entries (0 = unlimited)
    max_entries: usize,
    /// Maximum single object size (objects larger than this are not cached)
    max_object_size: usize,
    /// Access counter for ordering
    access_counter: u64,
    /// Cache hit count
    hits: u64,
    /// Cache miss count
    misses: u64,
    /// Total eviction count
    evictions: u64,
}

impl LruCache {
    /// Create new LRU cache with size limit
    ///
    /// # Arguments
    /// * `max_size` - Maximum cache size in bytes
    pub fn new(max_size: usize) -> Self {
        Self::with_limits(max_size, 0)
    }

    /// Create new LRU cache with size and count limits
    ///
    /// # Arguments
    /// * `max_size` - Maximum cache size in bytes
    /// * `max_entries` - Maximum number of entries (0 = unlimited)
    pub fn with_limits(max_size: usize, max_entries: usize) -> Self {
        Self::with_all_limits(max_size, max_entries, 50 * 1024 * 1024) // 50MB default
    }

    /// Create new LRU cache with all configurable limits
    ///
    /// # Arguments
    /// * `max_size` - Maximum cache size in bytes
    /// * `max_entries` - Maximum number of entries (0 = unlimited)
    /// * `max_object_size` - Maximum size of individual objects to cache (larger objects are skipped)
    pub fn with_all_limits(max_size: usize, max_entries: usize, max_object_size: usize) -> Self {
        Self {
            inner: Arc::new(RwLock::new(LruCacheInner {
                entries: HashMap::new(),
                access_queue: VecDeque::new(),
                current_size: 0,
                max_size,
                max_entries,
                max_object_size,
                access_counter: 0,
                hits: 0,
                misses: 0,
                evictions: 0,
            })),
        }
    }

    /// Get value from cache
    ///
    /// Updates access order on hit. Returns None on miss.
    pub async fn get(&self, key: &str) -> Option<Vec<u8>> {
        let mut inner = self.inner.write().await;

        // Check if key exists first
        if !inner.entries.contains_key(key) {
            inner.misses += 1;
            return None;
        }

        // Key exists - record hit
        inner.hits += 1;

        // Clone the data
        let data = inner.entries.get(key).map(|e| e.data.clone());

        // Update access order
        inner.access_counter += 1;
        let access_order = inner.access_counter;
        if let Some(entry) = inner.entries.get_mut(key) {
            entry.access_order = access_order;
        }

        // Move to end of queue (most recent)
        if let Some(pos) = inner.access_queue.iter().position(|k| k == key) {
            inner.access_queue.remove(pos);
        }
        inner.access_queue.push_back(key.to_string());

        data
    }

    /// Put value into cache
    ///
    /// Evicts least recently used entries if necessary to maintain limits.
    /// Objects larger than max_object_size are skipped to prevent cache thrashing.
    pub async fn put(&self, key: impl Into<String>, data: Vec<u8>) {
        let key = key.into();
        let size = data.len();
        let mut inner = self.inner.write().await;

        // Skip objects larger than max_object_size
        if size > inner.max_object_size {
            return;
        }

        // Remove old entry if exists
        if let Some(old_entry) = inner.entries.remove(&key) {
            inner.current_size -= old_entry.size;
            if let Some(pos) = inner.access_queue.iter().position(|k| k == &key) {
                inner.access_queue.remove(pos);
            }
        }

        // Evict until we have space
        while inner.current_size + size > inner.max_size
            || (inner.max_entries > 0 && inner.entries.len() >= inner.max_entries)
        {
            if let Some(evict_key) = inner.access_queue.pop_front() {
                if let Some(evicted) = inner.entries.remove(&evict_key) {
                    inner.current_size -= evicted.size;
                    inner.evictions += 1;
                }
            } else {
                break; // No more entries to evict
            }
        }

        // Add new entry
        inner.access_counter += 1;
        let access_order = inner.access_counter;
        inner.entries.insert(
            key.clone(),
            CacheEntry {
                data,
                size,
                access_order,
            },
        );
        inner.access_queue.push_back(key);
        inner.current_size += size;
    }

    /// Check if key exists in cache
    pub async fn contains(&self, key: &str) -> bool {
        let inner = self.inner.read().await;
        inner.entries.contains_key(key)
    }

    /// Remove entry from cache
    pub async fn remove(&self, key: &str) -> Option<Vec<u8>> {
        let mut inner = self.inner.write().await;

        if let Some(entry) = inner.entries.remove(key) {
            inner.current_size -= entry.size;
            if let Some(pos) = inner.access_queue.iter().position(|k| k == key) {
                inner.access_queue.remove(pos);
            }
            Some(entry.data)
        } else {
            None
        }
    }

    /// Clear all entries from cache
    pub async fn clear(&self) {
        let mut inner = self.inner.write().await;
        inner.entries.clear();
        inner.access_queue.clear();
        inner.current_size = 0;
        inner.access_counter = 0;
    }

    /// Get current cache statistics
    pub async fn stats(&self) -> CacheStats {
        let inner = self.inner.read().await;
        let total_accesses = inner.hits + inner.misses;
        let hit_rate = if total_accesses > 0 {
            inner.hits as f64 / total_accesses as f64
        } else {
            0.0
        };
        CacheStats {
            entry_count: inner.entries.len(),
            total_size: inner.current_size,
            max_size: inner.max_size,
            max_entries: inner.max_entries,
            max_object_size: inner.max_object_size,
            hits: inner.hits,
            misses: inner.misses,
            evictions: inner.evictions,
            hit_rate,
        }
    }
}

/// Cache statistics
#[derive(Debug, Clone, Copy)]
pub struct CacheStats {
    /// Current number of entries
    pub entry_count: usize,
    /// Current total size in bytes
    pub total_size: usize,
    /// Maximum size in bytes
    pub max_size: usize,
    /// Maximum entries (0 = unlimited)
    pub max_entries: usize,
    /// Maximum single object size (larger objects are skipped)
    pub max_object_size: usize,
    /// Total cache hits
    pub hits: u64,
    /// Total cache misses
    pub misses: u64,
    /// Total evictions
    pub evictions: u64,
    /// Hit rate (0.0 to 1.0)
    pub hit_rate: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_basic_get_put() {
        let cache = LruCache::new(1024);

        cache.put("key1", vec![1, 2, 3]).await;
        assert_eq!(cache.get("key1").await, Some(vec![1, 2, 3]));
        assert_eq!(cache.get("key2").await, None);
    }

    #[tokio::test]
    async fn test_size_eviction() {
        let cache = LruCache::new(10); // 10 bytes max

        cache.put("key1", vec![1, 2, 3, 4]).await; // 4 bytes
        cache.put("key2", vec![5, 6, 7, 8]).await; // 4 bytes
        cache.put("key3", vec![9, 10, 11]).await; // 3 bytes

        // key1 should be evicted (LRU)
        assert_eq!(cache.get("key1").await, None);
        assert_eq!(cache.get("key2").await, Some(vec![5, 6, 7, 8]));
        assert_eq!(cache.get("key3").await, Some(vec![9, 10, 11]));
    }

    #[tokio::test]
    async fn test_count_eviction() {
        let cache = LruCache::with_limits(1024, 2); // Max 2 entries

        cache.put("key1", vec![1]).await;
        cache.put("key2", vec![2]).await;
        cache.put("key3", vec![3]).await;

        // key1 should be evicted
        assert_eq!(cache.get("key1").await, None);
        assert_eq!(cache.get("key2").await, Some(vec![2]));
        assert_eq!(cache.get("key3").await, Some(vec![3]));
    }

    #[tokio::test]
    async fn test_lru_ordering() {
        let cache = LruCache::with_limits(1024, 2);

        cache.put("key1", vec![1]).await;
        cache.put("key2", vec![2]).await;

        // Access key1 to make it more recent
        let _ = cache.get("key1").await;

        // Add key3, should evict key2 (now LRU)
        cache.put("key3", vec![3]).await;

        assert_eq!(cache.get("key1").await, Some(vec![1]));
        assert_eq!(cache.get("key2").await, None);
        assert_eq!(cache.get("key3").await, Some(vec![3]));
    }

    #[tokio::test]
    async fn test_update_existing() {
        let cache = LruCache::new(1024);

        cache.put("key1", vec![1, 2, 3]).await;
        cache.put("key1", vec![4, 5, 6]).await; // Update

        assert_eq!(cache.get("key1").await, Some(vec![4, 5, 6]));
    }

    #[tokio::test]
    async fn test_remove() {
        let cache = LruCache::new(1024);

        cache.put("key1", vec![1, 2, 3]).await;
        assert_eq!(cache.remove("key1").await, Some(vec![1, 2, 3]));
        assert_eq!(cache.get("key1").await, None);
    }

    #[tokio::test]
    async fn test_clear() {
        let cache = LruCache::new(1024);

        cache.put("key1", vec![1]).await;
        cache.put("key2", vec![2]).await;
        cache.clear().await;

        assert_eq!(cache.get("key1").await, None);
        assert_eq!(cache.get("key2").await, None);
    }

    #[tokio::test]
    async fn test_stats() {
        let cache = LruCache::new(1024);

        cache.put("key1", vec![1, 2, 3]).await;
        cache.put("key2", vec![4, 5]).await;

        let stats = cache.stats().await;
        assert_eq!(stats.entry_count, 2);
        assert_eq!(stats.total_size, 5);
        assert_eq!(stats.max_size, 1024);
    }

    #[tokio::test]
    async fn test_concurrent_access() {
        use tokio::task;

        let cache = Arc::new(LruCache::new(1024 * 1024));
        let mut handles = vec![];

        // Spawn 10 concurrent writers
        for i in 0..10 {
            let cache = cache.clone();
            handles.push(task::spawn(async move {
                for j in 0..100 {
                    let key = format!("key_{}_{}", i, j);
                    let data = vec![i as u8; 100];
                    cache.put(key, data).await;
                }
            }));
        }

        // Spawn 10 concurrent readers
        for i in 0..10 {
            let cache = cache.clone();
            handles.push(task::spawn(async move {
                for j in 0..100 {
                    let key = format!("key_{}_{}", i, j);
                    let _ = cache.get(&key).await;
                }
            }));
        }

        // Wait for all tasks
        for handle in handles {
            handle.await.unwrap();
        }

        let stats = cache.stats().await;
        assert!(stats.entry_count > 0);
    }

    #[tokio::test]
    async fn test_cache_metrics_hits_misses() {
        let cache = LruCache::new(1024);

        // Initial stats should be zero
        let stats = cache.stats().await;
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.misses, 0);

        // Cache miss
        assert_eq!(cache.get("nonexistent").await, None);
        let stats = cache.stats().await;
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.hits, 0);

        // Add entry
        cache.put("key1", vec![1, 2, 3]).await;

        // Cache hit
        assert!(cache.get("key1").await.is_some());
        let stats = cache.stats().await;
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1);

        // Another hit
        let _ = cache.get("key1").await;
        let stats = cache.stats().await;
        assert_eq!(stats.hits, 2);
        assert_eq!(stats.misses, 1);
    }

    #[tokio::test]
    async fn test_cache_hit_rate() {
        let cache = LruCache::new(1024);

        // With no accesses, hit_rate should be 0
        let stats = cache.stats().await;
        assert_eq!(stats.hit_rate, 0.0);

        cache.put("key1", vec![1, 2, 3]).await;

        // 2 hits, 1 miss = 66% hit rate
        let _ = cache.get("key1").await; // hit
        let _ = cache.get("key1").await; // hit
        let _ = cache.get("missing").await; // miss

        let stats = cache.stats().await;
        assert!(stats.hit_rate > 0.6);
        assert!(stats.hit_rate < 0.7);
    }

    #[tokio::test]
    async fn test_cache_eviction_count() {
        let cache = LruCache::with_limits(10, 2); // Max 2 entries, 10 bytes

        // Initial evictions should be zero
        let stats = cache.stats().await;
        assert_eq!(stats.evictions, 0);

        // Add 3 entries, should trigger 1 eviction
        cache.put("key1", vec![1]).await;
        cache.put("key2", vec![2]).await;
        cache.put("key3", vec![3]).await;

        let stats = cache.stats().await;
        assert_eq!(stats.evictions, 1);

        // Add another, should trigger another eviction
        cache.put("key4", vec![4]).await;
        let stats = cache.stats().await;
        assert_eq!(stats.evictions, 2);
    }

    #[tokio::test]
    async fn test_skip_large_objects() {
        // Create cache with 100 byte max_object_size
        let cache = LruCache::with_all_limits(1024, 10, 100);

        // Small object should be cached
        let small_data = vec![1u8; 50];
        cache.put("small", small_data.clone()).await;
        assert_eq!(cache.get("small").await, Some(small_data));

        // Large object should NOT be cached (>100 bytes)
        let large_data = vec![2u8; 150];
        cache.put("large", large_data).await;
        assert_eq!(cache.get("large").await, None);

        // Verify stats show the small object, not the large one
        let stats = cache.stats().await;
        assert_eq!(stats.entry_count, 1);
        assert_eq!(stats.total_size, 50);
    }

    #[tokio::test]
    async fn test_max_object_size_threshold() {
        // 50MB default max_object_size
        let cache = LruCache::with_limits(100 * 1024 * 1024, 100);

        let stats = cache.stats().await;
        assert_eq!(stats.max_object_size, 50 * 1024 * 1024);
    }
}


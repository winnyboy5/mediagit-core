//! Comprehensive unit tests for MockBackend
//!
//! Tests all edge cases, error conditions, and concurrent operations
//! to achieve 90%+ coverage on the MockBackend implementation.

use mediagit_storage::{mock::MockBackend, StorageBackend};
use std::collections::HashMap;

/// Test MockBackend creation and initialization
#[tokio::test]
async fn test_mock_backend_new() {
    let backend = MockBackend::new();
    assert!(backend.is_empty().await);
    assert_eq!(backend.len().await, 0);
    assert_eq!(backend.keys().await.len(), 0);
}

/// Test MockBackend creation with initial data
#[tokio::test]
async fn test_mock_backend_with_data() {
    let mut initial_data = HashMap::new();
    initial_data.insert("key1".to_string(), vec![1, 2, 3]);
    initial_data.insert("key2".to_string(), vec![4, 5, 6]);
    initial_data.insert("key3".to_string(), vec![7, 8, 9]);

    let backend = MockBackend::with_data(initial_data);

    assert_eq!(backend.len().await, 3);
    assert!(!backend.is_empty().await);
    assert_eq!(backend.get("key1").await.unwrap(), vec![1, 2, 3]);
    assert_eq!(backend.get("key2").await.unwrap(), vec![4, 5, 6]);
    assert_eq!(backend.get("key3").await.unwrap(), vec![7, 8, 9]);
}

/// Test Default trait implementation
#[tokio::test]
async fn test_mock_backend_default() {
    let backend: MockBackend = Default::default();
    assert!(backend.is_empty().await);
    assert_eq!(backend.len().await, 0);
}

/// Test Debug trait implementation
#[test]
fn test_mock_backend_debug() {
    let backend = MockBackend::new();
    let debug_str = format!("{:?}", backend);
    assert!(debug_str.contains("MockBackend"));
}

/// Test put operation with various data sizes
#[tokio::test]
async fn test_put_various_sizes() {
    let backend = MockBackend::new();

    // Empty data
    backend.put("empty", b"").await.unwrap();
    assert_eq!(backend.get("empty").await.unwrap(), b"");

    // Small data
    backend.put("small", b"hello").await.unwrap();
    assert_eq!(backend.get("small").await.unwrap(), b"hello");

    // Medium data (1KB)
    let medium = vec![0x42u8; 1024];
    backend.put("medium", &medium).await.unwrap();
    assert_eq!(backend.get("medium").await.unwrap(), medium);

    // Large data (1MB)
    let large = vec![0xFFu8; 1024 * 1024];
    backend.put("large", &large).await.unwrap();
    assert_eq!(backend.get("large").await.unwrap(), large);

    assert_eq!(backend.len().await, 4);
}

/// Test put operation with empty key (should fail)
#[tokio::test]
async fn test_put_empty_key() {
    let backend = MockBackend::new();
    let result = backend.put("", b"data").await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("empty"));
}

/// Test put overwrites existing data
#[tokio::test]
async fn test_put_overwrite() {
    let backend = MockBackend::new();

    backend.put("key", b"original").await.unwrap();
    assert_eq!(backend.get("key").await.unwrap(), b"original");
    assert_eq!(backend.len().await, 1);

    backend.put("key", b"updated").await.unwrap();
    assert_eq!(backend.get("key").await.unwrap(), b"updated");
    assert_eq!(backend.len().await, 1); // Length should not change
}

/// Test put with special characters in keys
#[tokio::test]
async fn test_put_special_keys() {
    let backend = MockBackend::new();

    let keys = vec![
        "with spaces",
        "with-dashes",
        "with_underscores",
        "with.dots",
        "with/slashes/nested",
        "with@symbols!",
        "unicode_日本語",
    ];

    for key in &keys {
        backend.put(key, b"data").await.unwrap();
        assert_eq!(backend.get(key).await.unwrap(), b"data");
    }

    assert_eq!(backend.len().await, keys.len());
}

/// Test get operation with non-existent key
#[tokio::test]
async fn test_get_nonexistent() {
    let backend = MockBackend::new();
    let result = backend.get("nonexistent").await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
}

/// Test get operation with empty key (should fail)
#[tokio::test]
async fn test_get_empty_key() {
    let backend = MockBackend::new();
    let result = backend.get("").await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("empty"));
}

/// Test get operation returns cloned data (no reference to internal storage)
#[tokio::test]
async fn test_get_returns_copy() {
    let backend = MockBackend::new();
    backend.put("key", b"data").await.unwrap();

    let data1 = backend.get("key").await.unwrap();
    let data2 = backend.get("key").await.unwrap();

    assert_eq!(data1, data2);
    assert_eq!(data1, b"data");
}

/// Test get with binary data roundtrip
#[tokio::test]
async fn test_get_binary_roundtrip() {
    let backend = MockBackend::new();

    let binary_data: Vec<u8> = (0..=255).collect();
    backend.put("binary", &binary_data).await.unwrap();

    let retrieved = backend.get("binary").await.unwrap();
    assert_eq!(retrieved, binary_data);
}

/// Test exists operation
#[tokio::test]
async fn test_exists() {
    let backend = MockBackend::new();

    assert!(!backend.exists("key").await.unwrap());

    backend.put("key", b"data").await.unwrap();
    assert!(backend.exists("key").await.unwrap());

    backend.delete("key").await.unwrap();
    assert!(!backend.exists("key").await.unwrap());
}

/// Test exists with empty key (should fail)
#[tokio::test]
async fn test_exists_empty_key() {
    let backend = MockBackend::new();
    let result = backend.exists("").await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("empty"));
}

/// Test delete operation
#[tokio::test]
async fn test_delete() {
    let backend = MockBackend::new();

    backend.put("key1", b"data1").await.unwrap();
    backend.put("key2", b"data2").await.unwrap();
    assert_eq!(backend.len().await, 2);

    backend.delete("key1").await.unwrap();
    assert_eq!(backend.len().await, 1);
    assert!(!backend.exists("key1").await.unwrap());
    assert!(backend.exists("key2").await.unwrap());
}

/// Test delete is idempotent (deleting non-existent key succeeds)
#[tokio::test]
async fn test_delete_nonexistent() {
    let backend = MockBackend::new();

    backend.delete("nonexistent").await.unwrap();
    assert_eq!(backend.len().await, 0);
}

/// Test delete with empty key (should fail)
#[tokio::test]
async fn test_delete_empty_key() {
    let backend = MockBackend::new();
    let result = backend.delete("").await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("empty"));
}

/// Test list_objects with various prefixes
#[tokio::test]
async fn test_list_objects_prefixes() {
    let backend = MockBackend::new();

    backend.put("images/photo1.jpg", b"data").await.unwrap();
    backend.put("images/photo2.jpg", b"data").await.unwrap();
    backend.put("images/photo3.png", b"data").await.unwrap();
    backend.put("videos/video1.mp4", b"data").await.unwrap();
    backend.put("videos/video2.mp4", b"data").await.unwrap();
    backend.put("documents/doc1.pdf", b"data").await.unwrap();

    // List images
    let images = backend.list_objects("images/").await.unwrap();
    assert_eq!(images.len(), 3);
    assert!(images.contains(&"images/photo1.jpg".to_string()));

    // List videos
    let videos = backend.list_objects("videos/").await.unwrap();
    assert_eq!(videos.len(), 2);

    // List all (empty prefix)
    let all = backend.list_objects("").await.unwrap();
    assert_eq!(all.len(), 6);

    // List non-existent prefix
    let none = backend.list_objects("audio/").await.unwrap();
    assert_eq!(none.len(), 0);
}

/// Test list_objects returns sorted results
#[tokio::test]
async fn test_list_objects_sorted() {
    let backend = MockBackend::new();

    backend.put("zebra", b"data").await.unwrap();
    backend.put("alpha", b"data").await.unwrap();
    backend.put("charlie", b"data").await.unwrap();
    backend.put("bravo", b"data").await.unwrap();

    let objects = backend.list_objects("").await.unwrap();
    assert_eq!(objects, vec!["alpha", "bravo", "charlie", "zebra"]);
}

/// Test list_objects with partial prefix matches
#[tokio::test]
async fn test_list_objects_partial_prefix() {
    let backend = MockBackend::new();

    backend.put("test1", b"data").await.unwrap();
    backend.put("test2", b"data").await.unwrap();
    backend.put("testing", b"data").await.unwrap();
    backend.put("other", b"data").await.unwrap();

    let test_prefix = backend.list_objects("test").await.unwrap();
    assert_eq!(test_prefix.len(), 3);
    assert!(test_prefix.contains(&"test1".to_string()));
    assert!(test_prefix.contains(&"test2".to_string()));
    assert!(test_prefix.contains(&"testing".to_string()));
    assert!(!test_prefix.contains(&"other".to_string()));
}

/// Test clear operation
#[tokio::test]
async fn test_clear() {
    let backend = MockBackend::new();

    backend.put("key1", b"data").await.unwrap();
    backend.put("key2", b"data").await.unwrap();
    backend.put("key3", b"data").await.unwrap();
    assert_eq!(backend.len().await, 3);

    backend.clear().await;
    assert!(backend.is_empty().await);
    assert_eq!(backend.len().await, 0);
    assert_eq!(backend.keys().await.len(), 0);

    // Verify all keys are gone
    assert!(!backend.exists("key1").await.unwrap());
    assert!(!backend.exists("key2").await.unwrap());
    assert!(!backend.exists("key3").await.unwrap());
}

/// Test keys() helper method
#[tokio::test]
async fn test_keys() {
    let backend = MockBackend::new();

    assert_eq!(backend.keys().await.len(), 0);

    backend.put("apple", b"data").await.unwrap();
    backend.put("banana", b"data").await.unwrap();
    backend.put("cherry", b"data").await.unwrap();

    let keys = backend.keys().await;
    assert_eq!(keys.len(), 3);
    assert!(keys.contains(&"apple".to_string()));
    assert!(keys.contains(&"banana".to_string()));
    assert!(keys.contains(&"cherry".to_string()));
}

/// Test concurrent writes (no data races)
#[tokio::test]
async fn test_concurrent_writes() {
    let backend = MockBackend::new();

    let mut handles = vec![];

    for i in 0..50 {
        let backend_clone = backend.clone();
        let handle = tokio::spawn(async move {
            let key = format!("concurrent/write_{}", i);
            let data = format!("data_{}", i);
            backend_clone.put(&key, data.as_bytes()).await.unwrap();
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.await.unwrap();
    }

    assert_eq!(backend.len().await, 50);

    let objects = backend.list_objects("concurrent/").await.unwrap();
    assert_eq!(objects.len(), 50);
}

/// Test concurrent reads (no data corruption)
#[tokio::test]
async fn test_concurrent_reads() {
    let backend = MockBackend::new();

    backend.put("shared_key", b"shared_data").await.unwrap();

    let mut handles = vec![];

    for _ in 0..100 {
        let backend_clone = backend.clone();
        let handle = tokio::spawn(async move {
            let data = backend_clone.get("shared_key").await.unwrap();
            assert_eq!(data, b"shared_data");
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.await.unwrap();
    }
}

/// Test concurrent reads and writes
#[tokio::test]
async fn test_concurrent_reads_and_writes() {
    let backend = MockBackend::new();

    backend.put("counter", b"0").await.unwrap();

    let mut handles = vec![];

    // 25 writers
    for i in 0..25 {
        let backend_clone = backend.clone();
        let handle = tokio::spawn(async move {
            let key = format!("writer_{}", i);
            backend_clone.put(&key, b"write_data").await.unwrap();
        });
        handles.push(handle);
    }

    // 75 readers
    for _ in 0..75 {
        let backend_clone = backend.clone();
        let handle = tokio::spawn(async move {
            let _ = backend_clone.get("counter").await;
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.await.unwrap();
    }

    // 25 writes + 1 original = 26 total
    assert_eq!(backend.len().await, 26);
}

/// Test clone shares state
#[tokio::test]
async fn test_clone_shares_state() {
    let backend1 = MockBackend::new();
    backend1.put("key1", b"data1").await.unwrap();

    let backend2 = backend1.clone();
    assert_eq!(backend2.len().await, 1);
    assert_eq!(backend2.get("key1").await.unwrap(), b"data1");

    backend2.put("key2", b"data2").await.unwrap();
    assert_eq!(backend1.len().await, 2);
    assert_eq!(backend1.get("key2").await.unwrap(), b"data2");

    backend1.delete("key1").await.unwrap();
    assert_eq!(backend2.len().await, 1);
    assert!(!backend2.exists("key1").await.unwrap());
}

/// Test len() and is_empty() consistency
#[tokio::test]
async fn test_len_and_is_empty_consistency() {
    let backend = MockBackend::new();

    assert!(backend.is_empty().await);
    assert_eq!(backend.len().await, 0);

    backend.put("key1", b"data").await.unwrap();
    assert!(!backend.is_empty().await);
    assert_eq!(backend.len().await, 1);

    backend.put("key2", b"data").await.unwrap();
    assert!(!backend.is_empty().await);
    assert_eq!(backend.len().await, 2);

    backend.delete("key1").await.unwrap();
    assert!(!backend.is_empty().await);
    assert_eq!(backend.len().await, 1);

    backend.delete("key2").await.unwrap();
    assert!(backend.is_empty().await);
    assert_eq!(backend.len().await, 0);
}

/// Test with UTF-8 data
#[tokio::test]
async fn test_utf8_data() {
    let backend = MockBackend::new();

    let utf8_data = "Hello, 世界! Привет! مرحبا!".as_bytes();
    backend.put("utf8_key", utf8_data).await.unwrap();

    let retrieved = backend.get("utf8_key").await.unwrap();
    assert_eq!(retrieved, utf8_data);
    assert_eq!(String::from_utf8(retrieved).unwrap(), "Hello, 世界! Привет! مرحبا!");
}

/// Test storage trait object usage
#[tokio::test]
async fn test_trait_object() {
    let backend: Box<dyn StorageBackend> = Box::new(MockBackend::new());

    backend.put("key", b"data").await.unwrap();
    assert_eq!(backend.get("key").await.unwrap(), b"data");
    assert!(backend.exists("key").await.unwrap());
    backend.delete("key").await.unwrap();
    assert!(!backend.exists("key").await.unwrap());
}

/// Test large number of keys
#[tokio::test]
async fn test_many_keys() {
    let backend = MockBackend::new();

    for i in 0..1000 {
        let key = format!("key_{:04}", i);
        backend.put(&key, b"data").await.unwrap();
    }

    assert_eq!(backend.len().await, 1000);

    let all = backend.list_objects("").await.unwrap();
    assert_eq!(all.len(), 1000);

    // Verify sorted
    for i in 0..999 {
        assert!(all[i] < all[i + 1]);
    }
}

/// Test delete all keys one by one
#[tokio::test]
async fn test_delete_all_incrementally() {
    let backend = MockBackend::new();

    for i in 0..10 {
        let key = format!("key_{}", i);
        backend.put(&key, b"data").await.unwrap();
    }
    assert_eq!(backend.len().await, 10);

    for i in 0..10 {
        let key = format!("key_{}", i);
        backend.delete(&key).await.unwrap();
        assert_eq!(backend.len().await, 10 - i - 1);
    }

    assert!(backend.is_empty().await);
}

/// Test list with nested prefixes
#[tokio::test]
async fn test_list_nested_prefixes() {
    let backend = MockBackend::new();

    backend.put("a/b/c/file1", b"data").await.unwrap();
    backend.put("a/b/c/file2", b"data").await.unwrap();
    backend.put("a/b/file3", b"data").await.unwrap();
    backend.put("a/file4", b"data").await.unwrap();

    let abc = backend.list_objects("a/b/c/").await.unwrap();
    assert_eq!(abc.len(), 2);

    let ab = backend.list_objects("a/b/").await.unwrap();
    assert_eq!(ab.len(), 3);

    let a = backend.list_objects("a/").await.unwrap();
    assert_eq!(a.len(), 4);
}

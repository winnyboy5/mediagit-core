// Test compression functionality in ObjectDatabase

use mediagit_storage::mock::MockBackend;
use mediagit_storage::StorageBackend;
use mediagit_versioning::{ObjectDatabase, ObjectType};
use std::sync::Arc;

#[tokio::test]
async fn test_zlib_compression_enabled() {
    let storage = Arc::new(MockBackend::new());
    let odb = ObjectDatabase::new(storage.clone(), 100);

    // Write large compressible data
    let data = b"This is test data that compresses well. ".repeat(100);
    let oid = odb.write(ObjectType::Blob, &data).await.unwrap();

    // Read it back
    let retrieved = odb.read(&oid).await.unwrap();
    assert_eq!(retrieved, data);

    // Verify data is compressed in storage
    let key = oid.to_hex();
    let stored_data = storage.get(&key).await.unwrap();

    // Stored data should be smaller than original (compressed)
    assert!(
        stored_data.len() < data.len(),
        "Expected compressed data ({} bytes) to be smaller than original ({} bytes)",
        stored_data.len(),
        data.len()
    );

    // Stored data should have zlib header (0x78)
    assert_eq!(
        stored_data[0], 0x78,
        "Expected zlib header 0x78, got 0x{:02x}",
        stored_data[0]
    );
}

#[tokio::test]
async fn test_backward_compatibility_with_uncompressed_data() {
    let storage = Arc::new(MockBackend::new());

    // First, write uncompressed data (simulating old version)
    let odb_old = ObjectDatabase::without_compression(storage.clone(), 100);
    let data = b"old uncompressed data";
    let oid = odb_old.write(ObjectType::Blob, data).await.unwrap();

    // Verify it's stored uncompressed
    let key = oid.to_hex();
    let stored_data = storage.get(&key).await.unwrap();
    assert_eq!(stored_data, data, "Old version should store data uncompressed");

    // Now read with compression-enabled ODB (simulating new version)
    let odb_new = ObjectDatabase::new(storage, 100);
    let retrieved = odb_new.read(&oid).await.unwrap();

    // Should read successfully even though it was written uncompressed
    assert_eq!(retrieved, data);
}

#[tokio::test]
async fn test_high_compression_ratio() {
    let storage = Arc::new(MockBackend::new());
    let odb = ObjectDatabase::new(storage.clone(), 100);

    // Highly compressible data (repeated pattern)
    let data = vec![0x42u8; 10000];
    let oid = odb.write(ObjectType::Blob, &data).await.unwrap();

    // Get stored data
    let key = oid.to_hex();
    let stored_data = storage.get(&key).await.unwrap();

    // Compression ratio should be significant (>90% reduction for repeated data)
    let ratio = stored_data.len() as f64 / data.len() as f64;
    assert!(
        ratio < 0.1,
        "Expected high compression ratio (<0.1), got {}",
        ratio
    );

    // Verify integrity
    let retrieved = odb.read(&oid).await.unwrap();
    assert_eq!(retrieved, data);
}

#[tokio::test]
async fn test_large_file_compression() {
    let storage = Arc::new(MockBackend::new());
    let odb = ObjectDatabase::new(storage.clone(), 100);

    // Simulate a realistic file with some compressibility
    let data = (0..10000)
        .flat_map(|i| format!("Line {} content\n", i).into_bytes())
        .collect::<Vec<u8>>();

    println!("Original data size: {} bytes", data.len());

    let oid = odb.write(ObjectType::Blob, &data).await.unwrap();

    // Check stored size
    let key = oid.to_hex();
    let stored_data = storage.get(&key).await.unwrap();

    println!(
        "Compressed data size: {} bytes (ratio: {:.2}%)",
        stored_data.len(),
        (stored_data.len() as f64 / data.len() as f64) * 100.0
    );

    // Verify compression occurred
    assert!(stored_data.len() < data.len());

    // Verify integrity
    let retrieved = odb.read(&oid).await.unwrap();
    assert_eq!(retrieved, data);
}

#[tokio::test]
async fn test_empty_data_handling() {
    let storage = Arc::new(MockBackend::new());
    let odb = ObjectDatabase::new(storage, 100);

    let data = b"";
    let oid = odb.write(ObjectType::Blob, data).await.unwrap();

    let retrieved = odb.read(&oid).await.unwrap();
    assert_eq!(retrieved, data);
}

#[tokio::test]
async fn test_small_data_compression() {
    let storage = Arc::new(MockBackend::new());
    let odb = ObjectDatabase::new(storage, 100);

    let data = b"small test";
    let oid = odb.write(ObjectType::Blob, data).await.unwrap();

    let retrieved = odb.read(&oid).await.unwrap();
    assert_eq!(retrieved, data);
}

#[tokio::test]
async fn test_custom_zstd_compressor() {
    use mediagit_compression::{CompressionLevel, ZstdCompressor};

    let storage = Arc::new(MockBackend::new());
    let compressor = Arc::new(ZstdCompressor::new(CompressionLevel::Best));
    let odb = ObjectDatabase::with_compression(storage, 100, compressor, true);

    let data = b"test data with zstd compression".repeat(100);
    let oid = odb.write(ObjectType::Blob, &data).await.unwrap();

    let retrieved = odb.read(&oid).await.unwrap();
    assert_eq!(retrieved, data);
}

#[tokio::test]
async fn test_deduplication_with_compression() {
    let storage = Arc::new(MockBackend::new());
    let odb = ObjectDatabase::new(storage.clone(), 100);

    let data = b"duplicate content".repeat(100);

    // Write same content twice
    let oid1 = odb.write(ObjectType::Blob, &data).await.unwrap();
    let oid2 = odb.write(ObjectType::Blob, &data).await.unwrap();

    // Should return same OID
    assert_eq!(oid1, oid2);

    // Should only store once
    let metrics = odb.metrics().await;
    assert_eq!(metrics.unique_objects, 1);
    assert_eq!(metrics.total_writes, 2);
}

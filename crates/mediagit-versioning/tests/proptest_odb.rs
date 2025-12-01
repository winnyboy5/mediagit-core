//! Property-Based Tests for Object Database
//!
//! Uses proptest to verify ODB properties with random data:
//! - Store/retrieve roundtrip correctness
//! - Deduplication consistency
//! - Cache coherence
//! - OID collision resistance

use mediagit_storage::mock::MockBackend;
use mediagit_versioning::{ObjectDatabase, Oid};
use proptest::prelude::*;
use std::sync::Arc;

/// Generate random binary data for testing
fn arb_binary_data() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(any::<u8>(), 0..10000)
}

/// Property: Store then retrieve should give back original data
#[test]
fn proptest_store_retrieve_roundtrip() {
    let mut runner = proptest::test_runner::TestRunner::default();

    runner
        .run(&arb_binary_data(), |data| {
            tokio::runtime::Runtime::new().unwrap().block_on(async {
                let storage = Arc::new(MockBackend::new());
                let odb = ObjectDatabase::new(storage, 100);

                // Store data
                let oid = odb.write(mediagit_versioning::ObjectType::Blob, &data).await.unwrap();

                // Retrieve data
                let retrieved = odb.read(&oid).await.unwrap();

                // Verify roundtrip
                prop_assert_eq!(&data, &retrieved);
                Ok(())
            })
        })
        .unwrap();
}

/// Property: Same data should produce same OID (determinism)
#[test]
fn proptest_oid_determinism() {
    proptest!(|(data in arb_binary_data())| {
        let oid1 = Oid::hash(&data);
        let oid2 = Oid::hash(&data);
        prop_assert_eq!(oid1, oid2);
    });
}

/// Property: Different data should produce different OIDs (collision resistance)
#[test]
fn proptest_oid_uniqueness() {
    proptest!(|(data1 in arb_binary_data(), data2 in arb_binary_data())| {
        // Skip if data is the same
        prop_assume!(data1 != data2);

        let oid1 = Oid::hash(&data1);
        let oid2 = Oid::hash(&data2);

        // Different data should produce different OIDs
        prop_assert_ne!(oid1, oid2);
    });
}

/// Property: Deduplication - storing same data twice should produce same OID
#[test]
fn proptest_deduplication() {
    let mut runner = proptest::test_runner::TestRunner::default();

    runner
        .run(&arb_binary_data(), |data| {
            tokio::runtime::Runtime::new().unwrap().block_on(async {
                let storage = Arc::new(MockBackend::new());
                let odb = ObjectDatabase::new(storage, 100);

                // Store once
                let oid1 = odb.write(mediagit_versioning::ObjectType::Blob, &data).await.unwrap();

                // Store again (should be deduplicated - same OID)
                let oid2 = odb.write(mediagit_versioning::ObjectType::Blob, &data).await.unwrap();

                // Should return same OID (deduplication)
                prop_assert_eq!(oid1, oid2);
                Ok(())
            })
        })
        .unwrap();
}

/// Property: Cache should maintain consistency with storage
#[test]
fn proptest_cache_consistency() {
    let mut runner = proptest::test_runner::TestRunner::new(proptest::test_runner::Config {
        cases: 100,
        ..Default::default()
    });

    runner
        .run(&prop::collection::vec(arb_binary_data(), 1..20), |datasets| {
            tokio::runtime::Runtime::new().unwrap().block_on(async {
                let storage = Arc::new(MockBackend::new());
                let odb = ObjectDatabase::new(storage, 10); // Small cache to test eviction

                // Store all data
                let mut oids = Vec::new();
                for data in &datasets {
                    let oid = odb.write(mediagit_versioning::ObjectType::Blob, data).await.unwrap();
                    oids.push((oid, data.clone()));
                }

                // Retrieve all data (some from cache, some from storage)
                for (oid, expected_data) in &oids {
                    let retrieved = odb.read(&oid).await.unwrap();
                    prop_assert_eq!(expected_data, &retrieved);
                }

                Ok(())
            })
        })
        .unwrap();
}

/// Property: ODB should handle empty data correctly
#[tokio::test]
async fn proptest_empty_data() {
    let storage = Arc::new(MockBackend::new());
    let odb = ObjectDatabase::new(storage, 100);

    let empty_data = Vec::new();

    let oid = odb.write(mediagit_versioning::ObjectType::Blob, &empty_data).await.unwrap();
    let retrieved = odb.read(&oid).await.unwrap();

    assert_eq!(empty_data, retrieved);
}

/// Property: Large data (>10MB) should work correctly
#[tokio::test]
async fn proptest_large_data() {
    let storage = Arc::new(MockBackend::new());
    let odb = ObjectDatabase::new(storage, 100);

    // 11MB of data
    let large_data: Vec<u8> = (0..11_000_000).map(|i| (i % 256) as u8).collect();

    let oid = odb.write(mediagit_versioning::ObjectType::Blob, &large_data).await.unwrap();
    let retrieved = odb.read(&oid).await.unwrap();

    assert_eq!(large_data.len(), retrieved.len());
    assert_eq!(large_data, retrieved);
}

/// Property: Concurrent reads should all get correct data
#[tokio::test]
async fn proptest_concurrent_reads() {
    let storage = Arc::new(MockBackend::new());
    let odb = Arc::new(ObjectDatabase::new(storage, 100));

    let data = vec![1u8, 2, 3, 4, 5];

    let oid = odb.write(mediagit_versioning::ObjectType::Blob, &data).await.unwrap();

    // Spawn multiple concurrent reads
    let mut handles = vec![];
    for _ in 0..10 {
        let odb_clone = odb.clone();
        let oid_clone = oid;
        let data_clone = data.clone();

        handles.push(tokio::spawn(async move {
            let retrieved = odb_clone.read(&oid_clone).await.unwrap();
            assert_eq!(data_clone, retrieved);
        }));
    }

    // Wait for all reads to complete
    for handle in handles {
        handle.await.unwrap();
    }
}

/// Property: OID should be consistent across processes (serialization)
#[test]
fn proptest_oid_serialization() {
    proptest!(|(data in arb_binary_data())| {
        let oid = Oid::hash(&data);

        // Serialize and deserialize
        let serialized = serde_json::to_string(&oid).unwrap();
        let deserialized: Oid = serde_json::from_str(&serialized).unwrap();

        prop_assert_eq!(oid, deserialized);
    });
}

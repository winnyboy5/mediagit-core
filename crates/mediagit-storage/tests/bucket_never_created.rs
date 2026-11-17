// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! BUG-043: opening an S3 backend on a missing bucket must fail with
//! `BucketNotFound` and must never create the bucket.
//!
//! Runs against a local Silo/MinIO only (`MG_SILO_ENDPOINT`, default
//! http://127.0.0.1:9000, minioadmin). Skips when nothing listens there.

use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region};
use mediagit_storage::StorageError;
use mediagit_storage::b2_spaces_driver::{B2SpacesDriver, B2SpacesDriverConfig};
use mediagit_storage::minio::MinIOBackend;

fn endpoint() -> String {
    std::env::var("MG_SILO_ENDPOINT").unwrap_or_else(|_| "http://127.0.0.1:9000".into())
}

fn client() -> aws_sdk_s3::Client {
    let cfg = aws_sdk_s3::Config::builder()
        .behavior_version(BehaviorVersion::latest())
        .endpoint_url(endpoint())
        .region(Region::new("us-east-1"))
        .credentials_provider(Credentials::new(
            "minioadmin",
            "minioadmin",
            None,
            None,
            "t",
        ))
        .force_path_style(true)
        .http_client(aws_smithy_http_client::Builder::new().build_http())
        .build();
    aws_sdk_s3::Client::from_conf(cfg)
}

/// Bucket names, or None when Silo is down (test skips).
async fn buckets() -> Option<Vec<String>> {
    if tokio::net::TcpStream::connect(endpoint().trim_start_matches("http://"))
        .await
        .is_err()
    {
        return None; // Silo down: skip
    }
    let out = client()
        .list_buckets()
        .send()
        .await
        .expect("list buckets on Silo");
    Some(
        out.buckets()
            .iter()
            .filter_map(|b| b.name().map(String::from))
            .collect(),
    )
}

fn assert_bucket_not_found(err: &anyhow::Error) {
    assert!(
        matches!(
            err.downcast_ref::<StorageError>(),
            Some(StorageError::BucketNotFound(_))
        ),
        "expected BucketNotFound, got: {err:#}"
    );
}

#[tokio::test]
async fn minio_backend_never_creates_missing_bucket() {
    let Some(before) = buckets().await else {
        eprintln!("skip: no Silo at {}", endpoint());
        return;
    };
    let name = format!("mg-bug043-minio-{}", nonce());
    let err = MinIOBackend::new(&endpoint(), &name, "minioadmin", "minioadmin")
        .await
        .expect_err("opening a missing bucket must fail");
    assert_bucket_not_found(&err);
    assert_eq!(before, buckets().await.unwrap(), "bucket list changed");
}

#[tokio::test]
async fn b2_driver_never_creates_missing_bucket() {
    let Some(before) = buckets().await else {
        eprintln!("skip: no Silo at {}", endpoint());
        return;
    };
    let name = format!("mg-bug043-b2-{}", nonce());
    let cfg = B2SpacesDriverConfig {
        bucket: name,
        endpoint: Some(endpoint()),
        region: Some("us-east-1".into()),
        access_key_id: Some("minioadmin".into()),
        secret_access_key: Some("minioadmin".into()),
        ..Default::default()
    };
    let err = B2SpacesDriver::with_config(cfg)
        .await
        .expect_err("opening a missing bucket must fail");
    assert_bucket_not_found(&err);
    assert_eq!(before, buckets().await.unwrap(), "bucket list changed");
}

fn nonce() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

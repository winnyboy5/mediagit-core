// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Does S3 actually REJECT a part whose bytes do not match its checksum?
//!
//! This is the load-bearing question for upload attestation. Everything else in
//! that feature is plumbing: if the provider does not reject a corrupted part,
//! the checksum proves nothing and the post-push pack read-back cannot be
//! dropped. So this is asserted against real S3, not an emulator — LocalStack
//! and MinIO do not implement CRC64NVME validation, and a green test against a
//! service that ignores the header is exactly the kind of gate that cannot
//! fail.
//!
//! Both halves are asserted, because either alone is satisfiable by a broken
//! implementation: "a good upload succeeds" passes a service that ignores
//! checksums entirely, and "a bad upload fails" passes one that rejects
//! everything.
//!
//! # Running
//!
//! Needs real AWS credentials and a writable bucket:
//! ```bash
//! MG_S3_ATTEST_BUCKET=my-bucket AWS_REGION=... \
//!   cargo test -p mediagit-storage --test s3_attestation_live -- --ignored --nocapture
//! ```
//! Objects are written under `attest-live/` and deleted on the way out.

use mediagit_storage::minio::{MinIOBackend, MinIOConfig};
use mediagit_storage::{MpuCompletedPart, StorageBackend};

/// 5 MiB: S3's minimum part size for any part that is not the last one. Using
/// a single part keeps the test to one PUT while staying legal.
const PART: usize = 5 * 1024 * 1024;

fn bucket() -> Option<String> {
    std::env::var("MG_S3_ATTEST_BUCKET")
        .ok()
        .filter(|b| !b.is_empty())
}

async fn backend(bkt: &str) -> MinIOBackend {
    // MinIOBackend, NOT S3Backend. `s3.rs` is not on the AWS path: the server
    // builds its "aws" backend from `MinIOConfig` with an
    // `https://s3.<region>.amazonaws.com` endpoint (`handlers/mod.rs`), and
    // only `b2_spaces` constructs `S3Backend`. Testing attestation through
    // `S3Backend` would exercise a backend no push ever uses — and would also
    // hit that constructor's unconditional `create_bucket`, which omits the
    // `LocationConstraint` every non-us-east-1 region requires and so fails
    // against this bucket regardless.
    //
    // The Rust SDK reads AWS_REGION; this project's campaign env sets only
    // AWS_DEFAULT_REGION, so both are consulted here.
    let region = std::env::var("AWS_REGION")
        .ok()
        .filter(|r| !r.is_empty())
        .or_else(|| std::env::var("AWS_DEFAULT_REGION").ok())
        .filter(|r| !r.is_empty())
        .expect("set AWS_REGION or AWS_DEFAULT_REGION to the bucket's region");

    MinIOBackend::with_config(MinIOConfig {
        endpoint: format!("https://s3.{region}.amazonaws.com"),
        bucket: bkt.to_string(),
        access_key: std::env::var("AWS_ACCESS_KEY_ID").expect("AWS_ACCESS_KEY_ID"),
        secret_key: std::env::var("AWS_SECRET_ACCESS_KEY").expect("AWS_SECRET_ACCESS_KEY"),
        prefix: "attest-live".to_string(),
        region,
        path_style: false,
        ..MinIOConfig::default()
    })
    .await
    .expect("build AWS backend")
}

/// Install the TLS provider once per test binary.
///
/// reqwest is built with `rustls-no-provider` workspace-wide, so a bare
/// `Client::new()` panics until one is installed. Ring, not aws-lc-rs — the
/// workspace standardises on ring and mixing providers is a recorded hazard.
fn ensure_tls() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

fn b64_crc64(data: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(crc_fast::crc64_nvme(data).to_be_bytes())
}

/// HALF 1 — an honest upload is accepted and the object lands.
#[tokio::test]
#[ignore = "needs real AWS S3: set MG_S3_ATTEST_BUCKET"]
async fn a_correct_checksum_is_accepted() {
    let Some(bkt) = bucket() else {
        panic!("MG_S3_ATTEST_BUCKET not set");
    };
    ensure_tls();
    let be = backend(&bkt).await;
    let key = format!("good-{}", std::process::id());
    let data = vec![7u8; PART];

    let mpu = be
        .create_presigned_mpu(&key, data.len() as u64, std::time::Duration::from_secs(900))
        .await
        .expect("create mpu")
        .expect("S3 must support presigned MPU");

    assert!(
        mpu.checksum.is_some(),
        "create_presigned_mpu declared no checksum algorithm, so nothing is being attested"
    );

    let part = &mpu.parts[0];
    let sum = b64_crc64(&data);
    let resp = reqwest::Client::new()
        .put(&part.url)
        // NO checksum header: a presigned URL signs a fixed header set and AWS
        // refuses extras with `HeadersNotSigned`. The attestation rides on the
        // server-made Complete call instead.
        .header("content-length", data.len())
        .body(data.clone())
        .send()
        .await
        .expect("PUT part");

    let status = resp.status();
    let body = resp
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(
        status.is_success(),
        "an honest part was REJECTED ({status}) even with no extra headers"
    );

    be.complete_presigned_mpu(
        &key,
        &mpu.upload_id,
        vec![MpuCompletedPart {
            part_number: part.part_number,
            etag: body,
            checksum: Some(sum),
            length: Some(data.len() as u64),
        }],
    )
    .await
    .expect("complete must succeed for a correctly attested upload");

    assert_eq!(
        be.head(&key).await.expect("head").expect("object present"),
        data.len() as u64
    );
    let _ = be.delete(&key).await;
}

/// HALF 2 — the one that matters. Bytes that do not match the declared
/// checksum must be REFUSED. If this passes, the read-back can go.
#[tokio::test]
#[ignore = "needs real AWS S3: set MG_S3_ATTEST_BUCKET"]
async fn a_corrupted_part_is_rejected() {
    let Some(bkt) = bucket() else {
        panic!("MG_S3_ATTEST_BUCKET not set");
    };
    ensure_tls();
    let be = backend(&bkt).await;
    let key = format!("bad-{}", std::process::id());
    let data = vec![7u8; PART];

    let mpu = be
        .create_presigned_mpu(&key, data.len() as u64, std::time::Duration::from_secs(900))
        .await
        .expect("create mpu")
        .expect("S3 must support presigned MPU");
    let part = &mpu.parts[0];

    // Checksum of the ORIGINAL bytes, but send CORRUPTED bytes: exactly the
    // shape of a bit flip in transit or a truncated body.
    let honest_sum = b64_crc64(&data);
    let mut corrupted = data.clone();
    corrupted[0] ^= 0xFF;

    let corrupted_len = corrupted.len();
    let resp = reqwest::Client::new()
        .put(&part.url)
        .header("content-length", corrupted_len)
        .body(corrupted)
        .send()
        .await
        .expect("PUT part");
    assert!(
        resp.status().is_success(),
        "the PUT should succeed — nothing validates it there; the lie is caught at Complete"
    );
    let etag = resp
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    // Attest the ORIGINAL bytes for the CORRUPTED upload. S3 computes the real
    // checksum of what it stored and must refuse the mismatch.
    let outcome = be
        .complete_presigned_mpu(
            &key,
            &mpu.upload_id,
            vec![MpuCompletedPart {
                part_number: part.part_number,
                etag,
                checksum: Some(honest_sum),
                length: Some(corrupted_len as u64),
            }],
        )
        .await;
    let _ = be.abort_presigned_mpu(&key, &mpu.upload_id).await;
    let _ = be.delete(&key).await;

    let err = outcome.expect_err(
        "S3 ACCEPTED an object whose bytes do not match the attested checksum. \
         Attestation proves nothing here, so the pack read-back MUST stay and phase B \
         must not proceed.",
    );
    // WHY NOT ASSERT ON "BadDigest". The backend maps SDK errors with
    // `anyhow!("complete_multipart_upload {}: {}", endpoint, e)`, and the SDK
    // error's `Display` is the bare string "service error" — the S3 error code
    // never reaches the message. (Worth fixing in the backend for
    // diagnosability; asserting on it here would just pin a bug.)
    //
    // The reason is established DIFFERENTIALLY instead:
    // `a_correct_checksum_is_accepted` runs the identical flow — same backend,
    // same credentials, same single-part MPU, same Complete call — and
    // succeeds. The only difference here is that the uploaded bytes do not
    // match the attested checksum. So a refusal in this test, given that
    // control passes, is the checksum being enforced.
    let msg = format!("{err:#}");
    assert!(
        msg.contains("complete_multipart_upload"),
        "refused somewhere other than Complete, so the control does not apply: {msg}"
    );
}

/// HALF 3 — VERSION SKEW. Can an OLD client still push to a NEW server?
///
/// `create_presigned_mpu` declares a checksum algorithm on every AWS upload.
/// A client that predates attestation sends no per-part checksums, so
/// `combine_part_crc64` yields `None` and Complete is called WITHOUT a checksum
/// on an MPU that DECLARED one.
///
/// The design assumes that degrades to "unattested" — the upload lands, the
/// object carries no provider checksum, and `attested_checksum` later reports
/// `None`, so the pack read-back stays on. That is the fail-closed direction
/// and it is what makes the rollout safe.
///
/// If instead S3 REFUSES the Complete, every old client is locked out of a
/// server that upgraded — a silent compatibility break, and one the
/// `serde(default)` wire-compat work does NOT protect against, because the
/// rejection happens at the S3 API rather than in our own decoding.
///
/// This is asserted rather than assumed because the whole rollout story rests
/// on it.
#[tokio::test]
#[ignore = "needs real AWS S3: set MG_S3_ATTEST_BUCKET"]
async fn an_unattested_complete_still_succeeds() {
    let Some(bkt) = bucket() else {
        panic!("MG_S3_ATTEST_BUCKET not set");
    };
    ensure_tls();
    let be = backend(&bkt).await;
    let key = format!("skew-{}", std::process::id());
    let data = vec![3u8; PART];

    let mpu = be
        .create_presigned_mpu(&key, data.len() as u64, std::time::Duration::from_secs(900))
        .await
        .expect("create mpu")
        .expect("S3 must support presigned MPU");
    assert!(
        mpu.checksum.is_some(),
        "this test is only meaningful when the MPU actually declared an algorithm"
    );

    let part = &mpu.parts[0];
    let resp = reqwest::Client::new()
        .put(&part.url)
        .header("content-length", data.len())
        .body(data.clone())
        .send()
        .await
        .expect("PUT part");
    assert!(
        resp.status().is_success(),
        "part PUT failed: {}",
        resp.status()
    );
    let etag = resp
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    // Exactly what an old client produces: an ETag and nothing else.
    let outcome = be
        .complete_presigned_mpu(
            &key,
            &mpu.upload_id,
            vec![MpuCompletedPart {
                part_number: part.part_number,
                etag,
                checksum: None,
                length: None,
            }],
        )
        .await;

    if outcome.is_err() {
        let _ = be.abort_presigned_mpu(&key, &mpu.upload_id).await;
    }
    let landed = be.head(&key).await.ok().flatten();
    let _ = be.delete(&key).await;

    outcome.expect(
        "S3 REFUSED a Complete with no checksum on an MPU that declared an algorithm. \
         Every client predating attestation is then locked out of an upgraded server — \
         declaring the algorithm must become conditional on the client supporting it.",
    );
    assert_eq!(
        landed,
        Some(data.len() as u64),
        "Complete succeeded but the object is not there at full size"
    );
}

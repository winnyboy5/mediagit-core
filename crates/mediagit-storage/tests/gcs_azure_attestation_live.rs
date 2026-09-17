// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Does GCS attestation hold up against the real service, and does Azure
//! correctly refuse to claim any?
//!
//! The S3 equivalent (`s3_attestation_live.rs`) took four attempts to get past
//! things no amount of reading the docs would have caught: the wrong backend
//! entirely, a header a presigned URL will not carry, a region that broke
//! construction, and a missing TLS provider. Every one surfaced only against
//! the live service. These assert the same properties for the other two clouds.
//!
//! # Running
//!
//! ```bash
//! GCS_BUCKET_NAME=... GCS_PROJECT_ID=... GOOGLE_APPLICATION_CREDENTIALS=...
//! AZURE_STORAGE_ACCOUNT=... AZURE_STORAGE_KEY=... MG_QA_AZURE_CONTAINER=...
//!   cargo test -p mediagit-storage --features gcs,azure \
//!     --test gcs_azure_attestation_live -- --ignored --nocapture
//! ```

#![allow(unused_crate_dependencies)]

use mediagit_storage::{MpuCompletedPart, StorageBackend};

/// 5 MiB keeps a single-part upload legal on every backend here.
const PART: usize = 5 * 1024 * 1024;

fn ensure_tls() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

fn b64_crc32c(data: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(crc32c::crc32c(data).to_be_bytes())
}

// ---------------------------------------------------------------- GCS -----

#[cfg(feature = "gcs")]
mod gcs {
    use super::*;
    use mediagit_storage::gcs::{GcsBackend, GcsConfig};

    async fn backend() -> Option<GcsBackend> {
        let bucket = std::env::var("GCS_BUCKET_NAME")
            .ok()
            .filter(|b| !b.is_empty())?;
        let project = std::env::var("GCS_PROJECT_ID")
            .ok()
            .filter(|p| !p.is_empty())?;
        let mut cfg = GcsConfig::new(project, bucket);
        cfg.prefix = Some("attest-live".to_string());
        GcsBackend::with_default_credentials_and_config(cfg)
            .await
            .ok()
    }

    /// HALF 1 — an honest upload ends up ATTESTED.
    ///
    /// This is the half that proves the whole GCS design: the server folds the
    /// client's per-part crc32c and compares it against the crc32c GCS itself
    /// computed for the assembled object. If the fold disagrees with GCS's
    /// arithmetic in ANY way — byte order, part ordering, the combine itself —
    /// nothing is ever attested and the feature is silently inert.
    #[tokio::test]
    #[ignore = "needs real GCS: set GCS_BUCKET_NAME + GCS_PROJECT_ID + GOOGLE_APPLICATION_CREDENTIALS"]
    async fn an_honest_gcs_upload_is_attested() {
        ensure_tls();
        let be = backend().await.expect("build GCS backend");
        let key = format!("gcs-good-{}", std::process::id());
        let data = vec![7u8; PART];

        let mpu = be
            .create_presigned_mpu(&key, data.len() as u64, std::time::Duration::from_secs(900))
            .await
            .expect("create mpu")
            .expect("GCS must support presigned MPU");
        assert!(
            mpu.checksum.is_some(),
            "GCS declared no checksum algorithm, so the client sends no digests \
             and nothing can ever be compared"
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

        be.complete_presigned_mpu(
            &key,
            &mpu.upload_id,
            vec![MpuCompletedPart {
                part_number: part.part_number,
                etag,
                checksum: Some(b64_crc32c(&data)),
                length: Some(data.len() as u64),
            }],
        )
        .await
        .expect("complete");

        let attested = be.attested_checksum(&key).await.expect("attested_checksum");
        let _ = be.delete(&key).await;

        assert!(
            attested.is_some(),
            "GCS upload completed but is NOT attested — the folded crc32c did not \
             match GCS's own, so phase B will keep reading every pack back"
        );
    }

    /// HALF 2 — a WRONG checksum must NOT be attested.
    ///
    /// Without this, half 1 passes an implementation that attests
    /// unconditionally — which on GCS is a live hazard rather than a
    /// hypothetical, because GCS stores a crc32c for every object and a
    /// presence check would always say yes.
    #[tokio::test]
    #[ignore = "needs real GCS: set GCS_BUCKET_NAME + GCS_PROJECT_ID + GOOGLE_APPLICATION_CREDENTIALS"]
    async fn a_wrong_checksum_is_not_attested_on_gcs() {
        ensure_tls();
        let be = backend().await.expect("build GCS backend");
        let key = format!("gcs-bad-{}", std::process::id());
        let data = vec![7u8; PART];

        let mpu = be
            .create_presigned_mpu(&key, data.len() as u64, std::time::Duration::from_secs(900))
            .await
            .expect("create mpu")
            .expect("GCS must support presigned MPU");
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

        // Honest bytes, DISHONEST digest.
        let mut lie = data.clone();
        lie[0] ^= 0xFF;

        be.complete_presigned_mpu(
            &key,
            &mpu.upload_id,
            vec![MpuCompletedPart {
                part_number: part.part_number,
                etag,
                checksum: Some(b64_crc32c(&lie)),
                length: Some(data.len() as u64),
            }],
        )
        .await
        .expect("complete must still succeed — the upload itself was fine");

        let attested = be.attested_checksum(&key).await.expect("attested_checksum");
        let landed = be.head(&key).await.ok().flatten();
        let _ = be.delete(&key).await;

        assert!(
            attested.is_none(),
            "GCS reported ATTESTED for an object whose crc32c does not match the \
             client's — the gate is answering from presence, not comparison, and \
             would skip the read-back on every upload"
        );
        assert_eq!(
            landed,
            Some(data.len() as u64),
            "the object should still be stored; only its attestation is withheld"
        );
    }

    /// HALF 3 — the FOLD itself, against real GCS, with parts REPORTED OUT OF
    /// ORDER.
    ///
    /// The two tests above upload 5 MiB. GCS's part floor is 16 MiB
    /// (`mpu_part_size_gcs`), so both are single-part uploads: they take the
    /// `None => crc` arm of `combine_part_crc32c` and never reach
    /// `checksum_combine` at all. They prove the comparison and the plumbing;
    /// they do not prove the arithmetic that every real pack depends on.
    ///
    /// 32 MiB is the smallest size that yields two parts, so this is the
    /// cheapest upload that exercises the combine live rather than against the
    /// unit tests' own expectations.
    ///
    /// The completed parts are reported part 2 FIRST. CRC combination is not
    /// commutative — folding them in report order gives a different, wrong
    /// answer — and real pushes complete parts concurrently, so the order the
    /// server sees is arbitrary. This pins the `sort_by_key` as load-bearing;
    /// without the reversal, a fold that ignored part order would still pass.
    #[tokio::test]
    #[ignore = "needs real GCS: set GCS_BUCKET_NAME + GCS_PROJECT_ID + GOOGLE_APPLICATION_CREDENTIALS"]
    async fn a_multipart_upload_folds_to_the_crc32c_gcs_computed() {
        ensure_tls();
        let be = backend().await.expect("build GCS backend");
        let key = format!("gcs-multi-{}", std::process::id());

        // Distinct bytes per part: identical parts would fold correctly even
        // under a combine that mixed up which digest belongs to which length.
        let total = 32 * 1024 * 1024;
        let data: Vec<u8> = (0..total).map(|i| (i % 251) as u8).collect();

        let mpu = be
            .create_presigned_mpu(&key, total as u64, std::time::Duration::from_secs(900))
            .await
            .expect("create mpu")
            .expect("GCS must support presigned MPU");
        assert_eq!(
            mpu.parts.len(),
            2,
            "this test is only meaningful with 2+ parts; the part floor moved, \
             so raise `total` until it splits again"
        );

        let client = reqwest::Client::new();
        let mut completed = Vec::new();
        for part in &mpu.parts {
            let start = (part.part_number as usize - 1) * mpu.part_size as usize;
            let end = (start + mpu.part_size as usize).min(total);
            let chunk = &data[start..end];

            let resp = client
                .put(&part.url)
                .header("content-length", chunk.len())
                .body(chunk.to_vec())
                .send()
                .await
                .expect("PUT part");
            assert!(
                resp.status().is_success(),
                "part {} PUT failed: {}",
                part.part_number,
                resp.status()
            );
            completed.push(MpuCompletedPart {
                part_number: part.part_number,
                etag: resp
                    .headers()
                    .get("etag")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string(),
                checksum: Some(b64_crc32c(chunk)),
                length: Some(chunk.len() as u64),
            });
        }

        completed.reverse();
        be.complete_presigned_mpu(&key, &mpu.upload_id, completed)
            .await
            .expect("complete");

        let attested = be.attested_checksum(&key).await.expect("attested_checksum");
        let landed = be.head(&key).await.ok().flatten();
        let _ = be.delete(&key).await;

        assert_eq!(
            landed,
            Some(total as u64),
            "the multipart object did not assemble"
        );
        assert!(
            attested.is_some(),
            "the folded crc32c of 2 parts does not match the crc32c GCS computed \
             for the assembled object — every real pack is multi-part, so this is \
             the arithmetic that decides whether attestation ever fires in \
             production"
        );
    }
}

// -------------------------------------------------------------- Azure -----

#[cfg(feature = "azure")]
mod azure {
    use super::*;
    use mediagit_storage::AzureBackend;

    async fn backend() -> Option<AzureBackend> {
        let account = std::env::var("AZURE_STORAGE_ACCOUNT")
            .ok()
            .filter(|a| !a.is_empty())?;
        let key = std::env::var("AZURE_STORAGE_KEY")
            .ok()
            .filter(|k| !k.is_empty())?;
        let container = std::env::var("MG_QA_AZURE_CONTAINER")
            .ok()
            .filter(|c| !c.is_empty())?;
        AzureBackend::with_account_key_and_prefix(&account, &container, &key, "attest-live")
            .await
            .ok()
    }

    /// Azure must DECLINE to attest, so the pack read-back stays on.
    ///
    /// This asserts a deliberate "no", not a missing feature. Azure exposes no
    /// service-computed whole-blob digest: `x-ms-blob-content-md5` is
    /// client-set and never validated, and per-block `x-ms-content-crc64`
    /// cannot ride a presigned URL. Claiming attestation here would skip a real
    /// integrity check on the strength of nothing.
    ///
    /// If this ever FAILS, Azure has grown a whole-blob digest and the dead-end
    /// note in `azure.rs` is out of date — which is worth knowing.
    ///
    /// The blob is really uploaded first, deliberately. Asking an EMPTY
    /// container "is this attested?" gets `None` for the uninteresting reason
    /// that nothing is there — a result that would hold even if attestation
    /// worked perfectly. Only a blob that actually landed makes the `None`
    /// mean what the read-back gate depends on it meaning.
    #[tokio::test]
    #[ignore = "needs real Azure: set AZURE_STORAGE_ACCOUNT + AZURE_STORAGE_KEY + MG_QA_AZURE_CONTAINER"]
    async fn azure_declines_to_attest_a_blob_that_really_landed() {
        ensure_tls();
        let be = backend().await.expect("build Azure backend");
        let key = format!("azure-{}", std::process::id());
        let data = vec![7u8; PART];

        let mpu = be
            .create_presigned_mpu(&key, data.len() as u64, std::time::Duration::from_secs(900))
            .await
            .expect("create mpu")
            .expect("Azure must offer a staged block upload");
        assert!(
            mpu.checksum.is_none(),
            "Azure declared a checksum algorithm it cannot have the service \
             validate; the client would compute digests nothing checks"
        );

        let part = &mpu.parts[0];
        let resp = reqwest::Client::new()
            .put(&part.url)
            .header("content-length", data.len())
            .body(data.clone())
            .send()
            .await
            .expect("stage block");
        assert!(
            resp.status().is_success(),
            "block PUT failed: {}",
            resp.status()
        );

        be.complete_presigned_mpu(
            &key,
            &mpu.upload_id,
            vec![MpuCompletedPart {
                part_number: part.part_number,
                etag: String::new(),
                checksum: None,
                length: Some(data.len() as u64),
            }],
        )
        .await
        .expect("commit block list");

        let landed = be.head(&key).await.expect("head");
        let attested = be.attested_checksum(&key).await.expect("attested_checksum");
        let _ = be.delete(&key).await;

        assert_eq!(
            landed,
            Some(data.len() as u64),
            "the blob did not land, so the assertion below would prove nothing"
        );
        assert!(
            attested.is_none(),
            "Azure claimed an attestation it has no way to substantiate; the \
             server would skip the pack read-back on the strength of nothing"
        );
    }
}

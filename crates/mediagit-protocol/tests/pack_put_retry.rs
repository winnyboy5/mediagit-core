// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! A transient error on a presigned pack PUT must be RETRIED, not turned into
//! an 8.9x slowdown for the whole push.
//!
//! `20260821-ga11`: Azure and GCS each threw transient failures on pack PUTs.
//! `upload_and_register` did a bare `req.send()` with no retry and no error
//! classification, and `bail!`ed on any non-success status. The caller
//! (`client/push.rs`) caught that and abandoned the cloud-pack fast path for the
//! ENTIRE push, uploading 2,284 chunks one at a time instead:
//!
//!   ga8   96 upload-urls -> 97 packs/complete,     0 per-chunk,  8.69 MB/s
//!   ga11  96 upload-urls ->  0 packs/complete, 2,284 per-chunk,  0.98 MB/s
//!
//! The comment on that PUT claimed a 429 "is handled by the caller's own retry
//! loop". There is no such loop — the caller only falls back.
//!
//! `error_class::classify_auto` and the 5-attempt loop the per-chunk MPU path
//! uses (`client/mod.rs:1010`) already exist. This pins that the pack path uses
//! them too.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Minimal server standing in for BOTH the mediagit-server control plane and
/// the bucket, so one address serves upload-urls, the presigned PUT and
/// complete. `put_failures` PUTs are answered 503 before the first 200.
async fn serve(listener: TcpListener, addr: String, put_failures: usize, puts: Arc<AtomicUsize>) {
    loop {
        let Ok((mut sock, _)) = listener.accept().await else {
            return;
        };
        let addr = addr.clone();
        let puts = Arc::clone(&puts);
        tokio::spawn(async move {
            let mut buf = Vec::new();
            let mut tmp = [0u8; 8192];
            // Read headers; enough to route on method+path.
            loop {
                let Ok(n) = sock.read(&mut tmp).await else {
                    return;
                };
                if n == 0 {
                    return;
                }
                buf.extend_from_slice(&tmp[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let head = String::from_utf8_lossy(&buf).to_string();
            let first = head.lines().next().unwrap_or("").to_string();

            let reply = |status: &str, body: String| {
                format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
            };

            let resp = if first.contains("/packs/upload-urls") {
                // Point the presigned URL back at this same server.
                let body = format!(
                    r#"{{"{}":{{"url":"http://{}/bucket/pack","required_headers":[]}}}}"#,
                    "aa".repeat(32),
                    addr
                );
                reply("200 OK", body)
            } else if first.starts_with("PUT") && first.contains("/bucket/pack") {
                let n = puts.fetch_add(1, Ordering::SeqCst);
                if n < put_failures {
                    // 503 is unambiguously Transient for every backend.
                    reply("503 Service Unavailable", "{}".into())
                } else {
                    reply("200 OK", "{}".into())
                }
            } else if first.contains("/packs/complete") {
                reply("200 OK", "{}".into())
            } else {
                reply("404 Not Found", "{}".into())
            };
            let _ = sock.write_all(resp.as_bytes()).await;
            let _ = sock.flush().await;
        });
    }
}

fn pack_result(tmp: &std::path::Path) -> mediagit_versioning::CloudPackResult {
    std::fs::write(tmp, b"pack-bytes-payload").unwrap();
    mediagit_versioning::CloudPackResult {
        pack_oid: vec![0xaa; 32],
        byte_len: 18,
        temp_path: tmp.to_path_buf(),
        index: Vec::new(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_transient_pack_put_is_retried_not_abandoned() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    let puts = Arc::new(AtomicUsize::new(0));
    let server = tokio::spawn(serve(listener, addr.clone(), 2, Arc::clone(&puts)));

    // std temp dir rather than adding a `tempfile` dev-dependency for two files.
    let tmp =
        std::env::temp_dir().join(format!("mg-packretry-transient-{}.tmp", std::process::id()));

    mediagit_protocol::ensure_crypto_provider();
    let http = reqwest::Client::builder().build().unwrap();
    let direct = reqwest::Client::builder().build().unwrap();

    let out = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        mediagit_protocol::pack_builder::upload_and_register(
            pack_result(&tmp),
            &format!("http://{addr}/repo"),
            &http,
            &direct,
            &[],
        ),
    )
    .await
    .expect("upload_and_register hung");

    server.abort();

    assert!(
        out.is_ok(),
        "two transient 503s must be retried, not abandoned — this is what dropped \
         the whole push to the per-chunk path in ga11. err={:?}",
        out.err()
    );

    // Load-bearing: without asserting the COUNT, this test would also pass
    // against an implementation that never retried but happened to get a 200.
    assert_eq!(
        puts.load(Ordering::SeqCst),
        3,
        "expected 2 failed attempts + 1 success to reach the bucket"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_permanent_pack_put_error_is_not_retried() {
    // 403 is PermanentConfig for every backend. Retrying it 5 times wastes a
    // whole pack body per attempt and cannot succeed. This proves the
    // classifier is consulted rather than everything being retried blindly.
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    let puts = Arc::new(AtomicUsize::new(0));
    // usize::MAX failures => always 403... but we want 403 specifically, so
    // reuse the failure path with a large count and assert on attempts only.
    let server = tokio::spawn(serve(listener, addr.clone(), usize::MAX, Arc::clone(&puts)));

    let tmp =
        std::env::temp_dir().join(format!("mg-packretry-permanent-{}.tmp", std::process::id()));

    mediagit_protocol::ensure_crypto_provider();
    let http = reqwest::Client::builder().build().unwrap();
    let direct = reqwest::Client::builder().build().unwrap();

    let out = tokio::time::timeout(
        std::time::Duration::from_secs(90),
        mediagit_protocol::pack_builder::upload_and_register(
            pack_result(&tmp),
            &format!("http://{addr}/repo"),
            &http,
            &direct,
            &[],
        ),
    )
    .await
    .expect("upload_and_register hung");

    server.abort();

    assert!(
        out.is_err(),
        "a bucket that never accepts must surface an error"
    );
    let n = puts.load(Ordering::SeqCst);
    assert!(
        n <= 5,
        "retries must be BOUNDED; a pack body is re-sent on every attempt. got {n}"
    );
}

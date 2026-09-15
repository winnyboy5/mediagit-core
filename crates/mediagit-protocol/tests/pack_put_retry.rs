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
/// complete. The first `put_failures` PUTs are answered `fail_status`; the rest
/// get 200.
///
/// `fail_status` is a parameter and not a constant because the two tests below
/// need OPPOSITE classifications, and for a long time only one of them got it.
/// The permanent-error test reused the 503 path and therefore never sent a
/// permanent status at all -- see the note on that test.
async fn serve(
    listener: TcpListener,
    addr: String,
    put_failures: usize,
    fail_status: &'static str,
    puts: Arc<AtomicUsize>,
    // Counts POSTs to /packs/upload-urls, so a test can tell "re-presigned a
    // fresh URL" apart from "replayed the same dead one".
    presigns: Arc<AtomicUsize>,
) {
    loop {
        let Ok((mut sock, _)) = listener.accept().await else {
            return;
        };
        let addr = addr.clone();
        let puts = Arc::clone(&puts);
        let presigns = Arc::clone(&presigns);
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
                presigns.fetch_add(1, Ordering::SeqCst);
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
                    reply(fail_status, "{}".into())
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
    // 503 is unambiguously Transient for every backend.
    let server = tokio::spawn(serve(
        listener,
        addr.clone(),
        2,
        "503 Service Unavailable",
        Arc::clone(&puts),
        Arc::new(AtomicUsize::new(0)),
    ));

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
    // A permanent status must bail on the FIRST attempt: re-sending a whole
    // pack body against an error that cannot succeed is pure waste. This proves
    // the classifier is consulted rather than everything being retried blindly.
    //
    // THIS TEST DID NOT DO THAT UNTIL 2026-09-05, and it is worth saying why,
    // because it was wrong in three separate ways at once:
    //
    //   1. It served 503, not a permanent status. The stub had ONE failure path
    //      and that path was the transient one, so the code correctly retried
    //      and the "permanent" branch was never reached.
    //   2. Its comment asserted "403 is PermanentConfig for every backend".
    //      `classify_by_status` maps 403 to RefreshUrl -- a presigned URL that
    //      expired is retryable BY DESIGN -- and only 400..=499 otherwise to
    //      PermanentConfig. So even a real 403 would have been retried.
    //   3. Its assertion was `n <= 5`, the pre-0de5b7e attempt bound. 0de5b7e
    //      replaced that with a 120s wall-clock budget, which is longer than
    //      this test's own 90s ceiling, so once the code started retrying past
    //      90s the test failed as "hung" -- which is how it was finally noticed,
    //      on 2026-09-05, four days later.
    //
    // The failure mode is the one this suite keeps rediscovering: a guard that
    // cannot fire the way its name claims. It passed for years while asserting
    // nothing about permanence, then broke for a reason unrelated to its
    // subject. Now it sends a genuine PermanentConfig status and asserts the
    // count is EXACTLY one.
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    let puts = Arc::new(AtomicUsize::new(0));
    // 400 is PermanentConfig: `classify_by_status` maps 400..=499 there, having
    // already special-cased 403.
    let server = tokio::spawn(serve(
        listener,
        addr.clone(),
        usize::MAX,
        "400 Bad Request",
        Arc::clone(&puts),
        Arc::new(AtomicUsize::new(0)),
    ));

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
    assert_eq!(
        n, 1,
        "a PermanentConfig status must bail on the first attempt, not be retried; \
         a whole pack body is re-sent per attempt and none of them can succeed. got {n}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_expired_pack_put_signature_is_re_presigned_not_replayed() {
    // The gap this closes was named, correctly, in the comment on the permanent
    // test above: "`classify_by_status` maps 403 to RefreshUrl -- a presigned
    // URL that expired is retryable BY DESIGN". It was retryable, but it was
    // never REFRESHED: `upload_and_register` classified RefreshUrl identically
    // to Transient and re-sent the pack against the SAME already-rejected
    // signature, so every attempt failed the same way until the 120s budget ran
    // out. The per-chunk PUT path has re-presigned on 403 for a long time; this
    // path already held `base_url` and `http_client` and simply never used them.
    //
    // Serves 403 once, then 200. Asserts BOTH halves:
    //   * the upload ultimately succeeds, and
    //   * /packs/upload-urls was requested MORE THAN ONCE.
    // The second is the load-bearing one: without it this passes against the
    // old replay-the-dead-URL behaviour, because the stub's "expired" URL still
    // works on the retry.
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    let puts = Arc::new(AtomicUsize::new(0));
    let presigns = Arc::new(AtomicUsize::new(0));
    // 403 -> RefreshUrl for every backend, via classify_by_status.
    let server = tokio::spawn(serve(
        listener,
        addr.clone(),
        1,
        "403 Forbidden",
        Arc::clone(&puts),
        Arc::clone(&presigns),
    ));

    let tmp = std::env::temp_dir().join(format!("mg-packretry-refresh-{}.tmp", std::process::id()));

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
        "an expired signature must be recovered from, not fatal. err={:?}",
        out.err()
    );
    assert_eq!(
        puts.load(Ordering::SeqCst),
        2,
        "expected 1 rejected attempt + 1 success to reach the bucket"
    );
    assert!(
        presigns.load(Ordering::SeqCst) >= 2,
        "only {} POST(s) to /packs/upload-urls: the 403 was retried against the SAME \
         dead signature instead of re-presigning. On a real backend every such retry \
         fails identically until the budget expires.",
        presigns.load(Ordering::SeqCst)
    );
}

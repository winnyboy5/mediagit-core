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

//! A pack PUT that fails at the TRANSPORT layer must be retried too — not just
//! one that comes back with a bad HTTP status.
//!
//! `pack_put_retry.rs` fixed the status case: 503/429 and friends now go through
//! `error_class::classify_auto` and get five attempts. But that loop is only
//! reachable once a response EXISTS. A `send()` that returns `Err` — connection
//! reset, timeout, DNS, TLS — propagates straight out through `?` and never
//! reaches the classifier, so it got zero retries while a 503 got five.
//!
//! I originally reasoned that was desirable: not retrying a timeout avoids
//! re-sending a whole pack body five times. `20260821-s5check` proved it wrong.
//! The AWS S5 arm failed with exactly two transport errors and no status errors
//! at all:
//!
//!   1: error sending request for url (...s3.ap-south-1.amazonaws.com/.../packs/...)
//!   2: client error (SendRequest)
//!   3: connection closed before message completed
//!
//!   1: error sending request for url (...)
//!   2: operation timed out
//!
//! Result: packsOffered=32 packsCompleted=1 perChunkProxyPUTs=65, push exit=1.
//! On a WAN link the transport failure is the COMMON case, not the exotic one —
//! S3 closing a connection mid-upload is ordinary, and every other uploader in
//! this codebase already retries it (the per-chunk MPU path, the control plane,
//! and the server's own aws-sdk-s3, which logs "Failed after 5 retries").
//! The pack path was the sole holdout.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Serves upload-urls / complete normally. For the first `drop_count` PUTs it
/// reads the request and then CLOSES the socket without answering, which is what
/// "connection closed before message completed" is on the wire. After that it
/// answers 200.
async fn serve(listener: TcpListener, addr: String, drop_count: usize, puts: Arc<AtomicUsize>) {
    loop {
        let Ok((mut sock, _)) = listener.accept().await else {
            return;
        };
        let addr = addr.clone();
        let puts = Arc::clone(&puts);
        tokio::spawn(async move {
            let mut buf = Vec::new();
            let mut tmp = [0u8; 8192];
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

            if first.starts_with("PUT") && first.contains("/bucket/pack") {
                let n = puts.fetch_add(1, Ordering::SeqCst);
                if n < drop_count {
                    // Drop it on the floor: no status line, no body. This is a
                    // TRANSPORT failure, so nothing ever reaches the status
                    // classifier - which is the entire point of this test.
                    drop(sock);
                    return;
                }
                let _ = sock
                    .write_all(reply("200 OK", "{}".into()).as_bytes())
                    .await;
                let _ = sock.flush().await;
                return;
            }

            let resp = if first.contains("/packs/upload-urls") {
                reply(
                    "200 OK",
                    format!(
                        r#"{{"{}":{{"url":"http://{}/bucket/pack","required_headers":[]}}}}"#,
                        "aa".repeat(32),
                        addr
                    ),
                )
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
async fn a_dropped_connection_on_a_pack_put_is_retried() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    let puts = Arc::new(AtomicUsize::new(0));
    let server = tokio::spawn(serve(listener, addr.clone(), 2, Arc::clone(&puts)));

    let tmp = std::env::temp_dir().join(format!("mg-packxport-{}.tmp", std::process::id()));

    mediagit_protocol::ensure_crypto_provider();
    let http = reqwest::Client::builder().build().unwrap();
    let direct = mediagit_protocol::client::data_plane_client_builder()
        .build()
        .unwrap();

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
        out.is_ok(),
        "two dropped connections must be retried, not abandoned. This is the exact \
         failure that took the AWS arm of 20260821-s5check to packsCompleted=1 and \
         exit=1: 'connection closed before message completed'. err={:?}",
        out.err()
    );

    // Load-bearing, same as the status test: without asserting the COUNT this
    // would also pass against an implementation that never retried and happened
    // to get a 200 first try.
    assert_eq!(
        puts.load(Ordering::SeqCst),
        3,
        "expected 2 dropped attempts + 1 success to reach the bucket"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn transport_retries_are_bounded() {
    // The reason I talked myself out of this fix originally: a retried pack
    // re-sends the whole body, so an unbounded transport retry is worse than no
    // retry. It must be bounded by the SAME budget the status path uses.
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    let puts = Arc::new(AtomicUsize::new(0));
    let server = tokio::spawn(serve(listener, addr.clone(), usize::MAX, Arc::clone(&puts)));

    let tmp = std::env::temp_dir().join(format!("mg-packxport-bound-{}.tmp", std::process::id()));

    mediagit_protocol::ensure_crypto_provider();
    let http = reqwest::Client::builder().build().unwrap();
    let direct = mediagit_protocol::client::data_plane_client_builder()
        .build()
        .unwrap();

    let out = tokio::time::timeout(
        std::time::Duration::from_secs(120),
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
        "a peer that never accepts must surface an error"
    );
    let n = puts.load(Ordering::SeqCst);
    assert!(
        n <= 5,
        "transport retries must be BOUNDED by the same 5-attempt budget as the \
         status path; a whole pack body is re-sent per attempt. got {n}"
    );
}

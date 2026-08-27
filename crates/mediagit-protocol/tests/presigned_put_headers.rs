// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Wire-level guard for presigned PUT header construction.
//!
//! Campaigns 20260803-scale and 20260804-scale-verify2 logged 976 and 748
//! `SignatureDoesNotMatch` responses on the per-chunk presigned-PUT fallback
//! path, against real AWS S3 and MinIO. Presigned SigV4 commits to an exact set
//! of headers, so anything the client adds, drops, or duplicates relative to
//! what the server signed invalidates the signature.
//!
//! `content-length` is NOT in aws-sigv4's `excluded_headers`, so when the server
//! presigns with a concrete length it lands in `required_headers` and is part of
//! `SignedHeaders`. The client then replays `required_headers` verbatim — so any
//! *additional* explicit `.header(CONTENT_LENGTH, ..)` is a second instance,
//! because `reqwest::RequestBuilder::header` calls `HeaderMap::append`, not
//! `insert` (verified in reqwest 0.13.4, `async_impl/request.rs:226`).
//!
//! These tests observe the raw bytes on the wire rather than asserting on a
//! `HeaderMap`, because the question that matters is what the peer's signature
//! verifier actually parses — hyper is free to normalise framing headers on the
//! way out, and only the socket can settle whether it does.

use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Accepts one connection, reads until end-of-headers, replies 200, and returns
/// the raw request head as received.
async fn capture_one_request(listener: TcpListener) -> String {
    let (mut sock, _) = listener.accept().await.expect("accept");
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = sock.read(&mut chunk).await.expect("read");
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    let _ = sock
        .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n")
        .await;
    let _ = sock.flush().await;
    String::from_utf8_lossy(&buf).to_string()
}

fn count_header(head: &str, name: &str) -> usize {
    head.lines()
        .filter(|l| {
            l.split(':')
                .next()
                .is_some_and(|k| k.trim().eq_ignore_ascii_case(name))
        })
        .count()
}

/// Reproduces the defect shape: an explicit CONTENT_LENGTH followed by replaying
/// server-signed `required_headers` that already carry one.
///
/// This test documents reqwest/hyper's actual behaviour. If hyper collapsed
/// duplicate `content-length` on its own, the defect would be inert and this
/// asserts so honestly rather than assuming.
#[tokio::test]
async fn duplicate_content_length_reaches_the_wire() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(capture_one_request(listener));

    let body: Vec<u8> = vec![7u8; 1234];
    // Exactly the shape at push.rs:946-954 — explicit set, then replay of
    // required_headers which (server-side) already contains content-length.
    let required_headers = [["content-length".to_string(), body.len().to_string()]];

    mediagit_protocol::ensure_crypto_provider();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("client");
    let mut req = client
        .put(format!("http://{addr}/chunks/deadbeef"))
        .header(reqwest::header::CONTENT_LENGTH, body.len());
    for [k, v] in &required_headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let _ = req.body(body).send().await;

    let head = tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .expect("server task timed out")
        .expect("server task panicked");

    let n = count_header(&head, "content-length");
    assert_eq!(
        n, 2,
        "expected the duplicate to reach the wire (that is the defect); \
         saw {n} content-length header(s). If this is now 1, hyper started \
         collapsing duplicates and the SignatureDoesNotMatch theory needs \
         revisiting. Raw head:\n{head}"
    );
}

/// The fixed shape: replay `required_headers` only, letting the server-signed
/// value be the single source of truth. This is the invariant the presigned
/// upload path must hold — it is what `pack_builder.rs` already does, and that
/// path shows no `SignatureDoesNotMatch` in either campaign.
#[tokio::test]
async fn required_headers_only_sends_exactly_one_content_length() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(capture_one_request(listener));

    let body: Vec<u8> = vec![7u8; 1234];
    let required_headers = [["content-length".to_string(), body.len().to_string()]];

    mediagit_protocol::ensure_crypto_provider();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("client");
    let mut req = client.put(format!("http://{addr}/chunks/deadbeef"));
    for [k, v] in &required_headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let _ = req.body(body).send().await;

    let head = tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .expect("server task timed out")
        .expect("server task panicked");

    let n = count_header(&head, "content-length");
    assert_eq!(
        n, 1,
        "presigned PUT must send exactly one content-length — SigV4 signed one. \
         Raw head:\n{head}"
    );
}

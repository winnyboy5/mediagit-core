// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! `download_file_by_path` must not hang forever on a backend that accepts the
//! connection, starts a response, and then stops sending.
//!
//! This is the same failure the crate already documents and fixed once. From
//! `client/mod.rs`, describing why the pull deadline exists:
//!
//! > a backend that accepted the connection and then stopped sending left
//! > `next_object()` awaiting forever
//!
//! That fix (`with_pull_deadline`) was applied to `download_pack_streaming` and
//! `download_chunked_objects`. `download_file_by_path` is a separate one-off
//! streaming path and was missed, so `mediagit download` — which CI and scripts
//! use against a bare URL, unattended — could still hang indefinitely.
//!
//! Deliberately an integration test rather than a `#[cfg(test)]` module: it sets
//! `MEDIAGIT_PULL_DEADLINE_SECS`, and every file under `tests/` is compiled as
//! its own binary, so the env change cannot leak into another test's process.

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Accept one request, promise a 1 MiB body, send 8 bytes, then stall forever.
///
/// Never closes the socket: a close would surface as a clean transport error
/// and the client would return on its own, which would NOT reproduce the bug.
/// The whole point is a peer that stays alive and simply stops feeding us.
async fn serve_then_stall(listener: TcpListener) {
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

    let head = "HTTP/1.1 200 OK\r\ncontent-length: 1048576\r\n\r\n";
    let _ = sock.write_all(head.as_bytes()).await;
    let _ = sock.write_all(b"12345678").await;
    let _ = sock.flush().await;

    // Hold the connection open, sending nothing further.
    std::future::pending::<()>().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn download_file_by_path_gives_up_on_a_stalled_backend() {
    // Well under the outer guard below, so a correctly-deadlined client returns
    // long before the test's own patience runs out.
    unsafe { std::env::set_var("MEDIAGIT_PULL_DEADLINE_SECS", "2") };

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(serve_then_stall(listener));

    let client = mediagit_protocol::ProtocolClient::new(format!("http://{addr}/repo"));
    let mut sink = Vec::new();

    // The load-bearing assertion is ELAPSED/RETURNED, not the error text. An
    // undeadlined client never resolves, so this outer timeout is what fails —
    // asserting only on the Err would pass vacuously if the call returned for
    // some unrelated reason.
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        client.download_file_by_path("some/file.bin", "main", &mut sink),
    )
    .await;

    server.abort();

    let inner = outcome.expect(
        "download_file_by_path never returned against a backend that stalled mid-body: \
         it is not bounded by MEDIAGIT_PULL_DEADLINE_SECS",
    );

    assert!(
        inner.is_err(),
        "a stalled backend must surface as an error, not a short successful read \
         (got Ok, meaning a truncated file would be reported as a complete download)"
    );
}

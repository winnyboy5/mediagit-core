// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! The other half of the read-timeout change: a SLOW but PROGRESSING response
//! must still succeed.
//!
//! This is the regression guard. `build_control_plane_client` deliberately has
//! no total `.timeout()` because one was added, broke healthy slow uploads to
//! Azure/S3, and was reverted. A read timeout is only safe if it genuinely
//! measures the gap BETWEEN bytes rather than total duration — if it behaved
//! like a total ceiling it would re-break exactly that case.
//!
//! Without this test, `control_plane_read_timeout.rs` alone would happily pass
//! against an implementation that capped total request time, silently
//! reintroducing the reverted regression. Proving the guard FIRES is half a
//! proof; proving it stays QUIET on healthy traffic is the other half.

use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Reply in dribs and drabs: total wall time well past the read timeout, but
/// never a gap between bytes longer than it.
async fn serve_slowly(listener: TcpListener, chunks: usize, gap: Duration) {
    let Ok((mut sock, _)) = listener.accept().await else {
        return;
    };
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        let Ok(n) = sock.read(&mut tmp).await else {
            return;
        };
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }

    // A refs listing the client will parse; sent one small piece at a time.
    let body = r#"{"refs":[],"capabilities":[]}"#.to_string();
    let head = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = sock.write_all(head.as_bytes()).await;
    let _ = sock.flush().await;

    let bytes = body.into_bytes();
    let per = bytes.len().div_ceil(chunks).max(1);
    for piece in bytes.chunks(per) {
        tokio::time::sleep(gap).await;
        if sock.write_all(piece).await.is_err() {
            return;
        }
        let _ = sock.flush().await;
    }
    let _ = sock.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_slow_but_progressing_response_is_not_killed() {
    // 2s read timeout, 1s between pieces, ~6 pieces => ~6s total wall time.
    // A TOTAL timeout of 2s would kill this; a READ timeout must not, because
    // no single inter-byte gap ever reaches 2s.
    unsafe { std::env::set_var("MEDIAGIT_CONTROL_READ_TIMEOUT_SECS", "2") };

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(serve_slowly(listener, 6, Duration::from_secs(1)));

    let client = mediagit_protocol::ProtocolClient::new(format!("http://{addr}/repo"));

    let started = std::time::Instant::now();
    let outcome = tokio::time::timeout(Duration::from_secs(40), client.get_refs()).await;
    let elapsed = started.elapsed();

    server.abort();

    let inner = outcome.expect("get_refs did not return at all");
    assert!(
        inner.is_ok(),
        "a slow but continuously-progressing response was killed: the read \
         timeout is behaving like a TOTAL timeout, which is the Azure regression \
         that was previously reverted. elapsed={elapsed:?}, err={:?}",
        inner.err()
    );

    // Load-bearing: it really did outlive the timeout window. If the response
    // completed in under 2s this test would pass without exercising anything.
    assert!(
        elapsed >= Duration::from_secs(3),
        "expected the transfer to outlast the 2s read timeout (proving the gap, \
         not the total, is what is measured) but it finished in {elapsed:?}"
    );
}

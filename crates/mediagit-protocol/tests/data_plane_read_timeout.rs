// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! The DATA plane needs the same silent-peer bound the control plane got, and
//! for the same reason — but it is not the same fix, because the two planes
//! were not in the same state.
//!
//! Before this: four separate call sites each built their own data-plane client
//! (`client/packs.rs`, `client/pull.rs`, `client/push.rs` x2) and they did not
//! agree. Three carried `.timeout(300)`; the DOWNLOAD client carried no request
//! bound at all. `tcp_keepalive` was on three of the four.
//!
//! So the download path had nothing between it and
//! `MEDIAGIT_PULL_DEADLINE_SECS` — an ABSOLUTE 3600s ceiling on the whole
//! phase. One presigned GET going silent stalls an entire clone for up to an
//! hour before anything reports, and when it does the error names the phase,
//! not the socket.
//!
//! `.read_timeout()` bounds the gap BETWEEN bytes, so it is safe where a total
//! `.timeout()` is not: a 40-minute legitimate download of a large object
//! resets it on every byte and never trips, while a peer that accepts and stops
//! talking is caught in seconds. That distinction is why the total timeout was
//! removed from the control plane (it broke healthy slow Azure uploads) and why
//! this is NOT reintroducing it — `data_plane_client_builder` deliberately
//! leaves `.timeout()` to each call site, so downloads stay unbounded in TOTAL
//! and bounded per-stall.
//!
//! Both halves are proven here. A guard only seen to fire is half a guard: the
//! second test is the regression barrier against someone "simplifying" the read
//! timeout into a total one, which would pass the first test and silently break
//! every slow transfer.

use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Accept, drain the request, then hold the socket open and never reply.
///
/// Closing would surface as a clean transport error and the client would return
/// on its own — which is not the failure being reproduced. The point is a peer
/// that stays ALIVE and stops ANSWERING, which keeps keepalive satisfied.
async fn accept_and_never_answer(listener: TcpListener) {
    loop {
        let Ok((mut sock, _)) = listener.accept().await else {
            return;
        };
        tokio::spawn(async move {
            let mut buf = [0u8; 4096];
            loop {
                match sock.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
            std::future::pending::<()>().await;
        });
    }
}

/// Answer, then dribble the body one byte per second: always progressing, never
/// fast. Stands in for a real large object over a slow link.
async fn accept_and_dribble(listener: TcpListener, body_len: usize, gap: Duration) {
    loop {
        let Ok((mut sock, _)) = listener.accept().await else {
            return;
        };
        tokio::spawn(async move {
            let mut buf = [0u8; 4096];
            // Read just the request head.
            let _ = sock.read(&mut buf).await;
            let head = format!(
                "HTTP/1.1 200 OK\r\ncontent-length: {body_len}\r\nConnection: close\r\n\r\n"
            );
            if sock.write_all(head.as_bytes()).await.is_err() {
                return;
            }
            let _ = sock.flush().await;
            for _ in 0..body_len {
                tokio::time::sleep(gap).await;
                if sock.write_all(b"x").await.is_err() {
                    return;
                }
                let _ = sock.flush().await;
            }
        });
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_data_plane_get_gives_up_on_a_silent_peer() {
    unsafe { std::env::set_var("MEDIAGIT_DATA_READ_TIMEOUT_SECS", "3") };

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(accept_and_never_answer(listener));

    let client = mediagit_protocol::client::data_plane_client_builder()
        .build()
        .expect("build data-plane client");

    // Load-bearing: the assertion is that the call RETURNS. Without a read
    // timeout it never resolves and this outer guard is what fails. Asserting
    // only on `is_err()` would pass vacuously if it returned for some unrelated
    // reason, so the timeout wrapper is the real test and the Err is the detail.
    let outcome = tokio::time::timeout(
        Duration::from_secs(40),
        client.get(format!("http://{addr}/bucket/object")).send(),
    )
    .await;

    server.abort();

    let resolved = outcome.expect(
        "a presigned GET against a peer that accepts and goes silent must give up. \
         It did not — the download client has no per-socket bound and the only thing \
         left is the ABSOLUTE 3600s MEDIAGIT_PULL_DEADLINE_SECS.",
    );
    assert!(
        resolved.is_err(),
        "a silent peer must surface an error, not a response"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_slow_but_progressing_download_is_not_killed() {
    // The regression barrier. A total `.timeout(4)` would fail this — the
    // transfer takes ~8s — while a READ timeout of 4s never trips, because no
    // single inter-byte gap exceeds 1s. This is exactly the distinction that
    // broke healthy Azure uploads when a total timeout was tried before.
    unsafe { std::env::set_var("MEDIAGIT_DATA_READ_TIMEOUT_SECS", "4") };

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(accept_and_dribble(listener, 8, Duration::from_secs(1)));

    let client = mediagit_protocol::client::data_plane_client_builder()
        .build()
        .expect("build data-plane client");

    let outcome = tokio::time::timeout(Duration::from_secs(40), async {
        let resp = client
            .get(format!("http://{addr}/bucket/object"))
            .send()
            .await?;
        resp.bytes().await
    })
    .await;

    server.abort();

    let bytes = outcome
        .expect("slow download hung")
        .expect("a slow but PROGRESSING download must not be killed by the read timeout");
    assert_eq!(
        bytes.len(),
        8,
        "the whole body must arrive; a truncated read means the bound fired mid-transfer"
    );
}

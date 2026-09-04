// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! `read_timeout` is a stall detector on a DOWNLOAD and a total deadline on an
//! UPLOAD, and the difference silently broke every large cloud pack push.
//!
//! On a download the timer resets on every byte received, so a slow but moving
//! transfer never trips it — that is what `data_plane_read_timeout.rs` proves.
//! On an upload the client is WRITING. The bucket correctly sends nothing until
//! the body completes, so no read ever arrives to reset the timer, and a flat
//! value becomes a hard ceiling on how long the upload may take.
//!
//! Measured 2026-09-04 against real Azure, 2 GB as 32 packs of 64 MiB at
//! concurrency 8:
//!
//!   MEDIAGIT_DATA_READ_TIMEOUT_SECS=300    6/32 packs landed, push 918.66s
//!   MEDIAGIT_DATA_READ_TIMEOUT_SECS=1800  32/32 packs landed, push 368.27s
//!
//! Four packs in the failing run died at EXACTLY 300.0s within one second of
//! each other. Four connections going quiet independently do not do that; a
//! deadline measured from request start does. In the passing run the first wave
//! registered at 106..269s, so the old bound was leaving a 10% margin on work
//! whose cost is (pack size x concurrency / bandwidth) — a constant bound
//! against a variable cost, which is a cliff rather than a guard. That is why
//! ga38/ga39/ga40 recorded 32, 16 and 0 packs landed and read as flakiness.
//!
//! Both halves are proven, because a guard only ever seen to fire is half a
//! guard. The first test shows the flat bound really does kill a healthy upload
//! (without it the fix guards nothing); the second shows the size-derived bound
//! lets the same upload through. Anyone "simplifying" the upload client back to
//! the shared one fails the second test.
//!
//! Own file, not appended to `data_plane_read_timeout.rs`: these set
//! MEDIAGIT_DATA_READ_TIMEOUT_SECS process-wide, and cargo gives each
//! integration test file its own process. Sharing a file would race the env var
//! against tests that need a different value.
//!
//! That is only half the problem, and the first version of this file got the
//! other half wrong: the three tests here ALSO disagree about the value (4, 4,
//! 0) and cargo runs them in parallel THREADS within this one process. The
//! `=0` case cleared the `4` out from under the first test, which then observed
//! no timeout and failed — a fake failure that reads exactly like "the premise
//! is wrong". `build_client_with` below is the fix: the env is set and the
//! client BUILT under one lock, and reqwest reads the variable only while
//! building, so once a `Client` exists it no longer cares what the variable
//! says. The slow network part still runs concurrently.

use std::sync::Mutex;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Serialises `set_var` + `build()`, which is the only window that matters.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Set the read-timeout knob and build a client under the lock.
///
/// `make` picks the constructor under test, so a test names the one thing it is
/// actually varying and nothing else.
fn build_client_with(secs: &str, make: impl FnOnce() -> reqwest::ClientBuilder) -> reqwest::Client {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe { std::env::set_var("MEDIAGIT_DATA_READ_TIMEOUT_SECS", secs) };
    make().build().expect("build client")
}

/// Accept a request, read what arrives, stay silent for `quiet`, then answer.
///
/// This is a well-behaved bucket receiving an upload, not a broken one: HTTP
/// says the response comes after the request body, so a peer that is still
/// accepting a large PUT looks exactly like this from the client side. The
/// point of the test is that a read timeout cannot tell the two apart.
async fn accept_then_answer_after(listener: TcpListener, quiet: Duration) {
    if let Ok((mut sock, _)) = listener.accept().await {
        let mut buf = [0u8; 8192];
        // One read is enough: we only need the request line to have arrived.
        let _ = sock.read(&mut buf).await;
        tokio::time::sleep(quiet).await;
        let _ = sock
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            .await;
        let _ = sock.flush().await;
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// The bound the fix derives, for the sizes this test uses.
///
/// 1 MiB / 16 KiB/s = 64s, so `max(4, 64)` = 64s — comfortably past the 6s the
/// stub stays quiet, while the flat bound is 4s and is not.
const BODY_HINT_BYTES: u64 = 1024 * 1024;

#[tokio::test(flavor = "multi_thread")]
async fn a_flat_read_timeout_kills_an_upload_that_is_still_healthy() {
    // The half that proves the bug is real. Remove the fix and this is what
    // every 64 MiB pack on azure/gcs hit: the peer is fine, the transfer is
    // fine, and the clock runs out anyway.
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(accept_then_answer_after(listener, Duration::from_secs(6)));

    let client = build_client_with("4", mediagit_protocol::client::data_plane_client_builder);

    let outcome = tokio::time::timeout(
        Duration::from_secs(40),
        client
            .put(format!("http://{addr}/bucket/pack"))
            .body(vec![0u8; 4096])
            .send(),
    )
    .await;

    server.abort();

    let resolved = outcome.expect("the request must resolve, not hang");
    assert!(
        resolved.is_err(),
        "a 4s read timeout must kill an upload whose peer stays silent for 6s. \
         It did not, which means the premise of pack_upload_timeout is wrong and \
         the size-derived bound below is guarding nothing."
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_size_derived_bound_lets_the_same_upload_through() {
    // The regression barrier. Identical server, identical 6s of silence; the
    // only difference is which constructor built the client. This is the test
    // that fails if someone routes pack uploads back through
    // `data_plane_client_builder`.
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(accept_then_answer_after(listener, Duration::from_secs(6)));

    let client = build_client_with("4", || {
        mediagit_protocol::client::data_plane_upload_client_builder(BODY_HINT_BYTES)
    });

    let outcome = tokio::time::timeout(
        Duration::from_secs(40),
        client
            .put(format!("http://{addr}/bucket/pack"))
            .body(vec![0u8; 4096])
            .send(),
    )
    .await;

    server.abort();

    let resolved = outcome.expect("the request must resolve, not hang");
    let resp = resolved.expect(
        "an upload whose peer is merely slow to answer must NOT be killed. It was — \
         so the pack fast path will collapse to the per-chunk path on any link where \
         a pack takes longer than the flat read timeout, which is what azure and gcs \
         measured at 6/32 and 5/32 packs landed.",
    );
    assert!(
        resp.status().is_success(),
        "expected the stub's 200, got {}",
        resp.status()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_opt_out_still_disables_the_bound() {
    // MEDIAGIT_DATA_READ_TIMEOUT_SECS=0 is the documented escape hatch for the
    // flat bound. The new constructor must honour it too, or the one lever an
    // operator has for a pathologically slow link stops working on exactly the
    // path that needs it most.
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(accept_then_answer_after(listener, Duration::from_secs(6)));

    let client = build_client_with("0", || {
        mediagit_protocol::client::data_plane_upload_client_builder(BODY_HINT_BYTES)
    });

    let outcome = tokio::time::timeout(
        Duration::from_secs(40),
        client
            .put(format!("http://{addr}/bucket/pack"))
            .body(vec![0u8; 4096])
            .send(),
    )
    .await;

    server.abort();

    let resp = outcome
        .expect("the request must resolve, not hang")
        .expect("with the bound disabled the slow answer must still be accepted");
    assert!(resp.status().is_success());
}

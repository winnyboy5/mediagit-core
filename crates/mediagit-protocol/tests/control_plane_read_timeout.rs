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

//! A control-plane call against a peer that ACCEPTS and then goes silent must
//! not hang forever.
//!
//! `build_control_plane_client` deliberately sets no per-request `.timeout()`,
//! and that decision is correct: a chunked-blob PUT to Azure/S3 can legitimately
//! run for minutes, and a hard request ceiling caused spurious failures on
//! healthy slow uploads (the regression that motivated removing it). This test
//! must NOT push us back into that.
//!
//! But its stated safety net — "tcp_keepalive (30s) already detects truly dead
//! peers" — only covers peers that are *dead*. A peer that is alive, holds the
//! connection open and simply never answers keeps keepalive satisfied while the
//! client waits forever.
//!
//! That is not hypothetical. Captured live on 2026-08-21 (20260821-rl6hunt12):
//! a clone that normally takes 0.55s sat for 102s with cpu 0.11s -> 0.14s across
//! a 10s sample (flat), all six threads in Wait, holding one Established
//! connection, having completed `encryption-key` + `info/refs` and issued
//! nothing since. Same shape in 20260820-ga4 and 20260820-ga8.
//!
//! The fix is a READ timeout, not a total timeout: it bounds the gap BETWEEN
//! bytes, so a slow-but-progressing transfer never trips it while a peer that
//! goes silent does. That keeps the Azure regression fixed and closes the hang.

use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;

/// Accept, drain the request, then never reply while holding the socket open.
///
/// Closing would surface as a clean transport error and the client would return
/// on its own — which is precisely NOT the failure being reproduced. The whole
/// point is a peer that stays alive and stops talking.
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

#[tokio::test(flavor = "multi_thread")]
async fn control_plane_call_gives_up_on_a_silent_peer() {
    unsafe { std::env::set_var("MEDIAGIT_CONTROL_READ_TIMEOUT_SECS", "3") };

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(accept_and_never_answer(listener));

    let client = mediagit_protocol::ProtocolClient::new(format!("http://{addr}/repo"));

    // Load-bearing assertion: the call RETURNS. Without a read timeout it never
    // resolves and the outer guard is what fails. Asserting only on the Err
    // would pass vacuously if it returned for an unrelated reason.
    let outcome = tokio::time::timeout(Duration::from_secs(40), client.get_refs()).await;

    server.abort();

    let inner = outcome.expect(
        "get_refs never returned against a peer that accepted the connection and \
         then went silent: the control-plane client is unbounded",
    );

    assert!(
        inner.is_err(),
        "a silent peer must surface as an error, not a successful empty result"
    );
}

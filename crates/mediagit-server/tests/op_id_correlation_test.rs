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

//! OP-7: proves the client's correlation id actually reaches a server log
//! line, not just that the header gets set. A real `ProtocolClient` (the
//! same code a CLI push/pull/clone uses) makes a real HTTP request against a
//! real running server; the assertion is on the captured `tracing` output of
//! that server process, matching the client's own minted id against the log
//! line the handler emits deep inside request handling.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;

/// Captures everything written to it so the test can inspect log output
/// after the request completes. Cloned handles share the same buffer.
#[derive(Clone, Default)]
struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for CapturedLogs {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CapturedLogs {
    type Writer = CapturedLogs;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

async fn start_test_server(repos_dir: PathBuf) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");

    let state = Arc::new(mediagit_server::AppState::new(repos_dir));
    let app = mediagit_server::create_router(state);
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    (base_url, handle)
}

/// Single-threaded runtime: `tracing::subscriber::set_default` is
/// thread-local, and the spawned server task must run on the same OS thread
/// as the client request for the capture to see its log lines.
#[tokio::test]
async fn client_op_id_appears_on_the_servers_log_line() {
    let logs = CapturedLogs::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(logs.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    // No repo created — `download_chunk`'s handler hits its "Repository not
    // found" warn (crates/mediagit-server/src/handlers/chunks.rs) before
    // ever touching storage. That's the log line this test correlates
    // against: a real handler-level log emitted deep inside request
    // handling, exactly where OP-7 was written to help (storage-layer
    // errors), not the top-level request/response line TraceLayer itself
    // would emit either way.
    let repos_dir = tempfile::TempDir::new().unwrap().keep();
    let (base_url, _server) = start_test_server(repos_dir).await;

    let client = mediagit_protocol::ProtocolClient::new(format!("{base_url}/no-such-repo"));
    let op_id = client.operation_id().to_string();

    let chunk_id = mediagit_versioning::Oid::from_bytes([0x11u8; 32]);
    let result = client.download_chunk(&chunk_id).await;
    assert!(result.is_err(), "repo doesn't exist, download must fail");

    // Poll for the line rather than reading once. The client's response
    // arriving does NOT mean the server has finished emitting its log: the
    // handler's event is written on the server task, which the runtime may not
    // have polled again by the time we get here. Reading immediately failed 4
    // runs in 5 with an EMPTY capture — a race that reads exactly like "the id
    // is missing", i.e. the same symptom as the defect this test exists to
    // catch. Bounded so a genuine absence still fails fast rather than hanging.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let captured = loop {
        let snapshot = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
        if snapshot.contains("Repository not found") || std::time::Instant::now() >= deadline {
            break snapshot;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    };
    let log_line = captured
        .lines()
        .find(|l| l.contains("Repository not found"))
        .unwrap_or_else(|| panic!("expected a 'Repository not found' log line, got:\n{captured}"));

    assert!(
        log_line.contains(&op_id),
        "the server's 'Repository not found' log line must carry the \
         client's own correlation id ({op_id}) so the two can be joined — \
         got:\n{log_line}"
    );

    // ---- second half: an absent header must be harmless ----
    //
    // Deliberately part of THIS test rather than its own. `set_default` above
    // is thread-local, and with the two as separate `#[tokio::test]`s the
    // capture came up EMPTY in 2-4 runs out of 6 while they executed
    // concurrently — verified: the correlation half passes 6/6 in isolation
    // and fails intermittently only alongside a sibling. An empty capture
    // reads exactly like "the id is missing", i.e. it fails as though the
    // feature were broken. Sequencing them removes the interference outright,
    // which beats a sleep or a retry that only widens the window.
    missing_op_id_header_is_harmless().await;
}

/// Absent header must be harmless: a bare `reqwest` request with no
/// `OP_ID_HEADER` at all (curl, a health check, an older client) must not be
/// rejected -- it should behave exactly as before OP-7, just with a
/// server-minted id that can't be joined back to a client-side log.
async fn missing_op_id_header_is_harmless() {
    let repos_dir = tempfile::TempDir::new().unwrap().keep();
    let (base_url, _server) = start_test_server(repos_dir).await;

    // Install the rustls provider explicitly. reqwest is built with
    // `rustls-no-provider`, so `Client::new()` PANICS without one. The sibling
    // test gets it for free because `ProtocolClient::new` installs it — which
    // meant this test only passed when that one happened to run first, and both
    // are async tests on the same process. Relying on that ordering is a flake
    // waiting for a slower machine. The helper is idempotent.
    mediagit_protocol::ensure_crypto_provider();
    let response = reqwest::Client::new()
        .get(format!(
            "{base_url}/no-such-repo/chunks/1111111111111111111111111111111111111111111111111111111111111111"
        ))
        .send()
        .await
        .expect("request must not be rejected for lacking the op-id header");

    // Same outcome the header-carrying request gets: 404 because the repo
    // doesn't exist, not some auth/validation failure caused by the missing
    // header.
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
}

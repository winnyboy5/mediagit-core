// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! `GET /info/refs` must not starve the async runtime.
//!
//! ## What this test settled — read this before trusting the hypothesis it came from
//!
//! It was written to prove a *specific* explanation of campaign ga49, in which
//! the server's `accepted` counter climbed 4 -> 9 while `routed` stayed frozen
//! at 13 and `idle_s` reached 699: five TCP connections arrived and not one of
//! their requests ever reached the router. The endpoint was `/info/refs`.
//!
//! The hypothesis was worker starvation. `get_refs` (`handlers/repo.rs`) walks
//! `.mediagit/refs` with **synchronous** `std::fs::read_dir` plus a synchronous
//! `is_dir()`/`is_file()` stat per entry, inline in an `async fn`, with no
//! `spawn_blocking`. That is a genuine blocking-in-async violation, and
//! "accepted but never routed" is exactly the shape worker exhaustion produces.
//!
//! **The hypothesis is REFUTED, and this test is the measurement that refuted
//! it.** With 4000 refs a single `/info/refs` costs ~900 ms of real work, yet
//! `/health` is still served promptly while six of them are in flight on a
//! two-worker runtime — roughly 2.7 s of would-be occupancy against 2 workers.
//! Had the handler monopolised its worker, `/health` could not have been
//! answered at all.
//!
//! The reason it does not starve: `refdb.read(..).await` sits **inside** the
//! walk loop, so the handler yields to the scheduler on every single ref. The
//! blocking syscalls are fine-grained and continuously interleaved with yield
//! points, so no worker is held long enough to stall the runtime. The blocking
//! is real; the starvation is not.
//!
//! So ga49 remains unexplained, and `get_refs` is not the culprit. Do not
//! "fix" it with `spawn_blocking` expecting a stall to go away — that change
//! would be unfalsifiable by this test, which passes either way.
//!
//! ## What this test is now
//!
//! A regression guard on the invariant the measurement established: `/health`
//! stays responsive while `/info/refs` is under load. It passes today. It would
//! start failing if someone removed the per-ref `.await` from the walk — for
//! instance by batching the ref reads — which would convert the fine-grained
//! interleaving into one long uninterrupted block and create the starvation
//! this test was originally written to find.
//!
//! Note the separate, real finding it exposed: ~900 ms for 4000 refs is slow in
//! its own right. That is a performance question, not a correctness one, and is
//! deliberately not addressed here.

use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio::net::TcpListener;

/// Workers on the test runtime. Small on purpose: the fault is worker
/// exhaustion, and a 2-worker runtime reaches it with a handful of requests
/// instead of needing to match a production core count.
const WORKERS: usize = 2;

/// Concurrent `/info/refs` requests. Must exceed `WORKERS` so that, if the
/// handler blocks, there is no worker left for `/health`.
const CONCURRENT_REFS_REQUESTS: usize = 6;

/// Refs written into the test repo. The walk is one `read_dir` plus two stats
/// per entry, so this is ~3x this many syscalls per request — enough to hold a
/// worker measurably, without making the fixture slow to build.
const REF_COUNT: usize = 4000;

/// Budget for `/health` while `/info/refs` is in flight.
///
/// Generous by design. `/health` is a constant-response handler; on an
/// unblocked runtime it answers in single-digit milliseconds even under load.
/// Anything approaching this bound means it was waiting for a worker, not
/// doing work. Kept loose so the test does not become a flaky timing assertion
/// on a busy or degraded machine — this box is known to have degraded, and a
/// tight bound here would fail for the wrong reason.
const HEALTH_BUDGET: Duration = Duration::from_secs(2);

/// Build a repo whose ref tree is big enough for the walk to cost real time.
///
/// Refs are spread across nested directories because `get_refs` recurses, and a
/// flat directory would exercise only one `read_dir` call.
fn make_repo_with_many_refs(repos_dir: &std::path::Path, repo: &str) {
    let refs_dir = repos_dir.join(repo).join(".mediagit/refs/heads");
    std::fs::create_dir_all(&refs_dir).unwrap();
    // A plausible 40-hex oid; contents are irrelevant to the walk's cost.
    let oid = "0".repeat(64);
    for i in 0..REF_COUNT {
        // 40 subdirectories, so the walker recurses instead of doing one big
        // read_dir, matching the shape of a real repo with namespaced branches.
        let sub = refs_dir.join(format!("ns{:02}", i % 40));
        if i < 40 {
            std::fs::create_dir_all(&sub).unwrap();
        }
        std::fs::write(sub.join(format!("branch-{i:05}")), &oid).unwrap();
    }
}

async fn start_server(repos_dir: std::path::PathBuf) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = Arc::new(mediagit_server::AppState::new(repos_dir));
    let app = mediagit_server::create_router(state);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    // Let the listener reach accept() before the first request.
    tokio::time::sleep(Duration::from_millis(100)).await;
    format!("http://{addr}")
}

#[test]
fn info_refs_does_not_starve_the_runtime() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(WORKERS)
        .enable_all()
        .build()
        .unwrap();

    // reqwest here is built with `rustls-no-provider`, so constructing a Client
    // panics unless a crypto provider is installed first. Without this the test
    // fails while BUILDING the client -- red for the wrong reason, and it would
    // fail identically with the fix applied. Same idiom as the other server e2e
    // tests. See project_ring_tls_consistency.
    mediagit_protocol::ensure_crypto_provider();

    rt.block_on(async {
        let tmp = TempDir::new().unwrap();
        let repos_dir = tmp.path().to_path_buf();
        make_repo_with_many_refs(&repos_dir, "starve-test");
        let base = start_server(repos_dir).await;

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap();

        // Prove /health is fast on an idle runtime. If this fails the test is
        // broken, not the server, and the measurement below would be worthless.
        let warm = Instant::now();
        let resp = client.get(format!("{base}/health")).send().await.unwrap();
        assert!(resp.status().is_success(), "/health unhealthy before load");
        let idle_latency = warm.elapsed();
        assert!(
            idle_latency < HEALTH_BUDGET,
            "/health took {idle_latency:?} on an IDLE runtime - the test fixture \
             is wrong, not the server"
        );

        // Put the workers under /info/refs load.
        let mut inflight = Vec::new();
        for _ in 0..CONCURRENT_REFS_REQUESTS {
            let c = client.clone();
            let url = format!("{base}/starve-test/info/refs");
            inflight.push(tokio::spawn(async move { c.get(url).send().await }));
        }

        // Give the handlers a moment to actually enter the filesystem walk;
        // measuring instantly would race the requests being dispatched.
        tokio::time::sleep(Duration::from_millis(150)).await;

        let under_load = Instant::now();
        let health = client.get(format!("{base}/health")).send().await;
        let loaded_latency = under_load.elapsed();

        for h in inflight {
            let _ = h.await;
        }

        let health = health.expect("/health failed outright while /info/refs was in flight");
        assert!(
            health.status().is_success(),
            "/health returned {} under /info/refs load",
            health.status()
        );
        assert!(
            loaded_latency < HEALTH_BUDGET,
            "/health took {loaded_latency:?} (idle: {idle_latency:?}) while \
             {CONCURRENT_REFS_REQUESTS} /info/refs requests were in flight on a \
             {WORKERS}-worker runtime. The handler is holding workers inside \
             synchronous std::fs calls - this is the ga49 'accepted but never \
             routed' mechanism."
        );
    });
}

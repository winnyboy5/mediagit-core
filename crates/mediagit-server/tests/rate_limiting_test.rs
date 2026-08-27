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

//! Rate limiting integration tests
//!
//! Tests the tower_governor rate limiting middleware with MediaGit server.

use axum::http::StatusCode;
use mediagit_server::{AppState, RateLimitConfig, create_router_with_rate_limit};
use reqwest::Client;
use std::net::SocketAddr;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

/// Test server with rate limiting enabled
struct TestServer {
    addr: SocketAddr,
    _temp_dir: TempDir,
    shutdown_tx: Option<oneshot::Sender<()>>,
}

impl TestServer {
    async fn new_with_rate_limit(rate_config: RateLimitConfig) -> Self {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let repos_dir = temp_dir.path().join("repos");
        tokio::fs::create_dir_all(&repos_dir).await.unwrap();

        let state = Arc::new(AppState::new(repos_dir));
        let (router, cleanup) = create_router_with_rate_limit(state, rate_config);

        // Spawn cleanup task in background
        std::thread::spawn(cleanup);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

        tokio::spawn(async move {
            axum::serve(
                listener,
                router.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(async {
                shutdown_rx.await.ok();
            })
            .await
            .expect("Server failed");
        });

        // Wait for server to start
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        Self {
            addr,
            _temp_dir: temp_dir,
            shutdown_tx: Some(shutdown_tx),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

#[tokio::test]
async fn test_rate_limit_allows_requests_within_limit() {
    // Allow 10 requests per second with burst of 20
    let rate_config = RateLimitConfig::new(10, 20);
    let server = TestServer::new_with_rate_limit(rate_config).await;
    mediagit_protocol::ensure_crypto_provider();
    let client = Client::new();

    // Send 10 requests within burst limit - all should succeed
    for i in 0..10 {
        let resp = client
            .get(server.url("/test-repo/info/refs"))
            .send()
            .await
            .unwrap();

        // Should get either 200 OK or 404 NOT FOUND (repo doesn't exist)
        // but NOT 429 TOO MANY REQUESTS
        assert_ne!(
            resp.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "Request {} was rate limited unexpectedly",
            i + 1
        );
    }
}

#[tokio::test]
async fn test_rate_limit_blocks_requests_exceeding_burst() {
    // Very restrictive: 1 request per second with burst of 2
    let rate_config = RateLimitConfig::new(1, 2);
    let server = TestServer::new_with_rate_limit(rate_config).await;
    mediagit_protocol::ensure_crypto_provider();
    let client = Client::new();

    // First 2 requests should succeed (within burst)
    for i in 0..2 {
        let resp = client
            .get(server.url("/test-repo/info/refs"))
            .send()
            .await
            .unwrap();

        assert_ne!(
            resp.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "Request {} was rate limited unexpectedly",
            i + 1
        );
    }

    // Third request should be rate limited
    let resp = client
        .get(server.url("/test-repo/info/refs"))
        .send()
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "Request 3 should have been rate limited"
    );
}

#[tokio::test]
async fn test_rate_limit_headers_present() {
    // Standard rate limit config
    let rate_config = RateLimitConfig::new(10, 20);
    let server = TestServer::new_with_rate_limit(rate_config).await;
    mediagit_protocol::ensure_crypto_provider();
    let client = Client::new();

    let resp = client
        .get(server.url("/test-repo/info/refs"))
        .send()
        .await
        .unwrap();

    // Rate limit headers should be present (because we configured .use_headers())
    let headers = resp.headers();

    // Check for x-ratelimit-limit header
    assert!(
        headers.contains_key("x-ratelimit-limit")
            || headers.contains_key("ratelimit-limit")
            || headers.contains_key("x-rate-limit-limit"),
        "Rate limit headers should be present. Headers: {:?}",
        headers
    );
}

#[tokio::test]
async fn test_rate_limit_replenishment() {
    // 2 requests per second with burst of 2 (token period = 500ms)
    let rate_config = RateLimitConfig::new(2, 2);
    let server = TestServer::new_with_rate_limit(rate_config).await;
    mediagit_protocol::ensure_crypto_provider();
    let client = Client::new();

    // Use up the burst (2 requests)
    for _ in 0..2 {
        client
            .get(server.url("/test-repo/info/refs"))
            .send()
            .await
            .unwrap();
    }

    // Third request should be rate limited
    let resp = client
        .get(server.url("/test-repo/info/refs"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);

    // Wait for replenishment: 3000ms gives a 6× safety margin over the 500ms token period.
    // This is generous enough to be reliable even on loaded CI runners.
    tokio::time::sleep(tokio::time::Duration::from_millis(3000)).await;

    // Request should now succeed (tokens replenished)
    let resp = client
        .get(server.url("/test-repo/info/refs"))
        .send()
        .await
        .unwrap();
    assert_ne!(
        resp.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "Request after replenishment should succeed"
    );
}

#[tokio::test]
async fn test_rate_limit_per_ip_isolation() {
    // Note: This test simulates different IPs by using different ports
    // In production, SmartIpKeyExtractor would check x-forwarded-for headers

    let rate_config = RateLimitConfig::new(1, 1);
    let server = TestServer::new_with_rate_limit(rate_config).await;

    // Each client connection will have a different socket address
    mediagit_protocol::ensure_crypto_provider();
    let client1 = Client::new();
    mediagit_protocol::ensure_crypto_provider();
    let client2 = Client::new();

    // Client 1: use up its quota
    let resp1 = client1
        .get(server.url("/test-repo/info/refs"))
        .send()
        .await
        .unwrap();
    assert_ne!(resp1.status(), StatusCode::TOO_MANY_REQUESTS);

    // Client 1: second request should be rate limited
    let resp1_second = client1
        .get(server.url("/test-repo/info/refs"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp1_second.status(), StatusCode::TOO_MANY_REQUESTS);

    // Client 2: should still work (different IP/socket)
    // Note: This may not work as expected because both clients appear from 127.0.0.1
    // In real deployment with reverse proxy, x-forwarded-for would differentiate
    let resp2 = client2
        .get(server.url("/test-repo/info/refs"))
        .send()
        .await
        .unwrap();

    // This assertion documents expected behavior - may be same IP in test environment
    println!(
        "Client 2 status: {:?} (may be rate limited if same IP)",
        resp2.status()
    );
}

#[tokio::test]
async fn test_rate_limit_config_default_values() {
    // Sized for bulk media transfer and keyed per-identity, not per-IP: a
    // large push falls back to one request per chunk when packs are
    // unavailable, and the old 100/200 budget rejected healthy pushes.
    let config = RateLimitConfig::default();
    assert_eq!(config.requests_per_second, 1000);
    assert_eq!(config.burst_size, 2000);
}

#[tokio::test]
async fn test_rate_limit_config_custom_values() {
    let config = RateLimitConfig::new(50, 100);
    assert_eq!(config.requests_per_second, 50);
    assert_eq!(config.burst_size, 100);
}

#[tokio::test]
async fn test_rate_limit_with_high_throughput() {
    // Generous rate limit: 100 req/s with burst of 200
    let rate_config = RateLimitConfig::new(100, 200);
    let server = TestServer::new_with_rate_limit(rate_config).await;
    mediagit_protocol::ensure_crypto_provider();
    let client = Client::new();

    // Send 50 requests quickly - all should succeed
    let mut tasks = Vec::new();
    for _ in 0..50 {
        let client = client.clone();
        let url = server.url("/test-repo/info/refs");
        let task = tokio::spawn(async move { client.get(&url).send().await.unwrap().status() });
        tasks.push(task);
    }

    let results: Vec<StatusCode> = futures::future::join_all(tasks)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();

    // Count rate limited responses
    let rate_limited_count = results
        .iter()
        .filter(|&&status| status == StatusCode::TOO_MANY_REQUESTS)
        .count();

    // With burst of 200, we should handle 50 concurrent requests
    assert!(
        rate_limited_count == 0,
        "With burst of 200, should handle 50 requests. Rate limited: {}",
        rate_limited_count
    );
}

/// SV-1: the second listener (HTTPS in production) must enforce the *same*
/// budget as the first, not its own.
///
/// `main.rs` built the HTTPS router with `create_router` regardless of
/// configuration, so turning TLS on silently turned rate limiting off for the
/// port most likely to be exposed — while startup still logged "Rate limiting
/// ENABLED". Sharing rather than rebuilding the limiter also matters: two
/// independent governors would hand an attacker twice the budget for simply
/// splitting traffic across the two ports.
#[tokio::test]
async fn second_listener_shares_the_rate_limit_budget() {
    use mediagit_server::{create_rate_limited_router, create_router_sharing_rate_limit};

    let temp_dir = TempDir::new().unwrap();
    let repos_dir = temp_dir.path().join("repos");
    tokio::fs::create_dir_all(&repos_dir).await.unwrap();
    let state = Arc::new(AppState::new(repos_dir));

    // 1 req/s, burst 2 — the same restrictive budget the blocking test uses.
    let (router_a, cleanup, limiter) =
        create_rate_limited_router(Arc::clone(&state), RateLimitConfig::new(1, 2));
    std::thread::spawn(cleanup);
    let router_b = create_router_sharing_rate_limit(Arc::clone(&state), limiter);

    let mut addrs = Vec::new();
    for router in [router_a, router_b] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        addrs.push(listener.local_addr().unwrap());
        tokio::spawn(async move {
            axum::serve(
                listener,
                router.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .expect("Server failed");
        });
    }
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    mediagit_protocol::ensure_crypto_provider();
    let client = Client::new();

    // Spend the whole burst on listener A.
    for _ in 0..6 {
        let _ = client
            .get(format!("http://{}/test-repo/info/refs", addrs[0]))
            .send()
            .await
            .unwrap();
    }

    // Listener B must already be out of budget. If it answers normally, it is
    // running its own limiter (or none) — which is the SV-1 defect.
    let resp = client
        .get(format!("http://{}/test-repo/info/refs", addrs[1]))
        .send()
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "the second listener must share the first's rate-limit budget; \
         got {} — it is enforcing a separate budget or none at all",
        resp.status()
    );
}

/// Two credentials from one IP must not share a budget.
///
/// This is the whole point of `IdentityOrIpKeyExtractor`, and until 2026-08-17
/// it did not hold: `create_rate_limited_router` inlined its own builder keyed
/// by `SmartIpKeyExtractor`, so the extractor was dead code and every client
/// behind one NAT, VPN or CI runner pool drew on a single bucket. Nothing
/// caught it because every other test in this file is single-identity, where
/// per-IP and per-credential behave identically.
///
/// Also pins the claim the wiring rests on: the governor layer runs *before*
/// the auth middleware, so identity has to come from the raw `Authorization`
/// header rather than a request extension. If someone "fixes" that ordering,
/// the second assertion below starts failing.
#[tokio::test]
async fn identity_keyed_buckets_are_independent() {
    // One request, no refill within the test's lifetime.
    let server = TestServer::new_with_rate_limit(RateLimitConfig::new(1, 1)).await;
    mediagit_protocol::ensure_crypto_provider();
    let client = Client::new();

    let get = |token: &'static str| {
        let client = client.clone();
        let url = server.url("/test-repo/info/refs");
        async move {
            client
                .get(url)
                .header("Authorization", format!("Bearer {token}"))
                .send()
                .await
                .unwrap()
                .status()
        }
    };

    assert_ne!(
        get("alice").await,
        StatusCode::TOO_MANY_REQUESTS,
        "alice's first request should be inside her own burst"
    );

    // Same IP, different credential: a separate bucket, so this must pass even
    // though alice has already spent the whole burst.
    assert_ne!(
        get("bob").await,
        StatusCode::TOO_MANY_REQUESTS,
        "bob shares alice's IP but not her credential, so he must have his own budget \
         -- a 429 here means the limiter is keyed per-IP again"
    );

    // ...and alice really is exhausted, so this is not just a limiter that
    // never fires.
    assert_eq!(
        get("alice").await,
        StatusCode::TOO_MANY_REQUESTS,
        "alice's budget was 1 request; the second must be rejected"
    );
}

/// What does the server actually put in `Retry-After`, and is it actionable?
///
/// This exists because a 429 is only useful if the number attached to it tells
/// the client something. If the header says `0`, a client that honours it
/// literally sleeps for zero and hammers straight back -- burning its whole
/// retry budget in microseconds and turning a momentary limit into a hard
/// failure. Captured as a test rather than reasoned about, because the value is
/// produced by `tower_governor`'s `use_headers()` and is not ours to assume.
#[tokio::test]
async fn retry_after_carries_an_actionable_delay() {
    let server = TestServer::new_with_rate_limit(RateLimitConfig::new(1, 2)).await;
    mediagit_protocol::ensure_crypto_provider();
    let client = Client::new();

    let mut limited = None;
    for _ in 0..10 {
        let resp = client
            .get(server.url("/test-repo/info/refs"))
            .send()
            .await
            .unwrap();
        if resp.status() == StatusCode::TOO_MANY_REQUESTS {
            limited = Some(resp);
            break;
        }
    }
    let resp = limited.expect("a 1rps/2burst budget must reject within 10 rapid requests");

    let retry_after = resp
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let x_after = resp
        .headers()
        .get("x-ratelimit-after")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    println!("retry-after={retry_after:?} x-ratelimit-after={x_after:?}");

    let retry_after = retry_after.expect("a 429 must carry Retry-After");
    let secs: u64 = retry_after
        .trim()
        .parse()
        .unwrap_or_else(|_| panic!("Retry-After should be integer seconds, got {retry_after:?}"));

    // Characterisation, not a wish: `tower_governor` emits whole seconds and
    // rounds down, so a bucket refilling in under a second advertises 0. This
    // is pinned rather than asserted against, because the client is where it
    // gets handled -- `rate_limit_backoff` in mediagit-protocol floors the
    // header with its own jittered backoff for exactly this reason.
    //
    // If this ever starts returning >= 1 that is fine; the client takes the
    // larger of the two. What must NOT happen is someone deleting the
    // client-side floor because "the server sends a real number now" -- it
    // does not, for any limit that refills quickly, and a client honouring 0
    // literally spends its whole retry budget in microseconds. That is what
    // broke a 500-commit churn push on POST /packs/complete in 20260817-gagate3.
    assert_eq!(
        secs, 0,
        "expected 0 for a sub-second refill; if the server now sends a real \
            delay, keep the client-side floor in rate_limit_backoff regardless"
    );
}

/// `requests_per_second` must mean requests per second.
///
/// It did not. `tower_governor`'s `per_second(n)` sets the interval at which
/// ONE cell is replenished -- "replenish one element every n seconds" -- so the
/// field named `requests_per_second` was configuring its own reciprocal, and
/// the shipped default of 10 meant one request per TEN SECONDS once the burst
/// drained. That is what produced 429s on ordinary pushes, and raising the
/// number made the sustained rate worse while appearing to help, because a
/// larger burst hides the refill rate until the bucket empties.
///
/// The assertion is deliberately about REFILL, not about the burst: burst size
/// alone passes under either interpretation, which is exactly why this went
/// unnoticed. Spend the burst, wait a beat, and require the bucket to have
/// refilled at the configured rate.
#[tokio::test]
async fn requests_per_second_is_a_rate_not_an_interval() {
    // 100/s => one cell every 10ms. Burst of 1 so it is spent immediately.
    let server = TestServer::new_with_rate_limit(RateLimitConfig::new(100, 1)).await;
    mediagit_protocol::ensure_crypto_provider();
    let client = Client::new();
    let url = server.url("/test-repo/info/refs");

    // Drain the burst.
    for _ in 0..4 {
        let _ = client.get(&url).send().await.unwrap();
    }

    // At 100/s the bucket refills in 10ms; 300ms is ~30 cells of headroom.
    // Under the old interval reading this was one cell per 100 SECONDS, so the
    // request below could not possibly succeed.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let status = client.get(&url).send().await.unwrap().status();
    assert_ne!(
        status,
        StatusCode::TOO_MANY_REQUESTS,
        "after 300ms a 100/s limiter must have refilled. Still 429 means \
         `requests_per_second` is being applied as a replenish INTERVAL again \
         (per_second(100) = one request every 100 seconds)"
    );
}

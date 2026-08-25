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

//! Measures how long it takes to BUILD an HTTP client, serially and under
//! concurrency.
//!
//! WHY THIS EXISTS. Two open hangs both stall in a place where a client is being
//! constructed and no request has been sent yet:
//!
//!   - `auth key revoke` blocked ~60s before its DELETE ever reached the server;
//!     its prelude is resolve_server_target / resolve_credentials /
//!     `reqwest::Client::new()`.
//!   - clone and push block after `info/refs`, before any bulk request - which
//!     is where the DATA-PLANE client gets built, separately from the control
//!     plane one.
//!
//! Building a client is not free on Windows: it installs the crypto provider and
//! materialises a TLS root store, and the hung clone's threads were sitting in
//! `EventPairLow` waits, which is what an LPC/RPC call to a Windows service looks
//! like. The keychain - the other shared prelude component, and the previously
//! favoured suspect - was measured and exonerated at sub-millisecond even with 24
//! concurrent readers, so this is the remaining shared dependency on that path.
//!
//! A negative result is worth as much as a positive: if construction is
//! uniformly fast, both hangs are somewhere else and this rules out a whole
//! layer instead of leaving it as a maybe.
//!
//! Ignored by default - a measurement, not a correctness assertion:
//!
//!   cargo test -p mediagit-protocol --test client_construction_probe -- --ignored --nocapture
//!
//! MG_CLIENT_PROBE_ITERS   builds per thread (default 40)
//! MG_CLIENT_PROBE_THREADS concurrent threads (default 8)

use std::time::{Duration, Instant};

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn summarize(label: &str, mut samples: Vec<Duration>) {
    samples.sort_unstable();
    let n = samples.len();
    assert!(n > 0, "no samples collected");
    let total: Duration = samples.iter().sum();
    let pick = |q: f64| samples[((n as f64 * q) as usize).min(n - 1)];
    println!(
        "{label:<30} n={n:<5} mean={:>9.2?} p50={:>9.2?} p99={:>9.2?} max={:>9.2?}",
        total / n as u32,
        pick(0.50),
        pick(0.99),
        pick(1.0)
    );
}

fn time_control_plane() -> Duration {
    let start = Instant::now();
    mediagit_protocol::ensure_crypto_provider();
    let _c = reqwest::Client::new();
    start.elapsed()
}

fn time_data_plane() -> Duration {
    let start = Instant::now();
    let _c = mediagit_protocol::client::data_plane_client_builder()
        .build()
        .expect("build data-plane client");
    start.elapsed()
}

#[test]
#[ignore = "measurement; run explicitly"]
fn http_client_construction_latency() {
    let iters = env_usize("MG_CLIENT_PROBE_ITERS", 40);
    let threads = env_usize("MG_CLIENT_PROBE_THREADS", 8);

    // Warm the process-wide crypto provider first so the very first sample does
    // not carry one-time init that no later caller pays.
    mediagit_protocol::ensure_crypto_provider();

    summarize(
        "control serial",
        (0..iters).map(|_| time_control_plane()).collect(),
    );
    summarize(
        "data-plane serial",
        (0..iters).map(|_| time_data_plane()).collect(),
    );

    for (label, f) in [
        ("control", time_control_plane as fn() -> Duration),
        ("data-plane", time_data_plane as fn() -> Duration),
    ] {
        let handles: Vec<_> = (0..threads)
            .map(|_| std::thread::spawn(move || (0..iters).map(|_| f()).collect::<Vec<_>>()))
            .collect();
        let mut all = Vec::new();
        for h in handles {
            all.extend(h.join().expect("probe thread panicked"));
        }
        summarize(&format!("{label} concurrent x{threads}"), all);
    }
}

// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Measures how long an OS-keychain read actually takes, including under
//! concurrency.
//!
//! WHY THIS EXISTS. `resolve_credentials_tiered` consults the OS keychain on
//! every push/clone/pull/fetch that has no env or config credential - which in
//! the QA harness is most of them. On Windows that is an RPC to the Credential
//! Manager service, it is called from inside async fns on a tokio worker with no
//! `spawn_blocking`, and it has no timeout. Two open hangs both block in exactly
//! that prelude:
//!
//!   - `auth key revoke` blocked 60s BEFORE sending its DELETE (the server log
//!     has a 65s gap and never received the request)
//!   - clone/push block after `info/refs` with every thread waiting and flat CPU
//!
//! So the question this answers is narrow and falsifiable: **can a keychain read
//! take a long time, and does concurrency make it worse?** If the worst case
//! stays in single-digit milliseconds even under load, the keychain is exonerated
//! and the hangs must be elsewhere - which is worth just as much as a positive.
//!
//! Ignored by default: it is a measurement, not an assertion about correctness,
//! and it touches the real user keychain. Run it deliberately:
//!
//!   cargo test -p mediagit-cli --test keychain_latency_probe -- --ignored --nocapture
//!
//! MG_KEYCHAIN_PROBE_ITERS  iterations per thread (default 200)
//! MG_KEYCHAIN_PROBE_THREADS concurrent threads (default 8)

use std::time::{Duration, Instant};

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// One keychain lookup for an account that does not exist. A miss is the case
/// the hangs actually take - the harness's repos have no stored credential, so
/// resolution falls through every tier and asks the keychain for something that
/// is not there.
fn timed_miss(account: &str) -> Duration {
    let start = Instant::now();
    if let Ok(entry) = keyring::Entry::new("mediagit", account) {
        let _ = entry.get_password();
    }
    start.elapsed()
}

fn summarize(label: &str, mut samples: Vec<Duration>) {
    samples.sort_unstable();
    let n = samples.len();
    assert!(n > 0, "no samples collected");
    let total: Duration = samples.iter().sum();
    let pick = |q: f64| samples[((n as f64 * q) as usize).min(n - 1)];
    println!(
        "{label:<28} n={n:<6} mean={:>8.2?} p50={:>8.2?} p99={:>8.2?} max={:>8.2?}",
        total / n as u32,
        pick(0.50),
        pick(0.99),
        pick(1.0)
    );
}

#[test]
#[ignore = "measurement against the real OS keychain; run explicitly"]
fn keychain_read_latency_serial_and_concurrent() {
    let iters = env_usize("MG_KEYCHAIN_PROBE_ITERS", 200);
    let threads = env_usize("MG_KEYCHAIN_PROBE_THREADS", 8);

    // Serial baseline first: whatever concurrency does, it has to be read
    // against the uncontended cost, not against zero.
    let serial: Vec<Duration> = (0..iters)
        .map(|i| timed_miss(&format!("probe-serial-{i}")))
        .collect();
    summarize("serial", serial);

    // Then the case the campaign actually creates: many processes/threads asking
    // the same service at once.
    let handles: Vec<_> = (0..threads)
        .map(|t| {
            std::thread::spawn(move || {
                (0..iters)
                    .map(|i| timed_miss(&format!("probe-c{t}-{i}")))
                    .collect::<Vec<_>>()
            })
        })
        .collect();

    let mut concurrent = Vec::new();
    for h in handles {
        concurrent.extend(h.join().expect("probe thread panicked"));
    }
    summarize(&format!("concurrent x{threads}"), concurrent);
}

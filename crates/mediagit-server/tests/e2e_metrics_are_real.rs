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

//! DC-4: `/metrics` must report real traffic, not permanent zeros.
//!
//! The registry used to be built inside `main`'s metrics block and moved
//! straight into the metrics server, so no handler could reach it and no
//! `record_*` method was ever called anywhere in the workspace. The endpoint
//! answered, the scrape parsed, every counter read `0` — an operator wiring a
//! dashboard saw flatlines that look exactly like a quiet system.
//!
//! This asserts the property that was missing: after a real request, the
//! numbers move. Asserting the endpoint merely *responds* is what let the bug
//! survive.

use mediagit_metrics::MetricsRegistry;
use std::sync::Arc;
use tokio::net::TcpListener;

/// Sum of every sample of a counter family, across label sets.
fn counter_total(registry: &MetricsRegistry, name: &str) -> f64 {
    registry
        .registry()
        .gather()
        .iter()
        .filter(|f| f.name() == name)
        .flat_map(|f| f.get_metric())
        .map(|m| m.get_counter().value())
        .sum()
}

#[tokio::test]
async fn a_real_request_moves_the_counters() {
    let repos_root = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(repos_root.path().join("metrics-repo")).unwrap();

    let registry = MetricsRegistry::new().expect("registry must build");
    let state = Arc::new(
        mediagit_server::AppState::new(repos_root.path().to_path_buf())
            .with_metrics(registry.clone()),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = mediagit_server::create_router(Arc::clone(&state));
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // The whole point: zero before, non-zero after. Checking only the "after"
    // would pass against a registry that had been pre-seeded by another test.
    let before = counter_total(&registry, "mediagit_operation_total");
    assert_eq!(before, 0.0, "a fresh registry must start at zero");

    // GET /objects/pack without a valid X-Request-ID is refused — which is the
    // point. An *error* must be counted too; instrumenting only the success
    // path leaves the error rate at zero, and error rate is what gets alerted
    // on.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let _ = reqwest::Client::new()
        .get(format!("http://{addr}/metrics-repo/objects/pack"))
        .send()
        .await;

    let after = counter_total(&registry, "mediagit_operation_total");
    assert!(
        after > before,
        "a served request must move mediagit_operation_total; \
         got {before} -> {after}. If this is zero, the handler is recording \
         into a different registry than the one /metrics serves (DC-4)."
    );
}

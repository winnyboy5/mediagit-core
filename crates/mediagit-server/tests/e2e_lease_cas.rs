// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! BUG-032: the server enforces `force_with_lease` as an atomic
//! compare-and-swap on `old_oid` (including "expect absent").

use std::sync::Arc;
use tempfile::TempDir;
use tokio::net::TcpListener;

use mediagit_protocol::{ProtocolClient, RefUpdate, RefUpdateRequest};
use mediagit_versioning::{Oid, Ref, RefDatabase};

async fn start(repos: std::path::PathBuf) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = mediagit_server::create_router(Arc::new(mediagit_server::AppState::new(repos)));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    base
}

fn lease(old: Option<Oid>, new: Oid) -> RefUpdateRequest {
    RefUpdateRequest {
        updates: vec![RefUpdate {
            name: "refs/heads/main".to_string(),
            old_oid: old.map(|o| o.to_hex()),
            new_oid: new.to_hex(),
            delete: false,
        }],
        force: false,
        force_with_lease: true,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_leases_with_same_expected_value_exactly_one_wins() {
    let repos = TempDir::new().unwrap();
    let mg = repos.path().join("r").join(".mediagit");
    tokio::fs::create_dir_all(mg.join("refs/heads"))
        .await
        .unwrap();
    let base = start(repos.path().to_path_buf()).await;

    for round in 0..5u8 {
        let start_oid = Oid::hash(&[round, 0]);
        RefDatabase::new(&mg)
            .write(&Ref::new_direct("refs/heads/main".into(), start_oid))
            .await
            .unwrap();
        let mut tasks = Vec::new();
        for i in 1..=8u8 {
            let client = ProtocolClient::new(format!("{base}/r"));
            let req = lease(Some(start_oid), Oid::hash(&[round, i]));
            tasks.push(tokio::spawn(async move { client.update_refs(req).await }));
        }
        let mut wins = 0;
        for t in tasks {
            if t.await.unwrap().unwrap().success {
                wins += 1;
            }
        }
        assert_eq!(wins, 1, "round {round}: exactly one lease may win");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lease_expecting_absent_is_refused_when_branch_exists_and_vice_versa() {
    let repos = TempDir::new().unwrap();
    let mg = repos.path().join("r").join(".mediagit");
    tokio::fs::create_dir_all(mg.join("refs/heads"))
        .await
        .unwrap();
    let base = start(repos.path().to_path_buf()).await;
    let client = ProtocolClient::new(format!("{base}/r"));
    let (a, b) = (Oid::hash(b"a"), Oid::hash(b"b"));

    // Absent branch, lease expects a: refused (someone deleted it).
    assert!(!client.update_refs(lease(Some(a), b)).await.unwrap().success);
    // Absent branch, lease expects absent: created.
    assert!(client.update_refs(lease(None, a)).await.unwrap().success);
    // Branch exists, lease expects absent: refused.
    assert!(!client.update_refs(lease(None, b)).await.unwrap().success);
    let tip = RefDatabase::new(&mg).read("refs/heads/main").await.unwrap();
    assert_eq!(tip.oid, Some(a));
}

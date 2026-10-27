// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! #95: a pull interrupted after the pack (commit + tree) was stored but
//! before a large file's manifest was written must not wedge the repo.
//!
//! The kill-safety harness hit this state: on retry, pull saw the commit was
//! local, wrote the tracking ref early, advertised that commit as a `have`,
//! the server pruned its closure and sent no chunked objects, and checkout
//! failed with "object not found" on every later pull.
//!
//! This test recreates that state deterministically: a successful pull, then
//! the large file's manifest is deleted and the local branch, tracking ref and
//! working tree are put back to the old commit.

use assert_cmd::Command;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::net::TcpListener;

#[allow(deprecated)]
fn mediagit(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("mediagit").unwrap();
    c.env("MEDIAGIT_AUTHOR_NAME", "Test User")
        .env("MEDIAGIT_AUTHOR_EMAIL", "test@example.com")
        .current_dir(dir);
    c
}

async fn start_test_server(repos_dir: std::path::PathBuf) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = Arc::new(mediagit_server::AppState::new(repos_dir));
    let app = mediagit_server::create_router(state);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    format!("http://{}", addr)
}

/// Incompressible, non-repeating bytes so the file is stored chunked.
fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

fn read_ref(repo: &Path, name: &str) -> String {
    fs::read_to_string(repo.join(".mediagit").join(name))
        .unwrap()
        .trim()
        .to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pull_recovers_after_interrupt_between_pack_and_manifest() {
    let repos_root = TempDir::new().unwrap();
    let base_url = start_test_server(repos_root.path().to_path_buf()).await;
    fs::create_dir_all(repos_root.path().join("p95")).unwrap();
    let url = format!("{base_url}/p95");

    // Writer: small first commit, pushed.
    let writer = TempDir::new().unwrap();
    let w = writer.path();
    mediagit(w).args(["init", "-q"]).assert().success();
    fs::write(w.join("f.txt"), "first\n").unwrap();
    mediagit(w).args(["add", "f.txt"]).assert().success();
    mediagit(w)
        .args(["commit", "-m", "first"])
        .assert()
        .success();
    mediagit(w)
        .args(["remote", "add", "origin", &url])
        .assert()
        .success();
    mediagit(w)
        .args(["push", "-u", "origin"])
        .assert()
        .success();

    // Reader clones at commit A.
    let parent = TempDir::new().unwrap();
    let r = parent.path().join("reader");
    mediagit(parent.path())
        .arg("clone")
        .arg(&url)
        .arg(&r)
        .assert()
        .success();
    let commit_a = read_ref(&r, "refs/heads/main");

    // Writer adds a large (chunked) file as commit B and pushes.
    let big = noise(12 * 1024 * 1024, 95);
    fs::write(w.join("big.bin"), &big).unwrap();
    mediagit(w).args(["add", "big.bin"]).assert().success();
    mediagit(w).args(["commit", "-m", "big"]).assert().success();
    mediagit(w).args(["push", "origin"]).assert().success();

    // Reader pulls B fully once, so B's commit, tree and chunks are local.
    mediagit(&r).args(["pull", "origin"]).assert().success();
    assert_eq!(fs::read(r.join("big.bin")).unwrap(), big);

    // Recreate the interrupted state: manifest gone, refs and tree back at A.
    let storage = mediagit_cli::repo::create_storage_backend(&r)
        .await
        .unwrap();
    let manifests = storage.list_objects("manifests/").await.unwrap();
    assert!(!manifests.is_empty(), "big.bin should be stored chunked");
    for key in &manifests {
        storage.delete(key).await.unwrap();
    }
    drop(storage);
    for name in ["refs/heads/main", "refs/remotes/origin/main"] {
        fs::write(r.join(".mediagit").join(name), format!("{commit_a}\n")).unwrap();
    }
    fs::remove_file(r.join("big.bin")).unwrap();

    // Retry must complete and restore the file.
    mediagit(&r).args(["pull", "origin"]).assert().success();
    assert_eq!(fs::read(r.join("big.bin")).unwrap(), big);
    mediagit(&r).arg("fsck").assert().success();

    // Already-up-to-date pull still refreshes a stale tracking ref: STEP 1 no
    // longer writes the pulled branch's tracking ref, so the early return must.
    let commit_b = read_ref(&r, "refs/heads/main");
    fs::write(
        r.join(".mediagit").join("refs/remotes/origin/main"),
        format!(
            "{commit_a}
"
        ),
    )
    .unwrap();
    let out = mediagit(&r)
        .args(["pull", "origin", "-v"])
        .assert()
        .success();
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&out.get_output().stdout),
        String::from_utf8_lossy(&out.get_output().stderr)
    );
    assert_eq!(
        read_ref(&r, "refs/remotes/origin/main"),
        commit_b,
        "a={commit_a}
{log}"
    );
}

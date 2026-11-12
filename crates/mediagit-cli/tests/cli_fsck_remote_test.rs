// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! `fsck --repair` restores a damaged object from the remote when neither the
//! working file nor a pack has a good copy, and says why when it cannot.

use assert_cmd::Command;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::TempDir;
use tokio::net::TcpListener;

#[allow(deprecated)]
fn mediagit() -> Command {
    let mut c = Command::cargo_bin("mediagit").unwrap();
    c.env("MEDIAGIT_AUTHOR_NAME", "Test User")
        .env("MEDIAGIT_AUTHOR_EMAIL", "test@example.com");
    c
}

async fn start_test_server(repos_dir: PathBuf) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let state = Arc::new(mediagit_server::AppState::new(repos_dir));
    let app = mediagit_server::create_router(state);
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    (base_url, handle)
}

fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(files_under(&p));
        } else {
            out.push(p);
        }
    }
    out
}

fn object_files(repo: &Path) -> BTreeSet<PathBuf> {
    files_under(&repo.join(".mediagit/objects"))
        .into_iter()
        .collect()
}

fn run(repo: &Path, args: &[&str]) {
    mediagit().args(args).current_dir(repo).assert().success();
}

fn random_bytes(n: usize) -> Vec<u8> {
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

fn fsck(repo: &Path, args: &[&str]) -> (bool, String) {
    let out = mediagit()
        .arg("fsck")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

/// Commit and push `name`, damage one stored object of it, delete the working
/// file. Returns the original bytes. `pick` chooses the object file to damage.
fn pushed_and_damaged(
    repo: &Path,
    url: &str,
    name: &str,
    size: usize,
    pick: impl Fn(&[PathBuf]) -> PathBuf,
) -> Vec<u8> {
    run(repo, &["init", "-q"]);
    let data = random_bytes(size);
    std::fs::write(repo.join(name), &data).unwrap();
    let before = object_files(repo);
    run(repo, &["add", name]);
    let added: Vec<PathBuf> = object_files(repo).difference(&before).cloned().collect();
    run(repo, &["commit", "-m", "c"]);
    run(repo, &["remote", "add", "origin", url]);
    run(repo, &["push", "-u", "origin"]);

    let victim = pick(&added);
    let mut bytes = std::fs::read(&victim).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0xFF;
    std::fs::write(&victim, bytes).unwrap();
    std::fs::remove_file(repo.join(name)).unwrap();
    data
}

async fn repair_from_remote(name: &str, size: usize, pick: fn(&[PathBuf]) -> PathBuf) {
    let repos = TempDir::new().unwrap();
    let (base, _server) = start_test_server(repos.path().to_path_buf()).await;
    std::fs::create_dir_all(repos.path().join("r")).unwrap();
    let work = TempDir::new().unwrap();
    let repo = work.path();
    let data = pushed_and_damaged(repo, &format!("{base}/r"), name, size, pick);

    let (ok, text) = fsck(repo, &["--repair"]);
    assert!(ok, "the remote has a good copy: {text}");
    assert!(
        files_under(&repo.join(".mediagit/quarantine")).is_empty(),
        "restored, so the quarantined copy goes: {text}"
    );
    let (ok, text) = fsck(repo, &["--full"]);
    assert!(ok, "{text}");
    run(repo, &["reset", "--hard", "HEAD"]);
    assert_eq!(std::fs::read(repo.join(name)).unwrap(), data);
}

fn only_object(added: &[PathBuf]) -> PathBuf {
    assert_eq!(added.len(), 1, "fixture: add should write exactly the blob");
    added[0].clone()
}

fn a_chunk(added: &[PathBuf]) -> PathBuf {
    added
        .iter()
        .find(|p| p.components().any(|c| c.as_os_str() == "chunks"))
        .expect("fixture: file was not chunked")
        .clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fsck_repair_restores_a_damaged_blob_from_the_remote() {
    repair_from_remote("photo.bin", 64 * 1024, only_object).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fsck_repair_restores_a_damaged_chunk_from_the_remote() {
    repair_from_remote("big.bin", 6 * 1024 * 1024, a_chunk).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fsck_repair_says_so_when_the_server_cannot_be_reached() {
    let repos = TempDir::new().unwrap();
    let (base, server) = start_test_server(repos.path().to_path_buf()).await;
    std::fs::create_dir_all(repos.path().join("r")).unwrap();
    let work = TempDir::new().unwrap();
    let repo = work.path();
    pushed_and_damaged(
        repo,
        &format!("{base}/r"),
        "photo.bin",
        64 * 1024,
        only_object,
    );
    server.abort();
    let _ = server.await;

    let (ok, text) = fsck(repo, &["--repair"]);
    assert!(!ok, "nothing could restore it: {text}");
    assert!(text.contains("could not be reached"), "{text}");
    assert!(text.contains("photo.bin"), "{text}");
    assert_eq!(files_under(&repo.join(".mediagit/quarantine")).len(), 1);
    assert!(!fsck(repo, &[]).0, "still red");
}

#[test]
fn fsck_repair_without_a_remote_says_none_is_configured() {
    let work = TempDir::new().unwrap();
    let repo = work.path();
    run(repo, &["init", "-q"]);
    let data = random_bytes(64 * 1024);
    std::fs::write(repo.join("photo.bin"), &data).unwrap();
    let before = object_files(repo);
    run(repo, &["add", "photo.bin"]);
    let added: Vec<PathBuf> = object_files(repo).difference(&before).cloned().collect();
    run(repo, &["commit", "-m", "c"]);
    let mut bytes = std::fs::read(&added[0]).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0xFF;
    std::fs::write(&added[0], bytes).unwrap();
    std::fs::remove_file(repo.join("photo.bin")).unwrap();

    let (ok, text) = fsck(repo, &["--repair"]);
    assert!(!ok, "{text}");
    assert!(text.contains("No remote is configured"), "{text}");
}

// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Engine against the real `mediagit` binary in the same target directory.
//!
//! Needs `cargo build -p mediagit-cli` first (cargo does not build another
//! package's binary for this crate's tests). A missing binary fails loudly
//! rather than skipping, so CI cannot pass these vacuously.

#![allow(clippy::unwrap_used)]

use std::path::Path;
use std::process::Command;

use mediagit_desktop::engine::{Engine, EngineError};

fn engine() -> Engine {
    Engine::locate().unwrap_or_else(|e| panic!("{e} — run `cargo build -p mediagit-cli` first"))
}

fn init_repo(engine: &Engine, dir: &Path) {
    let out = Command::new(engine.bin())
        .arg("init")
        .arg(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[tokio::test]
async fn version_reports_matching_contract() {
    let v = engine().run(None, &["version"], |_| {}).await.unwrap();
    assert_eq!(v["contract"], 1);
    assert!(v["version"].as_str().is_some_and(|s| !s.is_empty()));
}

#[tokio::test]
async fn status_on_fresh_repo_lists_untracked_file() {
    let engine = engine();
    let tmp = tempfile::tempdir().unwrap();
    init_repo(&engine, tmp.path());
    std::fs::write(tmp.path().join("hero.psd"), b"not really a psd").unwrap();

    let s = engine
        .run(Some(tmp.path()), &["status"], |_| {})
        .await
        .unwrap();
    assert_eq!(s["branch"]["has_commits"], false);
    let untracked: Vec<&str> = s["untracked"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(untracked, ["hero.psd"]);
}

#[tokio::test]
async fn status_outside_repo_is_a_cli_error() {
    let tmp = tempfile::tempdir().unwrap();
    let err = engine()
        .run(Some(tmp.path()), &["status"], |_| {})
        .await
        .unwrap_err();
    assert!(
        matches!(err, EngineError::Cli { ref code, .. } if code == "error"),
        "{err:?}"
    );
}

#[tokio::test]
async fn unconverted_command_is_refused_not_run() {
    let engine = engine();
    let tmp = tempfile::tempdir().unwrap();
    init_repo(&engine, tmp.path());
    let err = engine
        .run(Some(tmp.path()), &["gc"], |_| {})
        .await
        .unwrap_err();
    assert!(
        matches!(err, EngineError::Cli { ref code, .. } if code == "json_unsupported"),
        "{err:?}"
    );
}

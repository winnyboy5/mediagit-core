// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! `mediagit-server init --non-interactive --admin-password-stdin` creates the
//! first admin from a password on stdin and never echoes it.

use std::io::Write;
use std::process::{Command, Stdio};

use mediagit_security::auth::CredentialsStore;

const PASSWORD: &str = "stdin-only horse battery 91";

fn init(dir: &std::path::Path, stdin: &[u8]) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_mediagit-server"))
        .args([
            "init",
            "--non-interactive",
            "--enable-auth",
            "--admin-password-stdin",
        ])
        .args([
            "--admin-username",
            "root",
            "--admin-email",
            "root@example.com",
        ])
        .arg("--config")
        .arg(dir.join("s.toml"))
        .arg("--data-dir")
        .arg(dir.join("repos"))
        .env_remove("MEDIAGIT_ADMIN_PASSWORD")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    child.wait_with_output().unwrap()
}

#[tokio::test]
async fn stdin_password_creates_the_admin_and_is_never_printed() {
    let tmp = tempfile::tempdir().unwrap();
    let out = init(tmp.path(), format!("{PASSWORD}\n").as_bytes());
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let all = [out.stdout, out.stderr].concat();
    assert!(
        !String::from_utf8_lossy(&all).contains(PASSWORD),
        "password leaked to output"
    );
    let cfg = std::fs::read_to_string(tmp.path().join("s.toml")).unwrap();
    assert!(!cfg.contains(PASSWORD), "password written to the config");
    // The auth store lives wherever the config resolves it; find the user by loading it.
    let config =
        mediagit_server::ServerConfig::load(tmp.path().join("s.toml").to_str().unwrap()).unwrap();
    let store = CredentialsStore::load_or_new(&config.resolved_auth_store_dir()).unwrap();
    let users = store.list_users().await;
    assert!(users.iter().any(|u| u.username == "root"), "{users:?}");
}

#[test]
fn empty_stdin_creates_no_admin_and_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let out = init(tmp.path(), b"\n");
    assert!(!out.status.success());
    assert!(
        !tmp.path().join("s.toml").exists(),
        "no config on a failed init"
    );
}

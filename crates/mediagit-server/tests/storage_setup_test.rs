// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Default storage for hosted repos: `init --storage ...` writes it, secrets
//! come from stdin and are never printed, and the probe tells good from bad.

use std::io::Write;
use std::process::{Command, Stdio};

use mediagit_config::{FileSystemStorage, StorageConfig};

const ACCESS: &str = "AKIAPROBEACCESS000";
const SECRET: &str = "probe/secret+key=0123456789";

fn run(args: &[&str], stdin: &str) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_mediagit-server"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn init_writes_s3_default_storage_with_stdin_secrets() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("s.toml");
    let out = run(
        &[
            "init",
            "--non-interactive",
            "--config",
            cfg.to_str().unwrap(),
            "--data-dir",
            tmp.path().join("repos").to_str().unwrap(),
            "--storage",
            "s3",
            "--storage-bucket",
            "media",
            "--storage-endpoint",
            "http://127.0.0.1:9000",
            "--storage-prefix",
            "studio",
            "--storage-secret-stdin",
        ],
        &format!("access-key-id={ACCESS}\nsecret-access-key={SECRET}\n"),
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let all = String::from_utf8_lossy(&[out.stdout, out.stderr].concat()).into_owned();
    assert!(
        !all.contains(SECRET) && !all.contains(ACCESS),
        "secret printed: {all}"
    );

    let loaded = mediagit_server::ServerConfig::load(cfg.to_str().unwrap()).unwrap();
    let Some(StorageConfig::S3(s3)) = loaded.default_storage.clone() else {
        panic!("no s3 default storage: {:?}", loaded.default_storage);
    };
    assert_eq!(s3.bucket, "media");
    assert_eq!(s3.endpoint.as_deref(), Some("http://127.0.0.1:9000"));
    assert_eq!(s3.prefix, "studio");
    assert_eq!(s3.secret_access_key.as_deref(), Some(SECRET));
    // The log form hides the keys.
    let logged = format!("{:?}", loaded.redacted());
    assert!(
        !logged.contains(SECRET) && !logged.contains(ACCESS),
        "{logged}"
    );
}

/// The startup log line dumps the whole config: no secret may reach it.
#[test]
fn startup_log_hides_jwt_and_storage_secrets() {
    const JWT: &str = "jwt-secret-for-log-test-0123456789abcdef";
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("s.toml");
    let init = run(
        &[
            "init",
            "--non-interactive",
            "--config",
            cfg.to_str().unwrap(),
            "--data-dir",
            tmp.path().join("repos").to_str().unwrap(),
            "--storage",
            "s3",
            "--storage-bucket",
            "media",
            "--storage-endpoint",
            "http://127.0.0.1:9",
            "--storage-secret-stdin",
        ],
        &format!("access-key-id={ACCESS}\nsecret-access-key={SECRET}\n"),
    );
    assert!(init.status.success());
    let body = std::fs::read_to_string(&cfg).unwrap();
    std::fs::write(&cfg, format!("jwt_secret = \"{JWT}\"\n{body}")).unwrap();

    let port = {
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        probe.local_addr().unwrap().port()
    };
    let mut child = Command::new(env!("CARGO_BIN_EXE_mediagit-server"))
        .args(["--config", cfg.to_str().unwrap(), "--host", "127.0.0.1"])
        .args(["--port", &port.to_string()])
        .current_dir(tmp.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Reachable, or already exited (storage probe may refuse): either way the line is logged first.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::time::Instant::now() < deadline
        && child.try_wait().unwrap().is_none()
        && std::net::TcpStream::connect(("127.0.0.1", port)).is_err()
    {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let _ = child.kill();
    let out = child.wait_with_output().unwrap();
    let all = String::from_utf8_lossy(&[out.stdout, out.stderr].concat()).into_owned();
    assert!(
        all.contains("Server configuration"),
        "no config line: {all}"
    );
    for s in [JWT, SECRET, ACCESS] {
        assert!(!all.contains(s), "{s} leaked into the startup log: {all}");
    }
}

#[test]
fn init_rejects_s3_without_bucket_and_writes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("s.toml");
    let out = run(
        &[
            "init",
            "--non-interactive",
            "--config",
            cfg.to_str().unwrap(),
            "--storage",
            "s3",
        ],
        "",
    );
    assert!(!out.status.success());
    assert!(!cfg.exists());
}

#[tokio::test]
async fn probe_passes_on_a_writable_backend_and_fails_on_a_broken_one() {
    let tmp = tempfile::tempdir().unwrap();
    let fs = |p: &std::path::Path| {
        StorageConfig::FileSystem(FileSystemStorage {
            base_path: p.to_string_lossy().into_owned(),
            ..Default::default()
        })
    };
    mediagit_server::handlers::probe_storage(&fs(&tmp.path().join("ok")))
        .await
        .unwrap();
    // A path beneath a regular file can never be created.
    let file = tmp.path().join("file");
    std::fs::write(&file, b"x").unwrap();
    assert!(
        mediagit_server::handlers::probe_storage(&fs(&file.join("sub")))
            .await
            .is_err()
    );
}

#[test]
fn storage_test_reports_failure_as_json_and_exit_code() {
    // Unreachable endpoint: the probe must fail fast with ok:false, not hang or succeed.
    let out = run(
        &[
            "storage-test",
            "--storage",
            "s3",
            "--storage-bucket",
            "nope",
            "--storage-endpoint",
            "http://127.0.0.1:1",
            "--storage-secret-stdin",
        ],
        &format!("access-key-id={ACCESS}\nsecret-access-key={SECRET}\n"),
    );
    assert!(!out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(v["ok"], false);
    let all = format!("{stdout}{}", String::from_utf8_lossy(&out.stderr));
    assert!(!all.contains(SECRET), "secret printed: {all}");
}

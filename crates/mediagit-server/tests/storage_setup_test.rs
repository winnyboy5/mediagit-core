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

/// BUG-043: a missing bucket is refused in plain words and never created.
/// Needs a local Silo (MG_SILO_ENDPOINT, default 127.0.0.1:9000); skips when down.
#[test]
fn storage_test_refuses_missing_bucket_without_creating_it() {
    let endpoint =
        std::env::var("MG_SILO_ENDPOINT").unwrap_or_else(|_| "http://127.0.0.1:9000".into());
    let addr = endpoint.trim_start_matches("http://");
    if std::net::TcpStream::connect(addr).is_err() {
        eprintln!("skip: no Silo at {endpoint}");
        return;
    }
    let bucket = format!(
        "mg-bug043-srv-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let out = run(
        &[
            "storage-test",
            "--storage",
            "s3",
            "--storage-bucket",
            &bucket,
            "--storage-endpoint",
            &endpoint,
            "--storage-secret-stdin",
        ],
        "access-key-id=minioadmin\nsecret-access-key=minioadmin\n",
    );
    assert!(!out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(v["ok"], false);
    assert_eq!(
        v["error"], "That bucket does not exist. Create it first, then test again.",
        "got: {stdout}"
    );
    // Creating the bucket would make a probe against it succeed on its own;
    // a second probe still failing the same way proves it was not created.
    let again = run(
        &[
            "storage-test",
            "--storage",
            "s3",
            "--storage-bucket",
            &bucket,
            "--storage-endpoint",
            &endpoint,
            "--storage-secret-stdin",
        ],
        "access-key-id=minioadmin\nsecret-access-key=minioadmin\n",
    );
    assert!(
        !again.status.success(),
        "bucket was created by the first probe"
    );
}

fn s3_to(endpoint: &str) -> StorageConfig {
    StorageConfig::S3(mediagit_config::S3Storage {
        bucket: "media".into(),
        region: "us-east-1".into(),
        endpoint: Some(endpoint.into()),
        access_key_id: Some(ACCESS.into()),
        secret_access_key: Some(SECRET.into()),
        prefix: String::new(),
    })
}

/// BUG-044: each way a connection can fail has its own sentence, quickly.
#[tokio::test]
async fn probe_names_the_cause_of_a_dead_endpoint() {
    // Nothing listens on port 1.
    let started = std::time::Instant::now();
    let e = mediagit_server::handlers::probe_storage(&s3_to("http://127.0.0.1:1"))
        .await
        .unwrap_err()
        .to_string();
    assert!(e.contains("Nothing is listening at 127.0.0.1:1"), "{e}");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    // A name that cannot exist (RFC 6761 .invalid).
    let e = mediagit_server::handlers::probe_storage(&s3_to("http://no-such-host.invalid:9000"))
        .await
        .unwrap_err()
        .to_string();
    assert!(e.contains("Cannot find no-such-host.invalid:9000"), "{e}");
    assert!(!e.contains(SECRET) && !e.contains(ACCESS), "{e}");
}

/// BUG-044: a server that answers 403 means the keys were refused.
#[tokio::test]
async fn probe_says_when_the_keys_are_refused() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf).await;
                let _ = s
                    .write_all(
                        b"HTTP/1.1 403 Forbidden\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                    )
                    .await;
            });
        }
    });
    let e = mediagit_server::handlers::probe_storage(&s3_to(&format!("http://{addr}")))
        .await
        .unwrap_err()
        .to_string();
    assert!(e.contains("refused the access key or secret key"), "{e}");
    assert!(!e.contains(SECRET) && !e.contains(ACCESS), "{e}");
}

/// IMP-101: a successful test leaves nothing behind, in particular no LAYOUT.
#[tokio::test]
async fn probe_leaves_no_layout_object_behind() {
    let tmp = tempfile::tempdir().unwrap();
    let store = tmp.path().join("bucket");
    mediagit_server::handlers::probe_storage(&StorageConfig::FileSystem(FileSystemStorage {
        base_path: store.to_string_lossy().into_owned(),
        ..Default::default()
    }))
    .await
    .unwrap();
    fn files(dir: &std::path::Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                files(&p, out);
            } else {
                out.push(p.to_string_lossy().into_owned());
            }
        }
    }
    let mut all = Vec::new();
    files(&store, &mut all);
    assert!(all.is_empty(), "left behind: {all:?}");
}

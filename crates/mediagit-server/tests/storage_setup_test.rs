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
    let msg = v["error"].as_str().unwrap_or_default();
    assert!(
        msg.starts_with("The bucket '")
            && msg.ends_with("does not exist. Create it first, then test again."),
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
    assert!(
        e.contains("refused access to bucket 'media'") && e.contains("another account"),
        "{e}"
    );
    assert!(!e.contains(SECRET) && !e.contains(ACCESS), "{e}");
}

/// A one-shot HTTP stub that answers every request with `reply`.
async fn serve(reply: &'static [u8]) -> String {
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
                let _ = s.write_all(reply).await;
            });
        }
    });
    format!("http://{addr}")
}

/// A bucket in another region: the region header says where, and is advice.
#[tokio::test]
async fn probe_names_the_region_of_a_bucket_in_another_region() {
    let ep = serve(b"HTTP/1.1 301 Moved Permanently\r\nx-amz-bucket-region: eu-west-1\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").await;
    let e = mediagit_server::handlers::probe_storage(&s3_to(&ep))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("'media' is in region eu-west-1; set the region to eu-west-1"),
        "{e}"
    );
    assert!(!e.contains(SECRET) && !e.contains(ACCESS), "{e}");
}

/// A wrong Azure account key is worded for Azure, not as an S3 access key.
#[tokio::test]
async fn probe_words_a_refused_azure_key_for_azure() {
    let ep =
        serve(b"HTTP/1.1 403 Forbidden\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").await;
    let cfg = StorageConfig::Azure(mediagit_config::AzureStorage {
        container: "qa-container".into(),
        prefix: String::new(),
        auth: Some(mediagit_config::AzureAuth::ConnectionString {
            value: format!(
                "AccountName=fakeacct;AccountKey=ZmFrZS1rZXk=;BlobEndpoint={ep}/fakeacct"
            ),
        }),
        legacy: Default::default(),
    });
    let e = mediagit_server::handlers::probe_storage(&cfg)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        e.starts_with("Azure refused the account key or connection string"),
        "{e}"
    );
    assert!(
        !e.contains("ZmFrZS1rZXk=") && !e.contains("secret key"),
        "{e}"
    );
}

/// A 400 with no region header is a bad bucket name.
#[tokio::test]
async fn probe_says_a_bad_bucket_name_is_not_valid() {
    let ep =
        serve(b"HTTP/1.1 400 Bad Request\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").await;
    let e = mediagit_server::handlers::probe_storage(&s3_to(&ep))
        .await
        .unwrap_err()
        .to_string();
    assert!(e.contains("bucket name 'media' is not valid"), "{e}");
}

/// A missing service-account key file names the file, with no HTTP status.
#[tokio::test]
async fn probe_names_a_missing_gcs_key_file() {
    let cfg = StorageConfig::GCS(mediagit_config::GCSStorage {
        bucket: "bkt".into(),
        project_id: "proj".into(),
        credentials_path: Some("/no-such-dir/no-key.json".into()),
        prefix: String::new(),
    });
    let e = mediagit_server::handlers::probe_storage(&cfg)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("no-key.json") && e.contains("cannot be found"),
        "{e}"
    );
    assert!(!e.contains("500") && !e.contains("see the log"), "{e}");
}

/// A connection string without account parts is "not valid", not a network error.
#[tokio::test]
async fn probe_says_a_garbage_azure_connection_string_is_not_valid() {
    let cfg = StorageConfig::Azure(mediagit_config::AzureStorage {
        container: "qa-container".into(),
        prefix: String::new(),
        auth: Some(mediagit_config::AzureAuth::ConnectionString {
            value: "not-a-connection-string".into(),
        }),
        legacy: Default::default(),
    });
    let e = mediagit_server::handlers::probe_storage(&cfg)
        .await
        .unwrap_err()
        .to_string();
    assert!(e.contains("connection string is not valid"), "{e}");
    assert!(
        !e.contains("Cannot reach") && !e.contains("not-a-connection-string"),
        "{e}"
    );
}

/// Valid JSON that is not a service-account key is named as such.
#[tokio::test]
async fn probe_says_a_json_file_that_is_not_a_service_account_key() {
    let tmp = tempfile::tempdir().unwrap();
    let key = tmp.path().join("not-a-key.json");
    std::fs::write(&key, "{\"hello\": 1}").unwrap();
    let cfg = StorageConfig::GCS(mediagit_config::GCSStorage {
        bucket: "bkt".into(),
        project_id: "proj".into(),
        credentials_path: Some(key.display().to_string()),
        prefix: String::new(),
    });
    let e = mediagit_server::handlers::probe_storage(&cfg)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("not-a-key.json") && e.contains("not a service-account key"),
        "{e}"
    );
}

/// The startup sentence names the backend and carries the cause as given.
#[test]
fn start_failure_sentence_names_backend_and_cause() {
    let azure = StorageConfig::Azure(mediagit_config::AzureStorage {
        container: "qa-container".into(),
        prefix: String::new(),
        auth: None,
        legacy: Default::default(),
    });
    let s = mediagit_server::handlers::start_failure_sentence(
        &azure,
        Some("The connection string is not valid."),
    );
    assert_eq!(
        s,
        "The server could not start (Azure): The connection string is not valid."
    );
}

/// The real binary refuses a bad cloud config with the one plain sentence on stderr,
/// and says nothing of the kind for a good one. The desktop app looks for that text.
#[test]
fn server_binary_prints_the_plain_refusal_only_when_storage_is_bad() {
    const PREFIX: &str = "The server could not start";
    // The desktop app matches the same text (it cannot link the server crate).
    let desktop = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../mediagit-desktop/src/server_manager.rs"),
    )
    .unwrap();
    assert!(desktop.contains(&format!("\"{PREFIX}\"")));
    assert_eq!(PREFIX, mediagit_server::handlers::START_FAILED_PREFIX);

    let run_with = |storage: StorageConfig| -> String {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join("s.toml");
        let repos = tmp.path().join("repos");
        let init = run(
            &[
                "init",
                "--non-interactive",
                "--config",
                cfg.to_str().unwrap(),
                "--data-dir",
                repos.to_str().unwrap(),
            ],
            "",
        );
        assert!(
            init.status.success(),
            "{}",
            String::from_utf8_lossy(&init.stderr)
        );
        let repo = repos.join("r1");
        std::fs::create_dir_all(&repo).unwrap();
        mediagit_config::Config {
            storage,
            ..Default::default()
        }
        .save(&repo)
        .unwrap();
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let mut child = Command::new(env!("CARGO_BIN_EXE_mediagit-server"))
            .args([
                "--config",
                cfg.to_str().unwrap(),
                "--host",
                "127.0.0.1",
                "--port",
                &port.to_string(),
            ])
            .current_dir(tmp.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while std::time::Instant::now() < deadline
            && child.try_wait().unwrap().is_none()
            && std::net::TcpStream::connect(("127.0.0.1", port)).is_err()
        {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let exited = child.try_wait().unwrap();
        let _ = child.kill();
        let out = child.wait_with_output().unwrap();
        let all = String::from_utf8_lossy(&[out.stdout, out.stderr].concat()).into_owned();
        if all.contains(PREFIX) {
            assert!(
                exited.is_some_and(|c| !c.success()),
                "refusal text but still running: {all}"
            );
        }
        all
    };

    let bad = run_with(StorageConfig::Azure(mediagit_config::AzureStorage {
        container: "qa-container".into(),
        prefix: String::new(),
        auth: Some(mediagit_config::AzureAuth::ConnectionString {
            value: "not-a-connection-string".into(),
        }),
        legacy: Default::default(),
    }));
    assert!(
        bad.contains(
            "Error: The server could not start (Azure): The connection string is not valid."
        ),
        "{bad}"
    );
    assert!(!bad.contains("not-a-connection-string"), "{bad}");

    let good_dir = tempfile::tempdir().unwrap();
    let good = run_with(StorageConfig::FileSystem(FileSystemStorage {
        base_path: good_dir.path().join("ok").to_string_lossy().into_owned(),
        ..Default::default()
    }));
    assert!(!good.contains(PREFIX), "{good}");
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

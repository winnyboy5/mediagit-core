// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Lifecycle of the local server against `examples/fake_server.rs` (built by `cargo test`).

#![allow(clippy::unwrap_used)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use mediagit_desktop::engine::{EngineError, Secret};
use mediagit_desktop::server_manager::ServerManager;

const PW: &str = "fake-admin horse battery 42";

fn fake_bin() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let name = format!("fake_server{}", std::env::consts::EXE_SUFFIX);
    exe.ancestors()
        .skip(1)
        .take(2)
        .map(|d| d.join("examples").join(&name))
        .find(|p| p.is_file())
        .unwrap_or_else(|| panic!("{name} not built: run `cargo test -p mediagit-desktop`"))
}

fn manager(dir: &Path) -> ServerManager {
    ServerManager::with_bin(dir.to_path_buf(), fake_bin())
}

async fn set_up(m: &ServerManager) {
    m.setup("root", "root@example.com", &Secret::new(PW.into()).unwrap())
        .await
        .unwrap();
}

fn code(e: &EngineError) -> &str {
    e.code()
}

fn get_healthz(port: u16) -> bool {
    let Ok(mut s) = TcpStream::connect(("127.0.0.1", port)) else {
        return false;
    };
    s.set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    s.write_all(b"GET /healthz HTTP/1.0\r\n\r\n").unwrap();
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    out.starts_with("HTTP/1.1 200")
}

#[tokio::test]
async fn setup_sends_the_password_on_stdin_only() {
    let tmp = tempfile::tempdir().unwrap();
    let m = manager(tmp.path());
    assert_eq!(m.status().await.state, "not_set_up");
    set_up(&m).await;
    let cfg = tmp.path().join("mediagit-server.toml");
    let cfg = cfg.to_str().unwrap();
    let argv = std::fs::read_to_string(format!("{cfg}.initargs")).unwrap();
    assert!(!argv.contains(PW), "password in argv");
    assert!(argv.contains("--admin-password-stdin") && argv.contains("--enable-auth"));
    assert!(argv.contains("127.0.0.1"));
    assert_eq!(std::fs::read_to_string(format!("{cfg}.stdin")).unwrap(), PW);
    assert_eq!(
        std::fs::read_to_string(format!("{cfg}.envleak")).unwrap(),
        "false",
        "password in the child's environment"
    );
    assert_eq!(m.status().await.state, "stopped");
    let again = m
        .setup("root", "root@example.com", &Secret::new(PW.into()).unwrap())
        .await
        .unwrap_err();
    assert_eq!(code(&again), "already_set_up");
}

#[tokio::test]
async fn start_serves_healthz_and_stop_ends_it() {
    let tmp = tempfile::tempdir().unwrap();
    let m = manager(tmp.path());
    assert_eq!(code(&m.start().await.unwrap_err()), "needs_setup");
    set_up(&m).await;
    let up = m.start().await.unwrap();
    assert_eq!(up.state, "running");
    let port = up.port.unwrap();
    assert_eq!(up.url.unwrap(), format!("http://127.0.0.1:{port}"));
    assert!(up.pid.is_some() && get_healthz(port));
    let serve = std::fs::read_to_string(tmp.path().join("mediagit-server.toml.serveargs")).unwrap();
    assert!(serve.contains("127.0.0.1") && !serve.contains("0.0.0.0"));
    assert!(m.log_tail(5).contains("listening"));
    let down = m.stop().await.unwrap();
    assert_eq!(down.state, "stopped");
    // Stop closed stdin and the server left on its own: no kill was needed.
    let marker = tmp.path().join("mediagit-server.toml.graceful");
    assert!(marker.is_file(), "the server did not exit on stdin EOF");
    let serve_args =
        std::fs::read_to_string(tmp.path().join("mediagit-server.toml.serveargs")).unwrap();
    assert!(serve_args.contains("--exit-on-stdin-eof"));
    assert!(!get_healthz(port), "the server is still answering");
}

#[tokio::test]
async fn a_server_that_never_answers_times_out_and_is_killed() {
    let tmp = tempfile::tempdir().unwrap();
    let m = manager(tmp.path());
    set_up(&m).await;
    std::fs::write(tmp.path().join("mediagit-server.toml"), "mode=slow\n").unwrap();
    let t = Instant::now();
    let e = m.start().await.unwrap_err();
    assert_eq!(code(&e), "health_timeout");
    assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
    assert_eq!(m.status().await.state, "stopped");
}

#[tokio::test]
async fn a_server_that_exits_reports_its_log() {
    let tmp = tempfile::tempdir().unwrap();
    let m = manager(tmp.path());
    set_up(&m).await;
    std::fs::write(tmp.path().join("mediagit-server.toml"), "mode=exit\n").unwrap();
    let e = m.start().await.unwrap_err();
    assert_eq!(code(&e), "start_failed");
    assert!(e.to_string().contains("boom"), "{e}");
}

#[tokio::test]
async fn a_busy_port_is_reported_not_taken() {
    let tmp = tempfile::tempdir().unwrap();
    let m = manager(tmp.path());
    set_up(&m).await;
    let port = m.status().await.port.unwrap();
    let _held = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
    assert_eq!(code(&m.start().await.unwrap_err()), "port_in_use");
}

#[tokio::test]
async fn a_server_this_app_did_not_start_is_never_started_twice_or_stopped() {
    let tmp = tempfile::tempdir().unwrap();
    let first = manager(tmp.path());
    set_up(&first).await;
    let port = first.start().await.unwrap().port.unwrap();

    // A second manager over the same folder stands in for another app instance.
    let second = manager(tmp.path());
    let s = second.status().await;
    assert_eq!(s.state, "elsewhere");
    assert_eq!(code(&second.start().await.unwrap_err()), "already_running");
    assert_eq!(code(&second.stop().await.unwrap_err()), "not_owned");
    second.stop_owned().await;
    assert!(get_healthz(port), "a non-owner stopped the server");

    first.stop_owned().await;
    assert!(!get_healthz(port));
    assert_eq!(code(&first.stop().await.unwrap_err()), "not_owned");
}

#[tokio::test]
async fn network_access_is_opt_in_and_only_while_stopped() {
    let tmp = tempfile::tempdir().unwrap();
    let m = manager(tmp.path());
    set_up(&m).await;
    assert!(!m.status().await.lan);
    assert!(m.set_lan(true).await.unwrap().lan);
    m.start().await.unwrap();
    assert_eq!(code(&m.set_lan(false).await.unwrap_err()), "running");
    let serve = std::fs::read_to_string(tmp.path().join("mediagit-server.toml.serveargs")).unwrap();
    assert!(serve.contains("0.0.0.0"));
    m.stop().await.unwrap();
}

#[tokio::test]
async fn setup_failure_leaves_no_config_behind() {
    let tmp = tempfile::tempdir().unwrap();
    let m = manager(tmp.path());
    // Secret::new refuses an empty password, so reach the fake's own refusal with a blank
    // line smuggled through a username check instead: a bad username never spawns init.
    let e = m
        .setup("-bad", "a@b.c", &Secret::new(PW.into()).unwrap())
        .await
        .unwrap_err();
    assert!(matches!(e, EngineError::InvalidArgument(_)));
    assert_eq!(m.status().await.state, "not_set_up");
    assert!(Secret::new(String::new()).is_err());
}

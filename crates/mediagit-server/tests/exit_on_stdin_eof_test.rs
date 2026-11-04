// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! `--exit-on-stdin-eof`: a parent that cannot send signals stops the server
//! by dropping the child's stdin pipe. The process must exit 0 (graceful).

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn spawn(extra: &[&str]) -> (std::process::Child, std::path::PathBuf) {
    let dir = tempfile::TempDir::new().unwrap().keep();
    let child = Command::new(env!("CARGO_BIN_EXE_mediagit-server"))
        .args(["--port", "0", "--host", "127.0.0.1", "--data-dir"])
        .arg(&dir)
        .args(extra)
        .current_dir(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn mediagit-server");
    (child, dir)
}

fn wait_exit(child: &mut std::process::Child, limit: Duration) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if let Some(s) = child.try_wait().unwrap() {
            return Some(s);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    None
}

#[test]
fn closing_stdin_stops_the_server_gracefully() {
    let (mut child, dir) = spawn(&["--exit-on-stdin-eof"]);
    // Let it reach the serve loop before closing the pipe.
    std::thread::sleep(Duration::from_secs(3));
    assert!(
        child.try_wait().unwrap().is_none(),
        "server died before stdin closed"
    );
    drop(child.stdin.take());
    let status = wait_exit(&mut child, Duration::from_secs(20));
    if status.is_none() {
        let _ = child.kill();
    }
    let _ = std::fs::remove_dir_all(&dir);
    let status = status.expect("server did not exit after stdin EOF");
    assert!(status.success(), "expected graceful exit 0, got {status}");
}

#[test]
fn without_the_flag_closing_stdin_does_not_stop_it() {
    let (mut child, dir) = spawn(&[]);
    std::thread::sleep(Duration::from_secs(3));
    drop(child.stdin.take());
    let status = wait_exit(&mut child, Duration::from_secs(5));
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        status.is_none(),
        "server exited on stdin EOF without the flag: {status:?}"
    );
}

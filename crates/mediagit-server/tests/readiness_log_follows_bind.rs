// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! "MediaGit server listening" must be an OBSERVATION, never a promise.
//!
//! The line used to be logged BEFORE `TcpListener::bind`, so a server that
//! failed to bind - or one that never reached `accept()` - produced a startup
//! log byte-identical to a healthy one. Three campaign wedges (ga15, ga18)
//! were unfalsifiable for exactly this reason: the only evidence available
//! after the fact could not distinguish "never bound" from "bound but not
//! serving", so every investigation stalled on the same missing bit.
//!
//! This test pins the ordering by taking the port away first. If the readiness
//! line can still appear when binding is impossible, the line means nothing.

use std::io::Read;
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const READY_MARKER: &str = "MediaGit server listening";

#[test]
fn readiness_is_not_announced_when_the_port_cannot_be_bound() {
    // Hold the port for the whole test. Rust's TcpListener does not set
    // SO_REUSEADDR on Windows, and binds with it on Unix only for TIME_WAIT
    // reuse, so a second bind to a live listener fails on both.
    let squatter = TcpListener::bind("127.0.0.1:0").expect("bind squatter");
    let port = squatter.local_addr().expect("local_addr").port();

    let data_dir = std::env::temp_dir().join(format!("mediagit-readiness-test-{port}"));
    std::fs::create_dir_all(&data_dir).expect("create data dir");

    let mut child = Command::new(env!("CARGO_BIN_EXE_mediagit-server"))
        .arg("--port")
        .arg(port.to_string())
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--data-dir")
        .arg(&data_dir)
        // Run from the empty data dir so the default `mediagit-server.toml`
        // lookup finds nothing and the built-in defaults apply. Passing a
        // non-existent path via `--config` instead makes the server exit on a
        // config error BEFORE it ever reaches the bind, which would make this
        // test pass without exercising anything.
        .current_dir(&data_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn mediagit-server");

    // A server that cannot bind must fail fast. If it is still alive well past
    // any plausible startup, that is itself the bug this test guards.
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => break Some(status),
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                break None;
            }
            None => std::thread::sleep(Duration::from_millis(100)),
        }
    };

    let mut stdout = String::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_string(&mut stdout);
    }
    let mut stderr = String::new();
    if let Some(mut err) = child.stderr.take() {
        let _ = err.read_to_string(&mut stderr);
    }
    let _ = std::fs::remove_dir_all(&data_dir);

    // The load-bearing assertion. The port was never available, so nothing was
    // ever listening on it; announcing otherwise makes the log a liar and
    // destroys the only evidence a post-mortem has.
    assert!(
        !stdout.contains(READY_MARKER) && !stderr.contains(READY_MARKER),
        "server announced {READY_MARKER:?} for a port it could not bind.\n\
         --- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
    );

    // And it must actually give up rather than sit there looking healthy.
    let status = status.expect("server did not exit after failing to bind");
    assert!(
        !status.success(),
        "server exited 0 despite never binding {port}"
    );

    drop(squatter);
}

/// The other half. Proving only "it stays quiet when the bind fails" would be
/// satisfied by deleting the log line entirely, which would be worse than the
/// bug: the campaign harness and every operator rely on that line to know the
/// port is live. So a server that CAN bind must still announce it, and the
/// announcement must name the address it is really listening on.
#[test]
fn readiness_is_announced_once_the_port_is_actually_bound() {
    // Take a port, then release it, so we know it was free a moment ago.
    let port = {
        let probe = TcpListener::bind("127.0.0.1:0").expect("bind probe");
        probe.local_addr().expect("local_addr").port()
    };

    let data_dir = std::env::temp_dir().join(format!("mediagit-readiness-ok-{port}"));
    std::fs::create_dir_all(&data_dir).expect("create data dir");

    let mut child = Command::new(env!("CARGO_BIN_EXE_mediagit-server"))
        .arg("--port")
        .arg(port.to_string())
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--data-dir")
        .arg(&data_dir)
        .current_dir(&data_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn mediagit-server");

    // Poll the socket rather than sleeping a fixed amount: the assertion we
    // want is "it is reachable", and a timeout that passes on a slow machine
    // by accident is not an assertion.
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut reachable = false;
    while Instant::now() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            reachable = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let _ = child.kill();
    let _ = child.wait();

    let mut stdout = String::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_string(&mut stdout);
    }
    let _ = std::fs::remove_dir_all(&data_dir);

    assert!(reachable, "server never became reachable on port {port}");
    assert!(
        stdout.contains(READY_MARKER),
        "a server that bound successfully must still announce readiness.\n\
         --- stdout ---\n{stdout}"
    );
    assert!(
        stdout.contains(&format!("127.0.0.1:{port}")),
        "the readiness line must name the address actually bound ({port}).\n\
         --- stdout ---\n{stdout}"
    );
}

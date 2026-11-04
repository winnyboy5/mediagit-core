// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Stand-in for `mediagit-server`, used only by the server manager tests.
//!
//! `init` records what it was given (argv, stdin, whether the secret reached the
//! environment) beside the config. Serving answers `GET /healthz`; the config's first
//! line picks the behaviour: `mode=ok`, `mode=slow` (binds after 6 s) or `mode=exit`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, Read, Write};
use std::net::TcpListener;
use std::time::Duration;

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let config = flag(&args, "--config").expect("--config");
    if args.first().map(String::as_str) == Some("init") {
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line).unwrap();
        let line = line.trim_end().to_owned();
        if line.is_empty() {
            eprintln!("no password on stdin");
            std::process::exit(1);
        }
        std::fs::write(&config, "mode=ok\n").unwrap();
        std::fs::write(format!("{config}.initargs"), args.join("\n")).unwrap();
        std::fs::write(format!("{config}.stdin"), &line).unwrap();
        let leaked = std::env::vars().any(|(_, v)| v == line);
        std::fs::write(format!("{config}.envleak"), leaked.to_string()).unwrap();
        return;
    }
    let mode = std::fs::read_to_string(&config).unwrap_or_default();
    std::fs::write(format!("{config}.serveargs"), args.join("\n")).unwrap();
    if mode.starts_with("mode=exit") {
        eprintln!("boom: cannot start");
        std::process::exit(3);
    }
    if mode.starts_with("mode=slow") {
        std::thread::sleep(Duration::from_secs(6));
    }
    if args.iter().any(|a| a == "--exit-on-stdin-eof") {
        let marker = format!("{config}.graceful");
        std::thread::spawn(move || {
            let mut sink = Vec::new();
            let _ = std::io::stdin().lock().read_to_end(&mut sink);
            std::fs::write(marker, "eof").unwrap();
            std::process::exit(0);
        });
    }
    let host = flag(&args, "--host").unwrap_or_else(|| "127.0.0.1".into());
    let port = flag(&args, "--port").expect("--port");
    let listener = TcpListener::bind(format!("{host}:{port}")).unwrap();
    println!("fake server listening on {host}:{port}");
    for stream in listener.incoming().flatten() {
        let mut stream = stream;
        let mut buf = [0u8; 512];
        let _ = stream.read(&mut buf);
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok");
    }
}

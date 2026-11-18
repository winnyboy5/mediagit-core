// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! BUG-036: `download_file_by_path_with_progress` reports real bytes and the
//! total from Content-Length, so the app can show a determinate bar.

use std::sync::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

async fn serve_in_two_parts(listener: TcpListener) {
    let (mut sock, _) = listener.accept().await.expect("accept");
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = sock.read(&mut chunk).await.expect("read");
        assert!(n > 0, "client closed before sending a request");
        buf.extend_from_slice(&chunk[..n]);
    }
    sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 10\r\nconnection: close\r\n\r\n01234")
        .await
        .expect("write head");
    sock.flush().await.expect("flush");
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    sock.write_all(b"56789").await.expect("write tail");
    sock.flush().await.expect("flush");
}

#[tokio::test(flavor = "multi_thread")]
async fn progress_reports_growing_bytes_and_the_total() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(serve_in_two_parts(listener));

    let client = mediagit_protocol::ProtocolClient::new(format!("http://{addr}/repo"));
    let mut sink = Vec::new();
    let seen = Mutex::new(Vec::new());
    let n = client
        .download_file_by_path_with_progress("a.bin", "main", &mut sink, &|pos, total| {
            seen.lock().unwrap().push((pos, total))
        })
        .await
        .expect("download");
    server.await.ok();

    assert_eq!(n, 10);
    assert_eq!(sink, b"0123456789");
    let seen = seen.into_inner().unwrap();
    assert!(
        seen.iter().all(|(_, t)| *t == Some(10)),
        "total comes from Content-Length: {seen:?}"
    );
    assert!(
        seen.windows(2).all(|w| w[0].0 <= w[1].0),
        "bytes never go backwards: {seen:?}"
    );
    assert_eq!(seen.last().map(|s| s.0), Some(10));
    assert!(
        seen.len() >= 2,
        "progress is reported as bytes arrive, not once at the end: {seen:?}"
    );
}

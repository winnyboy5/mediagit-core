// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Diagnostic for the reqsign Shared Key container-create signature.
//!
//! Run against the REAL Azure account (credentials from
//! dev-tests/qa-suite/scripts/campaign_env.ps1):
//!   cargo test -p mediagit-storage --features azure --test reqsign_diag -- --ignored --nocapture
//!
//! Prints the headers reqsign actually applies (signature value redacted) so a
//! 403 can be attributed to a concrete missing/incorrect header rather than
//! guessed at.
#![cfg(feature = "azure")]

use reqsign_azure_storage::{RequestSigner, StaticCredentialProvider};
use reqsign_core::{Context, Signer};

fn creds() -> Option<(String, String, String)> {
    let a = std::env::var("AZURE_STORAGE_ACCOUNT").ok()?;
    let k = std::env::var("AZURE_STORAGE_KEY").ok()?;
    let c = std::env::var("MG_QA_AZURE_CONTAINER").ok()?;
    if a.is_empty() || k.is_empty() || c.is_empty() {
        return None;
    }
    Some((a, k, c))
}

#[tokio::test]
#[ignore] // needs real Azure credentials
async fn reqsign_container_create_headers() {
    // reqwest is built with rustls-no-provider; without this the client panics.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let Some((account, key, _existing)) = creds() else {
        eprintln!("SKIP: no real-Azure credentials in environment");
        return;
    };

    // A container that does NOT exist, so we exercise the create path without
    // disturbing the shared QA container.
    let container = format!(
        "mgdiag{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    );
    let endpoint = format!("https://{account}.blob.core.windows.net");
    let url = format!("{endpoint}/{container}?restype=container");

    let mut parts = http::Request::builder()
        .method(http::Method::PUT)
        .uri(&url)
        .header(http::header::CONTENT_LENGTH, "0")
        .header("x-ms-version", "2023-11-03")
        .body(())
        .unwrap()
        .into_parts()
        .0;

    println!("--- headers BEFORE signing ---");
    for (n, v) in parts.headers.iter() {
        println!("  {}: {}", n.as_str(), v.to_str().unwrap_or("<binary>"));
    }

    let signer = Signer::new(
        Context::new(),
        StaticCredentialProvider::new_shared_key(&account, &key),
        RequestSigner::new(),
    );
    match signer.sign(&mut parts, None).await {
        Ok(()) => println!("sign(): OK"),
        Err(e) => {
            println!("sign() FAILED: {e}");
            panic!("reqsign could not sign the request");
        }
    }

    println!("--- headers AFTER signing ---");
    let mut saw_auth = false;
    let mut saw_date = false;
    for (n, v) in parts.headers.iter() {
        let name = n.as_str();
        let shown = if name.eq_ignore_ascii_case("authorization") {
            saw_auth = true;
            // Show only the scheme + account, never the signature material.
            let raw = v.to_str().unwrap_or("");
            match raw.split_once(':') {
                Some((lhs, _)) => format!("{lhs}:<redacted>"),
                None => "<unparseable>".to_string(),
            }
        } else {
            if name.eq_ignore_ascii_case("x-ms-date") {
                saw_date = true;
            }
            v.to_str().unwrap_or("<binary>").to_string()
        };
        println!("  {name}: {shown}");
    }
    println!("authorization present: {saw_auth}");
    println!("x-ms-date present:     {saw_date}");

    // Now actually send it — the ground truth.
    let client = reqwest::Client::new();
    let mut req = client.request(
        reqwest::Method::from_bytes(parts.method.as_str().as_bytes()).unwrap(),
        parts.uri.to_string(),
    );
    for (n, v) in parts.headers.iter() {
        req = req.header(n.as_str(), v.as_bytes());
    }
    let resp = req.send().await.expect("send");
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    println!("RESULT: {status}");
    if !status.is_success() && status != reqwest::StatusCode::CONFLICT {
        println!("body: {}", &body[..body.len().min(600)]);
    }

    // Clean up if we created it.
    if status.is_success() {
        println!("created {container} - cleaning up");
        let mut del = http::Request::builder()
            .method(http::Method::DELETE)
            .uri(&url)
            .header("x-ms-version", "2023-11-03")
            .body(())
            .unwrap()
            .into_parts()
            .0;
        if signer.sign(&mut del, None).await.is_ok() {
            let mut dreq = client.delete(del.uri.to_string());
            for (n, v) in del.headers.iter() {
                dreq = dreq.header(n.as_str(), v.as_bytes());
            }
            let d = dreq.send().await.map(|r| r.status().to_string());
            println!("cleanup: {d:?}");
        }
    }

    assert!(
        status.is_success() || status == reqwest::StatusCode::CONFLICT,
        "reqsign-signed container create failed: {status}"
    );
}

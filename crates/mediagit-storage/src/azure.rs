// MediaGit - Git for Media Files
// Copyright (C) 2025 MediaGit Contributors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU Affero General Public License for more details.

//! Azure Blob Storage backend, built on Apache OpenDAL.
//!
//! # Why OpenDAL and not Microsoft's SDK
//!
//! The community `azure_storage_blobs` 0.21 line this backend used to sit on
//! is EOL (moved to `/tree/legacy`), and carried five RUSTSEC advisories plus
//! a duplicate `reqwest 0.12` into the tree. Microsoft's GA line
//! (`azure_storage_blob` 1.x) cannot replace it: `BlobClient::new` accepts
//! only `Option<Arc<dyn TokenCredential>>` — Entra ID — while MediaGit
//! authenticates with a shared account key. Upstream
//! `Azure/azure-sdk-for-rust#2975` tracks that gap and is still open.
//!
//! OpenDAL supports shared key, SAS, connection strings, and the Azurite
//! emulator, and mints Service SAS presigned URLs from the account key —
//! the same model this backend has always used.
//!
//! # Known limitation: presign against Azurite
//!
//! OpenDAL hardcodes `sv=2020-12-06` in its Service SAS with no override
//! (`reqsign-azure-storage`'s `service_sas.rs`; `AzblobBuilder` exposes no
//! `sas_version()`). Azurite rejects that service version, so presigned URLs
//! cannot be exercised against the emulator — verified 2026-07-23, and
//! verified to be an *emulator* gap: real Azure accepts the same SAS
//! (`201 Created`), and a `sv=2022-11-02` SAS succeeds against the same
//! Azurite. Presign coverage therefore lives in the live-Azure leg of
//! `06_remote`, not in the Azurite suite.
//!
//! # Container creation
//!
//! OpenDAL is data-plane only and has no container management. Creating the
//! container is one signed REST call, made with `reqsign-azure-storage` — the
//! same signer OpenDAL uses internally — so we do not hand-roll Azure's
//! Shared Key signing. It requires an account key, so backends built from a
//! SAS token cannot auto-create (a SAS holder generally lacks that right
//! anyway); those surface a clear error if the container is absent.

use crate::StorageBackend;
use crate::error::StorageError;
use async_trait::async_trait;
use opendal::Operator;
use opendal::layers::{RetryLayer, TimeoutLayer};
use opendal::services::Azblob;
use std::fmt;
use std::time::Duration;

/// Chunk size for multipart uploads (4 MB)
/// This provides a good balance between memory usage and upload efficiency
const CHUNK_SIZE: usize = 4 * 1024 * 1024; // 4 MB

/// Block size for Azure block blob operations
const AZURE_BLOCK_SIZE: usize = 4 * 1024 * 1024; // 4 MB, Azure maximum is 4GB

/// Azure Blob Storage backend
///
/// Thread-safe implementation of `StorageBackend`. Supports SAS token,
/// account key, and connection-string authentication.
///
/// # Thread Safety
///
/// `Operator` is `Send + Sync` and internally reference-counted, so this type
/// is cheap to clone and safe to share across tasks.
#[derive(Clone)]
pub struct AzureBackend {
    account_name: String,
    container_name: String,
    /// Logical-key prefix applied to every blob name on the wire. Empty means
    /// keys land at container root (legacy behaviour). Non-empty values let
    /// multiple repos share one container without colliding on identical OIDs.
    /// The prefix is hidden from callers — `put("k")` writes "<prefix>k" to
    /// Azure but `get("k")` resolves it transparently, and `list_objects`
    /// strips it before returning.
    prefix: String,
    /// OpenDAL operator scoped to the container.
    op: Operator,
    /// Blob-service endpoint root (no container segment), used for the
    /// container-create REST call.
    endpoint: String,
    /// Account key, when we have one. `None` for SAS-authenticated backends,
    /// which cannot sign a container-create request.
    account_key: Option<String>,
}

impl fmt::Debug for AzureBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AzureBackend")
            .field("account_name", &self.account_name)
            .field("container_name", &self.container_name)
            .field("prefix", &self.prefix)
            .finish()
    }
}

/// Normalize a prefix so it ends with `/` (or is empty). Idempotent.
fn normalize_prefix(p: &str) -> String {
    if p.is_empty() || p.ends_with('/') {
        p.to_string()
    } else {
        format!("{}/", p)
    }
}

/// Map a logical key to its on-the-wire blob name under `prefix`.
///
/// Free function, not a method, so it is testable without constructing an
/// `Operator`. (Building one outside a tokio runtime panics OpenDAL's
/// executor `LazyLock` — and the previous implementation had the same shape
/// of problem, needing a throwaway `ClientBuilder` just to test string logic.)
fn full_key_with(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_string()
    } else {
        format!("{prefix}{key}")
    }
}

/// Inverse of [`full_key_with`]; returns `full` unchanged if unprefixed.
fn strip_prefix_with<'a>(prefix: &str, full: &'a str) -> &'a str {
    if prefix.is_empty() {
        full
    } else {
        full.strip_prefix(prefix).unwrap_or(full)
    }
}

/// Wrap an OpenDAL operator with the layers every code path expects.
fn with_layers(op: Operator) -> Operator {
    op.layer(RetryLayer::new()).layer(TimeoutLayer::new())
}

/// Install ring as the process-level rustls provider, once.
///
/// OpenDAL reaches Azure through the workspace `reqwest`, which is built with
/// `rustls-no-provider` (the workspace standardises on ring; reqwest 0.13's
/// plain `rustls` feature hard-wires aws-lc-rs). Without an installed
/// provider, building the HTTP client **panics** — and because OpenDAL holds
/// its executor behind a `LazyLock`, that first panic poisons the lock and
/// every later operation in the process fails with a misleading
/// "previously poisoned" message.
///
/// Doing this at backend construction rather than in each binary's `main()`
/// means test binaries and library consumers cannot forget it. Idempotent.
fn ensure_crypto_provider() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// Map an OpenDAL error onto the crate's typed [`StorageError`].
///
/// This replaces substring-matching on `"404"` / `"BlobNotFound"` / `"403"`,
/// which meant an SDK message change could silently break not-found
/// classification — and not-found is load-bearing here: `exists()` and the
/// dedup paths branch on it.
fn map_error(err: &opendal::Error, context: &str) -> StorageError {
    use opendal::ErrorKind;
    let msg = format!("{context}: {err}");
    match err.kind() {
        ErrorKind::NotFound => StorageError::NotFound(msg),
        ErrorKind::PermissionDenied => StorageError::PermissionDenied(msg),
        ErrorKind::AlreadyExists => StorageError::Backend(msg),
        ErrorKind::RateLimited => StorageError::Backend(msg),
        _ => StorageError::Backend(msg),
    }
}

/// Extract a `key=value` field from an Azure connection string.
fn conn_field(conn: &str, field: &str) -> Option<String> {
    let needle = format!("{field}=");
    conn.split(';')
        .find(|s| s.starts_with(&needle))
        .and_then(|s| s.strip_prefix(&needle))
        .map(str::to_owned)
}

impl AzureBackend {
    /// Back-compat shim: equivalent to `with_sas_token_and_prefix(..., "")`.
    pub async fn with_sas_token(
        account_name: impl Into<String>,
        container_name: impl Into<String>,
        sas_token: impl Into<String>,
    ) -> anyhow::Result<Self> {
        Self::with_sas_token_and_prefix(account_name, container_name, sas_token, "").await
    }

    /// Create a backend authenticated with a Shared Access Signature.
    ///
    /// Container auto-creation is unavailable on this path — signing that
    /// request needs the account key.
    pub async fn with_sas_token_and_prefix(
        account_name: impl Into<String>,
        container_name: impl Into<String>,
        sas_token: impl Into<String>,
        prefix: impl Into<String>,
    ) -> anyhow::Result<Self> {
        let account_name = account_name.into();
        let container_name = container_name.into();
        let sas_token = sas_token.into();
        let prefix = normalize_prefix(&prefix.into());

        if account_name.is_empty() {
            return Err(anyhow::anyhow!("account_name cannot be empty"));
        }
        if container_name.is_empty() {
            return Err(anyhow::anyhow!("container_name cannot be empty"));
        }
        if sas_token.is_empty() {
            return Err(anyhow::anyhow!("sas_token cannot be empty"));
        }

        ensure_crypto_provider();
        let endpoint = format!("https://{account_name}.blob.core.windows.net");
        let builder = Azblob::default()
            .account_name(&account_name)
            .container(&container_name)
            .endpoint(&endpoint)
            .sas_token(&sas_token);
        let op = with_layers(
            Operator::new(builder)
                .map_err(|e| anyhow::anyhow!("Failed to build Azure operator: {e}"))?
                .finish(),
        );

        Ok(Self {
            account_name,
            container_name,
            prefix,
            op,
            endpoint,
            account_key: None,
        })
    }

    /// Back-compat shim: equivalent to `with_account_key_and_prefix(..., "")`.
    pub async fn with_account_key(
        account_name: impl Into<String>,
        container_name: impl Into<String>,
        account_key: impl Into<String>,
    ) -> anyhow::Result<Self> {
        Self::with_account_key_and_prefix(account_name, container_name, account_key, "").await
    }

    /// Create a backend authenticated with a shared account key.
    pub async fn with_account_key_and_prefix(
        account_name: impl Into<String>,
        container_name: impl Into<String>,
        account_key: impl Into<String>,
        prefix: impl Into<String>,
    ) -> anyhow::Result<Self> {
        let account_name = account_name.into();
        let container_name = container_name.into();
        let account_key = account_key.into();
        let prefix = normalize_prefix(&prefix.into());

        if account_name.is_empty() {
            return Err(anyhow::anyhow!("account_name cannot be empty"));
        }
        if container_name.is_empty() {
            return Err(anyhow::anyhow!("container_name cannot be empty"));
        }
        if account_key.is_empty() {
            return Err(anyhow::anyhow!("account_key cannot be empty"));
        }

        let endpoint = format!("https://{account_name}.blob.core.windows.net");
        Self::build(
            account_name,
            container_name,
            prefix,
            endpoint,
            Some(account_key),
            None,
        )
        .await
    }

    /// Back-compat shim: equivalent to `with_connection_string_and_prefix(..., "")`.
    pub async fn with_connection_string(
        container_name: impl Into<String>,
        connection_string: impl Into<String>,
    ) -> anyhow::Result<Self> {
        Self::with_connection_string_and_prefix(container_name, connection_string, "").await
    }

    /// Create a backend from a full Azure connection string.
    ///
    /// `BlobEndpoint=` is honoured, which is how Azurite and sovereign-cloud
    /// endpoints are reached.
    pub async fn with_connection_string_and_prefix(
        container_name: impl Into<String>,
        connection_string: impl Into<String>,
        prefix: impl Into<String>,
    ) -> anyhow::Result<Self> {
        let container_name = container_name.into();
        let connection_string = connection_string.into();
        let prefix = normalize_prefix(&prefix.into());

        if container_name.is_empty() {
            return Err(anyhow::anyhow!("container_name cannot be empty"));
        }
        if connection_string.is_empty() {
            return Err(anyhow::anyhow!("connection_string cannot be empty"));
        }

        let account_name = conn_field(&connection_string, "AccountName")
            .ok_or_else(|| anyhow::anyhow!("Invalid connection string: missing AccountName"))?;
        let account_key = conn_field(&connection_string, "AccountKey")
            .ok_or_else(|| anyhow::anyhow!("Invalid connection string: missing AccountKey"))?;

        // BlobEndpoint already includes the account segment for Azurite
        // (http://host:port/devstoreaccount1); the cloud form does not.
        let endpoint = conn_field(&connection_string, "BlobEndpoint")
            .unwrap_or_else(|| format!("https://{account_name}.blob.core.windows.net"));

        Self::build(
            account_name,
            container_name,
            prefix,
            endpoint,
            Some(account_key),
            None,
        )
        .await
    }

    /// Shared constructor for the account-key-bearing paths.
    async fn build(
        account_name: String,
        container_name: String,
        prefix: String,
        endpoint: String,
        account_key: Option<String>,
        sas_token: Option<String>,
    ) -> anyhow::Result<Self> {
        ensure_crypto_provider();
        let mut builder = Azblob::default()
            .account_name(&account_name)
            .container(&container_name)
            .endpoint(&endpoint);
        if let Some(key) = &account_key {
            builder = builder.account_key(key);
        }
        if let Some(sas) = &sas_token {
            builder = builder.sas_token(sas);
        }
        let op = with_layers(
            Operator::new(builder)
                .map_err(|e| anyhow::anyhow!("Failed to build Azure operator: {e}"))?
                .finish(),
        );

        let backend = Self {
            account_name,
            container_name,
            prefix,
            op,
            endpoint,
            account_key,
        };

        // Bounded: the SDK default can hang for tens of seconds. The backend
        // is cached in AppState server-side, so this runs once per repo per
        // process. 30 s leaves room for cold DNS + TLS to high-latency regions.
        tokio::time::timeout(Duration::from_secs(30), backend.ensure_container_exists())
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "Azure container check timed out after 30s for {}/{}",
                    backend.account_name,
                    backend.container_name
                )
            })??;

        Ok(backend)
    }

    /// Validate a logical key.
    fn validate_key(key: &str) -> anyhow::Result<()> {
        if key.is_empty() {
            return Err(anyhow::anyhow!("key cannot be empty"));
        }
        Ok(())
    }

    /// Map a logical key to the on-the-wire blob name.
    fn full_key(&self, key: &str) -> String {
        full_key_with(&self.prefix, key)
    }

    /// Inverse of [`Self::full_key`]; returns `full` unchanged if unprefixed.
    fn strip_prefix<'a>(&self, full: &'a str) -> &'a str {
        strip_prefix_with(&self.prefix, full)
    }

    /// Create the container if it is absent.
    ///
    /// OpenDAL has no container API, so this is one Shared-Key-signed REST
    /// call via `reqsign-azure-storage` — the signer OpenDAL itself uses.
    /// Treats 409 `ContainerAlreadyExists` as success (another process may
    /// have won the race between our check and our create).
    async fn ensure_container_exists(&self) -> anyhow::Result<()> {
        // Check before mutating: a listable container needs no create call at
        // all, which is the overwhelmingly common case (the container is
        // provisioned once, then reused for the life of the deployment). This
        // also keeps the signed-create path — the fragile part — off the hot
        // path entirely.
        if self.op.list_with("").limit(1).await.is_ok() {
            tracing::debug!(container = %self.container_name, "Azure container reachable");
            return Ok(());
        }

        let Some(account_key) = &self.account_key else {
            // SAS-authenticated: cannot sign a create, and a SAS holder
            // generally lacks that right anyway.
            return Err(anyhow::anyhow!(
                "Azure container '{}' is not reachable and cannot be created with SAS                  authentication. Create it first, or configure account-key auth.",
                self.container_name
            ));
        };

        use reqsign_azure_storage::{RequestSigner, StaticCredentialProvider};
        use reqsign_core::{Context, Signer};

        let url = format!(
            "{}/{}?restype=container",
            self.endpoint.trim_end_matches('/'),
            self.container_name
        );

        let signer = Signer::new(
            // Shared Key signing is a local HMAC over the canonicalised
            // request — no network or filesystem access — so a bare Context
            // is sufficient.
            Context::new(),
            StaticCredentialProvider::new_shared_key(&self.account_name, account_key),
            RequestSigner::new(),
        );

        // Real Azure and Azurite disagree about Content-Length on a zero-body
        // PUT, and the two requirements are mutually exclusive (measured
        // 2026-07-23):
        //
        //   real Azure : header REQUIRED -> 411 Length Required without it,
        //                and signed as "" per the Shared Key spec.
        //   Azurite    : canonicalises the header's literal "0" instead of "",
        //                so sending it yields 403 AuthorizationFailure.
        //
        // Send the spec-correct form first and fall back once. Cheap (this
        // runs at most once per backend, only when the container is absent)
        // and avoids hardcoding emulator detection, which would misfire on
        // Azurite behind TLS or a custom endpoint.
        let mut last: Option<(reqwest::StatusCode, String)> = None;
        for send_content_length in [true, false] {
            let mut builder = http::Request::builder()
                .method(http::Method::PUT)
                .uri(&url)
                .header("x-ms-version", "2023-11-03");
            if send_content_length {
                builder = builder.header(http::header::CONTENT_LENGTH, "0");
            }
            let mut parts = builder
                .body(())
                .map_err(|e| anyhow::anyhow!("Failed to build container-create request: {e}"))?
                .into_parts()
                .0;

            signer
                .sign(&mut parts, None)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to sign container-create request: {e}"))?;

            let client = reqwest::Client::new();
            let mut req = client.request(
                reqwest::Method::from_bytes(parts.method.as_str().as_bytes())?,
                parts.uri.to_string(),
            );
            for (name, value) in parts.headers.iter() {
                req = req.header(name.as_str(), value.as_bytes());
            }
            let resp = req
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("Container-create request failed: {e}"))?;

            let status = resp.status();
            if status.is_success() || status == reqwest::StatusCode::CONFLICT {
                tracing::info!(
                    container = %self.container_name,
                    content_length_sent = send_content_length,
                    "Azure container ready"
                );
                return Ok(());
            }
            let body = resp.text().await.unwrap_or_default();
            tracing::debug!(
                container = %self.container_name,
                content_length_sent = send_content_length,
                %status,
                "container-create attempt rejected; trying alternate Content-Length form"
            );
            last = Some((status, body));
        }

        let (status, body) = last.unwrap_or((
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            "no response".to_string(),
        ));
        Err(anyhow::anyhow!(
            "Azure container '{}' does not exist and could not be created ({} {}).              Create it manually and retry.",
            self.container_name,
            status,
            body
        ))
    }

    /// Single-shot upload for payloads below the block threshold.
    async fn put_direct(&self, key: &str, data: &[u8]) -> anyhow::Result<()> {
        let full = self.full_key(key);
        self.op
            .write(&full, data.to_vec())
            .await
            .map_err(|e| map_error(&e, &format!("put {key}")))?;
        Ok(())
    }

    /// Staged block upload for large payloads.
    ///
    /// Maps to Azure Put Block / Put Block List. `concurrent` is honoured via
    /// `MEDIAGIT_AZURE_PUT_BLOCK_CONCURRENCY` (default 8), unchanged from the
    /// previous implementation.
    async fn put_chunked(&self, key: &str, data: &[u8]) -> anyhow::Result<()> {
        let full = self.full_key(key);
        let concurrency: usize = std::env::var("MEDIAGIT_AZURE_PUT_BLOCK_CONCURRENCY")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|n: &usize| *n > 0)
            .unwrap_or(8);

        let mut writer = self
            .op
            .writer_with(&full)
            .chunk(AZURE_BLOCK_SIZE)
            .concurrent(concurrency)
            .await
            .map_err(|e| map_error(&e, &format!("open chunked writer for {key}")))?;
        writer
            .write(data.to_vec())
            .await
            .map_err(|e| map_error(&e, &format!("chunked write {key}")))?;
        writer
            .close()
            .await
            .map_err(|e| map_error(&e, &format!("chunked close {key}")))?;
        Ok(())
    }
}

#[async_trait]
impl StorageBackend for AzureBackend {
    async fn get(&self, key: &str) -> anyhow::Result<Vec<u8>> {
        Self::validate_key(key)?;
        let full = self.full_key(key);
        let buf = self
            .op
            .read(&full)
            .await
            .map_err(|e| map_error(&e, &format!("get {key}")))?;
        Ok(buf.to_vec())
    }

    async fn get_streaming_range(
        &self,
        key: &str,
        range: std::ops::Range<u64>,
    ) -> anyhow::Result<
        std::pin::Pin<
            Box<dyn futures::Stream<Item = anyhow::Result<bytes::Bytes>> + Send + 'static>,
        >,
    > {
        Self::validate_key(key)?;
        let full = self.full_key(key);
        let buf = self
            .op
            .read_with(&full)
            .range(range)
            .await
            .map_err(|e| map_error(&e, &format!("get_range {key}")))?;
        let bytes = bytes::Bytes::from(buf.to_vec());
        Ok(Box::pin(futures::stream::once(async move { Ok(bytes) })))
    }

    async fn put(&self, key: &str, data: &[u8]) -> anyhow::Result<()> {
        Self::validate_key(key)?;
        if data.len() > CHUNK_SIZE {
            self.put_chunked(key, data).await
        } else {
            self.put_direct(key, data).await
        }
    }

    async fn exists(&self, key: &str) -> anyhow::Result<bool> {
        Self::validate_key(key)?;
        let full = self.full_key(key);
        match self.op.stat(&full).await {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == opendal::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(map_error(&e, &format!("exists {key}")).into()),
        }
    }

    async fn head(&self, key: &str) -> anyhow::Result<Option<u64>> {
        Self::validate_key(key)?;
        let full = self.full_key(key);
        match self.op.stat(&full).await {
            Ok(meta) => Ok(Some(meta.content_length())),
            Err(e) if e.kind() == opendal::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(map_error(&e, &format!("head {key}")).into()),
        }
    }

    async fn delete(&self, key: &str) -> anyhow::Result<()> {
        Self::validate_key(key)?;
        let full = self.full_key(key);
        // Idempotent by contract: deleting an absent key is success.
        match self.op.delete(&full).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == opendal::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(map_error(&e, &format!("delete {key}")).into()),
        }
    }

    async fn list_objects(&self, prefix: &str) -> anyhow::Result<Vec<String>> {
        let full_prefix = self.full_key(prefix);
        let entries = self
            .op
            .list_with(&full_prefix)
            .recursive(true)
            .await
            .map_err(|e| map_error(&e, &format!("list {prefix}")))?;

        let mut keys: Vec<String> = entries
            .into_iter()
            // Directory markers are an artefact of the listing model, not
            // objects callers stored.
            .filter(|e| e.metadata().is_file())
            .map(|e| self.strip_prefix(e.path()).to_string())
            .collect();
        // Contract: callers rely on sorted output.
        keys.sort();
        Ok(keys)
    }

    async fn presign_put(
        &self,
        key: &str,
        _content_length: u64,
        ttl: Duration,
    ) -> anyhow::Result<Option<crate::PresignedPut>> {
        Self::validate_key(key)?;
        let full = self.full_key(key);
        // Contract: presign failure is not fatal — callers fall back to the
        // server-proxy PUT, so degrade rather than abort the upload.
        let req = match self.op.presign_write(&full, ttl).await {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!(err = %e, "Azure presign_write unavailable; using proxy PUT");
                return Ok(None);
            }
        };
        // OpenDAL supplies x-ms-blob-type itself; forward whatever it sets
        // rather than hardcoding, so a future change stays correct.
        let required_headers = req
            .header()
            .iter()
            .filter_map(|(n, v)| {
                v.to_str()
                    .ok()
                    .map(|v| (n.as_str().to_string(), v.to_string()))
            })
            .collect();
        Ok(Some(crate::PresignedPut {
            url: req.uri().to_string(),
            method: req.method().as_str().to_string(),
            required_headers,
            expires_at: std::time::SystemTime::now() + ttl,
        }))
    }

    async fn presign_get(
        &self,
        key: &str,
        ttl: Duration,
    ) -> anyhow::Result<Option<crate::PresignedDownload>> {
        Self::validate_key(key)?;
        let full = self.full_key(key);
        let req = match self.op.presign_read(&full, ttl).await {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!(err = %e, "Azure presign_read unavailable; using proxy GET");
                return Ok(None);
            }
        };
        let headers = req
            .header()
            .iter()
            .filter_map(|(n, v)| {
                v.to_str()
                    .ok()
                    .map(|v| (n.as_str().to_string(), v.to_string()))
            })
            .collect();
        Ok(Some(crate::PresignedDownload {
            url: req.uri().to_string(),
            headers,
            expires_in_secs: ttl.as_secs(),
        }))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_key_empty() {
        assert!(AzureBackend::validate_key("").is_err());
    }

    #[test]
    fn test_validate_key_valid() {
        assert!(AzureBackend::validate_key("objects/abc123").is_ok());
    }

    #[test]
    fn test_validate_key_with_special_chars() {
        assert!(AzureBackend::validate_key("objects/ab-c_1.2").is_ok());
    }

    #[test]
    fn test_normalize_prefix_empty() {
        assert_eq!(normalize_prefix(""), "");
    }

    #[test]
    fn test_normalize_prefix_no_slash() {
        assert_eq!(normalize_prefix("repo"), "repo/");
    }

    #[test]
    fn test_normalize_prefix_already_slashed() {
        assert_eq!(normalize_prefix("repo/"), "repo/");
    }

    #[test]
    fn test_normalize_prefix_idempotent() {
        let once = normalize_prefix("repo");
        assert_eq!(normalize_prefix(&once), once);
    }

    // The prefix helpers are exercised through the free functions: building an
    // `Operator` here would panic OpenDAL's executor LazyLock (no tokio runtime
    // in a plain #[test]) and poison it for every other test in the binary —
    // which is exactly what happened on the first run of this rewrite.

    #[test]
    fn test_full_key_empty_prefix_is_passthrough() {
        assert_eq!(full_key_with("", "chunks/abc"), "chunks/abc");
    }

    #[test]
    fn test_full_key_with_prefix_prepends() {
        assert_eq!(
            full_key_with(&normalize_prefix("repo"), "chunks/abc"),
            "repo/chunks/abc"
        );
    }

    #[test]
    fn test_full_key_handles_already_slashed_prefix() {
        assert_eq!(
            full_key_with(&normalize_prefix("repo/"), "chunks/abc"),
            "repo/chunks/abc"
        );
    }

    #[test]
    fn test_strip_prefix_returns_logical_key() {
        assert_eq!(
            strip_prefix_with(&normalize_prefix("repo"), "repo/chunks/abc"),
            "chunks/abc"
        );
    }

    #[test]
    fn test_strip_prefix_no_match_returns_input() {
        assert_eq!(
            strip_prefix_with(&normalize_prefix("repo"), "other/chunks/abc"),
            "other/chunks/abc"
        );
    }

    #[test]
    fn test_strip_prefix_empty_prefix_is_passthrough() {
        assert_eq!(strip_prefix_with("", "chunks/abc"), "chunks/abc");
    }

    #[test]
    fn test_full_key_strip_prefix_roundtrip() {
        let prefix = normalize_prefix("repo");
        let logical = "chunks/deadbeef";
        assert_eq!(
            strip_prefix_with(&prefix, &full_key_with(&prefix, logical)),
            logical
        );
    }

    #[tokio::test]
    async fn test_empty_account_name_fails() {
        assert!(AzureBackend::with_account_key("", "c", "k").await.is_err());
    }

    #[tokio::test]
    async fn test_empty_container_name_fails() {
        assert!(AzureBackend::with_account_key("a", "", "k").await.is_err());
    }

    #[tokio::test]
    async fn test_empty_sas_token_fails() {
        assert!(AzureBackend::with_sas_token("a", "c", "").await.is_err());
    }

    #[tokio::test]
    async fn test_empty_account_key_fails() {
        assert!(AzureBackend::with_account_key("a", "c", "").await.is_err());
    }

    #[tokio::test]
    async fn test_empty_connection_string_fails() {
        assert!(AzureBackend::with_connection_string("c", "").await.is_err());
    }

    #[test]
    fn test_chunk_size_constant() {
        assert_eq!(CHUNK_SIZE, 4 * 1024 * 1024);
    }

    #[test]
    fn test_azure_block_size_constant() {
        assert_eq!(AZURE_BLOCK_SIZE, 4 * 1024 * 1024);
    }

    #[test]
    fn test_chunk_size_alignment() {
        assert_eq!(CHUNK_SIZE % AZURE_BLOCK_SIZE, 0);
    }

    #[test]
    fn conn_field_extracts_named_fields() {
        let conn = "DefaultEndpointsProtocol=http;AccountName=acct;AccountKey=KEY==;\
                    BlobEndpoint=http://127.0.0.1:10000/acct;";
        assert_eq!(conn_field(conn, "AccountName").as_deref(), Some("acct"));
        assert_eq!(conn_field(conn, "AccountKey").as_deref(), Some("KEY=="));
        assert_eq!(
            conn_field(conn, "BlobEndpoint").as_deref(),
            Some("http://127.0.0.1:10000/acct")
        );
        assert_eq!(conn_field(conn, "Missing"), None);
    }

    /// Not-found classification is load-bearing: `exists()` and the dedup
    /// paths branch on it, and it used to come from substring-matching the
    /// SDK's message.
    #[test]
    fn map_error_classifies_not_found_and_permission_denied() {
        let nf = opendal::Error::new(opendal::ErrorKind::NotFound, "blob missing");
        assert!(matches!(map_error(&nf, "get k"), StorageError::NotFound(_)));

        let pd = opendal::Error::new(opendal::ErrorKind::PermissionDenied, "nope");
        assert!(matches!(
            map_error(&pd, "get k"),
            StorageError::PermissionDenied(_)
        ));

        let other = opendal::Error::new(opendal::ErrorKind::Unexpected, "boom");
        assert!(matches!(
            map_error(&other, "get k"),
            StorageError::Backend(_)
        ));
    }
}

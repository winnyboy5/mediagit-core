// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

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
use futures::StreamExt;
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
///
/// `TimeoutLayer`'s `io_timeout` defaults to **10 s**, and it is a *per-IO*
/// deadline, not a whole-operation one. That is reasonable on a LAN and wrong
/// on a WAN: campaign 20260804-azgcs failed an Azure push outright with
/// `Unexpected (temporary) at write, context: { timeout: 10 } => io operation
/// timeout reached`, pushing 2 GB of chunks at ~1.8 MB/s. Under
/// `MEDIAGIT_AZURE_PUT_BLOCK_CONCURRENCY` (default 8) concurrent block writes,
/// each write gets a fraction of an already-slow link and a single one
/// comfortably exceeds 10 s. The server turned that into a 500, the client's
/// retry hit the same wall, and the whole push exited 1.
///
/// Only the IO deadline is raised. The non-IO timeout (stat/delete/list) keeps
/// OpenDAL's default, because those are small round-trips where a long hang is
/// a real fault worth surfacing quickly, not a slow transfer.
///
/// Layer order is OpenDAL's own documented production order — retry inner,
/// timeout outer. Do NOT reorder it to match the Go binding's guidance
/// ("timeout before retry"); the Rust docs specify this order, and the two
/// bindings differ.
fn with_layers(op: Operator) -> Operator {
    op.layer(RetryLayer::new())
        .layer(TimeoutLayer::new().with_io_timeout(Duration::from_secs(azure_io_timeout_secs())))
}

/// Per-IO deadline for Azure transfers, in seconds.
///
/// Generous by default because the failure mode it prevents is a failed push
/// of an entire repository, while the cost of being too generous is a slow
/// operation taking longer to report a genuine hang.
fn azure_io_timeout_secs() -> u64 {
    parse_io_timeout_secs(std::env::var("MEDIAGIT_AZURE_IO_TIMEOUT_SECS").ok())
}

/// Split from the env lookup so it is assertable: `#![forbid(unsafe_code)]` plus
/// edition 2024 make `set_var` an `unsafe` call, so a test that drove the real
/// variable could not be written without punching a hole in that. Same reason
/// `clamp_cap` in mediagit-versioning takes an `Option<String>`.
fn parse_io_timeout_secs(raw: Option<String>) -> u64 {
    const DEFAULT_IO_TIMEOUT_SECS: u64 = 120;
    raw.and_then(|v| v.trim().parse::<u64>().ok())
        // 0 means "deadline already passed" to OpenDAL, not "no timeout".
        // Accepting it would fail every transfer instantly, so it falls back.
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_IO_TIMEOUT_SECS)
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
                "Azure container '{}' is not reachable and cannot be created with SAS authentication. Create it first, or configure account-key auth.",
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
            "Azure container '{}' does not exist and could not be created ({} {}). Create it manually and retry.",
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

    /// Open an incremental byte stream over `full` for `range`, via OpenDAL's
    /// `Reader::into_bytes_stream`. Shared by `get_streaming` (`..`) and
    /// `get_streaming_range` (a bounded range) so neither materializes the
    /// full range into memory before streaming — unlike the old
    /// `read_with(...).await` + `stream::once` shim this replaces.
    async fn read_stream(
        &self,
        full: &str,
        range: impl std::ops::RangeBounds<u64> + Send + 'static,
        context: String,
    ) -> anyhow::Result<
        std::pin::Pin<
            Box<dyn futures::Stream<Item = anyhow::Result<bytes::Bytes>> + Send + 'static>,
        >,
    > {
        let reader = self
            .op
            .reader(full)
            .await
            .map_err(|e| map_error(&e, &context))?;
        let stream = reader
            .into_bytes_stream(range)
            .await
            .map_err(|e| map_error(&e, &context))?
            .map(move |r| r.map_err(|e| anyhow::anyhow!("{context} stream error: {e}")));
        Ok(Box::pin(stream))
    }
}

/// Block size for an Azure block-blob upload.
///
/// Azure's limits differ from S3's: up to 50,000 blocks per blob and 4000 MiB
/// per block, so the binding constraint is far looser. The 16 MiB floor is kept
/// anyway for the same reason as the other backends -- media-sized objects
/// should not become thousands of small round trips -- and so a pack behaves
/// the same shape everywhere.
fn block_size_azure(total_size: u64) -> u64 {
    const MAX_BLOCKS: u64 = 50_000;
    const MAX_BLOCK: u64 = 4000 * 1024 * 1024;
    const TARGET_PARTS: u64 = 96;
    const FLOOR: u64 = 16 * 1024 * 1024;

    if let Some(v) = std::env::var("MEDIAGIT_MPU_PART_SIZE")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|&n| n > 0 && n <= MAX_BLOCK)
    {
        return v;
    }
    let by_count = total_size.div_ceil(MAX_BLOCKS).max(1);
    let by_target = total_size.div_ceil(TARGET_PARTS).max(1);
    FLOOR.max(by_count).max(by_target).min(MAX_BLOCK)
}

/// Block id for a part.
///
/// Azure requires every block id in one blob to be the same length and
/// base64-encoded. Deriving it from the part number rather than storing it
/// means `complete_presigned_mpu` can rebuild the list without the client
/// having to send ids back -- which matters because the shared
/// `MpuCompletedPart` carries an ETag, and Azure's commit wants ids.
///
/// Deterministic ids are also what makes a retried upload safe: re-uploading
/// part N overwrites the same uncommitted block rather than adding a second
/// one. Pack keys are content-addressed, so two uploads of one key are the
/// same bytes.
fn block_id_for_part(part_number: i32) -> String {
    // 16 zero-padded digits. Azure requires a base64 string of consistent
    // length; it does not care what that decodes to. Digits are all in the
    // base64 alphabet and 16 is a multiple of 4, so this IS valid base64 --
    // and, unlike a real base64 encoding, it contains no `+`, `/` or `=`, so
    // it needs no URL-encoding when it goes into the query string.
    //
    // That is worth the two lines of explanation: it removes a base64 and a
    // urlencoding dependency from this crate for a value nothing ever decodes.
    // Verified against live Azure -- staged, committed, and read back correct.
    format!("{part_number:016}")
}

/// Append a query parameter to an already-signed URL.
///
/// Safe on Azure specifically, and NOT a pattern to copy to the other backends:
/// a service SAS signs a fixed set of FIELDS, so extra query parameters leave
/// the signature intact. AWS SigV4 and GCS V4 sign the whole canonical query
/// string, where the same move would invalidate it. Verified against live Azure
/// before this was written: a write-SAS with `comp=block&blockid=...` appended
/// returns 201 and the committed blob reads back byte-correct.
fn append_query(url: &str, extra: &str) -> String {
    if url.contains('?') {
        format!("{url}&{extra}")
    } else {
        format!("{url}?{extra}")
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

    /// Efficient ranged read via OpenDAL's HTTP Range-GET, overriding the
    /// trait default (whole-object `get` + slice).
    async fn get_range(&self, key: &str, offset: u64, len: u64) -> anyhow::Result<Vec<u8>> {
        Self::validate_key(key)?;
        let full = self.full_key(key);
        let buf = self
            .op
            .read_with(&full)
            .range(offset..offset + len)
            .await
            .map_err(|e| map_error(&e, &format!("get_range {key}")))?;
        Ok(buf.to_vec())
    }

    async fn get_streaming(
        &self,
        key: &str,
    ) -> anyhow::Result<
        std::pin::Pin<
            Box<dyn futures::Stream<Item = anyhow::Result<bytes::Bytes>> + Send + 'static>,
        >,
    > {
        Self::validate_key(key)?;
        let full = self.full_key(key);
        self.read_stream(&full, .., format!("get_streaming {key}"))
            .await
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
        self.read_stream(&full, range, format!("get_streaming_range {key}"))
            .await
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

    /// Presigned multipart upload for Azure, over **Put Block / Put Block List**.
    ///
    /// Azure has no S3-style multipart and no upload id: blocks are staged
    /// against the blob with client-chosen ids and then committed in one call.
    /// That maps onto this trait with two consequences worth stating, because
    /// neither is obvious from the S3 shape:
    ///
    /// * `upload_id` is a SENTINEL, not a server handle. Azure never issues
    ///   one. Block ids are derived from the part number instead, so `complete`
    ///   can rebuild the list without it.
    /// * The client's reported ETags are unused here. Azure's commit takes
    ///   block ids; the part NUMBERS are what carry the information.
    ///
    /// Without this, every Azure pack upload was one all-or-nothing PUT with
    /// the whole 64 MiB pack held in memory to make retry possible -- measured
    /// at 1,075.2 MB peak client working set on a 10.03 GB push, against
    /// 365.2 MB on S3. See FUTURE_TODOS items 29 and 33.
    async fn create_presigned_mpu(
        &self,
        key: &str,
        total_size: u64,
        ttl: Duration,
    ) -> anyhow::Result<Option<crate::PresignedMpu>> {
        if std::env::var_os("MEDIAGIT_AZURE_DISABLE_MPU").is_some() {
            return Ok(None);
        }
        Self::validate_key(key)?;
        let full = self.full_key(key);

        let part_size = block_size_azure(total_size);
        let num_parts = total_size.div_ceil(part_size).max(1) as i32;

        let mut parts = Vec::with_capacity(num_parts as usize);
        for part_number in 1..=num_parts {
            // Presign failure is never fatal: Ok(None) sends the caller to the
            // single PUT, which still works. Failing here would abandon a push
            // that had a usable path.
            let req = match self.op.presign_write(&full, ttl).await {
                Ok(r) => r,
                Err(e) => {
                    tracing::debug!(err = %e, "Azure presign_write unavailable; declining MPU");
                    return Ok(None);
                }
            };
            let block_id = block_id_for_part(part_number);
            let url = append_query(
                &req.uri().to_string(),
                &format!("comp=block&blockid={block_id}"),
            );
            parts.push(crate::PresignedMpuPart { part_number, url });
        }

        tracing::debug!(
            key = %full, parts = num_parts, part_size,
            "Azure block-blob staged upload created"
        );
        Ok(Some(crate::PresignedMpu {
            // Not a server handle -- see the doc comment. Named so it is
            // recognisable in a log rather than looking like a lost id.
            upload_id: "azure-blocklist".to_string(),
            parts,
            part_size,
            // UNATTESTED, AND THIS IS A DEAD END — investigated 2026-09-17,
            // recorded so nobody re-attempts it.
            //
            // Attestation needs a digest the SERVICE computed over the stored
            // object, so the server can compare it against what the client says
            // it sent. S3 validates a full-object CRC at CompleteMultipartUpload;
            // GCS computes a crc32c for every object which we compare at
            // complete. Azure offers neither for a block blob assembled by
            // `Put Block List`:
            //   - `x-ms-blob-content-md5` is CLIENT-SET metadata, stored and
            //     returned verbatim, never validated against the bytes.
            //   - per-block `x-ms-content-crc64` IS validated, but only per
            //     block at upload time, and the client cannot attach it here —
            //     blocks go up through PRESIGNED urls, which sign a fixed header
            //     set (measured on S3: `AccessDenied` / `HeadersNotSigned`).
            //   - there is no service-computed whole-blob hash to read back.
            //
            // A client-asserted CRC stored as metadata would NOT be equivalent:
            // it proves nothing a faulty or hostile client could not fake, and
            // checking it means reading the bytes — which is the read-back this
            // was meant to remove.
            //
            // So Azure keeps the pack read-back. That is current behaviour, not
            // a regression, and it is the correct answer until Azure exposes a
            // server-side whole-blob digest.
            checksum: None,
        }))
    }

    async fn complete_presigned_mpu(
        &self,
        key: &str,
        _upload_id: &str,
        parts: Vec<crate::MpuCompletedPart>,
    ) -> anyhow::Result<()> {
        Self::validate_key(key)?;
        let full = self.full_key(key);

        // Commit order is the blob's byte order, so the list is sorted by part
        // number rather than trusting the order the client reported completions
        // in -- those arrive from concurrent uploads.
        let mut numbers: Vec<i32> = parts.iter().map(|p| p.part_number).collect();
        numbers.sort_unstable();
        let mut xml = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?><BlockList>");
        for n in numbers {
            xml.push_str(&format!("<Latest>{}</Latest>", block_id_for_part(n)));
        }
        xml.push_str("</BlockList>");

        let req = self
            .op
            .presign_write(&full, Duration::from_secs(3600))
            .await
            .map_err(|e| anyhow::anyhow!("Azure presign blocklist: {e}"))?;
        let url = append_query(&req.uri().to_string(), "comp=blocklist");

        let resp = reqwest::Client::new()
            .put(&url)
            .header(reqwest::header::CONTENT_TYPE, "application/xml")
            .body(xml)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Azure put block list: {e}"))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Azure put block list returned {status}: {}", body.trim());
        }
        Ok(())
    }

    async fn abort_presigned_mpu(&self, _key: &str, _upload_id: &str) -> anyhow::Result<()> {
        // Azure has no abort, and needs none: blocks that are staged but never
        // committed are invisible to readers and the service garbage-collects
        // them after a week. There is deliberately no DELETE here -- deleting
        // the blob would destroy a PREVIOUS committed version of the same key,
        // which on a content-addressed store is the same object other repos are
        // already reading.
        Ok(())
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

    /// The 10s OpenDAL default failed a 2 GB Azure push on a WAN link
    /// (campaign 20260804-azgcs). Absent/garbage input must land on the
    /// generous default, not on something that reintroduces that failure.
    #[test]
    fn io_timeout_defaults_are_generous_enough_for_a_wan() {
        assert_eq!(super::parse_io_timeout_secs(None), 120);
        assert_eq!(
            super::parse_io_timeout_secs(Some("not-a-number".into())),
            120
        );
        assert!(
            super::parse_io_timeout_secs(None) > 10,
            "default must exceed the OpenDAL default that caused the failure"
        );
    }

    /// 0 means "deadline already passed" to OpenDAL, so honouring it would
    /// fail every transfer instantly. It must fall back, not be obeyed.
    #[test]
    fn io_timeout_rejects_zero_and_honours_valid_overrides() {
        assert_eq!(super::parse_io_timeout_secs(Some("0".into())), 120);
        assert_eq!(super::parse_io_timeout_secs(Some("45".into())), 45);
        assert_eq!(super::parse_io_timeout_secs(Some("  300  ".into())), 300);
    }
}

#[cfg(test)]
mod azure_block_mpu_tests {
    use super::{append_query, block_id_for_part, block_size_azure};

    /// Azure's contract: every block id in one blob must be the same length and
    /// a valid base64 string. These ids are digits only, which satisfies both
    /// and additionally needs no URL-encoding -- the property that let this
    /// crate avoid a base64 and a urlencoding dependency.
    #[test]
    fn block_ids_are_equal_length_valid_base64_and_url_safe() {
        let ids: Vec<String> = [1, 2, 99, 12_345, 50_000]
            .iter()
            .map(|n| block_id_for_part(*n))
            .collect();
        let len = ids[0].len();
        for id in &ids {
            assert_eq!(id.len(), len, "Azure rejects mixed-length block ids: {id}");
            assert_eq!(id.len() % 4, 0, "base64 length must be a multiple of 4");
            assert!(
                id.chars().all(|c| c.is_ascii_digit()),
                "id must stay URL-safe: {id}"
            );
        }
    }

    /// Ids must be distinct per part and ordered, because the commit list is
    /// built from part numbers and defines the blob's byte order.
    #[test]
    fn block_ids_are_distinct_and_ordered() {
        let a = block_id_for_part(1);
        let b = block_id_for_part(2);
        let c = block_id_for_part(10);
        assert_ne!(a, b);
        assert!(a < b && b < c, "lexical order must match numeric order");
    }

    /// Retry safety: the id depends only on the part number, so re-uploading a
    /// part overwrites the same staged block instead of adding a duplicate that
    /// would appear twice in the committed blob.
    #[test]
    fn a_part_always_gets_the_same_id() {
        assert_eq!(block_id_for_part(7), block_id_for_part(7));
    }

    /// A SAS carries its own query string, so the separator has to adapt. Doing
    /// this wrong produces a URL with two `?` that Azure rejects with a signature
    /// error -- which reads like a credentials problem, not a string-building one.
    #[test]
    fn query_is_appended_with_the_right_separator() {
        assert_eq!(
            append_query("https://x/blob?sv=2021&sig=abc", "comp=block"),
            "https://x/blob?sv=2021&sig=abc&comp=block"
        );
        assert_eq!(
            append_query("https://x/blob", "comp=block"),
            "https://x/blob?comp=block"
        );
    }

    /// Azure's ceiling is 50,000 blocks; a payload must never need more.
    #[test]
    fn block_size_keeps_the_count_under_azures_limit() {
        for total in [
            1u64,
            64 * 1024 * 1024,
            10 * 1024 * 1024 * 1024,
            4 * 1024u64.pow(4),
        ] {
            let bs = block_size_azure(total);
            assert!(bs > 0);
            assert!(
                bs <= 4000 * 1024 * 1024,
                "block {bs} over Azure's 4000 MiB maximum"
            );
            assert!(
                total.div_ceil(bs) <= 50_000,
                "total {total} needs more than 50,000 blocks at {bs}"
            );
        }
    }

    /// A 64 MiB pack is the payload this exists for: four 16 MiB blocks, the
    /// same shape the other backends use, so behaviour is comparable across them.
    #[test]
    fn a_64mib_pack_uses_four_16mib_blocks() {
        assert_eq!(block_size_azure(64 * 1024 * 1024), 16 * 1024 * 1024);
    }
}

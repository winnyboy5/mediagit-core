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

use super::*;

/// Attempts to retry a chunk GET that failed transiently, beyond the first try.
///
/// Deliberately small. The server has already exhausted its own storage retries
/// before it answers 503, so this is a second-order backstop for a
/// moment-in-time condition, not a substitute for backend resilience.
const CHUNK_GET_MAX_RETRIES: u32 = 3;

/// Is this chunk-GET outcome worth retrying?
///
/// 5xx and 429 are weather; 404/403/409 are verdicts. Retrying a verdict just
/// delays a failure the caller needs to see — and 409 specifically is
/// *meaningful* here (the chunk is delta-only and the caller re-routes to
/// `/chunk-deltas/<id>`), so retrying it would break that path.
fn chunk_get_is_transient(status: reqwest::StatusCode) -> bool {
    status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS
}

/// One place to turn a failed chunk GET into an error.
///
/// 503 earns its own text: it means the *server* could not reach its storage
/// backend, which is an operator problem, and "failed with status: 503" gives
/// no hint of that. This lived only on `download_chunk` (the sequential path)
/// while clone runs the parallel path below, so the actionable message was
/// unreachable in exactly the case it was written for — campaign
/// 20260804-sigfix hit it and reported the generic text.
fn chunk_get_error(chunk_id: &Oid, status: reqwest::StatusCode) -> anyhow::Error {
    if status == reqwest::StatusCode::SERVICE_UNAVAILABLE {
        anyhow::anyhow!(
            "GET /chunks/{} failed: server storage backend unreachable (503) — verify the storage service (MinIO/S3/Azure) is running and accessible to the server",
            chunk_id
        )
    } else {
        anyhow::anyhow!("GET /chunks/{} failed with status: {}", chunk_id, status)
    }
}

/// Bounded retry for the proxy chunk-GET fallback.
///
/// Without it, ONE transient failure aborts an entire multi-GB clone: campaign
/// 20260804-sigfix lost a 2 GB AWS clone to a single chunk whose GET returned
/// 503 after the server had already spent 137s on its own retries. Retrying a
/// handful of times costs seconds; not retrying costs the whole transfer and
/// every byte already downloaded.
///
/// Transport errors are retried alongside 5xx/429 — a dropped connection
/// mid-clone is the same class of weather on a WAN-bound product.
/// `MEDIAGIT_PULL_DEADLINE_SECS` still bounds the whole download, so this
/// cannot stall a clone forever.
async fn get_chunk_with_retry(
    client: &reqwest::Client,
    url: &str,
    chunk_id: &Oid,
) -> anyhow::Result<reqwest::Response> {
    let hex = chunk_id.to_hex();
    let mut attempt = 0u32;
    loop {
        let outcome = client.get(url).send().await;
        let retryable = match &outcome {
            Ok(r) => chunk_get_is_transient(r.status()),
            Err(_) => true,
        };
        // A 429 and a 503 are not the same kind of failure and must not share
        // a budget. 503 means the server already exhausted its own storage
        // retries, so trying many more times is just delaying a real error --
        // hence the deliberately small CHUNK_GET_MAX_RETRIES. A 429 means the
        // server is healthy and asking us to slow down; the correct response is
        // to wait it out, and giving up after 3 fails a clone that only needed
        // patience. Measured: with the push path fixed, a clone against a 2 rps
        // server still failed here alone.
        let rate_limited =
            matches!(&outcome, Ok(r) if r.status() == reqwest::StatusCode::TOO_MANY_REQUESTS);
        let budget = if rate_limited {
            super::rate_limit_max_retries()
        } else {
            CHUNK_GET_MAX_RETRIES
        };
        if retryable && attempt < budget {
            // Deliberately a floor plus jitter, not the shared Full Jitter
            // helper on its own. This loop retries 5xx and transport errors,
            // not just 429s -- it exists for a storage backend that is
            // already struggling, where jitter that can round down to ~0ms
            // would retry *harder* than the flat 500/1000/2000ms it replaces.
            // The floor keeps the old pacing; the jitter stops every
            // concurrent chunk GET in a clone from retrying in lockstep,
            // which is what turned one slow backend into a thundering herd.
            // Rate limited: use the shared backoff, which honours the
            // server's Retry-After. The 500ms<<attempt floor below is for a
            // STRUGGLING BACKEND (the 20260804-sigfix incident) and would
            // needlessly slow a limiter that is merely pacing us.
            let backoff_ms = if rate_limited {
                super::rate_limit_backoff(
                    attempt,
                    outcome
                        .as_ref()
                        .ok()
                        .and_then(|r| r.headers().get(reqwest::header::RETRY_AFTER)),
                )
                .as_millis() as u64
            } else {
                let floor_ms = 500u64 << attempt;
                floor_ms + super::rate_limit_backoff(attempt, None).as_millis() as u64
            };
            attempt += 1;
            tracing::warn!(
                chunk = %hex,
                attempt,
                backoff_ms,
                outcome = %match &outcome {
                    Ok(r) => r.status().to_string(),
                    Err(e) => e.to_string(),
                },
                "chunk GET failed transiently; retrying"
            );
            tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
            continue;
        }
        return outcome
            .map_err(|e| anyhow::anyhow!("Failed to download chunk {}: {}", chunk_id, e));
    }
}

impl ProtocolClient {
    /// Pull objects from remote and return pack data with chunked object OIDs
    ///
    /// # Arguments
    /// * `odb` - Local object database
    /// * `remote_ref` - Remote ref to pull
    /// * `local_oids` - List of OIDs we already have locally (for incremental pull)
    ///
    /// Pass local commit OIDs to avoid downloading objects we already have.
    /// If empty, all objects reachable from the remote ref will be downloaded.
    ///
    /// Returns (pack_data, chunked_oids)
    #[deprecated(note = "Use pull_streaming() instead for memory-efficient downloads")]
    #[allow(deprecated)]
    pub async fn pull_with_have(
        &self,
        _odb: &ObjectDatabase,
        remote_ref: &str,
        local_oids: Vec<String>,
    ) -> Result<(Vec<u8>, Vec<Oid>)> {
        // Get remote refs
        let remote_refs = self.get_refs().await?;

        // Find the ref we want
        let ref_info = remote_refs
            .refs
            .iter()
            .find(|r| r.name == remote_ref)
            .ok_or_else(|| anyhow::anyhow!("Remote ref '{}' not found", remote_ref))?;

        // Request objects we don't have
        let want = vec![ref_info.oid.clone()];
        let have = local_oids; // OIDs we already have locally

        self.download_pack(want, have).await
    }
    /// Pull objects from a remote ref (backwards compatible, downloads all objects)
    ///
    /// For incremental pulls, use `pull_streaming` instead.
    #[deprecated(note = "Use pull_streaming() instead for memory-efficient downloads")]
    #[allow(deprecated)]
    pub async fn pull(
        &self,
        odb: &ObjectDatabase,
        remote_ref: &str,
    ) -> Result<(Vec<u8>, Vec<Oid>)> {
        self.pull_with_have(odb, remote_ref, Vec::new()).await
    }
    /// Download a pack file from the server
    ///
    /// Returns (pack_data, chunked_oids) - chunked objects need separate transfer
    #[deprecated(note = "Use download_pack_streaming() instead for memory-efficient downloads")]
    pub async fn download_pack(
        &self,
        want: Vec<String>,
        have: Vec<String>,
    ) -> Result<(Vec<u8>, Vec<Oid>)> {
        // First, send want request
        let want_url = format!("{}/objects/want", self.base_url);
        tracing::debug!("POST {}", want_url);

        let want_req = WantRequest { want, have };

        let response = crate::client::send_with_rate_limit_retry(|| {
            self.client.post(&want_url).json(&want_req).send()
        })
        .await
        .context("Failed to send want request")?;

        if !response.status().is_success() {
            anyhow::bail!(
                "POST /objects/want failed with status: {}",
                response.status()
            );
        }

        // Parse the response to get the request_id (required for GET /objects/pack)
        let want_response: WantResponse = response
            .json()
            .await
            .context("Failed to parse want response")?;

        // Then download the pack with the request_id header
        let pack_url = format!("{}/objects/pack", self.base_url);
        tracing::debug!(
            "GET {} (request_id: {})",
            pack_url,
            want_response.request_id
        );

        // Wrapped despite being a streaming download. An earlier pass skipped
        // both `/objects/pack` GETs as "streaming, higher risk" -- wrong call:
        // this is the FIRST server-bound request a clone makes after the want
        // exchange, so an unretried 429 here kills the clone outright before a
        // single byte moves ("GET /objects/pack failed (429 Too Many Requests):
        // Wait for 0s", observed against a 2 rps server, dead in 2.3s).
        // Retrying is safe: the wrapper only re-sends on 429, which the limiter
        // returns before the handler runs, so no body has been consumed.
        let response = crate::client::send_with_rate_limit_retry(|| {
            self.client
                .get(&pack_url)
                .header("X-Request-ID", &want_response.request_id)
                .send()
        })
        .await
        .context("Failed to download pack file")?;

        if !response.status().is_success() {
            // Include the server's message. An incomplete-closure refusal is
            // operator-actionable ("run fsck on the server"), and a bare
            // status code strands the user with "500 Internal Server Error"
            // for a condition the server can name precisely.
            let status = response.status();
            let detail = response.text().await.unwrap_or_default();
            let detail = detail.trim();
            if detail.is_empty() {
                anyhow::bail!("GET /objects/pack failed with status: {}", status);
            }
            anyhow::bail!("GET /objects/pack failed ({}): {}", status, detail);
        }

        // Parse X-Chunked-Objects header for large files that need separate transfer
        let chunked_oids: Vec<Oid> = response
            .headers()
            .get("X-Chunked-Objects")
            .and_then(|h| h.to_str().ok())
            .map(|s| {
                s.split(',')
                    .filter_map(|oid_str| Oid::from_hex(oid_str.trim()).ok())
                    .collect()
            })
            .unwrap_or_default();

        if !chunked_oids.is_empty() {
            tracing::info!(
                count = chunked_oids.len(),
                "Received {} chunked objects for separate download",
                chunked_oids.len()
            );
        }

        let pack_data = response.bytes().await.context("Failed to read pack data")?;

        Ok((pack_data.to_vec(), chunked_oids))
    }

    /// Download pack using streaming (memory-efficient for large files)
    ///
    /// This method processes the pack incrementally without loading it entirely into memory.
    /// Objects are written directly to the ODB as they're received.
    ///
    /// Returns the list of chunked objects that need separate transfer.
    pub async fn download_pack_streaming(
        &self,
        odb: &ObjectDatabase,
        want: Vec<String>,
        have: Vec<String>,
    ) -> Result<Vec<Oid>> {
        // OP-2: bound the whole exchange, not just the initial requests. The
        // hang that mattered was mid-stream — a backend that answers the GET
        // and then stops sending leaves `next_object()` awaiting forever.
        // Wrapping the inner body covers every await inside it, including that
        // loop, for the cost of one indirection.
        super::with_pull_deadline(
            "download",
            self.download_pack_streaming_inner(odb, want, have),
        )
        .await
    }

    async fn download_pack_streaming_inner(
        &self,
        odb: &ObjectDatabase,
        want: Vec<String>,
        have: Vec<String>,
    ) -> Result<Vec<Oid>> {
        // Send want request
        let want_url = format!("{}/objects/want", self.base_url);
        tracing::debug!("POST {} (streaming)", want_url);

        // Retained for the post-transfer check below; both move into the
        // request.
        let requested: Vec<String> = want.clone();
        let declared_have: std::collections::HashSet<String> = have.iter().cloned().collect();
        let want_req = WantRequest { want, have };

        let response = crate::client::send_with_rate_limit_retry(|| {
            self.client.post(&want_url).json(&want_req).send()
        })
        .await
        .context("Failed to send want request")?;

        if !response.status().is_success() {
            anyhow::bail!(
                "POST /objects/want failed with status: {}",
                response.status()
            );
        }

        let want_response: WantResponse = response
            .json()
            .await
            .context("Failed to parse want response")?;

        // Download pack with streaming
        let pack_url = format!("{}/objects/pack", self.base_url);
        tracing::debug!(
            "GET {} (streaming, request_id: {})",
            pack_url,
            want_response.request_id
        );

        // Wrapped despite being a streaming download. An earlier pass skipped
        // both `/objects/pack` GETs as "streaming, higher risk" -- wrong call:
        // this is the FIRST server-bound request a clone makes after the want
        // exchange, so an unretried 429 here kills the clone outright before a
        // single byte moves ("GET /objects/pack failed (429 Too Many Requests):
        // Wait for 0s", observed against a 2 rps server, dead in 2.3s).
        // Retrying is safe: the wrapper only re-sends on 429, which the limiter
        // returns before the handler runs, so no body has been consumed.
        let response = crate::client::send_with_rate_limit_retry(|| {
            self.client
                .get(&pack_url)
                .header("X-Request-ID", &want_response.request_id)
                .send()
        })
        .await
        .context("Failed to download pack file")?;

        if !response.status().is_success() {
            // Include the server's message. An incomplete-closure refusal is
            // operator-actionable ("run fsck on the server"), and a bare
            // status code strands the user with "500 Internal Server Error"
            // for a condition the server can name precisely.
            let status = response.status();
            let detail = response.text().await.unwrap_or_default();
            let detail = detail.trim();
            if detail.is_empty() {
                anyhow::bail!("GET /objects/pack failed with status: {}", status);
            }
            anyhow::bail!("GET /objects/pack failed ({}): {}", status, detail);
        }

        // Parse X-Chunked-Objects header
        let chunked_oids: Vec<Oid> = response
            .headers()
            .get("X-Chunked-Objects")
            .and_then(|h| h.to_str().ok())
            .map(|s| {
                s.split(',')
                    .filter_map(|oid_str| Oid::from_hex(oid_str.trim()).ok())
                    .collect()
            })
            .unwrap_or_default();

        if !chunked_oids.is_empty() {
            tracing::info!(
                count = chunked_oids.len(),
                "Received {} chunked objects for separate download",
                chunked_oids.len()
            );
        }

        // Stream response body and write objects via ODB (ensures proper compression)
        use futures::stream::TryStreamExt;
        use tokio_util::io::StreamReader;

        let stream = response.bytes_stream().map_err(std::io::Error::other);

        let stream_reader = StreamReader::new(stream);

        let mut reader = mediagit_versioning::StreamingPackReader::new(stream_reader)
            .await
            .context("Failed to create streaming pack reader")?;

        tracing::info!("Processing streaming pack download");

        let mut object_count = 0;
        while let Some(result) = reader.next_object().await {
            let (_oid, obj_type, data) =
                result.context("Failed to read object from pack stream")?;

            // Write through ODB to ensure proper compression and storage format.
            // PackTransaction bypassed compression, causing read failures.
            odb.write(obj_type, &data)
                .await
                .context("Failed to write object from pack")?;

            object_count += 1;
            if object_count % 100 == 0 {
                tracing::debug!("Downloaded {} objects", object_count);
            }
        }

        // Verify the requested roots actually arrived.
        //
        // Nothing else does: the pack header declares an object count and this
        // loop stops there, so a server that omits objects produces a stream
        // that reads as complete.
        //
        // Scope is the requested roots only — O(wants), no walk. Verified by
        // experiment, this does **not** catch a missing deep object (the
        // partial-clone bug fixed server-side dropped a blob two levels below
        // the want, and this check passes on it). It catches the narrower case
        // of a root the client asked for and did not receive. Full-closure
        // verification would need a local walk plus the separately-transferred
        // chunked objects; `fsck` is the tool for that.
        let mut missing = Vec::new();
        for hex in &requested {
            // An object this client declared as `have` is one the server is
            // *supposed* to prune, so its absence says nothing about the
            // server. Whether the client really had it is its own
            // bookkeeping; treating that as a transfer failure would blame
            // the wrong side.
            if declared_have.contains(hex) {
                continue;
            }
            let Ok(oid) = Oid::from_hex(hex) else {
                continue;
            };
            // Chunked blobs travel separately, by design — absence here is
            // expected, not a defect.
            if chunked_oids.contains(&oid) {
                continue;
            }
            if odb.read(&oid).await.is_err() {
                missing.push(oid);
            }
        }
        if !missing.is_empty() {
            anyhow::bail!(
                "transfer incomplete: the server did not deliver {} requested object(s), \
                    first missing {}. The local repository is not usable for these objects; \
                    re-run the operation, and run `mediagit fsck` on the server if it persists.",
                missing.len(),
                missing[0]
            );
        }

        tracing::info!(
            "Successfully downloaded {} objects (streaming)",
            object_count
        );

        Ok(chunked_oids)
    }

    /// Pull using streaming (memory-efficient)
    ///
    /// Returns the list of chunked objects that need separate transfer.
    pub async fn pull_streaming(
        &self,
        odb: &ObjectDatabase,
        remote_ref: &str,
        local_oids: Vec<String>,
    ) -> Result<Vec<Oid>> {
        // Get remote refs
        let remote_refs = self.get_refs().await?;

        // Find the ref we want
        let ref_info = remote_refs
            .refs
            .iter()
            .find(|r| r.name == remote_ref)
            .ok_or_else(|| anyhow::anyhow!("Remote ref '{}' not found", remote_ref))?;

        // Request objects we don't have
        let want = vec![ref_info.oid.clone()];
        let have = local_oids;

        self.download_pack_streaming(odb, want, have).await
    }
    /// Download a manifest from the remote server
    pub async fn download_manifest(&self, oid: &Oid) -> Result<ChunkManifest> {
        let url = format!("{}/manifests/{}", self.base_url, oid.to_hex());

        let response = crate::client::send_with_rate_limit_retry(|| self.client.get(&url).send())
            .await
            .context(format!("Failed to GET /manifests/{}", oid))?;

        if !response.status().is_success() {
            anyhow::bail!(
                "GET /manifests/{} failed with status: {}",
                oid,
                response.status()
            );
        }

        let data = response.bytes().await?;
        mediagit_versioning::ChunkManifest::from_bytes(&data)
            .context("Failed to deserialize manifest")
    }
    /// Download a single chunk from the remote server
    pub async fn download_chunk(&self, chunk_id: &Oid) -> Result<Vec<u8>> {
        let url = format!("{}/chunks/{}", self.base_url, chunk_id.to_hex());

        let response = crate::client::send_with_rate_limit_retry(|| self.client.get(&url).send())
            .await
            .context(format!("Failed to GET /chunks/{}", chunk_id))?;

        if !response.status().is_success() {
            return Err(chunk_get_error(chunk_id, response.status()));
        }

        Ok(response.bytes().await?.to_vec())
    }
    /// Download all chunks for chunked objects with parallel downloads
    ///
    /// Uses 8 concurrent downloads for optimal throughput (>100MB/s target).
    /// Progress callback receives `(chunks_done, total_manifest_chunks, msg)` for
    /// smooth chunk-level ETA (avoids "211y" caused by object-level reporting).
    ///
    /// Delta-aware: for each manifest, asks the server which chunks exist as
    /// chunk-deltas via `POST /chunk-deltas/check`. Chunks present as deltas
    /// are downloaded via `GET /chunk-deltas/<id>` and persisted via
    /// `odb.write_chunk_delta`, preserving the storage savings the server has.
    /// Falls back to full-chunk downloads for any chunk the server doesn't
    /// report as a delta, and for old servers that don't expose the endpoint.
    pub async fn download_chunked_objects<F>(
        &self,
        odb: &ObjectDatabase,
        chunked_oids: &[Oid],
        on_progress: F,
    ) -> Result<(usize, u64)>
    where
        F: FnMut(u64, u64, &str),
    {
        // OP-2: chunked blobs travel *after* the pack, in a separate phase.
        // Deadlining only the pack would leave the phase that moves the actual
        // media bytes — the long one — able to hang forever.
        super::with_pull_deadline(
            "chunk download",
            self.download_chunked_objects_inner(odb, chunked_oids, on_progress),
        )
        .await
    }

    async fn download_chunked_objects_inner<F>(
        &self,
        odb: &ObjectDatabase,
        chunked_oids: &[Oid],
        mut on_progress: F,
    ) -> Result<(usize, u64)>
    where
        F: FnMut(u64, u64, &str),
    {
        use futures::stream::StreamExt;

        if chunked_oids.is_empty() {
            return Ok((0, 0));
        }

        // Concurrency for parallel chunk downloads. Default 24 paired with
        // MEDIAGIT_RANGE_PARALLEL=4 gives 96 effective TCP streams — enough
        // headroom without opening more sockets than the pool can keep warm.
        // Env var > builder > internal default.
        //
        // The comment above says 24; the code below uses 32, and the pack
        // range-GET path (`packs.rs`) uses 24 for the same env var. The split is
        // left as-is on purpose — changing a concurrency default is a
        // performance change that needs measurement, not a comment tidy-up — but
        // note the prose and the constant disagree here, so trust the constant.
        let concurrent_downloads: usize = std::env::var("MEDIAGIT_DOWNLOAD_CONCURRENCY")
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|n: &usize| *n > 0)
            .or(self.concurrent_downloads)
            .unwrap_or(32);
        let n_objects = chunked_oids.len();
        let _download_bench = crate::bench::maybe_start("download", concurrent_downloads);

        // Gate: set MEDIAGIT_DOWNLOAD_DIRECT_DISABLE=1 to bypass presigned GET entirely.
        let direct_download_enabled =
            std::env::var("MEDIAGIT_DOWNLOAD_DIRECT_DISABLE").as_deref() != Ok("1");

        // Data-plane client for presigned GET downloads.
        // HTTP/1.1: parallel TCP sockets beat h2 multiplexing for large bodies.
        // Pool size via MEDIAGIT_HTTP_POOL_MAX (see http_pool_max()).
        // NO total .timeout() on purpose: a downloaded object has no bounded
        // size, and a five-minute ceiling would kill legitimate large transfers.
        // The read timeout in the shared builder is what bounds a stall here —
        // previously nothing did, leaving one silent GET to hold the clone until
        // the absolute 3600s MEDIAGIT_PULL_DEADLINE_SECS.
        let direct_client = super::data_plane_client_builder()
            .build()
            // Fallback must be credential-free: self.client carries auth
            // default_headers, which must never reach presigned URLs.
            .unwrap_or_else(|_| reqwest::Client::new());

        // ── Phase 1: manifests (fast — small metadata payloads) ──────────────
        // Download all manifests upfront to know total_chunks before any data
        // transfer. Manifests are tiny (list of chunk IDs + sizes); storing them
        // all is at most ~KB total even for large repos.
        struct ObjectWork {
            oid: Oid,
            manifest: ChunkManifest,
            missing_chunks: Vec<Oid>,
        }

        // Knobs: set MEDIAGIT_PULL_PIPELINE=0 to disable concurrent manifest fetch.
        // MEDIAGIT_PULL_MANIFEST_CONCURRENCY controls max in-flight manifest requests (default 8).
        let pull_pipeline = std::env::var("MEDIAGIT_PULL_PIPELINE")
            .as_deref()
            .unwrap_or("1")
            != "0";
        let manifest_concurrency: usize = std::env::var("MEDIAGIT_PULL_MANIFEST_CONCURRENCY")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(8)
            .max(1);

        let manifest_start = std::time::Instant::now();

        let object_work: Vec<ObjectWork> = if pull_pipeline && n_objects > 1 {
            // Concurrent manifest fetch + chunk existence check.
            // buffer_unordered only requires Send (not 'static), so &ObjectDatabase
            // is safe to capture as long as ObjectDatabase: Sync.
            let http_client = self.client.clone();
            let base_url = self.base_url.clone();
            use futures::StreamExt;
            let results: Vec<Result<ObjectWork>> = futures::stream::iter(chunked_oids.iter())
                .map(|oid| {
                    let http_client = http_client.clone();
                    let base_url = base_url.clone();
                    async move {
                        // 1. Fetch manifest (HTTP — inlined download_manifest body)
                        let url = format!("{}/manifests/{}", base_url, oid.to_hex());
                        // Wrapped like every other server-bound call. The
                        // PARALLEL fan-out was missed when the sequential
                        // manifest fetch was wrapped, so a clone still died on
                        // an unretried 429 here - one unwrapped site in a fan-out
                        // is enough to fail the whole clone.
                        let resp = crate::client::send_with_rate_limit_retry(|| {
                            http_client.get(&url).send()
                        })
                        .await
                        .with_context(|| format!("Failed to GET /manifests/{}", oid))?;
                        if !resp.status().is_success() {
                            anyhow::bail!(
                                "GET /manifests/{} failed with status: {}",
                                oid,
                                resp.status()
                            );
                        }
                        let data = resp.bytes().await?;
                        let manifest = ChunkManifest::from_bytes(&data)
                            .context("Failed to deserialize manifest")?;

                        // 2. Check which chunks are already local (parallel filesystem stats).
                        let obj_total = manifest.chunks.len();
                        let missing: Vec<Oid> =
                            futures::stream::iter(manifest.chunks.iter().map(|c| c.id))
                                .map(|id| async move {
                                    if odb.chunk_exists(&id).await.unwrap_or(false) {
                                        None
                                    } else {
                                        Some(id)
                                    }
                                })
                                .buffer_unordered(64)
                                .filter_map(|x| async move { x })
                                .collect()
                                .await;

                        if missing.is_empty() {
                            tracing::debug!(oid = %oid, "All chunks already exist locally");
                        } else {
                            tracing::info!(
                                oid = %oid,
                                missing = missing.len(),
                                total = obj_total,
                                "Downloading missing chunks"
                            );
                        }

                        Ok::<ObjectWork, anyhow::Error>(ObjectWork {
                            oid: *oid,
                            manifest,
                            missing_chunks: missing,
                        })
                    }
                })
                .buffer_unordered(manifest_concurrency)
                .collect()
                .await;
            results.into_iter().collect::<Result<Vec<ObjectWork>>>()?
        } else {
            // Sequential fallback: MEDIAGIT_PULL_PIPELINE=0 or single object.
            let mut work = Vec::with_capacity(n_objects);
            for oid in chunked_oids.iter() {
                let manifest = self.download_manifest(oid).await?;
                let obj_total = manifest.chunks.len();
                // Check which chunks are already local — run in parallel to avoid
                // O(n) sequential filesystem stats on Windows (3ms × 4000 = 12s per object).
                let missing: Vec<Oid> = futures::stream::iter(manifest.chunks.iter().map(|c| c.id))
                    .map(|id| async move {
                        if odb.chunk_exists(&id).await.unwrap_or(false) {
                            None
                        } else {
                            Some(id)
                        }
                    })
                    .buffer_unordered(64)
                    .filter_map(|x| async move { x })
                    .collect()
                    .await;
                if missing.is_empty() {
                    tracing::debug!(oid = %oid, "All chunks already exist locally");
                } else {
                    tracing::info!(
                        oid = %oid,
                        missing = missing.len(),
                        total = obj_total,
                        "Downloading missing chunks"
                    );
                }
                work.push(ObjectWork {
                    oid: *oid,
                    manifest,
                    missing_chunks: missing,
                });
            }
            work
        };

        // Compute totals from collected manifests.
        let total_manifest_chunks: usize =
            object_work.iter().map(|w| w.manifest.chunks.len()).sum();
        let total_manifest_bytes: u64 = object_work
            .iter()
            .flat_map(|w| w.manifest.chunks.iter().map(|c| c.size as u64))
            .sum();

        // Signal total upfront so the progress bar initialises with correct length.
        on_progress(
            0,
            total_manifest_bytes,
            &format!(
                "{} objects, {} total chunks",
                n_objects, total_manifest_chunks
            ),
        );

        if let Some(b) = &_download_bench {
            b.record_manifest_to_first_byte(manifest_start.elapsed());
        }

        // ── Phase 2: chunk download — streaming write, O(concurrent × chunk) RAM
        // Writing each chunk as it arrives (while let Some) instead of collect()
        // means peak RAM = concurrent_downloads × max_chunk_size (≤64 KB on
        // Windows) rather than accumulating every result before any disk write.
        let mut total_chunks_downloaded = 0usize;
        // RP-2: `bytes_downloaded` was declared and displayed but never
        // assigned anywhere, so pull/clone reported no download figure at all.
        // Counted in wire bytes (what actually crossed the network) to match
        // the push side and to keep any derived rate under link capacity.
        let mut total_net_bytes = 0u64;
        let mut bytes_done: u64 = 0;

        for (
            obj_idx,
            ObjectWork {
                oid,
                manifest,
                missing_chunks,
            },
        ) in object_work.into_iter().enumerate()
        {
            let already_local = manifest.chunks.len() - missing_chunks.len();
            // Size hints for range-parallel GET (W5): chunk hex → uncompressed bytes.
            let chunk_size_map: std::sync::Arc<std::collections::HashMap<String, u64>> =
                std::sync::Arc::new(
                    manifest
                        .chunks
                        .iter()
                        .map(|c| (c.id.to_hex(), c.size as u64))
                        .collect(),
                );

            if !missing_chunks.is_empty() {
                // Ask the server which of these chunks are stored as deltas.
                // Best-effort: empty map on old servers / errors.
                let delta_map = self.check_chunk_deltas(&missing_chunks).await;

                // Split into "full chunks" and "delta chunks" based on what the
                // server actually stores. A chunk that the server reports as a
                // delta MUST be downloaded via `/chunk-deltas/<id>` — hitting
                // `/chunks/<id>` for it would 404 because no full copy exists.
                // Delta chains (depth ≥ 2) are fine: all chunks in the chain
                // land locally as deltas in this single pass; reads later
                // follow the chain via the .meta sidecars.
                //
                // Cycle pre-filter still applies: if writing a chunk as a delta
                // would create a cycle with the LOCAL on-disk chain (e.g. a
                // prior partial clone), degrade that one to a full download.
                // If the full endpoint also lacks the object the server is
                // misconfigured — surface the error rather than mask it.
                let mut full_chunks: Vec<Oid> = Vec::new();
                let mut delta_chunks: Vec<Oid> = Vec::new();
                for cid in &missing_chunks {
                    if let Some(&base) = delta_map.get(cid) {
                        let cycle_risk = base == *cid
                            || odb
                                .chunk_delta_chain_contains(&base, cid)
                                .await
                                .unwrap_or(false);
                        if cycle_risk {
                            tracing::debug!(
                                chunk = %cid,
                                base = %base,
                                "Routing to full-chunk download (would create local cycle)"
                            );
                            full_chunks.push(*cid);
                        } else {
                            delta_chunks.push(*cid);
                        }
                    } else {
                        full_chunks.push(*cid);
                    }
                }

                tracing::debug!(
                    full = full_chunks.len(),
                    deltas = delta_chunks.len(),
                    "Split chunk download: full first, then deltas"
                );

                // F7: pack-mode pull path — locate → presign → Range-GET.
                // Falls back to legacy per-chunk path if pack mode returns 0 chunks or errors.
                let cloud_packs_pull = std::env::var("MEDIAGIT_CLOUD_PACKS")
                    .as_deref()
                    .unwrap_or("1")
                    != "0";
                // F7: pack-mode pull returns the set of chunks written.
                // Remaining (not in written_set) fall through to per-chunk path.
                //
                // Build a thread-safe progress callback: pack downloads run concurrently so
                // bytes_done is accumulated via AtomicU64 and synced back after pack completes.
                let pack_bytes = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
                let pack_bytes_clone = pack_bytes.clone();
                let pack_total = total_manifest_bytes;
                let pack_progress_cb: std::sync::Arc<dyn Fn(u64) + Send + Sync> =
                    std::sync::Arc::new(move |chunk_bytes: u64| {
                        pack_bytes_clone
                            .fetch_add(chunk_bytes, std::sync::atomic::Ordering::Relaxed);
                    });
                let pack_written: std::collections::HashSet<mediagit_versioning::Oid> =
                    if cloud_packs_pull && !full_chunks.is_empty() {
                        // Drive the pack future while emitting progress ticks every 500 ms so
                        // the CLI bar advances during the (potentially long) GCS Range-GET phase.
                        // Intermediate credits use compressed bytes (pack_bytes) — an underestimate
                        // vs the uncompressed denominator — so bytes_done is corrected at the end.
                        let mut pack_fut = std::pin::pin!(self.pull_chunks_via_packs(
                            &full_chunks,
                            odb,
                            Some(pack_progress_cb),
                            _download_bench.as_ref()
                        ));
                        let mut tick = tokio::time::interval(std::time::Duration::from_millis(500));
                        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                        let pack_result = loop {
                            tokio::select! {
                                result = &mut pack_fut => { break result; }
                                _ = tick.tick() => {
                                    let in_flight =
                                        pack_bytes.load(std::sync::atomic::Ordering::Relaxed);
                                    if in_flight > 0 {
                                        on_progress(
                                            bytes_done.saturating_add(in_flight),
                                            pack_total,
                                            "Downloading via packs...",
                                        );
                                    }
                                }
                            }
                        };
                        match pack_result {
                            Ok(written) => {
                                tracing::debug!(chunks = written.len(), "pack-mode pull complete");
                                written
                            }
                            Err(e) => {
                                tracing::warn!(
                                    err = %e,
                                    "pack-mode pull failed; falling back to per-chunk"
                                );
                                std::collections::HashSet::new()
                            }
                        }
                    } else {
                        std::collections::HashSet::new()
                    };
                // Credit uncompressed bytes matching the denominator unit.
                // pack_bytes holds compressed data.len() — wrong unit; use chunk_size_map
                // (ChunkRef.size = uncompressed) so bytes_done reaches total_manifest_bytes.
                // Only credit on success; on partial error fallback covers all full_chunks.
                if !pack_written.is_empty() {
                    bytes_done += pack_written
                        .iter()
                        .map(|oid| chunk_size_map.get(&oid.to_hex()).copied().unwrap_or(0))
                        .sum::<u64>();
                    on_progress(bytes_done, pack_total, "Pack download complete");
                }
                // Chunks not written by pack-mode pull: fall through to per-chunk download.
                let chunks_for_fallback: Vec<Oid> = full_chunks
                    .into_iter()
                    .filter(|c| !pack_written.contains(c))
                    .collect();
                let fallback_hex: Vec<String> =
                    chunks_for_fallback.iter().map(|c| c.to_hex()).collect();

                if direct_download_enabled && !fallback_hex.is_empty() {
                    on_progress(
                        bytes_done,
                        total_manifest_bytes,
                        &format!("Preparing {} download URLs...", fallback_hex.len()),
                    );
                }
                let download_urls = if direct_download_enabled && !fallback_hex.is_empty() {
                    self.request_chunk_download_urls(&fallback_hex).await
                } else {
                    std::collections::HashMap::new()
                };

                // ── Pass A: full chunks not yet written (pack miss or pack disabled) ──
                if !chunks_for_fallback.is_empty() {
                    let download_urls = std::sync::Arc::new(download_urls);
                    let _dl_pass_a_t = std::time::Instant::now();
                    let mut _dl_pass_a_n = 0u64;
                    let mut _dl_pass_a_bytes = 0u64;
                    // B4: stream downloaded bytes directly to a temp file to reduce peak RAM.
                    // Default ON — Windows stress + MinIO/AWS/Azure 148/148 PASS (2026-05-22).
                    // Set MEDIAGIT_STREAM_CHUNK_TO_DISK=0 to revert to RAM buffering.
                    let stream_to_disk = std::env::var("MEDIAGIT_STREAM_CHUNK_TO_DISK")
                        .as_deref()
                        .unwrap_or("1")
                        == "1";
                    let mut stream = futures::stream::iter(chunks_for_fallback)
                        .map(|chunk_id| {
                            let client = self.client.clone();
                            let direct_client = direct_client.clone();
                            let base_url = self.base_url.clone();
                            let download_urls = std::sync::Arc::clone(&download_urls);
                            let chunk_size_map = std::sync::Arc::clone(&chunk_size_map);
                            let odb = odb.clone();
                            async move {
                                let hex = chunk_id.to_hex();
                                let size_hint = chunk_size_map.get(&hex).copied().unwrap_or(0);
                                // Try presigned direct download first; fall back to proxy on any error.
                                if let Some(Some(presigned)) = download_urls.get(&hex) {
                                    match download_chunk_direct(
                                        &direct_client,
                                        &presigned.url,
                                        &presigned.headers,
                                        &hex,
                                        size_hint,
                                    )
                                    .await
                                    {
                                        Ok(data) => {
                                            let net = data.len() as u64;
                                            odb.put_compressed_chunk(&chunk_id, &data).await?;
                                            return Ok::<_, anyhow::Error>((chunk_id, net));
                                        }
                                        Err(e) => {
                                            tracing::debug!(
                                                chunk = %hex,
                                                err = %e,
                                                "Direct download failed; falling back to proxy GET"
                                            );
                                        }
                                    }
                                }
                                // Proxy GET fallback
                                let url = format!("{}/chunks/{}", base_url, hex);
                                let response =
                                    get_chunk_with_retry(&client, &url, &chunk_id).await?;
                                // F3: server returns 409 when the chunk is delta-only.
                                // This happens when our POST /chunk-deltas/check probe failed
                                // silently and we ended up in the wrong (full-chunk) pass.
                                // Re-route: download via /chunk-deltas/<id> and store as delta.
                                if response.status() == reqwest::StatusCode::CONFLICT {
                                    let body: serde_json::Value =
                                        response.json().await.unwrap_or_default();
                                    let base_hex =
                                        body.get("base_id").and_then(|v| v.as_str()).unwrap_or("");
                                    if let Ok(base_id) = Oid::from_hex(base_hex) {
                                        let delta_url =
                                            format!("{}/chunk-deltas/{}", base_url, hex);
                                        match crate::client::send_with_rate_limit_retry(
                                            || client.get(&delta_url).send(),
                                        )
                                        .await
                                        {
                                            Ok(dr) if dr.status().is_success() => {
                                                let delta_bytes = dr.bytes().await?.to_vec();
                                                let net = delta_bytes.len() as u64;
                                                odb.write_chunk_delta(
                                                    &chunk_id,
                                                    &base_id,
                                                    &delta_bytes,
                                                )
                                                .await?;
                                                return Ok::<_, anyhow::Error>((chunk_id, net));
                                            }
                                            _ => {}
                                        }
                                    }
                                    anyhow::bail!(
                                        "GET /chunks/{} returned 409 but delta reclassification failed",
                                        chunk_id
                                    );
                                }
                                if !response.status().is_success() {
                                    return Err(chunk_get_error(&chunk_id, response.status()));
                                }
                                if stream_to_disk {
                                    // B4: stream proxy response to temp file to reduce peak RAM.
                                    use futures::StreamExt as _;
                                    use tokio::io::AsyncWriteExt as _;
                                    let temp_path =
                                        std::env::temp_dir().join(format!("mg-chunk-{}", hex));
                                    let mut f = tokio::fs::File::create(&temp_path)
                                        .await
                                        .map_err(|e| anyhow::anyhow!("B4 temp create: {}", e))?;
                                    let mut written = 0u64;
                                    let mut byte_stream = response.bytes_stream();
                                    while let Some(ch) = byte_stream.next().await {
                                        let b = ch.map_err(|e| {
                                            anyhow::anyhow!("Download stream error: {}", e)
                                        })?;
                                        f.write_all(&b).await.map_err(|e| {
                                            anyhow::anyhow!("Write temp chunk: {}", e)
                                        })?;
                                        written += b.len() as u64;
                                    }
                                    f.flush().await?;
                                    drop(f);
                                    odb.put_compressed_chunk_from_file(&chunk_id, &temp_path)
                                        .await?;
                                    let _ = tokio::fs::remove_file(&temp_path).await;
                                    Ok::<_, anyhow::Error>((chunk_id, written))
                                } else {
                                    let data = response.bytes().await.map_err(|e| {
                                        anyhow::anyhow!("Failed to read chunk {}: {}", chunk_id, e)
                                    })?;
                                    let net = data.len() as u64;
                                    odb.put_compressed_chunk(&chunk_id, &data).await?;
                                    Ok::<_, anyhow::Error>((chunk_id, net))
                                }
                            }
                        })
                        .buffer_unordered(concurrent_downloads);

                    while let Some(result) = stream.next().await {
                        let (chunk_id, net_bytes) = result?;
                        _dl_pass_a_n += 1;
                        _dl_pass_a_bytes += net_bytes;
                        total_net_bytes += net_bytes;
                        // ODB store happens inside the closure (B4 refactor).
                        total_chunks_downloaded += 1;
                        bytes_done += chunk_size_map.get(&chunk_id.to_hex()).copied().unwrap_or(0);
                        on_progress(
                            bytes_done,
                            total_manifest_bytes,
                            &format!("Object {}/{}", obj_idx + 1, n_objects),
                        );
                    }
                    if let Some(b) = &_download_bench {
                        b.record_batch(_dl_pass_a_n, _dl_pass_a_bytes, _dl_pass_a_t.elapsed());
                    }
                }

                // ── Pass B: delta chunks (bases are now local) ────────────────
                if !delta_chunks.is_empty() {
                    let delta_map = std::sync::Arc::new(delta_map);
                    let _dl_pass_b_t = std::time::Instant::now();
                    let mut _dl_pass_b_n = 0u64;
                    let mut _dl_pass_b_bytes = 0u64;
                    let mut stream = futures::stream::iter(delta_chunks)
                        .map(|chunk_id| {
                            let client = self.client.clone();
                            let base_url = self.base_url.clone();
                            let delta_map = std::sync::Arc::clone(&delta_map);
                            async move {
                                let url =
                                    format!("{}/chunk-deltas/{}", base_url, chunk_id.to_hex());
                                let response = crate::client::send_with_rate_limit_retry(|| {
                                    client.get(&url).send()
                                })
                                .await
                                .map_err(|e| {
                                    anyhow::anyhow!(
                                        "Failed to download chunk-delta {}: {}",
                                        chunk_id,
                                        e
                                    )
                                })?;
                                if !response.status().is_success() {
                                    anyhow::bail!(
                                        "GET /chunk-deltas/{} failed with status: {}",
                                        chunk_id,
                                        response.status()
                                    );
                                }
                                let data = response.bytes().await.map_err(|e| {
                                    anyhow::anyhow!(
                                        "Failed to read chunk-delta {}: {}",
                                        chunk_id,
                                        e
                                    )
                                })?;
                                let base = delta_map.get(&chunk_id).copied().ok_or_else(|| {
                                    anyhow::anyhow!(
                                        "Internal: chunk {} missing from delta_map",
                                        chunk_id
                                    )
                                })?;
                                Ok::<_, anyhow::Error>((chunk_id, base, data.to_vec()))
                            }
                        })
                        .buffer_unordered(concurrent_downloads);

                    while let Some(result) = stream.next().await {
                        let (chunk_id, base_id, delta_bytes) = result?;
                        _dl_pass_b_n += 1;
                        _dl_pass_b_bytes += delta_bytes.len() as u64;
                        total_net_bytes += delta_bytes.len() as u64;
                        if let Err(e) = odb
                            .write_chunk_delta(&chunk_id, &base_id, &delta_bytes)
                            .await
                        {
                            tracing::warn!(
                                chunk = %chunk_id,
                                base = %base_id,
                                error = %e,
                                "Delta write failed (cycle?), falling back to full chunk"
                            );
                            let full = self.download_chunk(&chunk_id).await?;
                            odb.put_compressed_chunk(&chunk_id, &full).await?;
                        }
                        total_chunks_downloaded += 1;
                        bytes_done += chunk_size_map.get(&chunk_id.to_hex()).copied().unwrap_or(0);
                        on_progress(
                            bytes_done,
                            total_manifest_bytes,
                            &format!("Object {}/{}", obj_idx + 1, n_objects),
                        );
                    }
                    if let Some(b) = &_download_bench {
                        b.record_batch(_dl_pass_b_n, _dl_pass_b_bytes, _dl_pass_b_t.elapsed());
                    }
                }
            }

            // Advance bar for chunks that were already local (dedup / re-clone).
            if already_local > 0 {
                let already_bytes: u64 = manifest
                    .chunks
                    .iter()
                    .filter(|c| !missing_chunks.iter().any(|m| m == &c.id))
                    .map(|c| c.size as u64)
                    .sum();
                bytes_done += already_bytes;
                on_progress(
                    bytes_done,
                    total_manifest_bytes,
                    &format!("Object {}/{}", obj_idx + 1, n_objects),
                );
            }

            // Store manifest locally
            odb.put_manifest(&oid, &manifest).await?;

            tracing::debug!(oid = %oid, "Chunked object downloaded");
        }

        if let Some(b) = &_download_bench {
            b.summary();
        }
        Ok((total_chunks_downloaded, total_net_bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::{chunk_get_error, chunk_get_is_transient, get_chunk_with_retry};
    use reqwest::StatusCode;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn server_errors_and_throttling_are_transient() {
        for s in [
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::BAD_GATEWAY,
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::GATEWAY_TIMEOUT,
            StatusCode::TOO_MANY_REQUESTS,
        ] {
            assert!(chunk_get_is_transient(s), "{s} should be retried");
        }
    }

    /// 409 is the load-bearing case: the server answers it when a chunk is
    /// delta-only, and the caller re-routes to `/chunk-deltas/<id>`. Retrying
    /// it would burn the backoff and then fail a request that was never going
    /// to change — and could mask the re-route path entirely.
    #[test]
    fn verdicts_are_not_retried() {
        for s in [
            StatusCode::NOT_FOUND,
            StatusCode::FORBIDDEN,
            StatusCode::CONFLICT,
            StatusCode::UNAUTHORIZED,
            StatusCode::OK,
        ] {
            assert!(!chunk_get_is_transient(s), "{s} must not be retried");
        }
    }

    /// The 503 text names the operator action. It previously existed only on
    /// the sequential path while clone runs the parallel one, so it was
    /// unreachable in the case it was written for.
    #[test]
    fn service_unavailable_reports_the_operator_action() {
        let oid = mediagit_versioning::Oid::from_bytes([7u8; 32]);
        let msg = format!("{}", chunk_get_error(&oid, StatusCode::SERVICE_UNAVAILABLE));
        assert!(msg.contains("storage backend unreachable"), "got: {msg}");
        assert!(msg.contains("MinIO/S3/Azure"), "got: {msg}");
    }

    #[test]
    fn other_statuses_report_the_status() {
        let oid = mediagit_versioning::Oid::from_bytes([7u8; 32]);
        let msg = format!("{}", chunk_get_error(&oid, StatusCode::NOT_FOUND));
        assert!(msg.contains("404"), "got: {msg}");
    }

    /// Serves `total_requests` sequential connections: the first `fail_count`
    /// get a bare 503, the rest get 200 + `body`. `Connection: close` on every
    /// reply forces the client onto a fresh socket per attempt, so this exercises
    /// the retry loop the same way a real transient backend failure would --
    /// each attempt is an independent request, not a replay on one connection.
    ///
    /// `served` counts connections actually accepted. That count is what makes
    /// the negative test load-bearing: without it, "budget exhausted" is
    /// satisfied just as well by a build that never retries at all, so the test
    /// would pass against the very defect it exists to catch.
    async fn serve_flaky_chunk(
        listener: TcpListener,
        fail_count: usize,
        total_requests: usize,
        body: Vec<u8>,
        served: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ) {
        for i in 0..total_requests {
            let (mut sock, _) = listener.accept().await.expect("accept");
            served.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let n = sock.read(&mut chunk).await.expect("read");
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            if i < fail_count {
                let _ = sock
                    .write_all(
                        b"HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .await;
            } else {
                let head = format!(
                    "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = sock.write_all(head.as_bytes()).await;
                let _ = sock.write_all(&body).await;
            }
            let _ = sock.flush().await;
        }
    }

    /// Proves the loop actually retries and recovers: two injected 503s, then
    /// a real body.
    ///
    /// This is the load-bearing test of the pair. Its expectations are
    /// **hardcoded** (2 failures, 3 total requests) rather than derived from
    /// `CHUNK_GET_MAX_RETRIES`, which is what lets it detect the budget being
    /// removed. Red-verified 2026-08-04: with the constant set to 0 it fails on
    /// `assertion failed: response.status().is_success()`, while the
    /// exhaustion test — whose expectation tracks the constant — still passes.
    /// Keep these counts literal; deriving them would make this test blind to
    /// the defect it exists for.
    #[tokio::test(flavor = "multi_thread")]
    async fn retries_past_transient_failures_and_recovers() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let body: Vec<u8> = (0u16..4096).map(|b| (b % 251) as u8).collect();
        let served = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let server = tokio::spawn(serve_flaky_chunk(
            listener,
            2,
            3,
            body.clone(),
            std::sync::Arc::clone(&served),
        ));

        crate::ensure_crypto_provider();
        let client = reqwest::Client::builder().build().expect("client");
        let oid = mediagit_versioning::Oid::from_bytes([9u8; 32]);
        let url = format!("http://{addr}/chunks/{}", oid.to_hex());

        let response = get_chunk_with_retry(&client, &url, &oid)
            .await
            .expect("should recover after transient 503s");
        assert!(response.status().is_success());
        let got = response.bytes().await.expect("body").to_vec();
        assert_eq!(got, body, "recovered body must be byte-identical");
        assert_eq!(
            served.load(std::sync::atomic::Ordering::SeqCst),
            3,
            "recovery must have taken exactly the 2 failed attempts plus 1 success"
        );

        tokio::time::timeout(std::time::Duration::from_secs(10), server)
            .await
            .expect("server task timed out")
            .expect("server task panicked");
    }

    /// The complementary negative: failures that exceed the retry budget must
    /// not spin forever. `CHUNK_GET_MAX_RETRIES` is 3, so 4 straight 503s (the
    /// initial attempt + all 3 retries) must exhaust the budget.
    ///
    /// `get_chunk_with_retry` itself doesn't turn a terminal non-success
    /// status into `Err` -- it hands back the last response as-is, same as
    /// before this loop was extracted, because the caller needs that response
    /// object intact to special-case 409 (delta re-route). The status check
    /// that turns a terminal failure into an error lives in the caller,
    /// immediately after the loop, unchanged by this extraction. So the
    /// contract this test pins is: bounded attempts (proved by the timeout
    /// below -- a loop that ignored `CHUNK_GET_MAX_RETRIES` would hang past
    /// it) and the failing status surfacing intact for that caller-side check.
    #[tokio::test(flavor = "multi_thread")]
    async fn gives_up_once_the_retry_budget_is_exhausted() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let total = (super::CHUNK_GET_MAX_RETRIES + 1) as usize;
        let served = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let server = tokio::spawn(serve_flaky_chunk(
            listener,
            total,
            total,
            Vec::new(),
            std::sync::Arc::clone(&served),
        ));
        let _ = &server;

        crate::ensure_crypto_provider();
        let client = reqwest::Client::builder().build().expect("client");
        let oid = mediagit_versioning::Oid::from_bytes([9u8; 32]);
        let url = format!("http://{addr}/chunks/{}", oid.to_hex());

        // Bounded wait: a loop that doesn't respect CHUNK_GET_MAX_RETRIES
        // (spins forever) fails this test instead of hanging the suite.
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            get_chunk_with_retry(&client, &url, &oid),
        )
        .await
        .expect("retry loop did not return -- it is not respecting the retry budget")
        .expect("transport-level error even though the server always answered");

        assert_eq!(
            response.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "budget exhausted: the still-failing status must surface for the \
             caller's post-loop check to turn into an error"
        );
        assert!(
            !chunk_get_error(&oid, response.status())
                .to_string()
                .is_empty(),
            "the caller-side check that follows this loop must be able to \
             turn this terminal response into an error"
        );
        // Pins that every attempt in the budget actually reached the server,
        // rather than the loop returning early.
        //
        // NOTE ON WHAT THIS CANNOT CATCH: `total` is derived from
        // CHUNK_GET_MAX_RETRIES, so mutating that constant moves the
        // expectation with it and this test still passes -- confirmed by
        // running it at 0. That is correct for a contract test (the constant is
        // the spec, and changing the budget deliberately should not fail it),
        // but it means this test does NOT independently prove retries happen.
        // `retries_past_transient_failures_and_recovers` is the load-bearing
        // one: its counts are hardcoded (2 failures, 3 requests) and it DOES
        // fail at 0 retries.
        assert_eq!(
            served.load(std::sync::atomic::Ordering::SeqCst),
            total,
            "expected the initial attempt plus all {} retries to reach the server",
            super::CHUNK_GET_MAX_RETRIES
        );

        tokio::time::timeout(std::time::Duration::from_secs(10), server)
            .await
            .expect("server task timed out -- client made fewer requests than expected")
            .expect("server task panicked");
    }
}

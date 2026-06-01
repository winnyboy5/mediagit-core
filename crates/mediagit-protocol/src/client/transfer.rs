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

impl ProtocolClient {
    /// Check which chunks exist on the remote server
    ///
    /// Returns list of chunk IDs that are MISSING (need to be uploaded)
    pub(crate) async fn check_chunks_exist(&self, chunk_ids: &[String]) -> Result<Vec<String>> {
        let url = format!("{}/chunks/check", self.base_url);
        tracing::debug!(
            count = chunk_ids.len(),
            "Checking chunk existence on remote"
        );

        let response = self
            .client
            .post(&url)
            .json(&chunk_ids)
            .send()
            .await
            .context("Failed to POST /chunks/check")?;

        if !response.status().is_success() {
            anyhow::bail!(
                "POST /chunks/check failed with status: {}",
                response.status()
            );
        }

        response
            .json::<Vec<String>>()
            .await
            .context("Failed to parse chunks check response")
    }

    /// Request presigned PUT URLs for a batch of chunk IDs.
    ///
    /// Returns a map of `chunk_hex → Option<PresignedPutInfo>`.
    /// `None` means the backend doesn't support presigning — caller must use
    /// the server-proxied `PUT /chunks/:id` path instead.
    /// Any network/parse failure is treated as "no presign" (graceful degradation).
    pub(crate) async fn request_chunk_upload_urls(
        &self,
        chunk_ids: &[String],
        sizes: &std::collections::HashMap<String, u64>,
    ) -> std::collections::HashMap<String, Option<PresignedPutInfo>> {
        #[derive(serde::Serialize)]
        struct Req<'a> {
            chunk_ids: &'a [String],
            sizes: &'a std::collections::HashMap<String, u64>,
        }

        let url = format!("{}/chunks/upload-urls", self.base_url);
        let result = async {
            let resp = self
                .client
                .post(&url)
                .json(&Req { chunk_ids, sizes })
                .send()
                .await?;
            if !resp.status().is_success() {
                anyhow::bail!("POST /chunks/upload-urls returned {}", resp.status());
            }
            resp.json::<std::collections::HashMap<String, Option<PresignedPutInfo>>>()
                .await
                .context("parse /chunks/upload-urls")
        }
        .await;

        match result {
            Ok(map) => map,
            Err(e) => {
                tracing::debug!(err = %e, "presign URL request failed; using proxy PUT for all chunks");
                std::collections::HashMap::new()
            }
        }
    }

    /// Request presigned GET URLs for a batch of chunk IDs.
    ///
    /// Returns a map of `chunk_hex → Option<PresignedGetInfo>`.
    /// `None` means the backend doesn't support presigning or chunk doesn't exist yet
    /// — caller must use the server-proxied `GET /chunks/:id` path instead.
    /// Any network/parse failure is treated as "no presign" (graceful degradation).
    pub(crate) async fn request_chunk_download_urls(
        &self,
        chunk_ids: &[String],
    ) -> std::collections::HashMap<String, Option<PresignedGetInfo>> {
        if chunk_ids.is_empty() {
            return std::collections::HashMap::new();
        }

        #[derive(serde::Serialize)]
        struct Req<'a> {
            chunks: &'a [String],
        }

        use futures::stream::StreamExt;

        // Split into smaller batches so each HTTP round-trip is bounded in size/latency.
        // Batches are fired concurrently (up to 4 in-flight) and merged into one map.
        let batch_size: usize = std::env::var("MEDIAGIT_PRESIGN_BATCH")
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|n: &usize| *n > 0)
            .unwrap_or(512);

        let batches: Vec<&[String]> = chunk_ids.chunks(batch_size).collect();
        let base_url = &self.base_url;
        let client = &self.client;

        let results: Vec<_> = futures::stream::iter(batches)
            .map(|batch| async move {
                let url = format!("{}/chunks/download-urls", base_url);
                let result = async {
                    let resp = client
                        .post(&url)
                        .json(&Req { chunks: batch })
                        .timeout(std::time::Duration::from_secs(20))
                        .send()
                        .await?;
                    if !resp.status().is_success() {
                        anyhow::bail!("POST /chunks/download-urls returned {}", resp.status());
                    }
                    resp.json::<std::collections::HashMap<String, Option<PresignedGetInfo>>>()
                        .await
                        .context("parse /chunks/download-urls")
                }
                .await;
                match result {
                    Ok(map) => map,
                    Err(e) => {
                        tracing::debug!(err = %e, "presign batch request failed; affected chunks use proxy GET");
                        std::collections::HashMap::new()
                    }
                }
            })
            .buffer_unordered(4)
            .collect()
            .await;

        let mut merged = std::collections::HashMap::with_capacity(chunk_ids.len());
        for map in results {
            merged.extend(map);
        }
        merged
    }
    pub(crate) async fn verify_chunk_uploads(
        &self,
        chunk_ids: &[String],
    ) -> anyhow::Result<Vec<String>> {
        #[derive(serde::Serialize)]
        struct Req<'a> {
            chunk_ids: &'a [String],
        }
        #[derive(serde::Deserialize)]
        struct Resp {
            missing: Vec<String>,
        }

        let url = format!("{}/chunks/complete", self.base_url);
        let resp = self
            .client
            .post(&url)
            .json(&Req { chunk_ids })
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("POST /chunks/complete returned {}", resp.status());
        }
        let r = resp
            .json::<Resp>()
            .await
            .context("parse /chunks/complete")?;
        Ok(r.missing)
    }

    /// POST /:repo/chunks/verify-integrity — BLAKE3 re-hash of every stored chunk.
    /// Only called when `MEDIAGIT_STRONG_VERIFY=1`. Returns chunk ids whose stored
    /// content does not match their claimed hash.
    pub(crate) async fn strong_verify_chunks(
        &self,
        chunk_ids: &[String],
    ) -> anyhow::Result<Vec<String>> {
        #[derive(serde::Serialize)]
        struct Req<'a> {
            chunk_ids: &'a [String],
        }
        #[derive(serde::Deserialize)]
        struct Resp {
            invalid: Vec<String>,
        }

        let url = format!("{}/chunks/verify-integrity", self.base_url);
        let resp = self
            .client
            .post(&url)
            .json(&Req { chunk_ids })
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("POST /chunks/verify-integrity returned {}", resp.status());
        }
        let r = resp
            .json::<Resp>()
            .await
            .context("parse /chunks/verify-integrity")?;
        Ok(r.invalid)
    }
    /// Ask the server which of the given chunk IDs exist as chunk-deltas.
    ///
    /// Returns a map `chunk_id → base_chunk_id` (both as `Oid`). Chunks not
    /// in the response are stored as full chunks (use `download_chunk`).
    ///
    /// On 404 (old server without the endpoint) or any error, returns an
    /// empty map — the caller falls back to full-chunk downloads. This keeps
    /// new clients compatible with old servers.
    pub(crate) async fn check_chunk_deltas(
        &self,
        chunk_ids: &[Oid],
    ) -> std::collections::HashMap<Oid, Oid> {
        const TIMEOUT_SECS: u64 = 5;
        match tokio::time::timeout(
            std::time::Duration::from_secs(TIMEOUT_SECS),
            self.check_chunk_deltas_inner(chunk_ids),
        )
        .await
        {
            Ok(map) => map,
            Err(_elapsed) => {
                tracing::debug!(
                    chunks = chunk_ids.len(),
                    timeout_secs = TIMEOUT_SECS,
                    "chunk-deltas/check timed out; treating as no deltas"
                );
                std::collections::HashMap::new()
            }
        }
    }

    async fn check_chunk_deltas_inner(
        &self,
        chunk_ids: &[Oid],
    ) -> std::collections::HashMap<Oid, Oid> {
        let mut empty = std::collections::HashMap::new();
        if chunk_ids.is_empty() {
            return empty;
        }

        let url = format!("{}/chunk-deltas/check", self.base_url);
        let payload: Vec<String> = chunk_ids.iter().map(|o| o.to_hex()).collect();

        let response = match self.client.post(&url).json(&payload).send().await {
            Ok(r) => r,
            Err(e) => {
                // Transport failure is unexpected — warn so production issues surface
                // instead of silently routing all chunks through /chunks/<id> and 404ing.
                tracing::warn!(error = %e, "chunk-deltas/check request failed (treating as no deltas)");
                return empty;
            }
        };

        if !response.status().is_success() {
            if response.status().as_u16() == 404 {
                // 404 means old server without this endpoint — expected, silently fall back.
                tracing::debug!(status = %response.status(), "chunk-deltas/check 404 (old server); treating as no deltas");
            } else {
                // Non-404 failures (500/503/etc.) are unexpected — log at warn so
                // production probe failures are visible rather than silently causing clone 404s.
                tracing::warn!(
                    status = %response.status(),
                    "chunk-deltas/check non-success (treating as no deltas)"
                );
            }
            return empty;
        }

        let map: std::collections::HashMap<String, String> = match response.json().await {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(error = %e, "chunk-deltas/check response parse failed (treating as no deltas)");
                return empty;
            }
        };

        for (id_hex, base_hex) in map {
            if let (Ok(id), Ok(base)) = (Oid::from_hex(&id_hex), Oid::from_hex(&base_hex)) {
                empty.insert(id, base);
            }
        }
        empty
    }
}

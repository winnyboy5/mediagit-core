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

use anyhow::{Context, Result};
use mediagit_versioning::{CloudPackResult, ObjectType, Oid, PackKind, StreamingPackWriter};
use std::path::PathBuf;

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

// ST-4: caps come from `mediagit_versioning::pack` so this path and
// `gc --repack` cannot disagree about them, and so an out-of-range value is
// clamped in exactly one place. They were previously parsed here and again in
// the ODB, unclamped in both.
use mediagit_versioning::{pack_bytes_cap, pack_chunks_cap};

/// Client-side cloud pack assembler.
///
/// Accumulates chunks and flushes a `CloudPackResult` when the byte cap
/// (`MEDIAGIT_PACK_BYTES`, default 64 MiB) or chunk cap
/// (`MEDIAGIT_PACK_CHUNKS`, default 1024) is reached.
///
/// Call `finish()` after all chunks are added to flush any remainder.
pub struct PackBuilder {
    temp_dir: PathBuf,
    writer: Option<StreamingPackWriter<tokio::fs::File>>,
    current_bytes: u64,
    current_chunks: u32,
    bytes_cap: u64,
    chunks_cap: u32,
}

impl PackBuilder {
    pub fn new(temp_dir: impl Into<PathBuf>) -> Self {
        Self {
            temp_dir: temp_dir.into(),
            writer: None,
            current_bytes: 0,
            current_chunks: 0,
            bytes_cap: pack_bytes_cap(),
            chunks_cap: pack_chunks_cap(),
        }
    }

    pub fn with_caps(temp_dir: impl Into<PathBuf>, bytes_cap: u64, chunks_cap: u32) -> Self {
        Self {
            temp_dir: temp_dir.into(),
            writer: None,
            current_bytes: 0,
            current_chunks: 0,
            bytes_cap,
            chunks_cap,
        }
    }

    /// Add a full (uncompressed) chunk to the current pack.
    ///
    /// Returns `Some(CloudPackResult)` if the pack was sealed (cap hit), or
    /// `None` if still accumulating.
    pub async fn add_chunk(&mut self, oid: Oid, data: &[u8]) -> Result<Option<CloudPackResult>> {
        if self.writer.is_none() {
            let w = StreamingPackWriter::new_open_ended(PackKind::CloudObject, &self.temp_dir)
                .await
                .context("create pack writer")?;
            self.writer = Some(w);
        }

        let w = self.writer.as_mut().unwrap();
        w.write_object(oid, ObjectType::Blob, data)
            .await
            .context("write chunk to pack")?;

        // +5 for the per-object type(1) + size(4) header bytes
        self.current_bytes += data.len() as u64 + 5;
        self.current_chunks += 1;

        if self.current_bytes >= self.bytes_cap || self.current_chunks >= self.chunks_cap {
            Ok(Some(self.seal().await?))
        } else {
            Ok(None)
        }
    }

    /// Force-flush all remaining chunks regardless of count. Returns `None` if
    /// the pack is empty.
    pub async fn finish(&mut self) -> Result<Option<CloudPackResult>> {
        if self.writer.is_none() || self.current_chunks == 0 {
            return Ok(None);
        }
        Ok(Some(self.seal().await?))
    }

    async fn seal(&mut self) -> Result<CloudPackResult> {
        let writer = self.writer.take().expect("seal called with no writer");
        let result = writer.finalize_cloud().await.context("finalize pack")?;
        self.current_bytes = 0;
        self.current_chunks = 0;
        tracing::debug!(
            pack_oid = bytes_to_hex(&result.pack_oid),
            byte_len = result.byte_len,
            chunks = result.index.len(),
            "Pack sealed"
        );
        Ok(result)
    }

    pub fn current_chunks(&self) -> u32 {
        self.current_chunks
    }

    pub fn current_bytes(&self) -> u64 {
        self.current_bytes
    }
}

/// Upload a finished pack to cloud storage via a presigned PUT URL and
/// register its manifest with the server.
///
/// This is the network-side of F4: the PackBuilder handles disk assembly,
/// `upload_and_register` handles transport and server registration.
/// `base_url` must already include the repo segment (e.g. `http://server/my-repo`).
///
/// Returns `true` when the pack bytes went out over a presigned PUT straight
/// to the bucket, `false` when they were proxied through the server. Callers
/// use this to distinguish the two in `[bench]` output — a presign request
/// that silently degraded to the proxy otherwise looks identical to a
/// successful direct upload.
pub async fn upload_and_register(
    result: CloudPackResult,
    base_url: &str,
    http_client: &reqwest::Client,
    direct_client: &reqwest::Client,
    compressed_hashes: &[(String, String)],
) -> Result<bool> {
    let pack_oid_hex = bytes_to_hex(&result.pack_oid);
    let byte_len = result.byte_len;
    let mut presigned_direct = false;

    // 1. Request presigned PUT URL for packs/<pack_oid>
    let presign_url = format!("{}/packs/upload-urls", base_url);
    let presign_body = serde_json::json!({
        "pack_ids": [pack_oid_hex],
        "sizes": [byte_len],
    });
    let presign_resp = http_client
        .post(&presign_url)
        .json(&presign_body)
        .send()
        .await
        .context("POST /packs/upload-urls")?;

    if !presign_resp.status().is_success() {
        anyhow::bail!("POST /packs/upload-urls returned {}", presign_resp.status());
    }

    let presign_map: std::collections::HashMap<String, Option<serde_json::Value>> = presign_resp
        .json()
        .await
        .context("parse /packs/upload-urls response")?;

    // 2. Upload pack bytes
    let pack_data = tokio::fs::read(&result.temp_path)
        .await
        .context("read pack temp file")?;

    if let Some(Some(purl)) = presign_map.get(&pack_oid_hex) {
        let put_url = purl["url"].as_str().unwrap_or("").to_string();
        let mut req = direct_client.put(&put_url).body(pack_data);
        if let Some(headers) = purl["required_headers"].as_array() {
            for h in headers {
                if let (Some(name), Some(val)) = (
                    h.get(0).and_then(|v| v.as_str()),
                    h.get(1).and_then(|v| v.as_str()),
                ) {
                    req = req.header(name, val);
                }
            }
        }
        let resp = req.send().await.context("presigned PUT of pack")?;
        if !resp.status().is_success() {
            anyhow::bail!("presigned PUT returned {}", resp.status());
        }
        presigned_direct = true;
        tracing::debug!(pack = %pack_oid_hex, bytes = byte_len, "Pack uploaded via presigned URL");
    } else {
        // Proxy fallback: PUT to /packs/<oid> so complete_pack's head("packs/<oid>") succeeds
        let proxy_url = format!("{}/packs/{}", base_url, pack_oid_hex);
        let resp = http_client
            .put(&proxy_url)
            .body(pack_data)
            .send()
            .await
            .context("proxy PUT of pack")?;
        if !resp.status().is_success() {
            anyhow::bail!("proxy PUT returned {}", resp.status());
        }
        tracing::debug!(pack = %pack_oid_hex, bytes = byte_len, "Pack uploaded via proxy");
    }

    // 3. POST /packs/complete with manifest
    let manifest: Vec<serde_json::Value> = result
        .index
        .iter()
        .map(|loc| {
            let chunk_hex = loc.chunk_oid.to_hex();
            let comp_hash = compressed_hashes
                .iter()
                .find(|(h, _)| h == &chunk_hex)
                .map(|(_, hash)| hash.clone());
            serde_json::json!({
                "chunk_oid": chunk_hex,
                "offset": loc.offset,
                "length": loc.length,
                "compressed_hash": comp_hash,
            })
        })
        .collect();

    let complete_url = format!("{}/packs/complete", base_url);
    let complete_body = serde_json::json!({
        "pack_oid": pack_oid_hex,
        "manifest": manifest,
    });
    let complete_resp = http_client
        .post(&complete_url)
        .json(&complete_body)
        .send()
        .await
        .context("POST /packs/complete")?;

    if !complete_resp.status().is_success() {
        anyhow::bail!("POST /packs/complete returned {}", complete_resp.status());
    }

    // 4. Clean up temp file
    let _ = tokio::fs::remove_file(&result.temp_path).await;

    tracing::info!(
        pack = %pack_oid_hex,
        bytes = byte_len,
        chunks = manifest.len(),
        presigned_direct,
        "Pack registered with server"
    );
    Ok(presigned_direct)
}

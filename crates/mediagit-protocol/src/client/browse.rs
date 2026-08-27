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

//! Raw file download via the server's browse endpoint (`GET
//! /{repo}/files/{*path}?ref=<ref>`) — used by `mediagit download` (#6).
//!
//! Deliberately a plain streaming GET, not `StreamingDownloader`:
//! `StreamingDownloader` assumes HEAD+Range support, which the browse
//! endpoint doesn't offer. Streams straight to the caller's writer with
//! O(64KB) memory, matching the server's own duplex-channel streaming.

use super::*;

impl ProtocolClient {
    /// Download a single file from committed state by its repo-relative
    /// path, streaming the response body into `out`. No local repo or ODB
    /// is required — this hits the server directly by path, which is the
    /// whole point (CI/scripting use `mediagit download` against a bare
    /// URL). Returns the number of bytes written.
    pub async fn download_file_by_path(
        &self,
        file_path: &str,
        ref_name: &str,
        out: &mut (impl tokio::io::AsyncWrite + Unpin),
    ) -> Result<u64> {
        use futures::StreamExt;
        use tokio::io::AsyncWriteExt;

        // Parsed from `self.base_url` (no trailing slash — e.g.
        // "http://host/repo") rather than a string with "/files/" baked in:
        // `path_segments_mut().push()` on a URL whose path already ends in
        // "/" treats the trailing empty segment ambiguously across `url`
        // crate versions. Pushing "files" then each path component onto a
        // slash-free base is unambiguous.
        let mut url = reqwest::Url::parse(&self.base_url).context("Invalid remote URL")?;
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| anyhow::anyhow!("Remote URL cannot be a file download base"))?;
            segments.push("files");
            for part in file_path.split('/').filter(|s| !s.is_empty()) {
                segments.push(part);
            }
        }
        url.query_pairs_mut().append_pair("ref", ref_name);

        // Bounded by MEDIAGIT_PULL_DEADLINE_SECS, the same guard the pack and
        // chunk download paths already use.
        //
        // Without it this hung forever against a backend that accepted the
        // connection, began a response and then stopped sending — the exact
        // failure `with_pull_deadline` was introduced for (see its note in
        // `client/mod.rs`: "a backend that accepted the connection and then
        // stopped sending left `next_object()` awaiting forever"). That fix
        // reached `download_pack_streaming` and `download_chunked_objects`;
        // this separate one-off streaming path was missed, leaving
        // `mediagit download` — which CI and scripts run unattended against a
        // bare URL — able to hang indefinitely with no output.
        //
        // The GET is inside the deadline too, not just the body loop: the
        // control-plane client is deliberately built with no per-request
        // timeout, so a peer that stalls before sending headers would
        // otherwise be just as unbounded as one that stalls mid-body.
        crate::client::with_pull_deadline("file download", async move {
            // `url` is cloned per attempt: the closure is `Fn` and is re-invoked
            // on a 429, and `reqwest::Url` is not `Copy`.
            let response =
                crate::client::send_with_rate_limit_retry(|| self.client.get(url.clone()).send())
                    .await
                    .context("Failed to GET file")?;

            if response.status() == reqwest::StatusCode::NOT_FOUND {
                anyhow::bail!("File '{}' not found at ref '{}'", file_path, ref_name);
            }
            if !response.status().is_success() {
                anyhow::bail!(
                    "GET file '{}' failed with status: {}",
                    file_path,
                    response.status()
                );
            }

            let mut total: u64 = 0;
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.context("Error while streaming file download")?;
                out.write_all(&chunk)
                    .await
                    .context("Failed to write downloaded file to disk")?;
                total += chunk.len() as u64;
            }
            out.flush().await.ok();
            Ok(total)
        })
        .await
    }
}

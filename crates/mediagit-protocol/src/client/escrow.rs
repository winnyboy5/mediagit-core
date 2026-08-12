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

//! DC-7/D4: handing this repository's key to the remote, and getting it back.
//!
//! The key travels over the transport as-is rather than wrapped to a server
//! public key. No server keypair exists, distributing one is its own
//! subsystem, and the threat model here is a compromised **object store** —
//! the server is trusted with the key by construction, since it has to open
//! commits and trees to compute a clone closure at all. Use TLS.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::ProtocolClient;

/// A repository key in transit, hex-encoded.
#[derive(Debug, Serialize, Deserialize)]
struct RepoKeyPayload {
    key: String,
}

/// What the remote had to say when asked for this repository's key.
#[derive(Debug, PartialEq, Eq)]
pub enum EscrowedKey {
    /// The remote holds a key for this repository.
    Present(Zeroizing<Vec<u8>>),
    /// The remote does escrow, but has no key for this repository yet.
    Absent,
    /// The remote does not do escrow at all — an older server, or one with
    /// encryption switched off. Indistinguishable from the client's side, and
    /// deliberately so: the honest message is the same either way.
    Unsupported,
}

impl ProtocolClient {
    /// Ask the remote for this repository's escrowed key.
    pub async fn get_encryption_key(&self) -> Result<EscrowedKey> {
        let url = format!("{}/encryption-key", self.base_url);
        tracing::debug!("GET {}", url);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to send GET /encryption-key")?;

        // 404 covers both "no key yet" and "no such route"; the server cannot
        // distinguish an old client asking and neither can we. Either way the
        // caller's next move is the same: escrow it, and find out.
        if response.status().as_u16() == 404 {
            return Ok(EscrowedKey::Absent);
        }
        if !response.status().is_success() {
            anyhow::bail!(
                "the remote refused to release this repository's encryption key (HTTP {}). \
                 Without it nothing here can be read back.",
                response.status()
            );
        }

        let payload: RepoKeyPayload = response
            .json()
            .await
            .context("Failed to parse the escrowed key response")?;
        let bytes = Zeroizing::new(
            hex::decode(payload.key.trim())
                .context("the remote returned an encryption key that is not valid hex")?,
        );
        if bytes.len() != 32 {
            anyhow::bail!(
                "the remote returned a {}-byte encryption key; expected 32",
                bytes.len()
            );
        }
        Ok(EscrowedKey::Present(bytes))
    }

    /// Hand this repository's key to the remote.
    ///
    /// Idempotent on the same key. A *different* key already escrowed is a
    /// hard error and nothing is uploaded: the remote's objects are sealed
    /// under the key it holds, nothing records which key an object used, and
    /// replacing it would orphan the whole repository silently.
    pub async fn put_encryption_key(&self, key: &[u8]) -> Result<()> {
        let url = format!("{}/encryption-key", self.base_url);
        tracing::debug!("PUT {}", url);

        let response = self
            .client
            .put(&url)
            .json(&RepoKeyPayload {
                key: hex::encode(key),
            })
            .send()
            .await
            .context("Failed to send PUT /encryption-key")?;

        match response.status().as_u16() {
            200 | 201 | 204 => Ok(()),
            404 => anyhow::bail!(
                "this remote does not support encrypted repositories. Either it is an older \
                 server, or at-rest encryption is switched off in its configuration \
                 (`[encryption] enabled`). Nothing was uploaded."
            ),
            409 => anyhow::bail!(
                "the remote already holds a DIFFERENT encryption key for this repository. \
                 Its objects are sealed under that key, so replacing it would make every one \
                 of them unreadable. Nothing was uploaded.\n\
                 \n\
                 This usually means the local repository was re-keyed, or two repositories \
                 of the same name are pushing to one remote."
            ),
            s => anyhow::bail!("failed to escrow this repository's encryption key (HTTP {s})"),
        }
    }
}

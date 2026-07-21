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

//! Throwaway M1 baseline helper: dump the full logical storage key set for a
//! repo, one key per line, sorted. Not part of the product; used to capture
//! pre/post layout-v2 comparison anchors. Safe to delete after M1.
//!
//! Usage: cargo run -p mediagit-cli --example dump_keys -- <repo_root>

use std::path::PathBuf;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let repo_root: PathBuf = std::env::args()
        .nth(1)
        .expect("usage: dump_keys <repo_root>")
        .into();
    let storage = mediagit_cli::repo::create_storage_backend(&repo_root).await?;
    let mut keys = storage.list_objects("").await?;
    keys.sort();
    for k in keys {
        println!("{}", k);
    }
    Ok(())
}

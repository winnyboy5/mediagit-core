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

//! Centralized binary serialization for all on-disk formats.
//!
//! Uses postcard (serde-compatible, stable wire format since v1.0) for all
//! object serialization. Clean break from bincode 1.x.

/// Serialize a value to postcard bytes.
pub fn serialize<T: serde::Serialize>(value: &T) -> anyhow::Result<Vec<u8>> {
    postcard::to_allocvec(value).map_err(|e| anyhow::anyhow!("Serialization error: {}", e))
}

/// Deserialize a value from postcard bytes.
pub fn deserialize<T: for<'de> serde::Deserialize<'de>>(data: &[u8]) -> anyhow::Result<T> {
    postcard::from_bytes(data).map_err(|e| anyhow::anyhow!("Deserialization error: {}", e))
}

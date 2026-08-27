// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

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

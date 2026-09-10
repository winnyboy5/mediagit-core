// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

#![allow(missing_docs)]
//! MediaGit network protocol implementation
//!
//! This crate provides client and server-side components for the MediaGit
//! network protocol, enabling push/pull operations between repositories.

/// Install ring as the process-level rustls crypto provider, once.
///
/// reqwest is built with the `rustls-no-provider` feature because the
/// workspace standardises on **ring** and reqwest 0.13's plain `rustls`
/// feature hard-wires aws-lc-rs. With no provider feature of its own, reqwest
/// panics on `Client` construction unless a process default is already
/// installed — so every path that may build a client calls this first.
///
/// This used to work by accident: `google-cloud-auth`'s default features
/// enabled reqwest's aws-lc-backed `rustls`, which silently supplied a
/// provider (and quietly did the TLS). Removing aws-lc-rs from the tree made
/// the dependency explicit, which is where it belongs.
///
/// Idempotent and safe to call from anywhere, including concurrently.
pub fn ensure_crypto_provider() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // Err means another entry point installed one first — fine either way.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

pub mod bench;
pub mod client;
pub mod error_class;
pub mod journal;
pub mod pack_builder;
pub mod streaming;
pub mod types;

// Re-export commonly used types
pub use client::{
    Credentials, LockInfo, ProtocolClient, PushPhase, PushProgress, PushStats, RepairReport,
};
pub use streaming::{
    DownloadConfig, DownloadHandle, StreamingDownloader, StreamingUploader, TransferProgress,
    UploadConfig, UploadHandle,
};
pub use types::{
    RefInfo, RefUpdate, RefUpdateRequest, RefUpdateResponse, RefUpdateResult, RefsResponse,
    WantRequest, WantResponse,
};

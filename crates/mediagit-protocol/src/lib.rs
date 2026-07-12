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

#![allow(missing_docs)]
//! MediaGit network protocol implementation
//!
//! This crate provides client and server-side components for the MediaGit
//! network protocol, enabling push/pull operations between repositories.

pub mod adaptive_config;
pub mod bench;
pub mod client;
pub mod error_class;
pub mod journal;
pub mod pack_builder;
pub mod streaming;
pub mod types;

// Re-export commonly used types
pub use client::{Credentials, ProtocolClient, PushPhase, PushProgress, PushStats};
pub use streaming::{
    DownloadConfig, DownloadHandle, StreamingDownloader, StreamingUploader, TransferProgress,
    UploadConfig, UploadHandle,
};
pub use types::{
    RefInfo, RefUpdate, RefUpdateRequest, RefUpdateResponse, RefUpdateResult, RefsResponse,
    WantRequest, WantResponse,
};

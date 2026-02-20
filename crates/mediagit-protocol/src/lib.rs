// Copyright (C) 2026  winnyboy5
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//! MediaGit network protocol implementation
//!
//! This crate provides client and server-side components for the MediaGit
//! network protocol, enabling push/pull operations between repositories.

pub mod adaptive_config;
pub mod client;
pub mod streaming;
pub mod types;

// Re-export commonly used types
pub use client::{ProtocolClient, PushPhase, PushProgress, PushStats};
pub use streaming::{
    DownloadConfig, DownloadHandle, StreamingDownloader, StreamingUploader, TransferProgress,
    UploadConfig, UploadHandle,
};
pub use types::{
    RefInfo, RefUpdate, RefUpdateRequest, RefUpdateResponse, RefUpdateResult, RefsResponse,
    WantRequest, WantResponse,
};

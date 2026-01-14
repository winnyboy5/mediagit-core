//! MediaGit network protocol implementation
//!
//! This crate provides client and server-side components for the MediaGit
//! network protocol, enabling push/pull operations between repositories.

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

//! MediaGit network protocol implementation
//!
//! This crate provides client and server-side components for the MediaGit
//! network protocol, enabling push/pull operations between repositories.

pub mod client;
pub mod types;

// Re-export commonly used types
pub use client::ProtocolClient;
pub use types::{
    RefInfo, RefUpdate, RefUpdateRequest, RefUpdateResponse, RefUpdateResult, RefsResponse,
    WantRequest,
};

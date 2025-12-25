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

//! Versioning and object database for MediaGit
//!
//! This crate implements the core version control functionality:
//! - Content-addressable object database with SHA-256 addressing
//! - Automatic content deduplication
//! - LRU caching for performance
//! - Observable metrics for deduplication efficiency
//!
//! # Architecture
//!
//! The object database (ODB) provides Git-compatible content-addressable storage:
//!
//! - **Content Addressing**: Objects are identified by SHA-256 hash of their content
//! - **Automatic Deduplication**: Identical content is stored only once
//! - **LRU Caching**: Frequently accessed objects cached with Moka
//! - **Pluggable Storage**: Works with any `StorageBackend` implementation
//!
//! # Examples
//!
//! ```no_run
//! use mediagit_versioning::{ObjectDatabase, ObjectType};
//! use mediagit_storage::LocalBackend;
//! use std::sync::Arc;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // Create object database with local storage
//!     let storage = Arc::new(LocalBackend::new("/tmp/mediagit-odb")?);
//!     let odb = ObjectDatabase::new(storage, 1000);
//!
//!     // Write an object
//!     let data = b"Hello, MediaGit!";
//!     let oid = odb.write(ObjectType::Blob, data).await?;
//!     println!("Stored object: {}", oid);
//!
//!     // Read it back
//!     let retrieved = odb.read(&oid).await?;
//!     assert_eq!(retrieved, data);
//!
//!     // Check metrics
//!     let metrics = odb.metrics().await;
//!     println!("Cache hit rate: {:.1}%", metrics.hit_rate() * 100.0);
//!     println!("Dedup ratio: {:.1}%", metrics.dedup_ratio() * 100.0);
//!
//!     Ok(())
//! }
//! ```

mod branch;
mod checkout;
mod chunking;
mod commit;
mod config;
mod conflict;
mod delta;
mod diff;
mod fsck;
mod similarity;
mod index;
mod lca;
mod merge;
mod metrics;
mod object;
mod odb;
mod oid;
mod pack;
mod refs;
mod revision;
mod tree;

pub use branch::{BranchInfo, BranchManager, DetachedHead};
pub use checkout::{CheckoutManager, CheckoutStats};
pub use chunking::{
    ChunkId, ChunkManifest, ChunkRef, ChunkStore, ChunkStoreStats, ChunkStrategy, ChunkType,
    ContentChunk, ContentChunker,
};
pub use commit::{Commit, Signature};
pub use config::{ChunkingStrategyConfig, StorageConfig};
pub use conflict::{Conflict, ConflictDetector, ConflictSide, ConflictStats, ConflictType};
pub use delta::{Delta, DeltaDecoder, DeltaEncoder, DeltaInstruction};
pub use diff::{ModifiedEntry, ThreeWayDiff, TreeDiff, TreeDiffer};
pub use index::{Index, IndexEntry};
pub use similarity::{ObjectMetadata, SimilarityDetector, SimilarityScore};
pub use lca::{LcaFinder, LcaResult};
pub use merge::{FastForwardInfo, MergeEngine, MergeResult, MergeStrategy};
pub use metrics::OdbMetrics;
pub use object::ObjectType;
pub use odb::{ObjectDatabase, RepackStats};
pub use oid::Oid;
pub use pack::{PackHeader, PackIndex, PackMetadata, PackObjectEntry, PackReader, PackWriter};
pub use refs::{normalize_ref_name, Ref, RefDatabase, RefType};
pub use revision::resolve_revision;
pub use tree::{FileMode, Tree, TreeEntry};

// Re-export fsck module
pub use fsck::{
    FsckChecker, FsckIssue, FsckOptions, FsckRepair, FsckReport, IssueCategory, IssueSeverity,
};

#[cfg(test)]
mod tests {
    #[test]
    fn versioning_compiles() {
        // Foundation test
    }
}

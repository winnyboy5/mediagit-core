// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

#![allow(missing_docs)]
//! Versioning and object database for MediaGit
//!
//! This crate implements the core version control functionality:
//! - Content-addressable object database with BLAKE3 addressing
//! - Automatic content deduplication
//! - LRU caching for performance
//! - Observable metrics for deduplication efficiency
//!
//! # Architecture
//!
//! The object database (ODB) provides Git-compatible content-addressable storage:
//!
//! - **Content Addressing**: Objects are identified by BLAKE3 hash of their content
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
//!     let storage: Arc<dyn mediagit_storage::StorageBackend> =
//!         Arc::new(LocalBackend::new("/tmp/mediagit-odb").await?);
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

pub mod hash;

pub mod add_phases;
pub mod atomic_write;
mod bitmap;
mod branch;
mod checkout;
pub mod chunking;
mod commit;
mod config;
mod conflict;
mod delta;
mod diff;
pub mod format;
pub mod fsck;
mod index;
mod lca;
mod merge;
mod metrics;
mod object;
mod odb;
mod oid;
mod pack;
pub mod reachability;
mod reflog;
mod refs;
mod revision;
mod similarity;
mod sparse;
mod streaming_index;
mod streaming_pack;
mod tag_object;
mod transaction;
mod tree;

/// Test-only view of `read_to_file`'s read-ahead sizing.
///
/// Exposed because the bug worth pinning is arithmetic, not behaviour: a fixed
/// read-ahead COUNT silently costs memory proportional to chunk size, and chunk
/// size is tuned to file size. Proving that at runtime would mean pushing
/// hundreds of MB through a unit test; proving the sizing function is cheap.
#[doc(hidden)]
pub fn checkout_chunk_prefetch_for_test(total_bytes: u64, chunk_count: usize) -> usize {
    odb::chunks::checkout_chunk_prefetch_for(total_bytes, chunk_count)
}

pub use bitmap::{ReachabilityBitmap, bitmap_enabled, bitmap_key};
pub use branch::{BranchInfo, BranchManager, DetachedHead};
pub use checkout::{CheckoutManager, CheckoutStats, FreshCheckoutPlan};
pub use chunking::{
    ChunkId, ChunkManifest, ChunkRef, ChunkStore, ChunkStoreStats, ChunkStrategy, ChunkType,
    CodecHint, ContentChunk, ContentChunker,
};
pub use commit::{Commit, Signature};
pub use config::{ChunkingStrategyConfig, StorageConfig};
pub use conflict::{Conflict, ConflictDetector, ConflictSide, ConflictStats, ConflictType};
pub use delta::{Delta, DeltaDecoder, DeltaEncoder};
pub use diff::{ModifiedEntry, ThreeWayDiff, TreeDiff, TreeDiffer};
pub use index::{Index, IndexEntry, is_stage_debris_key};
pub use lca::{LcaFinder, LcaResult};
pub use merge::{FastForwardInfo, MergeEngine, MergeResult, MergeStrategy, apply_merge_to_workdir};
pub use metrics::OdbMetrics;
pub use object::ObjectType;
pub use odb::{ObjectDatabase, RepackStats};
pub use oid::{Oid, StorageKey};
pub use pack::{
    DEFAULT_PACK_BYTES, DEFAULT_PACK_CHUNKS, PackHeader, PackIndex, PackKind, PackMetadata,
    PackObjectEntry, PackReader, PackWriter, pack_bytes_cap, pack_chunks_cap,
};
pub use reachability::walk_reachable;
pub use reflog::{Reflog, ReflogEntry};
pub use refs::{Ref, RefDatabase, RefType, normalize_ref_name};
pub use revision::resolve_revision;
pub use similarity::{ObjectMetadata, SimilarityDetector, SimilarityScore};
pub use sparse::{SparseFilter, SparseMode};
pub use streaming_index::StreamingPackIndex;
pub use streaming_pack::{
    CloudChunkLoc, CloudPackResult, StreamingPackReader, StreamingPackWriter,
};
pub use tag_object::Tag;
pub use transaction::PackTransaction;
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

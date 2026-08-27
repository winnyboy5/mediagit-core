// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! P4a: pHash-guided delta-base nomination for images.
//!
//! Perceptual hashing is advisory only: it nominates which prior image
//! object to try as a delta base — it never decides whether the delta is
//! actually used. That decision still belongs entirely to
//! `ObjectDatabase::write_delta_against_base`'s existing cycle/depth guards
//! and 80%-of-original size gate. A bad nomination costs one failed delta
//! attempt and never produces a wrong result.
//!
//! Kill switch: `MEDIAGIT_PHASH=0` disables everything (no hashing, no
//! index reads/writes) — restores exact pre-P4a add behavior for images.

use mediagit_media::phash::PerceptualHasher;
use mediagit_versioning::Oid;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tokio::sync::Mutex;
use tracing::debug;

/// Index file format version. Bump on any layout change; readers reject
/// unknown versions by treating the file as absent (empty index).
const FORMAT_VERSION: u8 = 1;

/// Hamming distance (out of 64 bits — PerceptualHasher::new()'s default
/// hash size) at or below which a prior entry nominates its object as a
/// delta base. This is looser than `PerceptualHasher::are_similar()`'s
/// ~0.85-similarity (~10-bit) threshold on purpose: a nomination only costs
/// a single extra delta attempt, and the 80% size gate is the real
/// backstop, not this distance.
const NOMINATION_MAX_DISTANCE: u32 = 10;

fn phash_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("MEDIAGIT_PHASH").ok().as_deref() != Some("0"))
}

fn max_hash_bytes() -> u64 {
    static MAX_MB: OnceLock<u64> = OnceLock::new();
    *MAX_MB.get_or_init(|| {
        std::env::var("MEDIAGIT_PHASH_MAX_MB")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(64)
    }) * 1024
        * 1024
}

/// Image extensions pHash will hash. Must match the codec features the
/// `image` crate is compiled with in mediagit-media/Cargo.toml (png, jpeg,
/// tiff, webp — no bmp, that codec feature isn't enabled there).
pub fn is_hashable_image(filename: &str) -> bool {
    let ext = Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    matches!(
        ext.to_lowercase().as_str(),
        "jpg" | "jpeg" | "png" | "webp" | "tiff" | "tif"
    )
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
struct Entry {
    hash: u64,
    oid: [u8; 32],
}

#[derive(Default)]
struct PhashIndex {
    entries: Vec<Entry>,
}

impl PhashIndex {
    fn index_path(repo_root: &Path) -> PathBuf {
        repo_root.join(".mediagit").join("phash.idx")
    }

    /// Load the index from disk. Missing, corrupt, or wrong-version file →
    /// empty index (logged at debug, never an error) — it rebuilds itself
    /// over time as adds append fresh entries.
    async fn load(repo_root: &Path) -> Self {
        let path = Self::index_path(repo_root);
        let bytes = match tokio::fs::read(&path).await {
            Ok(b) => b,
            Err(_) => return Self::default(),
        };

        if bytes.is_empty() || bytes[0] != FORMAT_VERSION {
            debug!(path = %path.display(), "phash index missing/wrong version byte, starting empty");
            return Self::default();
        }

        match postcard::from_bytes::<Vec<Entry>>(&bytes[1..]) {
            Ok(entries) => Self { entries },
            Err(e) => {
                debug!(error = %e, path = %path.display(), "phash index corrupt, starting empty");
                Self::default()
            }
        }
    }

    /// Nearest prior entry by Hamming distance; `None` if the index is
    /// empty or the best match exceeds `NOMINATION_MAX_DISTANCE`.
    fn nearest(&self, hash: u64) -> Option<Oid> {
        self.entries
            .iter()
            .map(|e| (e, (e.hash ^ hash).count_ones()))
            .min_by_key(|(_, distance)| *distance)
            .filter(|(_, distance)| *distance <= NOMINATION_MAX_DISTANCE)
            .map(|(e, _)| Oid::from_bytes(e.oid))
    }

    /// Atomic write: serialize to a temp file, then rename over the real
    /// path. The format-version byte is the guard checked on load, and it
    /// is only ever visible via the atomic rename — a reader never observes
    /// a partially-written file.
    async fn save(&self, repo_root: &Path) -> anyhow::Result<()> {
        let path = Self::index_path(repo_root);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let mut bytes = vec![FORMAT_VERSION];
        bytes.extend(postcard::to_allocvec(&self.entries)?);

        let tmp_path = path.with_extension("idx.tmp");
        tokio::fs::write(&tmp_path, &bytes).await?;
        tokio::fs::rename(&tmp_path, &path).await?;
        Ok(())
    }
}

/// Process-wide index cache: loaded lazily (on first image add in this
/// process) and mutex-serialized so concurrent add tasks (see
/// `AddCmd`'s parallel file processing) don't race and lose each other's
/// appends when reading-modifying-writing the index file.
static INDEX: OnceLock<Mutex<Option<PhashIndex>>> = OnceLock::new();

fn index_lock() -> &'static Mutex<Option<PhashIndex>> {
    INDEX.get_or_init(|| Mutex::new(None))
}

/// Result of hashing an image and checking the index for a nomination.
pub struct PhashLookup {
    pub hash: u64,
    pub nominated_base: Option<Oid>,
}

/// Hash `data` (already known to be a supported image file) and look up a
/// delta-base nomination against the persisted index.
///
/// Returns `None` on the kill switch, an oversized file, or a decode
/// failure — all silent (debug log only). pHash is best-effort: any of
/// these just means "no nomination this time", never an add failure.
pub async fn compute_and_nominate(repo_root: &Path, data: &[u8]) -> Option<PhashLookup> {
    if !phash_enabled() {
        return None;
    }
    if data.len() as u64 > max_hash_bytes() {
        return None;
    }

    let hasher = PerceptualHasher::new();
    let hash = match hasher.hash(data).await {
        Ok(h) => h,
        Err(e) => {
            debug!(error = %e, "phash decode failed, skipping nomination");
            return None;
        }
    };

    let hash_u64 = match <[u8; 8]>::try_from(hash.hash.as_slice()) {
        Ok(b) => u64::from_be_bytes(b),
        Err(_) => {
            debug!(len = hash.hash.len(), "unexpected phash width, skipping");
            return None;
        }
    };

    let mut guard = index_lock().lock().await;
    if guard.is_none() {
        *guard = Some(PhashIndex::load(repo_root).await);
    }
    let nominated_base = guard.as_ref().and_then(|idx| idx.nearest(hash_u64));

    Some(PhashLookup {
        hash: hash_u64,
        nominated_base,
    })
}

/// Append `(hash, oid)` to the index after a successful add. No-op if the
/// kill switch is set. Best-effort: a failed save is logged at debug and
/// otherwise ignored — the index is advisory, never load-bearing.
pub async fn record(repo_root: &Path, hash: u64, oid: Oid) {
    if !phash_enabled() {
        return;
    }

    let mut guard = index_lock().lock().await;
    if guard.is_none() {
        *guard = Some(PhashIndex::load(repo_root).await);
    }
    let idx = guard.as_mut().expect("just initialized above");
    idx.entries.push(Entry {
        hash,
        oid: *oid.as_bytes(),
    });

    if let Err(e) = idx.save(repo_root).await {
        debug!(error = %e, "failed to persist phash index, will retry on next add");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn oid_from_byte(b: u8) -> Oid {
        Oid::from_bytes([b; 32])
    }

    #[test]
    fn test_is_hashable_image() {
        assert!(is_hashable_image("photo.jpg"));
        assert!(is_hashable_image("photo.JPEG"));
        assert!(is_hashable_image("photo.png"));
        assert!(is_hashable_image("photo.webp"));
        assert!(is_hashable_image("photo.tiff"));
        assert!(is_hashable_image("photo.tif"));
        assert!(!is_hashable_image("photo.bmp")); // image crate has no bmp feature enabled
        assert!(!is_hashable_image("video.mp4"));
        assert!(!is_hashable_image("noext"));
    }

    #[test]
    fn test_nearest_picks_closest_within_threshold() {
        // Target hash = 0. Distances: A=1 bit, B=2 bits, C=64 bits (all differ).
        // The nearest (A) must win even though B is also within threshold.
        let idx = PhashIndex {
            entries: vec![
                Entry {
                    hash: 0b011, // distance 2
                    oid: *oid_from_byte(2).as_bytes(),
                },
                Entry {
                    hash: 0b001, // distance 1 — closest
                    oid: *oid_from_byte(1).as_bytes(),
                },
                Entry {
                    hash: u64::MAX, // distance 64 — far beyond threshold
                    oid: *oid_from_byte(3).as_bytes(),
                },
            ],
        };

        let nearest = idx.nearest(0);
        assert_eq!(nearest, Some(oid_from_byte(1)));
    }

    #[test]
    fn test_nearest_none_when_beyond_threshold() {
        let idx = PhashIndex {
            entries: vec![Entry {
                hash: 0,
                oid: *oid_from_byte(1).as_bytes(),
            }],
        };
        // All 64 bits differ - distance 64, way beyond NOMINATION_MAX_DISTANCE
        assert!(idx.nearest(u64::MAX).is_none());
    }

    #[test]
    fn test_nearest_empty_index() {
        let idx = PhashIndex::default();
        assert!(idx.nearest(12345).is_none());
    }

    #[tokio::test]
    async fn test_index_round_trip() {
        let tmp = TempDir::new().unwrap();
        let repo_root = tmp.path();

        let mut idx = PhashIndex::default();
        idx.entries.push(Entry {
            hash: 0xdead_beef,
            oid: *oid_from_byte(42).as_bytes(),
        });
        idx.entries.push(Entry {
            hash: 0x1234_5678,
            oid: *oid_from_byte(7).as_bytes(),
        });
        idx.save(repo_root).await.unwrap();

        let loaded = PhashIndex::load(repo_root).await;
        assert_eq!(loaded.entries.len(), 2);
        assert_eq!(loaded.entries[0].hash, 0xdead_beef);
        assert_eq!(loaded.entries[1].hash, 0x1234_5678);
    }

    #[tokio::test]
    async fn test_index_load_missing_file_is_empty() {
        let tmp = TempDir::new().unwrap();
        let loaded = PhashIndex::load(tmp.path()).await;
        assert_eq!(loaded.entries.len(), 0);
    }

    #[tokio::test]
    async fn test_index_load_corrupt_file_is_empty() {
        let tmp = TempDir::new().unwrap();
        let path = PhashIndex::index_path(tmp.path());
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, b"\x01not valid postcard data at all!!")
            .await
            .unwrap();

        let loaded = PhashIndex::load(tmp.path()).await;
        assert_eq!(loaded.entries.len(), 0);
    }

    #[tokio::test]
    async fn test_index_load_wrong_version_is_empty() {
        let tmp = TempDir::new().unwrap();
        let path = PhashIndex::index_path(tmp.path());
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();

        let mut idx = PhashIndex::default();
        idx.entries.push(Entry {
            hash: 1,
            oid: *oid_from_byte(1).as_bytes(),
        });
        let mut bytes = postcard::to_allocvec(&idx.entries).unwrap();
        bytes.insert(0, 99); // wrong version byte
        tokio::fs::write(&path, &bytes).await.unwrap();

        let loaded = PhashIndex::load(tmp.path()).await;
        assert_eq!(loaded.entries.len(), 0);
    }
}

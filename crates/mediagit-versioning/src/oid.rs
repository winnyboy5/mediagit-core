// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Object Identifier (OID) for content-addressable storage
//!
//! An OID is a BLAKE3 hash of an object's content, providing:
//! - Unique identification of objects
//! - Automatic content deduplication
//! - Content verification capability

use crate::hash::Hasher;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Object Identifier - BLAKE3 hash of object content
///
/// The OID is a 32-byte (256-bit) BLAKE3 hash that uniquely identifies
/// an object by its content. This provides automatic deduplication: identical
/// content produces identical OIDs.
///
/// # Examples
///
/// ```
/// use mediagit_versioning::Oid;
///
/// let data = b"Hello, World!";
/// let oid = Oid::hash(data);
/// println!("OID: {}", oid);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Oid([u8; 32]);

/// Buffer size at or above which `Oid::hash` switches to BLAKE3's tree-parallel
/// hashing.
///
/// B1. Measured on this machine (20 cores, release build, 25 reps, see the
/// `b1_rayon_threshold_measurement` test below which can be re-run anywhere):
///
/// ```text
/// size      sequential   rayon      speedup
///  64 KiB     0.0147ms   0.0585ms     0.25x   <-- 4x SLOWER
/// 128 KiB     0.0338ms   0.0316ms     1.07x   <-- noise
/// 256 KiB     0.0635ms   0.0380ms     1.67x
///   1 MiB     0.2366ms   0.0855ms     2.77x
///   4 MiB     0.9022ms   0.1584ms     5.70x
///  16 MiB     3.6655ms   0.4320ms     8.48x
/// ```
///
/// Blanket-enabling rayon would make every small hash 4x slower, and small
/// hashes are the common case (refs, metadata, tree entries). 128 KiB is the
/// true break-even but wins there are inside the noise, so the threshold sits
/// at 256 KiB where the gain is unambiguous.
///
/// CAVEAT, deliberately not designed around: those numbers are one hash at a
/// time on an otherwise idle machine. The download and add paths already hash
/// many chunks concurrently, and concurrent callers contend for the same global
/// rayon pool, so the aggregate gain under real load will be smaller than the
/// single-shot figures — possibly much smaller when the cores are already busy.
/// The threshold is chosen to never LOSE, which holds either way; the size of
/// the win is what varies.
///
/// Output is byte-identical in both modes: BLAKE3's tree-hash spec guarantees
/// sequential and parallel agree, which `hash_is_identical_either_side_of_the_threshold`
/// asserts rather than assumes.
const RAYON_HASH_THRESHOLD_BYTES: usize = 256 * 1024;

/// Compile-time guard on the measured decision above.
///
/// rayon was 4x SLOWER at 64 KiB on the reference machine, so a threshold that
/// drifted below the break-even would be a silent performance regression on the
/// common small-hash case (refs, metadata, tree entries). A `const` assertion
/// rather than a test: this cannot be skipped by a filtered run, and it fails
/// the build rather than a suite someone might not execute.
const _: () = assert!(
    RAYON_HASH_THRESHOLD_BYTES >= 128 * 1024,
    "RAYON_HASH_THRESHOLD_BYTES is below the measured break-even; small hashes      would get slower, not faster"
);

impl Oid {
    /// Create an OID by hashing the given data
    ///
    /// # Examples
    ///
    /// ```
    /// use mediagit_versioning::Oid;
    ///
    /// let data = b"test content";
    /// let oid = Oid::hash(data);
    /// assert_eq!(oid.to_string().len(), 64); // 32 bytes = 64 hex chars
    /// ```
    pub fn hash(data: &[u8]) -> Self {
        let mut hasher = Hasher::new();
        if data.len() >= RAYON_HASH_THRESHOLD_BYTES {
            hasher.update_rayon(data);
        } else {
            hasher.update(data);
        }
        Oid(hasher.finalize())
    }

    /// Compute OID from file using streaming hash (constant memory)
    ///
    /// This method reads the file in 64KB chunks, maintaining constant memory
    /// usage regardless of file size. Suitable for files of any size including
    /// multi-terabyte files.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the file to hash
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use mediagit_versioning::Oid;
    /// use std::path::Path;
    ///
    /// let oid = Oid::from_file(Path::new("large_video.mp4")).unwrap();
    /// println!("File OID: {}", oid);
    /// ```
    pub fn from_file<P: AsRef<std::path::Path>>(path: P) -> anyhow::Result<Self> {
        use std::io::Read;

        let mut file = std::fs::File::open(path.as_ref())?;
        let mut hasher = Hasher::new();
        let mut buffer = [0u8; 64 * 1024]; // 64KB buffer - stack allocated

        loop {
            let bytes_read = file.read(&mut buffer)?;
            if bytes_read == 0 {
                break;
            }
            hasher.update(&buffer[..bytes_read]);
        }

        Ok(Oid(hasher.finalize()))
    }

    /// Compute OID from file using async streaming hash (constant memory)
    ///
    /// Async version of `from_file` that uses tokio for non-blocking I/O.
    /// Suitable for use in async contexts where blocking I/O would be problematic.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the file to hash
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use mediagit_versioning::Oid;
    /// use std::path::Path;
    ///
    /// # async fn example() -> anyhow::Result<()> {
    /// let oid = Oid::from_file_async(Path::new("large_video.mp4")).await?;
    /// println!("File OID: {}", oid);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn from_file_async<P: AsRef<std::path::Path>>(path: P) -> anyhow::Result<Self> {
        use tokio::io::AsyncReadExt;

        let mut file = tokio::fs::File::open(path.as_ref()).await?;
        let mut hasher = Hasher::new();
        let mut buffer = vec![0u8; 64 * 1024]; // 64KB buffer - heap for async

        loop {
            let bytes_read = file.read(&mut buffer).await?;
            if bytes_read == 0 {
                break;
            }
            hasher.update(&buffer[..bytes_read]);
        }

        Ok(Oid(hasher.finalize()))
    }

    /// Compute OID using memory-mapped I/O and BLAKE3 tree-parallel hashing via rayon.
    ///
    /// Produces a byte-identical result to `from_file` / `from_file_async` — BLAKE3's
    /// tree-hash spec guarantees sequential and parallel modes agree. On multi-core hardware
    /// this is typically 2–4× faster than the sequential path for files ≥ 1 MiB.
    ///
    /// This is a blocking function; call it inside `tokio::task::spawn_blocking`.
    /// Gated by `MEDIAGIT_HASH_PARALLEL=1` at the call site.
    #[allow(unsafe_code)] // audited: read-only mmap, file not modified while mapped
    pub fn from_file_mmap_parallel<P: AsRef<std::path::Path>>(path: P) -> anyhow::Result<Self> {
        let file = std::fs::File::open(path.as_ref())?;
        // Safety: file opened read-only; mapping is read-only.
        let mmap = unsafe { memmap2::Mmap::map(&file)? };
        let mut hasher = Hasher::new();
        hasher.update_rayon(&mmap[..]);
        Ok(Oid(hasher.finalize()))
    }

    /// Create OID from raw bytes
    ///
    /// # Examples
    ///
    /// ```
    /// use mediagit_versioning::Oid;
    ///
    /// let bytes = [0u8; 32];
    /// let oid = Oid::from_bytes(bytes);
    /// assert_eq!(oid.as_bytes(), &bytes);
    /// ```
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Oid(bytes)
    }

    /// Get the raw bytes of the OID
    ///
    /// # Examples
    ///
    /// ```
    /// use mediagit_versioning::Oid;
    ///
    /// let oid = Oid::hash(b"data");
    /// let bytes = oid.as_bytes();
    /// assert_eq!(bytes.len(), 32);
    /// ```
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Convert OID to hex string
    ///
    /// # Examples
    ///
    /// ```
    /// use mediagit_versioning::Oid;
    ///
    /// let oid = Oid::hash(b"test");
    /// let hex = oid.to_hex();
    /// assert_eq!(hex.len(), 64);
    /// assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
    /// ```
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Create OID from hex string
    ///
    /// # Errors
    ///
    /// Returns error if the string is not 64 hex characters
    ///
    /// # Examples
    ///
    /// ```
    /// use mediagit_versioning::Oid;
    ///
    /// let oid1 = Oid::hash(b"test");
    /// let hex = oid1.to_hex();
    /// let oid2 = Oid::from_hex(&hex).unwrap();
    /// assert_eq!(oid1, oid2);
    /// ```
    pub fn from_hex(s: &str) -> anyhow::Result<Self> {
        if s.len() != 64 {
            anyhow::bail!("OID hex string must be 64 characters, got {}", s.len());
        }

        let bytes = hex::decode(s)?;
        if bytes.len() != 32 {
            anyhow::bail!("Decoded OID must be 32 bytes, got {}", bytes.len());
        }

        let mut oid_bytes = [0u8; 32];
        oid_bytes.copy_from_slice(&bytes);
        Ok(Oid(oid_bytes))
    }

    /// Get object path for Git-like object storage
    ///
    /// Returns path in format: `{first2hex}/{remaining62hex}`
    ///
    /// # Examples
    ///
    /// ```
    /// use mediagit_versioning::Oid;
    ///
    /// let oid = Oid::hash(b"test");
    /// let path = oid.to_path();
    /// // Format: "ab/cdef..." (first 2 hex chars / remaining 62)
    /// assert!(path.contains('/'));
    /// ```
    pub fn to_path(&self) -> String {
        let hex = self.to_hex();
        format!("{}/{}", &hex[..2], &hex[2..])
    }
}

impl fmt::Display for Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

impl fmt::Debug for Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Oid({})", self.to_hex())
    }
}

impl From<[u8; 32]> for Oid {
    fn from(bytes: [u8; 32]) -> Self {
        Oid(bytes)
    }
}

impl From<Oid> for [u8; 32] {
    fn from(oid: Oid) -> Self {
        oid.0
    }
}

/// Cloud storage key — forward-compat type for Track F cloud packs.
///
/// Today every key is `Chunk(_)`. Track F adds `Pack(_,offset,len)` so
/// server can bundle chunks into pack objects with Range-GET resolution.
/// Call-sites use `Chunk` everywhere until Track F lands — no ODB changes needed now.
#[doc = "Sole storage-key abstraction. New variants extend here for Track F."]
pub enum StorageKey {
    /// A single chunk stored as its own object (current default).
    Chunk(Oid),
    /// A byte range within a pack object (Track F / cloud-side packs).
    Pack(Oid, u64, u64), // pack-oid, byte-offset, byte-length
}

impl StorageKey {
    pub fn to_storage_path(&self) -> String {
        match self {
            StorageKey::Chunk(oid) => format!("chunks/{}", oid.to_hex()),
            StorageKey::Pack(oid, _, _) => format!("packs/{}", oid.to_hex()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_deterministic() {
        let data = b"test content";
        let oid1 = Oid::hash(data);
        let oid2 = Oid::hash(data);
        assert_eq!(oid1, oid2, "Same content should produce same OID");
    }

    #[test]
    fn test_hash_different_content() {
        let oid1 = Oid::hash(b"content1");
        let oid2 = Oid::hash(b"content2");
        assert_ne!(
            oid1, oid2,
            "Different content should produce different OIDs"
        );
    }

    #[test]
    fn test_hex_roundtrip() {
        let oid1 = Oid::hash(b"test");
        let hex = oid1.to_hex();
        let oid2 = Oid::from_hex(&hex).unwrap();
        assert_eq!(oid1, oid2, "Hex roundtrip should preserve OID");
    }

    #[test]
    fn test_hex_length() {
        let oid = Oid::hash(b"test");
        let hex = oid.to_hex();
        assert_eq!(hex.len(), 64, "BLAKE3 hex should be 64 characters");
    }

    #[test]
    fn test_invalid_hex() {
        assert!(Oid::from_hex("too_short").is_err());
        assert!(Oid::from_hex(&"z".repeat(64)).is_err());
    }

    #[test]
    fn test_path_format() {
        let oid = Oid::hash(b"test");
        let path = oid.to_path();
        let parts: Vec<&str> = path.split('/').collect();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].len(), 2);
        assert_eq!(parts[1].len(), 62);
    }

    #[test]
    fn test_display() {
        let oid = Oid::hash(b"test");
        let display = format!("{}", oid);
        assert_eq!(display.len(), 64);
        assert!(display.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_from_file_matches_hash() {
        // Create a temp file with known content
        let temp_dir = std::env::temp_dir();
        let test_path = temp_dir.join("mediagit_test_oid_streaming.bin");
        let test_data = b"Hello, World! This is test content for streaming hash.";
        std::fs::write(&test_path, test_data).expect("Failed to write test file");

        // Compute both hashes
        let memory_oid = Oid::hash(test_data);
        let file_oid = Oid::from_file(&test_path).expect("Failed to hash file");

        // Cleanup
        let _ = std::fs::remove_file(&test_path);

        // Verify they match
        assert_eq!(
            memory_oid, file_oid,
            "Streaming hash should match in-memory hash"
        );
    }

    #[test]
    fn test_from_file_empty() {
        // Test with empty file
        let temp_dir = std::env::temp_dir();
        let test_path = temp_dir.join("mediagit_test_oid_empty.bin");
        std::fs::write(&test_path, b"").expect("Failed to write empty test file");

        let memory_oid = Oid::hash(b"");
        let file_oid = Oid::from_file(&test_path).expect("Failed to hash empty file");

        let _ = std::fs::remove_file(&test_path);

        assert_eq!(
            memory_oid, file_oid,
            "Empty file hash should match empty slice hash"
        );
    }

    #[tokio::test]
    async fn test_from_file_async_matches_hash() {
        let temp_dir = std::env::temp_dir();
        let test_path = temp_dir.join("mediagit_test_oid_async.bin");
        let test_data = b"Async streaming hash test content with more data to ensure buffer works.";
        tokio::fs::write(&test_path, test_data)
            .await
            .expect("Failed to write test file");

        let memory_oid = Oid::hash(test_data);
        let file_oid = Oid::from_file_async(&test_path)
            .await
            .expect("Failed to hash file async");

        let _ = tokio::fs::remove_file(&test_path).await;

        assert_eq!(
            memory_oid, file_oid,
            "Async streaming hash should match in-memory hash"
        );
    }

    #[test]
    fn test_from_file_large_data() {
        // Test with data larger than buffer (64KB)
        let temp_dir = std::env::temp_dir();
        let test_path = temp_dir.join("mediagit_test_oid_large.bin");
        let test_data: Vec<u8> = (0..100_000).map(|i| (i % 256) as u8).collect();
        std::fs::write(&test_path, &test_data).expect("Failed to write large test file");

        let memory_oid = Oid::hash(&test_data);
        let file_oid = Oid::from_file(&test_path).expect("Failed to hash large file");

        let _ = std::fs::remove_file(&test_path);

        assert_eq!(
            memory_oid, file_oid,
            "Large file streaming hash should match in-memory hash"
        );
    }
}

/// B1 — the size gate must never change the digest.
///
/// BLAKE3's tree-hash spec guarantees sequential and parallel agree, but
/// `Oid::hash` now picks between them based on a length comparison, and a
/// digest that varies with buffer size would corrupt content addressing
/// silently and irreversibly: objects written on one side of the threshold
/// would be unfindable from the other. Asserted rather than trusted.
#[cfg(test)]
mod b1_threshold_identity_tests {
    use super::{Oid, RAYON_HASH_THRESHOLD_BYTES};

    fn sequential(data: &[u8]) -> Oid {
        let mut h = blake3::Hasher::new();
        h.update(data);
        Oid::from_bytes(*h.finalize().as_bytes())
    }

    #[test]
    fn hash_is_identical_either_side_of_the_threshold() {
        // Straddle the boundary exactly, plus sizes well clear of it in both
        // directions. The two interesting cases are threshold-1 (last
        // sequential) and threshold (first parallel).
        let sizes = [
            0usize,
            1,
            RAYON_HASH_THRESHOLD_BYTES - 1,
            RAYON_HASH_THRESHOLD_BYTES,
            RAYON_HASH_THRESHOLD_BYTES + 1,
            RAYON_HASH_THRESHOLD_BYTES * 4 + 7,
        ];
        for size in sizes {
            // Position-dependent bytes: a constant fill would hide a chunk
            // ordering fault inside the tree hash.
            let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
            assert_eq!(
                Oid::hash(&data),
                sequential(&data),
                "Oid::hash disagreed with sequential BLAKE3 at {size} bytes                  (threshold {RAYON_HASH_THRESHOLD_BYTES}). The size gate must be                  a performance choice only -- if it changes the digest, content                  addressing breaks across the boundary."
            );
        }
    }
}

#[cfg(test)]
mod b1_rayon_threshold_measurement {
    use blake3::Hasher;
    use std::time::Instant;

    /// TEMPORARY measurement, not a gate. Prints sequential vs rayon BLAKE3 for
    /// a range of buffer sizes so the B1 threshold is chosen from THIS machine's
    /// numbers rather than BLAKE3's documented ~128 KiB, which varies by CPU.
    #[test]
    #[ignore = "measurement, run explicitly"]
    fn measure_sequential_vs_rayon() {
        let sizes = [
            64 * 1024usize,
            128 * 1024,
            256 * 1024,
            1024 * 1024,
            4 * 1024 * 1024,
            16 * 1024 * 1024,
        ];
        // Enough repeats that a single scheduling hiccup does not set the verdict.
        const REPS: usize = 25;
        println!("size_kib,seq_ms,rayon_ms,speedup");
        for size in sizes {
            let data = vec![0xA5u8; size];
            // Warm caches and the rayon pool before timing either arm.
            let _ = Hasher::new().update(&data).finalize();
            let _ = Hasher::new().update_rayon(&data).finalize();

            let t = Instant::now();
            for _ in 0..REPS {
                let mut h = Hasher::new();
                h.update(&data);
                std::hint::black_box(h.finalize());
            }
            let seq = t.elapsed().as_secs_f64() * 1000.0 / REPS as f64;

            let t = Instant::now();
            for _ in 0..REPS {
                let mut h = Hasher::new();
                h.update_rayon(&data);
                std::hint::black_box(h.finalize());
            }
            let par = t.elapsed().as_secs_f64() * 1000.0 / REPS as f64;

            println!(
                "{},{:.4},{:.4},{:.2}x",
                size / 1024,
                seq,
                par,
                seq / par.max(f64::MIN_POSITIVE)
            );
        }
        println!(
            "cores={}",
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(0)
        );
    }
}

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

//! Per-extension dedup/compression/perf benchmark harness.
//!
//! Walks the corpus described by `dev-tests/dedup-corpus.txt`, runs the real
//! chunking, dedup, compression, and delta pipeline stages via the public
//! library APIs, and prints a single deterministic JSON report to stdout.
//! This is the measurement instrument that later phases of the
//! smart-media-handling work are gated against (see `dev-tests/compare_dedup.ps1`).
//!
//! Run with: `cargo run --release -p mediagit-versioning --example dedup_report`
//!
//! Env:
//!   `MEDIAGIT_BENCH_CORPUS` - override the corpus root directory that
//!   manifest paths (above the `# pairs` marker) are resolved against.
//!   Defaults to repo-root `test-files/`. The `# pairs` section is always
//!   resolved against `dev-tests/dedup-pairs/`, regardless of this override.
//!
//! Peak working set (Windows `PeakWorkingSet64`) is printed to stderr as
//! `PEAK_RSS_MB: <value>` rather than embedded in the JSON, since it is not
//! reproducible byte-for-byte between runs the way chunk/compression/delta
//! statistics are.

use anyhow::{Context, Result};
use mediagit_compression::{ObjectType as CompObjectType, SmartCompressor, TypeAwareCompressor};
use mediagit_versioning::chunking::{ChunkStrategy, ContentChunker};
use mediagit_versioning::{
    DeltaEncoder, ObjectMetadata, ObjectType as GitObjectType, Oid, SimilarityDetector,
};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Mirrors `similarity::MIN_SIMILARITY_THRESHOLD`, which lives in a private
/// module and isn't reachable from an example binary. Kept in sync manually;
/// only used to exercise the same code path as production, not to gate
/// pass/fail here (that's based on the actual delta size, see `compute_deltas`).
const MIN_SIMILARITY_THRESHOLD: f64 = 0.30;

#[derive(Debug, Default, Clone, Serialize)]
struct ExtStats {
    /// How many files contributed to this bucket.
    ///
    /// Without it, `dedup_pct` reads as a property of the *format* when it is
    /// largely a property of the *corpus*: a bucket of unpaired singles can
    /// only ever report 0 %, and one big unpaired file drags a paired bucket
    /// down. Both `psd` (0 %, no pair existed) and `mov` (12.2 %, 39.5 MB of
    /// unpaired ProRes against a 12.1 % ceiling) were misread as pipeline
    /// defects for exactly this reason.
    file_count: u64,
    total_bytes: u64,
    unique_chunk_bytes: u64,
    dedup_pct: f64,
    chunk_count: u64,
    avg_chunk_size: f64,
    post_compression_bytes: u64,
    add_ms: f64,
    delta_hit_rate: Option<f64>,
    delta_pct: Option<f64>,
}

/// Running sums per extension; converted to `ExtStats` (percentages/averages
/// computed) once the whole corpus has been processed.
#[derive(Debug, Default)]
struct ExtAccum {
    file_count: u64,
    total_bytes: u64,
    unique_chunk_bytes: u64,
    chunk_count: u64,
    post_compression_bytes: u64,
    add_ms: f64,
    delta_hits: u32,
    delta_pairs: u32,
    delta_pct_sum: f64,
}

impl ExtAccum {
    fn finalize(&self) -> ExtStats {
        let dedup_pct = if self.total_bytes > 0 {
            100.0 * (1.0 - self.unique_chunk_bytes as f64 / self.total_bytes as f64)
        } else {
            0.0
        };
        let avg_chunk_size = if self.chunk_count > 0 {
            self.total_bytes as f64 / self.chunk_count as f64
        } else {
            0.0
        };
        let (delta_hit_rate, delta_pct) = if self.delta_pairs > 0 {
            (
                Some(self.delta_hits as f64 / self.delta_pairs as f64),
                Some(self.delta_pct_sum / self.delta_pairs as f64),
            )
        } else {
            (None, None)
        };

        ExtStats {
            file_count: self.file_count,
            total_bytes: self.total_bytes,
            unique_chunk_bytes: self.unique_chunk_bytes,
            dedup_pct,
            chunk_count: self.chunk_count,
            avg_chunk_size,
            post_compression_bytes: self.post_compression_bytes,
            add_ms: self.add_ms,
            delta_hit_rate,
            delta_pct,
        }
    }

    fn add(&mut self, other: &ExtAccum) {
        self.file_count += other.file_count;
        self.total_bytes += other.total_bytes;
        self.unique_chunk_bytes += other.unique_chunk_bytes;
        self.chunk_count += other.chunk_count;
        self.post_compression_bytes += other.post_compression_bytes;
        self.add_ms += other.add_ms;
        self.delta_hits += other.delta_hits;
        self.delta_pairs += other.delta_pairs;
        self.delta_pct_sum += other.delta_pct_sum;
    }
}

#[derive(Debug, Serialize)]
struct Report {
    schema_version: u32,
    generated: String,
    corpus: String,
    totals: ExtStats,
    per_extension: BTreeMap<String, ExtStats>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let repo_root = repo_root()?;
    let manifest_path = repo_root.join("dev-tests").join("dedup-corpus.txt");
    let corpus_root = match std::env::var("MEDIAGIT_BENCH_CORPUS") {
        Ok(p) => PathBuf::from(p),
        Err(_) => repo_root.join("test-files"),
    };
    let pairs_root = repo_root.join("dev-tests").join("dedup-pairs");

    let (corpus_files, pair_files) = parse_manifest(&manifest_path)?;

    let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
    let compressor = SmartCompressor::new();
    let mut seen_chunks: HashSet<Oid> = HashSet::new();
    let mut per_ext: BTreeMap<String, ExtAccum> = BTreeMap::new();

    for rel in &corpus_files {
        let path = corpus_root.join(rel);
        process_file(
            &path,
            rel,
            &chunker,
            &compressor,
            &mut seen_chunks,
            &mut per_ext,
        )
        .await?;
    }
    for rel in &pair_files {
        let path = pairs_root.join(rel);
        process_file(
            &path,
            rel,
            &chunker,
            &compressor,
            &mut seen_chunks,
            &mut per_ext,
        )
        .await?;
    }

    compute_deltas(&pairs_root, &pair_files, &mut per_ext)?;

    let mut totals = ExtAccum::default();
    for acc in per_ext.values() {
        totals.add(acc);
    }

    let report = Report {
        schema_version: 1,
        generated: chrono::Utc::now().to_rfc3339(),
        corpus: corpus_root.display().to_string(),
        totals: totals.finalize(),
        per_extension: per_ext
            .iter()
            .map(|(k, v)| (k.clone(), v.finalize()))
            .collect(),
    };

    println!("{}", serde_json::to_string_pretty(&report)?);

    if let Some(mb) = peak_rss_mb() {
        eprintln!("PEAK_RSS_MB: {:.1}", mb);
    }

    Ok(())
}

/// Resolve the repo root from `CARGO_MANIFEST_DIR` (crates/mediagit-versioning),
/// independent of the process's current working directory.
fn repo_root() -> Result<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .context("failed to resolve repo root from CARGO_MANIFEST_DIR")
}

/// Parse the corpus manifest into (corpus-relative paths, pair-relative paths).
/// Lines after a `# pairs` marker (case-insensitive) are pair paths; all other
/// non-comment, non-blank lines are corpus paths. Order is preserved as
/// written so processing (and therefore first-seen chunk attribution) is
/// deterministic across runs.
fn parse_manifest(path: &Path) -> Result<(Vec<String>, Vec<String>)> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("reading manifest {}", path.display()))?;

    let mut corpus = Vec::new();
    let mut pairs = Vec::new();
    let mut in_pairs = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.eq_ignore_ascii_case("# pairs") {
            in_pairs = true;
            continue;
        }
        if trimmed.starts_with('#') {
            continue;
        }
        if in_pairs {
            pairs.push(trimmed.to_string());
        } else {
            corpus.push(trimmed.to_string());
        }
    }

    Ok((corpus, pairs))
}

fn extension_of(rel: &str) -> String {
    Path::new(rel)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase()
}

/// Chunk, dedup-account, and compress one file, folding results into the
/// per-extension accumulator keyed by the file's (lowercased) extension.
async fn process_file(
    path: &Path,
    rel: &str,
    chunker: &ContentChunker,
    compressor: &SmartCompressor,
    seen_chunks: &mut HashSet<Oid>,
    per_ext: &mut BTreeMap<String, ExtAccum>,
) -> Result<()> {
    let ext = extension_of(rel);
    let filename = Path::new(rel)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(rel);
    let obj_type = CompObjectType::from_path(rel);

    let data = tokio::fs::read(path)
        .await
        .with_context(|| format!("reading {}", path.display()))?;

    let start = Instant::now();
    let chunks = chunker
        .chunk(&data, filename)
        .await
        .with_context(|| format!("chunking {}", path.display()))?;

    let acc = per_ext.entry(ext).or_default();
    acc.file_count += 1;

    for chunk in &chunks {
        acc.total_bytes += chunk.size as u64;
        acc.chunk_count += 1;

        if seen_chunks.insert(chunk.id) {
            acc.unique_chunk_bytes += chunk.size as u64;
            let compressed = compressor
                .compress_typed_with_size(&chunk.data, obj_type)
                .with_context(|| format!("compressing chunk of {}", path.display()))?;
            acc.post_compression_bytes += compressed.len() as u64;
        }
    }

    acc.add_ms += start.elapsed().as_secs_f64() * 1000.0;

    Ok(())
}

/// Find `<name>_v1.<ext>` / `<name>_v2.<ext>` pairs among the pair-section
/// manifest entries and run similarity + delta encoding v2-against-v1,
/// folding the result into the same per-extension accumulator used above.
fn compute_deltas(
    pairs_root: &Path,
    pair_files: &[String],
    per_ext: &mut BTreeMap<String, ExtAccum>,
) -> Result<()> {
    let mut groups: BTreeMap<(String, String), (Option<String>, Option<String>)> = BTreeMap::new();

    for rel in pair_files {
        let p = Path::new(rel);
        let ext = extension_of(rel);
        let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if let Some(base) = stem.strip_suffix("_v1") {
            groups.entry((base.to_string(), ext)).or_default().0 = Some(rel.clone());
        } else if let Some(base) = stem.strip_suffix("_v2") {
            groups.entry((base.to_string(), ext)).or_default().1 = Some(rel.clone());
        }
    }

    for ((_, ext), (v1_rel, v2_rel)) in groups {
        let (Some(v1_rel), Some(v2_rel)) = (v1_rel, v2_rel) else {
            continue; // incomplete pair, skip
        };

        let v1_data = std::fs::read(pairs_root.join(&v1_rel))
            .with_context(|| format!("reading pair base {}", v1_rel))?;
        let v2_data = std::fs::read(pairs_root.join(&v2_rel))
            .with_context(|| format!("reading pair target {}", v2_rel))?;

        // Exercise the same similarity-detection path production code uses to
        // pick a delta base. The result isn't used to gate pass/fail here
        // (the pairs are already known to be related by construction); the
        // hit/miss decision below is based on the actual encoded delta size.
        let mut v1_meta = ObjectMetadata::new(
            Oid::hash(&v1_data),
            v1_data.len(),
            GitObjectType::Blob,
            Some(v1_rel.clone()),
        );
        v1_meta.generate_samples(&v1_data);
        let mut detector = SimilarityDetector::new(1);
        detector.add_object(v1_meta);

        let mut v2_meta = ObjectMetadata::new(
            Oid::hash(&v2_data),
            v2_data.len(),
            GitObjectType::Blob,
            Some(v2_rel.clone()),
        );
        v2_meta.generate_samples(&v2_data);
        let _ = detector.find_similar(&v2_meta, MIN_SIMILARITY_THRESHOLD);

        let delta = DeltaEncoder::encode(&v1_data, &v2_data);
        let delta_size = delta.to_bytes().len() as f64;
        let ratio_pct = if v2_data.is_empty() {
            0.0
        } else {
            100.0 * delta_size / v2_data.len() as f64
        };
        let hit = ratio_pct < 80.0;

        let acc = per_ext.entry(ext).or_default();
        acc.delta_pairs += 1;
        if hit {
            acc.delta_hits += 1;
        }
        acc.delta_pct_sum += ratio_pct;
    }

    Ok(())
}

/// Peak working set (RSS equivalent on Windows) for the current process, in MiB.
///
/// ponytail: shells out to PowerShell's `Get-Process` rather than pulling in
/// a memory-stats crate or the `windows`/`winapi` crates (none of which are
/// already dependencies of this crate). Good enough for a once-per-run,
/// informational number; upgrade if a memory-profiling need arises that this
/// can't cover.
fn peak_rss_mb() -> Option<f64> {
    let pid = std::process::id();
    let output = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!("(Get-Process -Id {}).PeakWorkingSet64", pid),
        ])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let bytes: u64 = text.trim().parse().ok()?;
    Some(bytes as f64 / (1024.0 * 1024.0))
}

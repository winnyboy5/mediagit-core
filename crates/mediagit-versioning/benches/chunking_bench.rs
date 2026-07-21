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

//! Chunking throughput baseline.
//!
//! Locks the fastcdc 3.2 throughput numbers (MediaAware + Rolling strategies)
//! over representative fixture files, BEFORE the planned P1 fastcdc upgrade.
//! Re-run this after that upgrade and compare MB/s to catch regressions.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use mediagit_versioning::chunking::{ChunkStrategy, ContentChunker};
use std::hint::black_box;
use std::path::PathBuf;
use std::time::Duration;

/// Resolve a `test-files/` path from `CARGO_MANIFEST_DIR`, independent of CWD.
fn test_file(rel: &str) -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("failed to resolve repo root from CARGO_MANIFEST_DIR")
        .join("test-files")
        .join(rel)
}

/// Representative fixtures for the throughput baseline. Chosen to stay valid
/// and bounded so the full run (2 strategies x 3 files x 10 samples) finishes
/// well under 2 minutes:
///  - video: a complete MOV, not a truncated slice of a larger file (slicing
///    mid-atom risks the MP4/MOV structure parser erroring out)
///  - creative: a PSD (exercises the creative-container chunk params)
///  - generic: a ZIP (Fixed 4MB chunking path, no structure parser involved)
fn fixtures() -> Vec<(&'static str, PathBuf)> {
    vec![
        ("video_mov", test_file("video-variants/prores-pcm.mov")),
        ("creative_psd", test_file("psd/26952784_food_flyer_19.psd")),
        (
            "generic_zip",
            test_file("1965-ac-shelby-427-cobra-sc/source/FINAL_MODEL_427.zip"),
        ),
    ]
}

fn bench_chunking_throughput(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("chunking_throughput");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(10));

    for (name, path) in fixtures() {
        let data = std::fs::read(&path)
            .unwrap_or_else(|e| panic!("failed to read fixture {}: {}", path.display(), e));
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(name)
            .to_string();
        group.throughput(Throughput::Bytes(data.len() as u64));

        let media_aware = ContentChunker::new(ChunkStrategy::MediaAware);
        group.bench_with_input(BenchmarkId::new("media_aware", name), &data, |b, data| {
            b.to_async(&rt)
                .iter(|| async { black_box(media_aware.chunk(data, &filename).await.unwrap()) });
        });

        // Fixed mid-tier params, applied uniformly across fixtures for a
        // simple apples-to-apples Rolling-strategy comparison.
        let rolling = ContentChunker::new(ChunkStrategy::Rolling {
            avg_size: 1024 * 1024,
            min_size: 512 * 1024,
            max_size: 4 * 1024 * 1024,
        });
        group.bench_with_input(BenchmarkId::new("rolling", name), &data, |b, data| {
            b.to_async(&rt)
                .iter(|| async { black_box(rolling.chunk(data, &filename).await.unwrap()) });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_chunking_throughput);
criterion_main!(benches);

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

//! Chunk-delta chain depth when similar files are written CONCURRENTLY.
//!
//! `add_of_many_similar_versions_never_exceeds_max_delta_depth` writes its
//! versions one after another and passes. QA drill A11 stages 25 progressively
//! edited files and runs a single `mediagit add`, which processes them at the
//! same time -- and that measured depths of 11, 12 and 15 against a cap of 10,
//! roughly one run in three, leaving a repository that fails fsck, push and
//! clone.
//!
//! The difference is concurrency, so that is what this reproduces: N versions
//! of one asset written through the same `ObjectDatabase` at once.

use mediagit_storage::StorageBackend;
use mediagit_storage::mock::MockBackend;
use mediagit_versioning::{ChunkStrategy, ObjectDatabase, ObjectType};
use std::collections::HashMap;
use std::sync::Arc;

/// Longest `.meta` chain on disk, measured the way the QA drill measures it:
/// follow `base:` pointers and count hops.
///
/// Returns the chain as well as its length. This failed once in 2026-08-19 at
/// depth 13 and has not been reproduced in ~24 loaded runs since, so the ONE
/// artefact a future failure leaves behind has to be worth reading: a bare
/// "depth 13" says a chain got too long, while the chain itself says which
/// chunks were involved and can be matched against the write order. Every
/// mechanism ruled out so far was ruled out by inspection, because there was
/// never any evidence to inspect.
async fn max_on_disk_depth(storage: &Arc<MockBackend>) -> (usize, Vec<String>) {
    let keys = storage.list_objects("chunk-deltas/").await.unwrap();
    let mut bases: HashMap<String, String> = HashMap::new();
    for k in keys.iter().filter(|k| k.ends_with(".meta")) {
        let bytes = storage.get(k).await.unwrap();
        let txt = String::from_utf8_lossy(&bytes);
        if let Some(base) = txt.trim().strip_prefix("base:") {
            let child = k
                .trim_start_matches("chunk-deltas/")
                .trim_end_matches(".meta")
                .to_string();
            bases.insert(child, base.to_string());
        }
    }
    let mut worst = 0usize;
    let mut worst_chain: Vec<String> = Vec::new();
    for start in bases.keys() {
        let mut seen = std::collections::HashSet::new();
        let mut cur = start.clone();
        let mut depth = 0usize;
        let mut chain = vec![cur.clone()];
        while let Some(next) = bases.get(&cur) {
            if !seen.insert(cur.clone()) {
                break;
            }
            cur = next.clone();
            chain.push(cur.clone());
            depth += 1;
        }
        if depth > worst {
            worst = depth;
            worst_chain = chain;
        }
    }
    (worst, worst_chain)
}

/// Render a chain child -> base -> ... with short oids, for a failure message.
fn render_chain(chain: &[String]) -> String {
    chain
        .iter()
        .map(|o| o.chars().take(12).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n     -> ")
}

/// Versions of one asset, each a small edit of the previous.
fn versions(count: usize, size: usize) -> Vec<Vec<u8>> {
    // Random, like the drill's fixture: incompressible content changes both
    // the CDC boundaries and what the similarity detector nominates, and the
    // regular `i % 251` pattern did not reproduce.
    let mut state = 0x9E3779B97F4A7C15u64;
    let mut content: Vec<u8> = (0..size)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 24) as u8
        })
        .collect();
    let mut out = Vec::with_capacity(count);
    for v in 0..count {
        for k in 0..4096usize {
            let idx = (v * 65536 + k) % content.len();
            content[idx] = (((v * 7 + k) & 0x7F) | 0x80) as u8;
        }
        out.push(content.clone());
    }
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_writes_of_similar_versions_never_exceed_max_delta_depth() {
    const CAP: usize = 10;

    // Repeated because the defect is intermittent -- roughly one run in three
    // at the shell level. A single iteration would pass often enough to look
    // like a fix.
    for attempt in 1..=6 {
        let storage = Arc::new(MockBackend::new());
        let odb = Arc::new(ObjectDatabase::with_optimizations(
            storage.clone(),
            10_000_000,
            Some(ChunkStrategy::MediaAware),
            true,
            0,
        ));

        let payloads = versions(25, 4 * 1024 * 1024);

        // All at once, through one ODB -- what `mediagit add` does with a
        // directory of 25 staged files.
        let mut tasks = Vec::new();
        for (i, data) in payloads.iter().enumerate() {
            let odb = Arc::clone(&odb);
            let data = data.clone();
            tasks.push(tokio::spawn(async move {
                odb.write_chunked_parallel(ObjectType::Blob, &data, &format!("asset{i}.psd"))
                    .await
                    .expect("chunked write must succeed")
            }));
        }
        let mut oids = Vec::new();
        for t in tasks {
            oids.push(t.await.expect("write task panicked"));
        }

        let delta_count = storage
            .list_objects("chunk-deltas/")
            .await
            .unwrap()
            .iter()
            .filter(|k| k.ends_with(".meta"))
            .count();
        assert!(
            delta_count > 0,
            "attempt {attempt}: no chunk deltas written — this proves nothing"
        );

        let (depth, chain) = max_on_disk_depth(&storage).await;
        assert!(
            depth <= CAP,
            "attempt {attempt}: chunk-delta chain reached depth {depth}, deeper than the \
             {CAP} `get_chunk` will reconstruct — the repository would be unpushable \
             and unclonable ({delta_count} deltas written)\n\
             offending chain (child -> base):\n     -> {}",
            render_chain(&chain)
        );

        // Depth alone is not enough: bounding a chain by losing data would
        // also satisfy the assertion above.
        for (oid, expected) in oids.iter().zip(payloads.iter()) {
            let got = odb
                .read(oid)
                .await
                .unwrap_or_else(|e| panic!("attempt {attempt}: {oid} unreadable: {e}"));
            assert_eq!(
                &got, expected,
                "attempt {attempt}: {oid} round-tripped wrong"
            );
        }
    }
}

/// `write_chunk_delta` (the pull/clone ingest path) must respect the depth cap
/// the add path respects.
///
/// **What this pins, honestly:** the cap on that path, nothing more. It passes
/// against the pre-fix code as well -- checked -- because the old hand-rolled
/// guard walked *storage*, and by then the `.meta` sidecars were on disk, so it
/// reached the same verdict sequentially. What the fix changed is that the
/// decision and the write are now atomic and the edge is registered in the
/// in-memory graph, restoring the documented "in-memory edges superset of
/// on-disk edges" invariant. Demonstrating a failure from the missing
/// registration needs a concurrent second writer using the in-memory guard
/// while a pull is in flight, which this does not construct.
///
/// It is kept anyway: it is a real regression guard for the cap on a path that
/// had no test at all, and the shape it covers -- a clone hanging one delta
/// after another off a growing chain -- is exactly how the reported
/// unpushable-repository bug arrived.
#[tokio::test(flavor = "multi_thread")]
async fn pull_side_delta_writes_respect_the_depth_cap() {
    use mediagit_versioning::Oid;

    let storage = Arc::new(MockBackend::new());
    let odb = ObjectDatabase::with_optimizations(
        storage.clone(),
        10_000_000,
        Some(ChunkStrategy::MediaAware),
        true,
        0,
    );

    // A root chunk, then a chain hung off it one `write_chunk_delta` at a time
    // -- what a clone does when the server reports each chunk as a delta.
    let root_payload = vec![0xABu8; 64 * 1024];
    let root = odb
        .write(ObjectType::Blob, &root_payload)
        .await
        .expect("root write");

    let mut prev = root;
    let mut accepted = 0usize;
    for i in 0..40u32 {
        // The payload is irrelevant to the guard; the base pointer is the point.
        let child = Oid::hash(&[&i.to_le_bytes()[..], b"child"].concat());
        match odb.write_chunk_delta(&child, &prev, b"delta-bytes").await {
            Ok(()) => {
                accepted += 1;
                prev = child;
            }
            // Refusing is the correct outcome once the cap is reached.
            Err(_) => break,
        }
    }

    assert!(
        accepted > 0,
        "no delta was accepted at all — this test is measuring nothing"
    );
    let (depth, chain) = max_on_disk_depth(&storage).await;
    assert!(
        depth <= 10,
        "pull-side chunk-delta chain reached depth {depth} against a cap of 10 \
         ({accepted} accepted) — a cloned repository would be unpushable\n\
         offending chain (child -> base):\n     -> {}",
        render_chain(&chain)
    );
}

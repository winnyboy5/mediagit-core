# MediaGit Core — Refactor Review Report  (2026-06-01)

## Baseline

- **Branch:** `refactor/god-files-split` (cut from `feat/cloud-packs`, c5a47c8)
- **Tier-0 results:**
  - `cargo build --workspace --all-features`: **PASS**
  - `cargo test --workspace --all-features`: **PASS** — 291+ unit tests passing (compression: 152, protocol: 42, versioning: 291 across 8 test binaries, 8 ignored; full integration suite at 614/614 requires external backends — MinIO/AWS/Azure/GCS — not run here)
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings`: **PASS** (0 warnings, finished in ~2m 10s from warm cache)
  - `cargo fmt --all -- --check`: **PASS**
- **Pre-existing failures:** None. All Tier-1 gates green on this branch.

---

## Per-File Analysis

### `client.rs` (`crates/mediagit-protocol/src/client.rs`)

- **Size:** 4561 lines
- **Structure:** Single `impl ProtocolClient` block from L121 to ~L4089, plus 3 private structs (L30–55), 2 public structs (`PushStats` L58, `PushProgress` L84), and 4 module-level free functions (L4091–L4553).
- **Public API:** `ProtocolClient`, `PushStats`, `PushPhase`, `PushProgress` (re-exported from `lib.rs` L30).

**Overly long methods (>100 lines):**

| Method | Approx. lines | Concern |
|---|---|---|
| `push_one_object` (L1272) | ~826 | Handles loose objects, chunked objects, pack logic, delta branching, presigned URLs — at least 4 distinct concerns in one body |
| `upload_chunked_objects` (L2098) | ~838 | Orchestrates chunk existence check, presigned upload, MPU, verify, pack push — the entire chunked push pipeline |
| `download_chunked_objects` (L3079) | ~584 | Mirror of above for pull — delta resolution, pack lookup, direct download, range-parallel download |
| `upload_chunk_mpu` (L4305, free fn) | ~249 | Multi-part upload state machine with inline retry logic |
| `pull_chunks_via_packs` (L3901) | ~190 | Pack pull + fallback to direct; mixes HTTP orchestration with chunk reassembly |
| `collect_reachable_objects` (L787) | ~161 | BFS over ODB + presigned manifest upload interleaved with reachability traversal |
| `push_full_chunks_via_packs` (L3663) | ~142 | Pack build + upload; duplicates some MPU logic from `upload_chunk_mpu` |
| `push_with_progress<F>` (L309) | ~140 | Progress callback wiring duplicated from `push` (L223) — essentially the same body |
| `download_pack_streaming` (L599) | ~117 | Streaming HTTP + range reassembly inlined |

**Smells / coupling notes:**
- `push_one_object` (L1272) is the worst offender: it decides chunked-vs-loose, calls into presigned logic, decides pack-vs-direct, and drives progress — four concerns in one method with no helper extraction.
- `PresignedPutInfo` (L30–37) has two `#[allow(dead_code)]` fields (`method`, `required_headers`). `PresignedGetInfo` (L39–47) has two more (`method`, `expires_in_secs`). These structs are populated but fields never read — presigned response parsing stores them and discards. Safe to remove the fields after split.
- `#[allow(deprecated)]` at L448 and L476 on the `pull_with_have`/`pull` entry points: these methods are deprecated but still present; their bodies delegate to the new streaming variants.
- `#[allow(clippy::type_complexity)]` at L3662 on `push_full_chunks_via_packs`: the return type is a nested tuple of `Vec`s; extract a named type during split.
- No `TODO`/`FIXME`/`HACK` comments anywhere in this file.
- No `.unwrap()` or `.expect()` in non-test production code — error propagation via `?` throughout.
- `push` (L223) and `push_with_progress` (L309) share nearly identical bodies; the only difference is the progress callback parameter. The no-callback variant calls the with-callback variant with a no-op closure — this is fine structurally but the ~140-line duplication near the call site is confusing.

**Dead code observations:**
- `PresignedPutInfo.method`, `PresignedPutInfo.required_headers`, `PresignedGetInfo.method`, `PresignedGetInfo.expires_in_secs` — suppressed with `#[allow(dead_code)]`; fields are populated from JSON but never read.

**Arch notes:**
- The file conflates three protocol layers: transport (HTTP requests), orchestration (push/pull pipelines), and pack format (pack assembly, index parsing). These map cleanly to the proposed split: `push.rs` / `pull.rs` for orchestration, `transfer.rs` for raw HTTP helpers, `packs.rs` for pack assembly/retrieval.
- Free functions `coalesce_chunk_ranges` (L4091), `download_chunk_direct` (L4124), `download_chunk_ranged` (L4215), `upload_chunk_mpu` (L4305) are stateless helpers that belong in `transfer.rs`.

**Confirmed split map:** `client/mod.rs`, `push.rs`, `pull.rs`, `transfer.rs`, `packs.rs`

---

### `odb.rs` (`crates/mediagit-versioning/src/odb.rs`)

- **Size:** 4474 lines
- **Structure:** Module-level helpers (L1–258), `ObjectDatabase` struct + `Clone` impl (L262–316), single `impl ObjectDatabase` (L317–4055), `RepackStats` struct (L4057), test module (L4075–end).
- **Public API:** `ObjectDatabase`, `RepackStats` (re-exported from `lib.rs` L110).

**Overly long methods (>100 lines):**

| Method | Approx. lines | Concern |
|---|---|---|
| `write_chunked_from_file` (L1582) | ~484 | File I/O, chunking, delta detection, parallel upload, manifest write |
| `write_chunked_parallel` (L1112) | ~470 | Same as above but from in-memory bytes; duplicates significant logic with `write_chunked_from_file` |
| `write_with_delta` (L2066) | ~193 | Delta encoding + storage; blends delta decision logic with binary writing |
| `repack` (L3807) | ~165 | Full GC/repack pipeline inline |
| `read` (L3018) | ~158 | Dispatch across loose/chunked/delta/pack paths with fallback chain |
| `chunk_delta_chain_contains_impl` (L175, free fn) | ~125 | Recursive delta chain walk; the `_impl` suffix hints at a missing public wrapper |
| `write_chunked` (L876) | ~236 | Chunking + delta probing + storage — largely duplicates `write_chunked_parallel` |
| `read_from_packs` (L2389) | ~136 | Pack index loading + offset seek + decompression |
| `get_chunk` (L3318) | ~115 | Multi-path: local cache → storage → delta reconstruction |
| `read_chunked` (L2525) | ~127 | Reassembly loop + delta fallback |
| `read_delta` (L2652) | ~? | Delta chain reconstruction |
| `read_delta_with_depth_internal` (L2765) | ~134 | Recursive delta decode; sync fn calling blocking decompression |
| `read_to_file` (L2899) | ~119 | Stream-to-file with chunked reassembly path |

**Smells / coupling notes:**
- `write_chunked` (L876, ~236 lines) and `write_chunked_parallel` (L1112, ~470 lines) and `write_chunked_from_file` (L1582, ~484 lines) form a triad with substantial duplicated logic (delta probing, chunk existence check, manifest write). They should share a common inner helper after split.
- `try_store_chunk_as_delta` (L734, ~142 lines) is a private method that does delta ratio computation, encoding, and storage atomically — a prime extraction candidate for `delta.rs`.
- `read_delta_with_depth` (L2752) / `read_delta_with_depth_internal` (L2765) are sync functions that call `tokio::task::block_in_place` internally (inferred from the pattern); the sync/async boundary is non-obvious at the call site.
- `chunk_delta_chain_contains_impl` (L175) is a top-level free function (not a method) with `_impl` suffix, suggesting a refactor was started but not completed — the public method `chunk_delta_chain_contains` (L2361) exists in the `impl` block and delegates to this free function.
- `ObjectDatabase.chunk_strategy: Option<ChunkStrategy>` — the `.unwrap()` at L955 and L1177 (production code, not in `#[cfg(test)]`) will panic if the field is `None`. No safety comment. This is the only unsafe `.unwrap()` in non-test production code across the five files.
- No `TODO`/`FIXME`/`HACK` and no `#[allow]` attributes.

**Dead code observations:**
- None suppressed. The `chunk_strategy: Option<ChunkStrategy>` unwrap risk noted above.

**Arch notes:**
- Clean conceptual split into three domains already visible: (1) object read/write core, (2) delta encode/decode, (3) chunk manifest/pack I/O. The proposed `core.rs` / `delta.rs` / `chunks.rs` split aligns with this.
- `list_pack_files` (L2372) and `read_from_packs` (L2389) belong in `chunks.rs` (pack reading is chunk-level concern).
- `repack` (L3807) and `list_loose_objects` (L4026) belong in `core.rs`.

**Confirmed split map:** `odb/mod.rs`, `core.rs`, `delta.rs`, `chunks.rs`

---

### `chunking.rs` (`crates/mediagit-versioning/src/chunking.rs`)

- **Size:** 2983 lines
- **Structure:** Module-level helpers (L1–248), `ContentChunker` impl (L251–1749), format helpers (L1750–2013), `ChunkStore` + `ChunkManifest` impl (L2015–end), tests (L2168–end).
- **Public API:** `ContentChunker`, `ChunkStore`, `ChunkManifest`, `ChunkRef`, `ChunkStoreStats`, `ContentChunk`, `ChunkStrategy`, `ChunkType`, `CodecHint` (re-exported from `lib.rs` L96–99).

**Overly long methods (>100 lines):**

| Method | Approx. lines | Concern |
|---|---|---|
| `chunk_mp4` (L992) | ~261 | Full MP4 atom parser + chunk boundary logic inline |
| `chunk_matroska` (L1253) | ~176 | EBML cluster walk + subdivision |
| `chunk_glb` (L1429) | ~157 | GLB binary/JSON section parsing |
| `chunk_file_streaming` (L313) | ~121 | Streaming chunker with progress callback; complex async generics |
| `collect_file_chunks_blocking` (L434) | ~129 | Sync blocking version of file chunking |
| `parse_avi_block_chunks` (L863) | ~129 | AVI RIFF sub-chunk parser |
| `chunk_media_aware` (L635) | ~125 | Dispatcher that calls format-specific chunkers |
| `chunk_3d_text` (L1586) | ~103 | Line-oriented 3D text format chunker |

**Smells / coupling notes:**
- `chunk_media_aware` (L635) is a dispatcher that holds format detection + per-format routing in one method; format detection belongs in `formats.rs` and the dispatch table should be a match arm, not embedded logic.
- `parse_mp4_atoms` (L1806), `read_ebml_id` (L1889), `read_ebml_size` (L1918), `parse_ebml_elements` (L1957) are free functions implementing binary parsers for specific formats. These clearly belong in a `formats.rs` module.
- `fill_coverage_gaps` (L1750) is a generic utility used by multiple format chunkers — shared helper, belongs at module level in `chunker.rs` or a common helper.
- `collect_file_chunks_blocking` (L434) duplicates the logic of `chunk_file_streaming` (L313) synchronously; both do the same chunking + hash pipeline. A common inner helper would reduce ~130 lines of duplication.
- No `TODO`/`FIXME`/`HACK` and no `#[allow]` attributes.
- All `.unwrap()` occurrences are inside `#[cfg(test)]` — clean production code.

**Dead code observations:**
- None.

**Arch notes:**
- Two concerns in one file: (1) chunking algorithms and the `ContentChunker` struct, (2) format-specific binary parsers (MP4 atoms, EBML, AVI RIFF, GLB). The proposed `chunker.rs` / `formats.rs` split is accurate.
- `ChunkStore` and `ChunkManifest` (L2015–end) are data structures unrelated to chunking algorithms; they could move to a `types.rs` in a later pass but are out of scope for this split.

**Confirmed split map:** `chunking/mod.rs`, `chunker.rs`, `formats.rs`

---

### `handlers.rs` (`crates/mediagit-server/src/handlers.rs`)

- **Size:** 3041 lines
- **Structure:** No `impl` blocks — all top-level `async fn` HTTP handlers plus storage-building helpers. No test module.
- **Public API:** All `pub async fn` handlers are registered directly in `mediagit_server/src/router.rs` (or equivalent); not re-exported via a lib — this is a binary crate (`main.rs`).

**Overly long methods (>100 lines):**

| Function | Approx. lines | Concern |
|---|---|---|
| `update_refs` (L895) | ~231 | Ref update validation + fast-forward check + ODB write + branch manager all inline |
| `download_pack` (L535) | ~226 | BFS object collection + pack serialization + streaming response |
| `build_storage_backend` (L120) | ~141 | Backend dispatch (local/MinIO/AWS/GCS/Azure) with config parsing |
| `complete_pack` (L2672) | ~114 | Pack index ingestion + chunk registration |
| `download_chunk` (L1359) | ~132 | Chunk retrieval + delta fallback + presigned redirect |
| `upload_chunk_delta` (L1611) | ~110 | Delta write + integrity check |
| `download_file_by_path` (L2468) | ~103 | Path validation + blob resolution + streaming response |
| `collect_objects_bfs` (L761) | ~102 | BFS traversal helper; inlined in handler file |

**Smells / coupling notes:**
- `build_storage_backend` (L120), `build_minio_compatible_storage` (L261), `build_aws_s3_storage` (L289) are infrastructure-construction helpers that do not belong alongside HTTP handler functions; they should move to a `storage_factory.rs` or be part of `state.rs`.
- `get_or_init_storage` (L71) and `get_or_init_odb` (L97) are lazy-init helpers for `AppState` — these are state management concerns, not request handlers.
- `update_refs` (L895, ~231 lines) mixes HTTP request parsing, authorization checks, ref validation, ODB writes, and branch manager calls. It is the only handler that touches four different subsystems.
- `collect_objects_bfs` (L761) is a pure graph traversal helper inlined in this file. It belongs in `mediagit-versioning` (already has `walk_reachable`) — worth noting as a duplication risk.
- `detect_object_type` (L1126) is a pure utility function (content-type sniffing) inlined in the handler file. Belongs in a shared utilities module.
- `parse_chunk_delta_meta` (L1549) and `validate_file_path` (L2341) are small pure helpers inlined here; fine to keep inline after split.
- No `TODO`/`FIXME`/`HACK` and no `#[allow]` attributes.
- No `.unwrap()` or `.expect()` in production code — clean error propagation throughout.

**Dead code observations:**
- None suppressed.

**Arch notes:**
- Handlers naturally cluster by domain: repo/ref operations, chunk I/O, pack transfer, browse/path operations, MPU lifecycle. The proposed split is well-justified.
- All handlers take `State(state): State<Arc<AppState>>` — straightforward to move to sub-modules; no hidden closure captures.

**Confirmed split map:** `handlers/mod.rs`, `repo.rs`, `chunks.rs`, `transfer.rs`, `browse.rs`

---

### `smart_compressor.rs` (`crates/mediagit-compression/src/smart_compressor.rs`)

- **Size:** 2264 lines
- **Structure:** `ObjectType` enum (L26), `ObjectCategory` enum (L621), `CompressionStrategy` enum (L642), `ChunkCodecHint` enum (L814), `SmartCompressor` struct (L901), multiple `impl` blocks (L909–end), large test module (L1113–end).
- **Public API:** `ObjectType`, `ObjectCategory`, `CompressionStrategy`, `ChunkCodecHint`, `SmartCompressor`, `TypeAwareCompressor` trait (re-exported from `lib.rs` L111–114).

**Overly long methods (>100 lines):**

| Method | Approx. lines | Concern |
|---|---|---|
| `ObjectType::from_extension` (L168) | ~167 | Giant match arm over hundreds of file extensions |
| `ObjectType::category` (L513) | ~148 | Giant match arm mapping types to categories |
| `ObjectType::from_magic_bytes` (L344) | ~128 | Magic byte dispatch table |
| `CompressionStrategy::for_object_type` (L661) | ~137 | Strategy selection match; duplicates some category logic |

**Smells / coupling notes:**
- `ObjectType` (L26–619, ~593 lines) is a 100+ variant enum with three large `impl` blocks (`from_extension`, `from_path`, `from_magic_bytes`, `is_already_compressed`, `category`). This single enum + methods is 25% of the file.
- `ObjectType` logically belongs in a separate `object_type.rs` module — it is imported by `odb.rs`, `chunking.rs`, and `handlers.rs` and has no dependency on `SmartCompressor`. Splitting it enables other crates to import just the type without pulling in compressor logic.
- `CompressionStrategy` has two `impl CompressionStrategy` blocks (L659 and L835) — the second block adds `for_codec_hint`. Split across the file; should be consolidated.
- `#[allow(missing_docs)]` at L24 and L619 suppresses doc warnings on the two large public enums. After split, docs should be added.
- `#[allow(clippy::unwrap_used)]` at L1113 gates the entire test module — all `unwrap()` in tests is intentional and correctly suppressed.
- No `TODO`/`FIXME`/`HACK` comments.
- No `.unwrap()` or `.expect()` in production code (the `#[allow(clippy::unwrap_used)]` at L1113 is scoped to `#[cfg(test)]`).

**Dead code observations:**
- None suppressed.

**Arch notes:**
- Three distinct concerns: (1) `ObjectType` classification (extension/magic/category), (2) `CompressionStrategy` selection logic, (3) `SmartCompressor` implementation. The proposed `object_type.rs` / `strategy.rs` / `compressor.rs` split is a clean match.
- `ChunkCodecHint` (L814) is a bridge type used by `odb.rs` → `smart_compressor.rs`; it belongs in `strategy.rs` alongside `CompressionStrategy`.

**Confirmed split map:** `smart_compressor/mod.rs`, `object_type.rs`, `strategy.rs`, `compressor.rs`

---

## Invariants for all splits

1. Multiple `impl` blocks across files in same crate — no field visibility widening (all `ObjectDatabase` fields remain `pub(crate)` or private as-is).
2. `lib.rs` `pub use` paths unchanged — no external crate import changes (consuming crates import from crate root, not from sub-module paths).
3. No logic rewrites — verbatim code moves only; no renaming, no signature changes.
4. Tier-1 gate (`cargo build`, `cargo test --lib`, `cargo clippy -- -D warnings`, `cargo fmt --check`) after every commit.
5. Cross-crate imports: `ObjectType` is re-exported from `mediagit-compression`; `odb.rs` imports it as `use mediagit_compression::ObjectType`. After `smart_compressor` split, the re-export in `lib.rs` must remain at the same path.

---

## Summary

### Pre-existing issues (from Tier-0)

None. All four Tier-1 gate commands pass cleanly on the branch HEAD (`c5a47c8`).

### Smells to address during splits

1. **`client.rs` — dead `#[allow(dead_code)]` struct fields** (`PresignedPutInfo.method`, `PresignedPutInfo.required_headers`, `PresignedGetInfo.method`, `PresignedGetInfo.expires_in_secs`): remove fields when moving structs to `transfer.rs`.
2. **`client.rs` — `#[allow(clippy::type_complexity)]` on `push_full_chunks_via_packs`**: extract a named type alias when moving to `packs.rs`.
3. **`odb.rs` — bare `.unwrap()` on `self.chunk_strategy` at L955 and L1177** (production code): add a safety comment or convert to `expect("chunk_strategy required for chunked write")` during the move to `chunks.rs`.
4. **`odb.rs` — `write_chunked` / `write_chunked_parallel` / `write_chunked_from_file` logic triad**: flag for a follow-up deduplication pass (out of scope for mechanical split, but document the shared inner helper opportunity).
5. **`smart_compressor.rs` — two disjoint `impl CompressionStrategy` blocks**: consolidate into one block in `strategy.rs`.
6. **`smart_compressor.rs` — `#[allow(missing_docs)]`** on public enums: remove suppression and add doc comments when moving to `object_type.rs`.
7. **`handlers.rs` — `build_storage_backend` / `build_minio_compatible_storage` / `build_aws_s3_storage` in handler file**: consider moving to `state.rs` or a `storage_factory.rs` in a follow-up (out of scope for this split but noted).
8. **`handlers.rs` — `collect_objects_bfs`**: potential duplication with `walk_reachable` in `mediagit-versioning`; investigate consolidation in a follow-up.

### Out of scope

- Crate decomposition (`mediagit-chunking`, `mediagit-journal`) — post-split.
- Logic optimization (delta triad deduplication in `odb.rs`, BFS deduplication in `handlers.rs`).
- `s3.rs`, `minio.rs`, `local.rs` in `mediagit-storage` — not god-files, not in this refactor.
- Any changes to `mediagit-storage/src/lib.rs`, `mediagit-storage/src/s3.rs`, `mediagit-storage/src/minio.rs` (currently modified on branch but unrelated to this refactor).
- Performance tuning.
- API surface changes.

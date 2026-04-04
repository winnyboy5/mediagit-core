# FUTURE_TODOS.md

Consolidated and **priority-ordered** registry of planned features, code-level TODOs, and
known limitations for MediaGit. Items are sourced from documentation, source code, and
historical claudedocs analyses.

> Last updated: 2026-04-03 | v0.2.6-beta.1 | Items 1 (.mediagitignore) + 4 (Streaming Format-Aware Chunker, S1–S5) **DONE**

**Priority levels:**
- **P0** — Quick win or active blocker — ≤1 day effort, implement immediately
- **P1** — High impact, near-term — 1-2 weeks, next milestone target
- **P2** — Medium impact or complex — 2-6 weeks, planned but not urgent
- **P3** — Low priority / long-term — deferred until triggered by demand

---

## Quick Reference — Priority Matrix

| # | Item | Priority | Effort | Blocks / Enables |
|---|------|----------|--------|-----------------|
| 1 | `.mediagitignore` support in `add` + `status` | ~~**P1**~~ **✅ DONE** | 2-3 days | Shipped in v0.2.6-beta.1 |
| 2 | Pack negotiation / bitmap index | **P1** | 1 wk | Incremental fetch (currently full-pack always) |
| 3 | Parallel object I/O during checkout | **P1** | 1 wk | Branch switch latency |
| 4 | Streaming format-aware chunker (MKV/MP4/GLB, S1-S5) | ~~**P1**~~ **✅ DONE** | 8-12 days | Shipped in v0.2.6-beta.1 |
| 5 | Container-aware delta (AI/PDF/INDD) | **P1** | 2-3 wk | 60-80% savings on design file edits |
| 6 | Direct file serving endpoints + `mediagit download` CLI | **P1** | 2-3 days | Web UI, CI integration, CDN |
| 7 | `mediagit media info` command | **P2** | ~200 LOC | UX for media inspection |
| 8 | Sparse checkout | **P2** | ~500 LOC | Large repos, partial working trees |
| 9 | CLI command unit tests | **P2** | Large | Test coverage completeness |
| 10 | Annotated tag objects (PGP signing) | **P2** | 1 wk | Full tag semantics |
| 11 | HTTP/3 via reqwest feature flag | **P3** | 1 day | When reqwest `http3` stabilizes (~2026 Q4) |
| 12 | Git migration tooling (re-add filter/install/track) | **P3** | 1-2 wk | When user base requests migration |
| 13 | `mediagit://` URL scheme | **P3** | 1 day | Post-HTTP/3 adoption |
| 14 | Differential checkout (only changed files) | **P3** | 1-2 wk | 70% branch switch latency reduction |
| 15 | Incremental status scan (inode/mtime cache) | **P3** | 1-2 wk | Repeated `status` performance |
| 16 | Pack file format documentation | **P3** | 0.5 day | Book completeness |
| 17 | TOML-configurable similarity thresholds | **P3** | 0.5 day | User tunability |
| 18 | Windows ARM64 native binaries | **P3** | — | Blocked on GitHub runner availability |
| 19 | macOS Metal GPU acceleration | **P3** | 2-3 wk | Apple Silicon image processing |
| 20 | Security / Audit enhancements (v0.3.0+) | **P3** | — | Compliance, SIEM |
| 21 | SSO integration, multi-region, Web UI (v1.0.0) | **P3** | — | Enterprise features |

---

## P1 — High Priority (Next Milestone)

### ~~1. `.mediagitignore` Support in `add` and `status`~~ ✅ DONE — v0.2.6-beta.1
*Source: book docs `book/src/cli/add.md:43`, `book/src/cli/status.md:382`*

**Implemented** in v0.2.6-beta.1 using the `ignore` crate. Full `.gitignore`-compatible
pattern matching: globs, directory markers, negation (`!`), comments. `add --force` bypasses
rules. `status --ignored` shows the ignored files section. Porcelain `!! path` prefix.
Integration test suite: 8 tests, all pass.

See `crates/mediagit-cli/src/ignore_rules.rs`, `add.rs`, `status.rs`,
`tests/ignore_integration_test.rs`.

---

### 2. Pack Negotiation / Bitmap Index
*Source: `crates/mediagit-protocol/src/client.rs:122`; `claudedocs/` optimization roadmap*

Pull/fetch sends an **empty have-set**, so the server always sends a full pack. For repos
with many commits, this means downloading all objects on every fetch even when the client
already has 99% of them.

**What's needed:**
- Compute local have-set (all reachable OIDs from local refs) before fetch negotiation
- Send have-set to server; server computes the minimal pack to send
- Optional: bitmap index over refs for fast "what's missing" detection

Effort: **~1 week**. Enables efficient incremental sync for teams.

---

### 3. Parallel Object I/O During Checkout
*Source: `claudedocs/` optimization roadmap; `crates/mediagit-versioning/src/checkout.rs`*

Branch switching reads tree entries **sequentially**. A `JoinSet`-based parallel read
approach (with bounded concurrency to avoid IOPS saturation) would reduce checkout latency
significantly, especially for repos with hundreds of large files.

Effort: **~1 week**.

---

### 4. Streaming Format-Aware Chunker (TB-Scale Support)
*Source: protocol R&D analysis 2026-03; `crates/mediagit-protocol/src/streaming.rs`; `docs/FUTURE_TODOS.md` §Media Chunking Optimizations*

**Problem**: Files ≥ 100MB currently get generic FastCDC chunks — no structural awareness
(MKV Cluster boundaries, MP4 atom boundaries). A 2TB MKV gets `ChunkType::Generic` for
everything, harming deduplication and delta encoding.

**Target**: ALL files ≥ 5MB get format-aware streaming with O(max_chunk_size) memory.

**Implementation (v0.2.6-beta)**: Instead of the originally planned `StreamingFormatChunker`
trait, format-aware chunking was achieved via **`memmap2` memory-mapped I/O** in
`collect_file_chunks_blocking()`. This routes files of any size through the existing
`chunk_media_aware()` parsers (MP4 sample-table, MKV EBML, AVI RIFF, GLB, FBX) without
loading the entire file into heap memory. Falls back to `StreamCDC` on mmap failure
(network/FUSE filesystems, 32-bit targets).

> **Architectural note**: The mmap approach is simpler than a streaming trait (no new
> abstraction layer, reuses all existing format parsers as-is) but requires addressable
> file access — it won't work on stdin/pipes. For the primary use case (local/NFS files),
> this is the right trade-off. A streaming trait can be added later if pipe support is needed.

**Additional capabilities built in v0.2.6-beta:**
- **`CodecHint` enum** — per-chunk codec detection (H.264, ProRes, AAC, PCM, etc.)
- **Codec-aware compression** — per-chunk optimal strategy (Store for H.264, Zstd for PCM,
  Brotli for text subtitles) via `ChunkCodecHint` + `SmartCompressor::compress_by_codec()`
- **Delta skip for high-entropy codecs** — H.264/H.265/VP9/AV1/AAC/Opus/Vorbis/MP3 chunks
  bypass delta encoding entirely, saving CPU on futile attempts
- **Adaptive delta ratio thresholds** — ProRes/DNxHR/Raw → 0.60, subtitles → 0.90

**Phased implementation status:**

| Phase | Work | Status |
|---|---|---|
| S1 | MKV/WebM EBML chunking with per-element metadata + Cluster CDC subdivision | ✅ Done (mmap) |
| S2 | MP4 mdat CDC subdivision + AVI movi CDC descent | ✅ Done (mmap) |
| S3 | Wire into `collect_file_chunks_blocking` in `odb.rs` | ✅ Done (mmap) |
| S4 | Lower `STREAMING_THRESHOLD` 100MB→5MB in `add.rs` AND `status.rs` | ✅ Done — v0.2.6-beta.3 |
| S5 | GLB BIN CDC sub-chunking (>4MB payloads) + FBX basic header extraction | ✅ Done — v0.2.6-beta.3 |


**Performance targets:**
| Metric | Current | Target |
|---|---|---|
| 2TB MKV memory | ~32MB (generic CDC) | ~96MB peak (mmap virtual, not resident) |
| 2TB MKV chunk types | 100% Generic | 95%+ Metadata/VideoStream |
| 264MB MP4 throughput | 238 MB/s | 220+ MB/s |

**Standalone Deep Test Results (v0.2.6-beta.1, 2026-04-03):**

| Metric | Result |
|---|---|
| Format tests | 36/36 passed (all fsck verified) |
| Video deep tests | 9/9 passed (MKV EBML, MOV Atom, H265, ProRes+PCM) |
| Audio deep tests | 3/3 passed (WAV 54% savings, FLAC/OGG Store correct) |
| CLI command tests | 89/91 passed |
| Server push/clone/pull | 4/4 passed |
| .mediagitignore tests | 7/7 passed |
| Overall storage savings | 26% (1.43 GB → 1.06 GB across all formats) |
| Avg add throughput | 9.3 MB/s (debug build) |
| Avg delta efficiency | 54% |

**Top storage savings:**
| Category | Format | Savings | Ratio |
|---|---|---|---|
| 3D Text | DAE / FBX-ascii | 81% | 5.27–5.37x |
| Vector | SVG | 80.8% | 5.20x |
| Creative | PSD-xl (213MB) | 70.9% | 3.44x |
| 3D Mesh | PLY / STL | 70–73% | 3.36–3.69x |
| Creative | EPS | 65.5% | 2.90x |
| Audio (uncompressed) | WAV (54MB) | 54.1% | 2.18x |
| 3D Binary | GLB (13MB) | 50.6% | 2.03x |

**Top delta efficiency:**
| Format | Efficiency | Overhead |
|---|---|---|
| GLB (13–24MB) | 100% | 3–4 KB |
| AI-lg (123MB) | 100% | 4.5 KB |
| PSD-xl (213MB) | 99.8% | 424 KB |
| WAV (54MB) | 99.8% | 139 KB |
| Archive ZIP (656MB) | 99.9% | 569 KB |

---

### 5. Container-Aware Delta Encoding for PDF/ZIP Formats [DELTA-001]
*Source: `docs/FUTURE_TODOS_2.md` — recorded 2026-03-01*

**Problem**: Adobe Illustrator (`.ai`), InDesign (`.indd`), PDF (`.pdf`) files are
PDF/ZIP containers with DEFLATE-compressed inner streams. A single-byte change causes
DEFLATE to reshuffle all subsequent bytes — the similarity detector finds near-zero
matches between versions, producing deltas nearly as large as the original.

**Current workaround** (`add.rs` `should_use_delta()`): `.ai/.ait/.indd/.idml/.pdf` skip
delta for files < 50 MB. For ≥ 50 MB, delta is attempted with partial results.

**Real-world data** (2026-03-01, 124 MB + 206 MB AI files):
- 328.90 MiB original → 238.85 MiB stored (27.4% saved, 42.7% on chunks)
- With container-aware approach: expected **60-80% savings** for typical edit workflows

**Required implementation:**
- `crates/mediagit-versioning/src/container_delta.rs` — **NEW**:
  - **PDF path**: parse xref → inflate streams → delta per stream (matched by xref ID) →
    re-deflate + re-assemble on read
  - **ZIP path** (`.indd`, `.idml`): unzip entries → delta per named entry → repack on read
- New `ObjectKind::ContainerDelta` or metadata flag in `format.rs`
- Integration into ODB write/read path

**Dependencies to evaluate**: `lopdf = "0.35"` (PDF), `zip = "2"` (ZIP),
`miniz_oxide = "0.8"` (DEFLATE).

**Risks**: PDF streams use mixed compression (JPEG2000, raw); InDesign `.indd` has
proprietary sections; **100% round-trip byte fidelity required** (reconstructed file must
open identically in Illustrator/Acrobat).

**Success criteria**: AI "move one path node" → delta < 5% of original; PDF paragraph
change → delta < 10% of original. Effort: **2-3 weeks** + extensive fidelity testing.

---

### 6. Direct File Serving Endpoints + `mediagit download` CLI
*Source: protocol R&D analysis 2026-03; `crates/mediagit-protocol/src/streaming.rs`**

New server endpoints for raw file download ("GitHub Download Raw" equivalent). Enables
web UI, CI pipelines, preview tools, and CDN integration.

**Endpoints:**

| Endpoint | Description |
|---|---|
| `GET /{repo}/files/{*path}?ref=HEAD` | Download file by path from committed state |
| `GET /{repo}/tree/{*path}?ref=HEAD` | List directory contents as JSON |
| `GET /{repo}/tree?ref=HEAD` | List root tree |

**Server changes** (`handlers.rs`):
- `resolve_path_to_blob()` — tree walk by path components → blob OID
- `download_file_by_path()` — streaming O(64KB) via `duplex` + `tokio::spawn` + `ReaderStream`
  (same pattern as `download_pack` at `handlers.rs:494-541`)
- `list_tree()` — JSON directory listing

**Phase 2** (after server endpoints): `mediagit download origin assets/logo.psd --ref main`
CLI command in `crates/mediagit-cli/src/commands/download.rs`, adapting `StreamingDownloader`
in `streaming.rs` for VCS-scoped URLs.

**Path security**: reject `..`, absolute paths, null bytes; validate against tree entry names.

Effort: **2-3 days** (server only). CLI phase: additional 2-3 days.

---

## P2 — Medium Priority (Planned, Not Urgent)

### 7. `mediagit media info` Command
*Source: `docs/FUTURE_TODOS.md` §Phase 3*

Display metadata for local/committed media files using existing parsers in `mediagit-media`.

**CLI usage**: `mediagit media info <FILES...> [--format text|json] [--verbose] [--hash]`

**Files to create/modify:**
- `crates/mediagit-cli/src/commands/media.rs` — new command (~200 LOC)
- `crates/mediagit-cli/src/commands/mod.rs`, `main.rs`, `Cargo.toml`

**Parsers to use** (all exist in `mediagit-media`):

| Parser | Returns |
|---|---|
| `VideoParser` | duration, codec, resolution, audio tracks |
| `AudioParser` | duration, sample rate, channels, codec |
| `ImageMetadataParser` | dimensions, EXIF, pHash |
| `PsdParser` | layers, dimensions, color mode |
| `Model3DParser` | vertices, faces, materials |

Effort: **~200 LOC**.

---

### 8. Sparse Checkout (`mediagit sparse-checkout`)
*Source: `docs/FUTURE_TODOS.md` §Phase 4; v0.3.0 roadmap*

Work with partial repository contents — critical for large media repos where artists need
only specific asset subdirectories.

**CLI usage:**
```
mediagit sparse-checkout init [--cone]
mediagit sparse-checkout set <PATTERNS...>
mediagit sparse-checkout add <PATTERNS...>
mediagit sparse-checkout list / reapply / disable
```

**Files to create/modify:**
- `crates/mediagit-versioning/src/sparse.rs` — `SparseCheckout` struct + pattern matching
- `crates/mediagit-versioning/src/checkout.rs` — integrate sparse filtering into `CheckoutManager`
- `crates/mediagit-cli/src/commands/sparse_checkout.rs` — CLI command
- Pattern storage: `.mediagit/info/sparse-checkout` (one pattern per line)

Two modes: **cone** (directory-prefix, efficient) and **pattern** (glob, flexible).

Effort: **~500 LOC**.

---

### 9. CLI Command Unit Tests [CLI-001]
*Source: `docs/FUTURE_TODOS_2.md` — recorded 2026-02-28*

Integration tests in `crates/mediagit-cli/tests/` cover all commands end-to-end but
require a full repository setup. Individual command modules have **no unit tests** for
argument parsing, flag interactions, or error handling in isolation.

**What's needed**: Unit tests per command using mocked storage backend
(`crates/mediagit-test-utils/src/`), testing flag combinations, edge cases, and error
paths without a real ODB/refs layer.

Start with high-complexity commands: `merge`, `rebase`, `cherry-pick`, `stash`.

Effort: **Large** — prioritize incrementally alongside feature development.

---

### 10. Annotated Tag Objects (Full PGP Signing)
*Source: `crates/mediagit-cli/src/commands/tag.rs:229`*

```rust
// TODO: Implement proper tag objects when object storage supports Tag type
```

`mediagit tag -a` creates a lightweight tag with metadata in a companion file. Full
annotated tag objects (with PGP-signing support) require:
- Adding `ObjectKind::Tag` to the ODB
- Serialization/deserialization in `mediagit-versioning/src/format.rs`
- PGP signing integration in `mediagit-security`

Effort: **~1 week**.

---

## P3 — Low Priority / Long-Term

### 11. HTTP/3 Support via reqwest Feature Flag
*Source: protocol R&D analysis 2026-03; `crates/mediagit-protocol/src/streaming.rs`*1 Phase 2*

**Trigger**: reqwest `http3` feature hitting stable/production-ready (~2026 Q3-Q4).

Zero code changes needed beyond a feature flag:
```toml
# crates/mediagit-protocol/Cargo.toml
[features]
http3 = ["reqwest/http3"]
```

reqwest handles QUIC/HTTP/3 negotiation internally via Alt-Svc discovery. Recommended
deployment: **Caddy** reverse proxy in front of Axum for HTTP/3 termination — Axum keeps
speaking HTTP/2, Caddy handles QUIC.

Native HTTP/3 in the server (using `h3` + `quinn`) should only be pursued if `h3` reaches
1.0 and Caddy becomes a bottleneck. Effort: **1 day** (when triggered).

---

### 12. Git Migration Tooling (Re-add `filter`/`install`/`track`)
*Source: `CHANGELOG.md` §Unreleased → Removed*

The `mediagit-git` crate remains in the workspace and compiles independently. Re-integration
as a first-class migration CLI flow is deferred until there is user demand.

Trigger: user requests for git/git-LFS → MediaGit migration tooling. Effort: **1-2 weeks**.

---

### 13. `mediagit://` URL Scheme
*Source: protocol R&D analysis 2026-03*

A native `mediagit://` URL scheme for brand identity, post-HTTP/3 adoption. Maps to
`https://` or `quic://` internally. Effort: **1 day** (low value until HTTP/3 is live).

---

### 14. Differential Checkout (Only Changed Files)
*Source: `claudedocs/` optimization roadmap*

Branch switching currently rewrites all files even if only a subset changed. Diffing the
source and target trees and only updating changed paths targets **~70% latency reduction**
(estimated 496ms → ~150ms for medium repos).

Requires tree diff engine in `mediagit-versioning`. Effort: **1-2 weeks**.

---

### 15. Incremental Status Scan (inode / mtime Cache)
*Source: `claudedocs/` optimization roadmap*

Full-tree scan on every `status` invocation. An inode cache / mtime-based incremental scan
(similar to git's index) would reduce repeated-status overhead significantly for repos with
large working trees.

Effort: **1-2 weeks**.

---

### 16. Pack File Format Documentation
*Source: `book/src/reference/file-formats.md:15`*

`.mediagit/objects/pack/` is reserved for future pack-file storage. The directory layout
and on-disk format are not documented in the book. Effort: **0.5 day**.

---

### 17. TOML-Configurable Similarity Thresholds
*Source: `book/src/guides/performance.md:60-65`*

Similarity thresholds (controlling when delta encoding is triggered) are hardcoded in
`smart_compressor.rs`. Planned config keys:
- `[performance] ai_pdf_similarity_threshold = 0.15`
- `[performance] office_similarity_threshold = 0.20`
- `[performance] default_similarity_threshold = 0.80`

Effort: **0.5 day** (config schema + read + pass-through).

---

### 18. Windows ARM64 Native Binaries
*Source: `book/src/installation/windows-arm64.md`*

Blocked on GitHub Actions native ARM64 Windows runner availability. Currently, x64 binary
runs via Windows ARM emulation at reduced performance.

---

### 19. macOS Metal GPU Acceleration
*Source: `book/src/installation/macos-arm64.md:88`*

GPU-accelerated image processing via Apple Metal for Apple Silicon builds. No concrete
implementation plan yet. Effort: **2-3 weeks** (research + implementation).

---

### 20. Security / Audit Enhancements (v0.3.0+)
*Source: `claudedocs/2026-02-27/UNIMPLEMENTED_FEATURES.md`*

| Enhancement | Description |
|---|---|
| Async Audit Writer | Non-blocking audit log writes |
| Log Rotation | Built-in log rotation support |
| SIEM Integration | Native connectors for Splunk, ELK, etc. |
| Audit Retention | Configurable retention policies |

---

## Release Milestones

### v0.3.0 — Developer Experience and Ecosystem
- `mediagit diff` with media-aware visual diffing (image pixel diff, audio waveform)
- Conflict markers for PSD/Blend/FBX with editor integrations
- Shallow clone (`--depth N`) for large repositories
- Partial/sparse checkout — pull only specific asset subdirectories *(see item 8)*
- `mediagit migrate` — import from Git-LFS repositories *(see item 12)*
- Chocolatey and Homebrew package managers
- Official VS Code extension (file status, staging UI)

### v1.0.0 — Production-Grade Enterprise Features
- Stable API and wire protocol (v1 guarantee)
- SSO integration (OIDC/SAML) for enterprise auth
- Multi-region active-active replication
- Audit log export (compliance — SOC 2, GDPR)
- Plugin system for custom media type handlers
- Web UI for repository browsing and review workflows *(see item 6 as prerequisite)*
- Commercial support tiers

> See [FUTURE_TODOS.md](./FUTURE_TODOS.md) for individual item details (this file).

---

## Code TODOs (from source — grouped by crate)

### `mediagit-cli`

**`crates/mediagit-cli/src/commands/tag.rs:229`** *(→ item 10)*
```
// TODO: Implement proper tag objects when object storage supports Tag type
```

**`crates/mediagit-cli/src/commands/bisect.rs`** *(DONE — 2026-03-15)*
`bisect replay` now parses the log format and dispatches `good`/`bad`/`skip`/`start` entries to the existing async handlers.

### `mediagit-versioning`

**`crates/mediagit-versioning/tests/fsck_integration_test.rs:37`**
```
// FIXME: FSCK functionality is under development - tests may fail due to incomplete implementation
```
The `mediagit fsck` integration test suite is gated behind this marker. FSCK is functional
in the CLI but its test coverage is incomplete.

### `mediagit-protocol`

**`crates/mediagit-protocol/src/client.rs:122`** *(→ item 2)*

Pack negotiation have-set is empty — server always sends a full pack. Efficient incremental
fetch requires computing the local have-set before negotiation.

---

## Known Limitations

| # | Priority | Area | Description | Source |
|---|----------|------|-------------|--------|
| 1 | ~~P1~~ ✅ | **`.mediagitignore`** | **DONE** — v0.2.6-beta.1. `ignore` crate integration in `add` + `status` | `add.md:43` |
| 2 | P1 | **Pack negotiation** | Pull/fetch always downloads full pack (no incremental negotiation) | `client.rs:122` |
| 3 | P1 | **Parallel checkout I/O** | Checkout reads blobs sequentially; no parallel fetch | `checkout.rs` |
| 4 | ~~P1~~ ✅ | **TB-scale chunking** | **DONE** — Streaming format-aware chunking via mmap for all file sizes | v0.2.6-beta.1–3 |
| 5 | P1 | **Container-aware delta** | AI/PDF/INDD delta skipped <50 MB; 60-80% savings possible | `FUTURE_TODOS_2.md` |
| 6 | P1 | **Direct file serving** | No HTTP endpoint to download committed files by path | R&D 2026-03 |
| 7 | P2 | **`media info` command** | No CLI command to inspect media metadata | `FUTURE_TODOS.md` |
| 8 | P2 | **Sparse checkout** | Full tree checkout required; no partial working tree support | `FUTURE_TODOS.md` |
| 9 | P2 | **CLI unit tests** | All coverage is integration tests; no per-command unit tests | `FUTURE_TODOS_2.md` |
| 10 | P2 | **Annotated tags** | `tag -a` uses companion file, not a Tag ODB object; no PGP signing | `tag.rs:229` |
| 11 | P3 | **HTTP/3** | reqwest `http3` feature not yet stable | R&D 2026-03 |
| 12 | P3 | **Git migration CLI** | `mediagit-git` crate exists; `filter/install/track` removed from binary | CHANGELOG |
| 13 | P3 | **`mediagit://` scheme** | No native URL scheme; uses `http://` | R&D 2026-03 |
| 14 | P3 | **Differential checkout** | Full tree rewritten on branch switch; ~70% latency reduction possible | claudedocs |
| 15 | P3 | **Incremental status** | Full-tree scan on every `status` invocation | claudedocs |
| 16 | P3 | **Pack file docs** | `.mediagit/objects/pack/` format not documented | `file-formats.md` |
| 17 | P3 | **Similarity thresholds** | Delta thresholds hardcoded, not configurable via `config.toml` | `performance.md` |
| 18 | P3 | **Windows ARM64** | No native pre-built binary; x64 emulation works but slower | `windows-arm64.md` |
| 19 | P3 | **Metal GPU** | No GPU-accelerated image processing on Apple Silicon | `macos-arm64.md:88` |
| 20 | P3 | **SIEM / audit** | No Splunk/ELK connectors; SOC 2/GDPR export is v1.0.0 | claudedocs |
| 21 | P3 | **FSCK test coverage** | Integration tests marked as potentially failing | `fsck_test.rs:37` |

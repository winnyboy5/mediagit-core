# Changelog

All notable changes to MediaGit will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [v0.2.6-beta.1] - 2026-04-03


### Changed
- **`STREAMING_THRESHOLD` lowered 100MB → 5MB** (S4, phase 4 of item #4) — `add` and
  `status` now route all files ≥ 5MB through `write_chunked_from_file()`, which uses
  mmap + `chunk_media_aware()` for structure-aware deduplication. Previously only files
  ≥ 100MB got format-aware chunking; the 5-100MB range received generic CDC with no
  structural parsing. Both constants updated together to prevent OID mismatches between
  the two commands. (`crates/mediagit-cli/src/commands/add.rs:507`,
  `crates/mediagit-cli/src/commands/status.rs:199`)
- **GLB BIN large-payload CDC sub-chunking** (S5, phase 5 of item #4) — `chunk_glb()`
  now emits each GLB section header (8 bytes) as a stable `Metadata` chunk, then
  CDC-subdivides BIN payloads \> 4MB using FastCDC (1MB avg / 512KB min / 4MB max),
  matching the MKV large-Cluster pattern. Small BIN chunks (≤ 4MB) and all JSON chunks
  remain as single chunks. Common for scanned meshes, photogrammetry, and terrain models
  where the binary buffer is 20-200MB. (`crates/mediagit-versioning/src/chunking.rs`)
- **mmap-based format-aware chunking for all file sizes** — `collect_file_chunks_blocking()`
  now memory-maps files of any size and routes them through `chunk_media_aware()`, eliminating
  the previous StreamCDC fallback that made files ≥100 MB get generic CDC chunking with no
  format awareness. mmap fails gracefully to StreamCDC on network/FUSE filesystems and 32-bit
  targets. (`crates/mediagit-versioning/src/chunking.rs`)
- **MP4 mdat CDC sub-chunking** — `chunk_mp4()` emits the `mdat` atom header as a stable
  `Metadata` chunk, then CDC-subdivides the payload using FastCDC (2MB avg / 1MB min / 8MB max)
  to produce byte-exact `VideoStream` chunks. Fragmented MP4 (DASH/fMP4, detected by `moof`
  atoms) follows the same path. (`crates/mediagit-versioning/src/chunking.rs`)
- **MKV Cluster CDC sub-chunking** — `chunk_matroska()` emits each Cluster header as a stable
  `Metadata` chunk, then CDC-subdivides the cluster payload using FastCDC (2MB avg / 1MB min /
  8MB max) for byte-exact `VideoStream` chunks. Removes the earlier per-stream block-walking
  approach which violated the reconstruction invariant for interleaved A/V data.
  (`crates/mediagit-versioning/src/chunking.rs`)
- **Adaptive CDC params for video content** — FastCDC subdivision within `mdat` and Cluster
  now uses video-optimized parameters (2 MB avg / 1 MB min / 8 MB max) instead of the generic
  1 MB / 512 KB / 4 MB, improving delta dictionary matching for large video sub-elements.
  (`crates/mediagit-versioning/src/chunking.rs`)
- **Lower parallel processing threshold** — `write_chunked_parallel()` activates parallel
  chunk I/O at 2 chunks instead of 4; even 2-chunk files now benefit from concurrent storage
  writes. (`crates/mediagit-versioning/src/odb.rs`)
- **Delta encoding skip for pre-compressed VideoStream chunks** — streaming workers skip the
  delta-encode attempt for `ChunkType::VideoStream` chunks when the file type uses
  `CompressionStrategy::Store` (MP4, MOV, AVI, MKV, WebM, FLV, WMV, MPEG). Saves CPU on
  futile delta attempts against already-compressed H.264/H.265 frames; audio and metadata
  chunks are unaffected. (`crates/mediagit-versioning/src/odb.rs`)
- **Matroska EBML chunking: per-element metadata splitting** — each top-level metadata
  element (Info, Tracks, SeekHead, Cues, Chapters, Tags) now gets its own chunk instead
  of being grouped into one monolithic metadata blob. Re-tagging a video only invalidates
  the Tags chunk, not the entire metadata block. Consistent with MP4's per-atom approach.
  (`crates/mediagit-versioning/src/chunking.rs`)
- **Matroska large Cluster CDC subdivision** — Clusters > 4MB are now sub-chunked using
  FastCDC (1MB avg / 512KB min / 4MB max), matching the MP4 `mdat` subdivision strategy.
  Cluster header emitted separately for stable dedup across re-muxes.
  (`crates/mediagit-versioning/src/chunking.rs`)
- **AVI/RIFF chunking rewrite: movi CDC descent + OpenDML AVIX support** — rewrote
  `chunk_avi()` to walk at the RIFF-block level, handling both AVI 1.0 (`RIFF/AVI `) and
  AVI 2.0 OpenDML (`RIFF/AVIX`) extension blocks. Descends into `LIST/movi` and CDC-subdivides
  its payload using FastCDC (2MB avg / 1MB min / 8MB max) for byte-exact `VideoStream` chunks.
  Structural chunks (`LIST/hdrl`, `LIST/INFO`, `idx1`, `JUNK`) emitted as `Metadata`.
  (`crates/mediagit-versioning/src/chunking.rs`)

### Added
- **GLB unit tests** (6 tests) — `test_glb_small_bin_single_chunk`,
  `test_glb_large_bin_is_subdivided`, `test_glb_json_chunk_always_single_metadata`,
  `test_glb_large_bin_different_data_different_chunk_ids`, `test_glb_no_bin_sections_ok`,
  `test_glb_invalid_data_falls_back`. (`crates/mediagit-versioning/src/chunking.rs`)
- **Matroska chunking tests** — `test_chunk_matroska_metadata_splitting` verifies each
  metadata element (Info, Tracks, Tags) is emitted as its own chunk with distinct hashes.
  `test_chunk_matroska_large_cluster_subdivision` verifies 5MB Clusters get CDC-subdivided
  into multiple chunks. (`crates/mediagit-versioning/src/chunking.rs`)
- **AVI chunking tests** — `test_chunk_avi_movi_descends_into_subchunks` verifies movi
  payload is CDC-subdivided into `VideoStream` chunks and reconstruction is byte-exact.
  (`crates/mediagit-versioning/src/chunking.rs`)

### Fixed
- **Branch switch reconstruction size mismatch** — `branch switch` produced
  `Reconstructed size mismatch` errors (extra or missing bytes) for AVI, MP4, and MKV files.
  Root cause: per-stream batching helpers (`chunk_mdat_by_tracks`, `chunk_cluster_by_tracks`,
  `parse_avi_movi_subchunks`) accumulated non-contiguous interleaved bytes but stored
  `(offset, size)` as if contiguous; `fill_coverage_gaps` then re-read those byte ranges
  from the file, duplicating them (+22MB for a 228MB AVI). Fixed by replacing all
  per-stream batching with CDC (`chunk_fastcdc`, 2MB/1MB/8MB) which guarantees
  `chunk.data == file[offset..offset+size]`. Added `fill_coverage_gaps` as a free function
  called at the end of all three container parsers to patch EBML Void/CRC-32 and other
  structural gaps that format parsers intentionally skip. Covers all video/audio containers:
  AVI, MP4/MOV/M4V/M4A/3GP, MKV/WebM/MKA/MK3D. (`crates/mediagit-versioning/src/chunking.rs`)
- **`mka`/`mk3d` missing from ObjectType** — Matroska Audio (`.mka`) and Matroska 3D
  (`.mk3d`) extensions now map to `ObjectType::Mkv` in the smart compressor, ensuring
  they receive `CompressionStrategy::Store` instead of wastefully compressing
  pre-compressed media data. (`crates/mediagit-compression/src/smart_compressor.rs`)
- **AVI LIST chunk type detection was dead code** — the old `chunk_avi()` matched on
  `b"movi"` and `b"hdrl"` as FourCC values, but RIFF LIST chunks always have FourCC
  `b"LIST"` with the list type at offset +8. The match arms never fired, causing the
  entire `movi` LIST (all interleaved A/V data) to be stored as one opaque chunk with
  zero dedup potential. Now correctly reads the list type field.
  (`crates/mediagit-versioning/src/chunking.rs`)
- **AVI chunk size overflow on 32-bit targets** — `block_end` / `data_end` calculations
  used plain addition that could wrap on 32-bit `usize` with crafted RIFF headers.
  Switched to `saturating_add()` in all three AVI parsing functions.
  (`crates/mediagit-versioning/src/chunking.rs`)


---

## [v0.2.5-beta.1] - 2026-03-26

### Added
- **`.mediagitignore` support** in `add` and `status` commands — `.gitignore`-compatible
  pattern matching using the `ignore` crate (`v0.4.25` / `globset v0.4.18`).
  - `add`: files and directories matching `.mediagitignore` are silently skipped during
    file discovery. Entire ignored directories are pruned (no recursion), preventing
    unnecessary I/O. Explicit named paths that are ignored print a warning; `--force`
    bypasses all ignore rules entirely. `--verbose` logs each skipped path.
  - `status`: ignored files are hidden from the "Untracked files:" section by default.
    `--ignored` flag activates a new "Ignored files:" section listing all excluded files.
    `--porcelain --ignored` uses the `!! path` prefix, matching Git convention.
  - Graceful fallback: missing `.mediagitignore` is a no-op; malformed file logs a warning
    and continues without rules.
  - Pattern syntax: full `.gitignore` semantics — globs (`*.tmp`), directory markers
    (`build/`), negation (`!important.log`), comments (`#`), anchored paths (`/src`).
  - New module: `crates/mediagit-cli/src/ignore_rules.rs` — `IgnoreMatcher` struct wrapping
    the `ignore` crate for consistent use across commands.
  - `crates/mediagit-cli/Cargo.toml` — added `ignore = "0.4"` dependency.
  - (`crates/mediagit-cli/src/commands/add.rs`, `crates/mediagit-cli/src/commands/status.rs`)

- **Integration test suite for `.mediagitignore`** (`crates/mediagit-cli/tests/ignore_integration_test.rs`):
  8 tests covering basic glob ignore, `--force` override, directory pruning, negation (`!`
  pattern), `--ignored` flag display, porcelain `!!` prefix, and no-file fallback.
  All 8 tests pass.

### Changed
- `book/src/cli/add.md` — Options section corrected (removed non-existent `--chunk-size`;
  added `--no-chunking`, `--no-delta`, `--no-parallel`, `-j`). New `.mediagitignore` section
  with full syntax reference, ignore example, and `--force` override example.
- `book/src/cli/status.md` — `--ignored` option corrected from mode-based description to
  simple boolean flag matching the implementation. Updated example shows real output format
  with "Ignored files:" section. Added `--porcelain --ignored` example with `!! path` prefix.
  Notes section updated to reference `.mediagitignore` properly.

## [v0.2.4-beta.1] - 2026-03-26


### Changed
- Delta encoder replaced: suffix-array (divsufsort/sacabase) sliding-window approach replaced
  with **zstd dictionary compression**. Base chunk is used as a raw zstd dictionary at level 19
  to compress target chunks. Wire format v2: `[0x5A, 0x44]` magic + varint sizes + zstd bytes.
  Results: +1.3-2.1pp better savings on AI files, 1.4-2.4× faster throughput, 73% less code.
  (`crates/mediagit-versioning/src/delta.rs`)

### Added
- `/health` route alias added alongside `/healthz` in both `create_router` and
  `create_router_with_rate_limit`. Kubernetes liveness probes, load balancers, and uptime
  monitors that probe `/health` (without the `z`) now get a 200 response.
  (`crates/mediagit-server/src/lib.rs`)
- `bisect replay` now executes scripted bisect sessions: parses the log file format
  (`YYYY-MM-DD HH:MM:SS: command: args`), strips the timestamp prefix, and dispatches
  `good`/`bad`/`skip`/`start` entries to the existing async bisect handlers. Previously
  the command printed log lines without acting on them.
  (`crates/mediagit-cli/src/commands/bisect.rs`)
- `log <REVISION>` now resolves branch names, tags, and abbreviated OIDs via `resolve_revision`,
  so `mediagit log main` or `mediagit log feat/my-branch` shows that branch's history.
  (`crates/mediagit-cli/src/commands/log.rs`)
- Standalone test suite passes 173/173 tests (release build, Windows/WSL2). Covers all
  active CLI commands, MinIO S3 backend, and push/pull/clone over local HTTP server.
- HTTP/2 adaptive window tuning (`http2_adaptive_window`, 2 MB stream window, 8 MB connection
  window) in the protocol client for 2-4× throughput improvement on WAN connections.
  (`crates/mediagit-protocol/src/client.rs`)
- Server TLS config now advertises HTTP/2 via ALPN (`h2`, `http/1.1`), enabling HTTP/2
  negotiation over TLS. Plaintext HTTP/1.1 connections (local dev, CI) are unaffected.
  (`crates/mediagit-server/src/main.rs`)
- Raw file serving endpoints on the HTTP server: `GET /{repo}/files/{*path}` streams a file
  at a given path from any commit ref, and `GET /{repo}/tree[/{*path}]` lists tree entries
  as JSON. (`crates/mediagit-server/src/handlers.rs`, `crates/mediagit-server/src/lib.rs`)
- Abbreviated OID resolution: `show`, `revert`, `verify`, and all other revision-accepting
  commands now accept shortened commit hashes (≥4 hex chars), matching `git log --oneline`
  output. Prefix-scans the object store; errors on ambiguous matches.
  (`crates/mediagit-versioning/src/odb.rs`, `crates/mediagit-versioning/src/revision.rs`)
- `stash push` subcommand as a git-compatible alias for `stash save`. Accepts `-m/--message`
  flag and positional paths, identical to `stash save`. (`crates/mediagit-cli/src/commands/stash.rs`)
- `verify [COMMIT]` optional positional argument: pass a commit OID, abbreviated hash,
  branch name, or `HEAD` to verify a specific commit and its reachable objects rather than
  the full repository. (`crates/mediagit-cli/src/commands/verify.rs`)

### Fixed
- `show <short-hash>` now resolves abbreviated OIDs instead of failing with "OID hex string
  must be 64 characters". (shared fix: abbreviated OID resolution in `revision.rs`)
- `revert <short-hash>` now resolves abbreviated OIDs instead of failing with the same error.
- `verify HEAD` and `verify <short-hash>` no longer fail with "unexpected argument". The
  `verify` command now accepts an optional `[COMMIT]` positional argument.
- `stash push -m "msg"` now works — previously rejected as an unrecognised subcommand.
- `verify` `resolve_commit` now uses `refdb.resolve()` (which follows symbolic refs like HEAD)
  instead of `refdb.read()`, so `verify HEAD` correctly resolves to the HEAD commit.

### Removed
- Removed `filter`, `install`, `track`, and `untrack` commands — git migration tooling is a
  future milestone. The `mediagit-git` crate remains in the workspace and compiles
  independently for when the migration milestone arrives.
- Removed `mediagit-git` dependency from the CLI binary.

## [0.2.3-beta.1] - 2026-03-13

### Fixed
- `add` command: ETA showed wildly incorrect values (e.g. "eta 2d") when most files were
  unchanged. Skipped (stat-cache / HEAD-match) files now advance the byte progress counter
  so `indicatif`'s ETA calculation is based on total work, not just newly staged bytes.
  (`crates/mediagit-cli/src/commands/add.rs`)
- `add` command: Speed dropped to "0 B/s" and ETA reached astronomical values (e.g.
  "eta 11710991569y") while staging large files (≥100 MB). Added a per-chunk `on_progress`
  callback to `ObjectDatabase::write_chunked_from_file` that fires after every chunk
  (deduped, delta, or full), giving continuous byte-level progress updates during multi-GB
  file ingestion. (`crates/mediagit-versioning/src/odb.rs`,
  `crates/mediagit-cli/src/commands/add.rs`)

### Changed
- `ObjectDatabase::write_chunked_from_file` now accepts an optional
  `on_progress: Option<Arc<dyn Fn(u64) + Send + Sync>>` callback for incremental byte
  reporting. Pass `None` to retain previous behaviour.

### Security
- Upgraded `quinn-proto` from 0.11.13 → 0.11.14 (RUSTSEC-2026-0037, CVSS 8.7 — DoS in
  Quinn QUIC endpoints). Transitive dependency via `reqwest → quinn → quinn-proto`.
  Only `Cargo.lock` updated; no `Cargo.toml` changes required.

### Code Quality
- `crates/mediagit-cli/src/commands/log.rs`: Changed `walk_tree` parameter from
  `&'a PathBuf` to `&'a Path` (clippy `ptr_arg` warning).
- `crates/mediagit-cli/src/commands/show.rs`: Same `&PathBuf` → `&Path` fix.
- `crates/mediagit-security/src/auth/jwt.rs`: Marked `JwtAuth::new` doctest as `no_run`
  to prevent Avast false-positive (`rust_out.exe` blocked on Windows) from failing CI.
- `crates/mediagit-versioning/src/odb.rs`: Updated `write_chunked_from_file` doctest to
  pass the new `None` argument.

## [0.2.1-beta.2]

### Fixed
- PowerShell install warning: added `-UseBasicParsing` to `iwr` in `install.ps1` usage comment,
  `install.sh` (Windows fallback message), `RELEASING.md`, and `.github/workflows/release.yml`
  release notes body — prevents IE-engine security prompt on Windows PowerShell
- Install scripts (`install.ps1`, `install.sh`) now fall back to the `/releases` list API when
  `/releases/latest` returns 404 — this occurs when only pre-release versions exist (e.g. before
  the first stable release); scripts pick the most recent release including pre-releases

### Changed
- `README.md`: Added complete 32-command CLI reference section, grouped by workflow with flag docs
- `README.md`: Replaced compression efficiency table with accurate per-type data (conservative
  numbers — ~30% average across mixed media projects; pre-compressed formats explicitly shown
  as Store / 0% additional reduction)
- `README.md`: Added scenario-based deduplication table (replaces single "66% identical files" row)
- `README.md`: Updated roadmap to match actual CHANGELOG history (v0.1.0 → v0.2.0 → v0.2.1
  → v0.3.0 planned → v1.0.0 stable); removed fictional v0.1.1 entry
- `README.md`: Fixed Statistics section — staging throughput corrected to 80–240 MB/s (release
  build); removed misleading 3-35 MB/s figure
- `README.md`: Added "Could not fetch latest version" troubleshooting entry with install workaround

## [0.2.1-beta.1] - 2026-03-06

### Changed
- Automated version extraction from Cargo.toml in release workflow dry-run mode
- Updated all documentation to reflect correct version, URLs, and archive names
- Added `scripts/bump-version.sh` for automated version bumping across the project

## [0.2.0] - 2026-03-05

### Added
- Dual-layer delta encoding (bsdiff + sliding-window)
- AES-256-GCM client-side encryption with Argon2id key derivation
- TLS 1.3 for all network operations
- JWT + API key authentication for server mode
- Video and audio track-based merging (fully implemented)
- Multi-platform distribution (Linux, macOS, Windows, Docker, crates.io)
- Automated release pipeline with cross-compilation

### Changed
- Delta max chain depth reduced from 50 to 10 for faster reads
- Chunk sizes now adaptive (1-8 MB) instead of fixed 64 MB
- Similarity thresholds tuned per file type for better delta compression
- macOS Intel CI runner updated to macos-15-intel

### Fixed
- macOS Intel (x86_64-apple-darwin) build failure due to retired macos-13 runner
- Docker push to GHCR (added packages:write permission)
- Comprehensive documentation sync with codebase (book, architecture docs, CLI reference)

## [0.1.0] - 2026-02-27

### Added
- Core MediaGit CLI implementation
- Object database with SHA-256 content addressing
- Intelligent compression (Zstd, Brotli)
- Branch management system
- 3-way merge algorithm
- Media-aware merge intelligence (PSD layer-aware)
- Git integration layer
- Multi-cloud storage backends:
  - Local filesystem
  - AWS S3
  - Azure Blob Storage
  - Google Cloud Storage
  - MinIO (S3-compatible)
  - Backblaze B2
  - DigitalOcean Spaces
- Security: AES-256-GCM encryption at rest
- Observability: Structured logging with Tracing
- Metrics: Prometheus metrics endpoint
- Operations: Garbage collection, FSCK, storage migration
- Comprehensive test suite (960 tests, 80%+ coverage)
- Documentation and user guide
- Multi-platform binaries (Linux, macOS, Windows on x86_64 and ARM64)

### Security
- AGPL-3.0 license enforcement
- Dependency security audits in CI
- Encryption at rest with Argon2 key derivation

[Unreleased]: https://github.com/winnyboy5/mediagit-core/compare/v0.2.6-beta.3...HEAD
[v0.2.6-beta.3]: https://github.com/winnyboy5/mediagit-core/compare/v0.2.6-beta.2...v0.2.6-beta.3
[v0.2.6-beta.2]: https://github.com/winnyboy5/mediagit-core/compare/v0.2.6-beta.1...v0.2.6-beta.2
[v0.2.6-beta.1]: https://github.com/winnyboy5/mediagit-core/compare/v0.2.5-beta.1...v0.2.6-beta.1
[v0.2.5-beta.1]: https://github.com/winnyboy5/mediagit-core/compare/v0.2.4-beta.1...v0.2.5-beta.1
[v0.2.4-beta.1]: https://github.com/winnyboy5/mediagit-core/compare/v0.2.3-beta.1...v0.2.4-beta.1
[v0.2.3-beta.1]:https://github.com/winnyboy5/mediagit-core/compare/v0.2.1-beta.2...v0.2.3-beta.1
[v0.2.1-beta.2]: https://github.com/winnyboy5/mediagit-core/compare/v0.2.1-beta.1...v0.2.1-beta.2
[0.2.1-beta.1]: https://github.com/winnyboy5/mediagit-core/compare/v0.2.0...v0.2.1-beta.1
[0.2.0]: https://github.com/winnyboy5/mediagit-core/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/winnyboy5/mediagit-core/releases/tag/v0.1.0

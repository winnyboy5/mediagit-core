# FUTURE_TODOS.md

Consolidated and **priority-ordered** registry of planned features, code-level TODOs, and
known limitations for MediaGit. Items are sourced from documentation, source code, and
historical claudedocs analyses.

> Last updated: 2026-07-18 | v0.3.0-rc.1 | Items 1 (.mediagitignore) + 4 (Streaming Format-Aware Chunker, S1–S5) + Push Progress/Throughput fixes + BLAKE3 + B2/B4/B7 pipeline + **5 (Phase-3 Track F / cloud packs, F1–F11)** + Presigned-URL transfer (W1–W5) + God-file refactor (handlers/ smart_compressor/ client/ odb/ chunking/) + Server direct file-serving endpoints + **5c (Smart-media cycle P0–P5: dedup harness/gate, keyed CDC seed, codec detection, blend/STL/PLY walkers, pHash image delta, show/stats media metadata)** + chunk-delta cycle fix + fsck chunk-delta validation + zstd Best 22→19 (3.2× add speedup) + **Server file locking (MEDIAGIT_LOCKS_ENFORCE) + Auth persistence (users.jsonl, api_keys.jsonl, grants.jsonl, per-repo MEDIAGIT_GRANTS_ENFORCE) + OS-keychain credential storage + path-traversal validation + push --repair + verify-integrity endpoints + compat-fixture freeze gate** **DONE**

**Priority levels:**
- **P0** — Quick win or active blocker — ≤1 day effort, implement immediately
- **P1** — High impact, near-term — 1-2 weeks, next milestone target
- **P2** — Medium impact or complex — 2-6 weeks, planned but not urgent
- **P3** — Low priority / long-term — deferred until triggered by demand

---

## ⚠ P0 — Security Remediation: leaked secrets in git history (2026-07-18)

`.mcp.json` (Morph API key), `enc_key`/`enc_key.pub` (OpenSSH ed25519 pair), and an
Anthropic API key were committed to a **public** repo (github.com/winnyboy5/mediagit-core).
Treat both API keys as fully compromised regardless of any cleanup — scrubbing history
never un-leaks a key.

- [x] 1. **REVOKE the Anthropic API key (`sk-ant-api03-t5EdN1…`)** — done (user-confirmed 2026-07-18)
- [x] 2. **REVOKE the Morph API key (`sk-3uQSl4Vrz…`)** — done (user-confirmed 2026-07-18). Note: `.mcp.json` (carrying this key) has been in history since `90f8cf8` and on **every branch** — the key was exposed for the file's whole lifetime, so revocation was the load-bearing fix.
- [x] 3. `git rm --cached .mcp.json enc_key enc_key.pub` — done 2026-07-18 (staged deletions in working tree; `.mcp.json` kept on disk for local MCP config)
- [x] 4. `.gitignore` — `enc_key`/`enc_key.pub` added 2026-07-18 (L224-225); `.mcp.json` (L195) and `.env` (L65) were already covered
- [x] 5. `enc_key` usage check — done 2026-07-18: **zero references** in crates/, scripts/, dev-tests/ → orphan artifact; files deleted from disk (recoverable from git history until step 7 runs)
- [x] 6. Key rotation — **not needed**: the pair is used by nothing; deleted instead (see 5). If a future use surfaces, generate a fresh pair — never restore this one
- [ ] 7. **Scrub git history** — scope verified 2026-07-18: **full-repo rewrite**, not a recent-commit trim. `.mcp.json` enters at `90f8cf8` and is carried by main + all ~58 remote branches (incl. every dependabot branch); `enc_key`/`enc_key.pub` enter at `82295e8` (feat/smart-media-handling only). `.env` was **never committed** (verified: absent from all trees in history) — excluded from the path list. `git-filter-repo` installed (`python -m git_filter_repo`). Agreed sequence:
       1. USER commits the pending GA-cycle work on feat/smart-media-handling
       2. Pristine backup: `git clone --mirror` to a separate location; verify non-empty **before** any rewrite (backup gate)
       3. `python -m git_filter_repo --path .mcp.json --path enc_key --path enc_key.pub --invert-paths --force`
       4. Force-push **all** branches + tags to origin (`git push origin --force --all` + `--tags`)
       5. Post-push: close/regenerate dependabot PRs (branches invalidated), any collaborator re-clones, and request cached-view purge via GitHub Support (public repo — old commits stay servable from caches until purged)

---

## Quick Reference — Priority Matrix

| # | Item | Priority | Effort | Blocks / Enables |
|---|------|----------|--------|-----------------|
| 1 | `.mediagitignore` support in `add` + `status` | ~~**P1**~~ **✅ DONE** | 2-3 days | Shipped in v0.2.6-beta.1 |
| 2 | Pack negotiation / bitmap index | ~~**P1**~~ **✅ DONE** | — | Pack negotiation shipped v0.2.6-beta.1; bitmap index deferred (see §2b) |
| 3 | Parallel object I/O during checkout | ~~**P1**~~ **✅ DONE** | 1 wk | M3 — JoinSet+Semaphore with MEDIAGIT_CHECKOUT_PARALLELISM (default cpus cap 8) |
| 4 | Streaming format-aware chunker (MKV/MP4/GLB, S1-S5) | ~~**P1**~~ **✅ DONE** | 8-12 days | Shipped in v0.2.6-beta.1 |
| 5 | **Phase-3 Track F — Cloud-side pack objects** (xorb-style chunk bundling) | ~~**P1**~~ **✅ DONE** | — | Shipped v0.2.7-beta.1 (F1–F11, streaming_pack.rs, F8 integrity) |
| 6 | `mediagit download` CLI subcommand | ~~**P1**~~ **✅ DONE** | 2-3 days | M2 — plain streaming GET, full-URL mode, no-repo requirement for CI/scripting |
| 7 | `mediagit media info` command | ~~**P2**~~ **✅ DONE** | ~100 LOC | M5b — `commands/media.rs` wired into CLI; surfaces full parsed-struct detail for image/video/audio/PSD/3D formats |
| 8 | Sparse checkout | ~~**P2**~~ **✅ DONE** | ~500 LOC | M5b — cone + pattern modes, `.mediagit/info/sparse-checkout`, set/list/disable subcommands |
| 9 | CLI command unit tests | ~~**P2**~~ **✅ DONE** | Large | M6 — mock-storage scaffold + 46 new unit tests for merge/rebase/cherry-pick/stash, un-ignored 4 fsck integration tests |
| 10 | Annotated tag objects (SSH signing) | ~~**P2**~~ **✅ DONE** | 1 wk | M5a — ObjectType::Tag + postcard format + SSHSIG signing with MEDIAGIT_SIGN/MEDIAGIT_SIGN_KEY (ed25519, TOFU verify) |
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
| 22 | GA knob-policy execution (remove/keep each `MEDIAGIT_*` knob) | **P1** (at GA) | 1 day | `docs/next-set/knob-policy.md` is the decision record |
| 23 | `checkout` doesn't re-materialize a deleted working-tree file | ~~**P2**~~ **✅ DONE** | 2-3 days | M0 — fixed in checkout.rs: checks `exists()` in addition to OID-equality for re-materialization |
| 24 | FBX Objects-descending walker (or delete walker at GA) | **P3** | 1-2 wk | Fair trial closed 2026-07-07: top-level cuts ≈ CDC (+0.003pp) |
| 25 | EXR structure-aware chunking | **P3** | 3-5 days | Needs real EXR fixtures first (creating them requires the `exr` crate) |
| 26 | .sketch/.fig ZIP-entry-aware chunking | **P3** | 3-5 days | No fixtures in corpus yet |
| 27 | Video pHash (keyframe extract + image_hasher) | **P3** | 1-2 wk | No viable video-phash crate (verified 2026-07-07) |
| 28 | Cross-process chunk-delta write lock | **P3** | 2-3 days | In-process race fixed 2026-07-07; multi-process writers to one local repo could still race (CLI never does this) |
| 29 | phash.idx compaction | **P3** | 0.5 day | Append-only today; only matters >1M entries (~16 MB) |
| 30 | PSD spot-color channel parse failure | **P3** | Unscoped (needs upstream fix or crate swap) | `psd` crate 0.3.5 errors "invalid channel id 3" on PSDs with a spot-color channel; found 2026-07-10, M5b |

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

### ~~2. Pack Negotiation~~ ✅ DONE — v0.2.6-beta.1
*Source: `crates/mediagit-protocol/src/client.rs:122`; `claudedocs/` optimization roadmap*

**Implemented** in v0.2.6-beta.1. Full have-set negotiation pipeline across all crates:

- **Client**: `collect_local_have(refdb)` walks heads/remotes/tags, resolves symbolic refs,
  deduplicates. Used by `fetch.rs` (once per fetch) and `pull.rs` (before streaming).
  `clone.rs` correctly sends empty have for full clone.
- **Wire protocol**: `WantRequest{want, have}` in `types.rs`, sent via `download_pack_streaming`.
- **Server**: `download_pack` expands have-closure via `walk_reachable` (BFS object-graph
  walker in `reachability.rs`), then prunes the want-walk via `collect_objects_recursive(stop_at)`.
  Lenient error handling for stale/unknown have OIDs.
- **Deterministic delta**: Producer-side similarity detection ensures reproducible chunk
  storage for consistent have-set comparison.

See `crates/mediagit-cli/src/repo.rs` (`collect_local_have`),
`crates/mediagit-versioning/src/reachability.rs` (`walk_reachable` + 5 unit tests),
`crates/mediagit-server/src/handlers.rs` (`download_pack`, `collect_objects_recursive`).

---

### 2b. Bitmap Index (Follow-up Optimization)
*Depends on: ~~Pack Negotiation~~ (done)*

The current `walk_reachable` does a full BFS traversal — O(objects) per fetch. For repos
with <1K commits this is fast enough. At 10K+ commits, BFS dominates server latency.

**What's needed:**
- Roaring bitmap index over refs for fast reachability queries
- Bitmap generation on push / GC / repack
- Bitmap-accelerated "what's missing" detection in `download_pack`

Effort: **~3–4 days**. Becomes valuable only at scale (10K+ commits).

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

### ~~5. Phase-3 Track F — Cloud-side Pack Objects~~ ✅ DONE — v0.2.7-beta.1

xorb-style chunk bundling shipped as `streaming_pack.rs` / `CloudPackResult`, phases F1–F11
complete including F8 integrity verification. 614/614 tests pass across all four backends.
10k S3 objects → ~100 per push/clone; 5-10× clone speedup on small-chunk repos confirmed.

See `plans/squishy-whistle.md` for the approved plan, and
`crates/mediagit-protocol/src/streaming_pack.rs` for the implementation.

---

### ~~5c. Smart-Media Handling Cycle (P0–P5)~~ ✅ DONE — 2026-07-07

Shipped on `feat/smart-media-handling` (awaits commit), validated 614/614 across
MinIO/AWS/Azure/GCS. Full detail: `docs/next-set/knob-policy.md` (every knob + GA fate)
and `dev-tests/deep-tests/reports/consolidated_report_2026-07-07.md`.

- **P0 harness**: `examples/dedup_report.rs` + `dev-tests/compare_dedup.ps1` +
  locked `dedup-baseline.json` — every later change gated on per-format
  dedup/compression/timing. Caught two real regressions the same day it shipped.
- **P1**: fastcdc 3.2→4.0.1 (seed=0 byte-identical, gate-proven) + per-repo keyed CDC
  seed (`cdc_seed` config, `cdc-seed` protocol capability to clones) — chunking-attack
  surface closed at zero throughput cost.
- **P2**: codec detection fills `CodecHint` from MP4 stsd / MKV Tracks / AVI strh —
  H.264/AAC→Store, PCM→Zstd, subs→Brotli; boundaries untouched.
- **P3**: .blend BHEAD / binary STL / binary PLY walkers (byte-perfect reassembly
  proofs); audio tier for MP3/OGG. **Measurement-rejected**: parquet/safetensors
  walkers (generic CDC already at 99%+ of theoretical-ideal dedup on real edit pairs —
  no crate dep taken); FBX walker default-off (fair trial on a structure-valid edit:
  +0.003pp, see item 24). WAV/FLAC audio tier rejected (+140% add-time for <1pp).
- **P4**: pHash-guided delta-base nomination — images can delta at all now
  (metadata-edit jpg → 0.012% delta; re-encodes correctly gate-rejected);
  `show`/`stats` media metadata lines (`media_meta.rs`).
- **P5**: all-knobs-off run reproduces the pre-plan baseline **byte-for-byte**.
- **Chunk-delta cycle fix** (data-loss class, pre-existing TOCTOU race): A→B→C→A
  chunk-delta cycles under parallel adds; `delta_written_pairs` lock now spans
  [chain re-walk + meta write] at all 3 write sites.
- **fsck `check_chunk_deltas`**: detects cycles / missing bases / over-deep chains
  (on by default; the AWS corruption shape is now catchable).
- **zstd `Best` 22→19 + FLAC/ALAC→Default**: 0.0002% ratio cost, corpus add
  42s→13s (3.2×), FLAC add 33× faster, ultra-OOM class eliminated.

---

### 5b. Container-Aware Delta Encoding for PDF/ZIP Formats [DELTA-001]
*Source: `docs/FUTURE_TODOS_2.md` — recorded 2026-03-01*

> **⚠️ ATTEMPTED 2026-04-07 — REVERTED. See post-mortem below before re-attempting.**
>
> **Status 2026-04-18 (v6 verification, aka FEAT-001):** Re-confirmed out-of-scope.
> `delta_ai_lg` measured 73.4 % growth on the `test-label-org.ai` → `test-label-alt.ai`
> pair (129 MB → 215 MB — +86 MB genuinely new content, theoretical floor ~40 %).
> The v6 creative-chunk-params fix (1 MB / 4 MB FastCDC on ai/pdf/eps/psd/indd)
> landed `delta_psd` at **21.4 %** but AI held at 73.4 %, matching the 2026-04-07
> finding that Illustrator's proprietary DEFLATE makes whole-file normalization
> produce *worse* results than baseline.
>
> **Decision:** Not scheduled. Closing AI < 35 % requires the per-stream OID +
> custom delta codec path (4–6 weeks, "What would be needed" below). AI is
> intentionally **not** a gating criterion in
> `dev-tests/standalone-deep-v6/reports/verification-plan-v6.md`. PSD < 35 %
> remains the regression target and is protected by workspace tests.

**Problem**: Adobe Illustrator (`.ai`), InDesign (`.indd`), PDF (`.pdf`) files are
PDF/ZIP containers with DEFLATE-compressed inner streams. A single-byte change causes
DEFLATE to reshuffle all subsequent bytes — the similarity detector finds near-zero
matches between versions, producing deltas nearly as large as the original.

**Current workaround** (`add.rs` `should_use_delta()`): `.ai/.ait/.indd/.idml/.pdf` files
≥ 50 MB attempt delta; smaller files skip it. Current savings: ~27% (vs 60-80% target).

**Real-world data** (2026-03-01, 124 MB + 206 MB AI files):
- 328.90 MiB original → 238.85 MiB stored (27.4% saved, 42.7% on chunks)

---

#### Post-mortem: Why the 2026-04-07 attempt failed

A full branch (`feat/container-aware-delta`) implemented a two-OID pre-processing layer:
inflate all DEFLATE streams → feed normalized (inflated) bytes to existing CDC+delta
pipeline. All code was reverted after testing revealed it produced **worse results than
the baseline** for Adobe Illustrator files.

**Root cause**: Adobe Illustrator uses a **proprietary DEFLATE encoder** that neither
`flate2` nor `miniz_oxide` can reproduce byte-for-byte at any compression level 0–9.
Every stream in a `.ai` file is therefore **opaque** (Tier 3 fallback). When all streams
are opaque:

1. Inflated data (~160 MiB) is larger than the original compressed file (~143 MiB)
2. The opaque compressed bytes must be retained for round-trip fidelity, adding overhead
3. Net storage was **worse** than the baseline (no normalization), not better

The approach works correctly for **standard PDFs** with re-deflatable streams, but real
Adobe Illustrator `.ai` files are effectively an all-opaque edge case that defeats it.

**What would be needed for a successful attempt:**

- **Per-stream opaque detection BEFORE committing to normalization**: If opaque ratio
  exceeds ~80%, skip normalization entirely for that file (fall back to baseline pipeline).
  This was added in the last iteration but came too late — the architecture was already
  built around the assumption most streams are re-deflatable.

- **OR: stream-by-stream delta matching** instead of whole-file normalization — match
  individual streams between versions by PDF xref ID/ZIP entry name, even if they can't
  be re-deflated. Deltas on compressed bytes of the *same stream* are far smaller than
  whole-file deltas. This requires a custom delta codec, not the existing zstd-dict path.

- **OR: accept AI as out-of-scope** — the current baseline (27% savings on AI files) is
  actually competitive for a format with custom DEFLATE. Focus future effort on formats
  with standard DEFLATE (IDML, DOCX, standard PDFs) where stream re-inflation is reliable.

**Effort for a correct implementation**: **4-6 weeks** (stream-by-stream delta matching
with per-stream OID keying). **Do not attempt again** with the whole-file normalization
approach for AI/PDF without first validating opaque-stream ratio on target files.

> **Status 2026-07-16 (C1 measurement spike, `dev-tests/transform-spike/RESULTS.md`):**
> Re-measured with a per-file 20%-opaque gate (the fix this post-mortem recommended).
> Gate (+15pp savings AND ≥90% stream reproducibility) cleared by **no format**:
> - **PNG is also ~100% opaque** — real-world PNG encoders are not bit-reproducible
>   by flate2 even with the `zlib-rs` backend. Extends this post-mortem beyond AI.
> - **flate2's default miniz_oxide backend gives 0% reproducibility on everything** —
>   any future recompression work MUST use the `zlib-rs` feature.
> - **ONNX contains no zlib streams** — out of scope for this transform class entirely.
> - **Standard PDF** is the only directional positive (+8.5pp, 83.3% repro on a
>   synthetic corpus) — below gate; revisit only with a real-world PDF corpus and a
>   recompressor with zlib strategy control.
> - `.ai` negative control reproduced the 2026-04-07 finding exactly (100% opaque).
> - The ungated whole-file approach would have regressed PNG by −3.3pp — the failure
>   mode generalizes; the per-file opaque gate correctly prevents it.
>
> **Decision:** reversible-inflate transform (C2) not shipped. DELTA-001 stays closed.

> **Status 2026-07-16 (G0 spike, `dev-tests/transform-spike/G0_RESULTS.md`):** the OTHER
> sanctioned path — stream-by-stream keyed matching + compressed-bytes delta ("What
> would be needed", option 2 above) — is now ALSO measured dead on the reference pair:
> - `test-label-org.ai` → `test-label-alt.ai` (129→216 MB): only **50.6% of container
>   keys survive** an Illustrator save — the assumption that object numbering is stable
>   across versions is false for real .ai files.
> - Stream-keyed delta scored **−2.2pp vs baseline** on that pair and loses badly to
>   what the production CDC pipeline already achieves on it (27.4%).
> - Small-fixture rows showed apparent gains (+7.9/+33.4pp) but FAILED the shuffled-key
>   negative control — small-sample confound, not mechanism. Any future measurement
>   MUST include that control.
> - Projected mixed corpus: 26.5% → 26.2%. Gate (≥30%) not met.
>
> **Decision:** DELTA-001 remains closed. Both sanctioned approaches (inflate
> normalization, stream-keyed delta) are measured dead. Closing AI further would
> require semantic Illustrator-format parsing (private format) — not scheduled.
> The honest M-C position: ~26.5% mixed is the pipeline's ceiling on this corpus.

---

### 6. `mediagit download` CLI Subcommand
*Source: protocol R&D analysis 2026-03; `crates/mediagit-protocol/src/streaming.rs`*

> **Server side DONE.** `crates/mediagit-server/src/handlers/browse.rs` ships
> `download_file_by_path`, `list_tree`, and `resolve_path_to_blob` — the full HTTP
> endpoint surface (`GET /{repo}/files/{*path}?ref=HEAD`, `GET /{repo}/tree/{*path}?ref=HEAD`).

**Remaining gap**: the CLI client subcommand does not exist yet.

**Target usage**: `mediagit download origin assets/logo.psd --ref main`

**File to create**: `crates/mediagit-cli/src/commands/download.rs` — adapt
`StreamingDownloader` in `streaming.rs` for VCS-scoped URLs. Wire into `commands/mod.rs`
and `main.rs`.

**Path security** (already enforced server-side): rejects `..`, absolute paths, null bytes.

Effort: **2-3 days** (CLI only).

---

## P2 — Medium Priority (Planned, Not Urgent)

### 7. `mediagit media info` Command
*Source: `docs/FUTURE_TODOS.md` §Phase 3*

Display metadata for local/committed media files using existing parsers in `mediagit-media`.

> **Rescoped 2026-07-07 (P4b shipped):** `mediagit-media` is now a dependency of
> `mediagit-cli`, and `crates/mediagit-cli/src/media_meta.rs` already formats media
> metadata for `show` and `stats` (knob: `MEDIAGIT_MEDIA_META`). This item is now a
> thin dedicated-subcommand wrapper over that module — **~100 LOC**, mostly clap
> surface + JSON output.

**CLI usage**: `mediagit media info <FILES...> [--format text|json] [--verbose] [--hash]`

**Files to create/modify:**
- `crates/mediagit-cli/src/commands/media.rs` — new command (~100 LOC, reuse `media_meta.rs`)
- `crates/mediagit-cli/src/commands/mod.rs`, `main.rs`

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
in the CLI but its test coverage is incomplete. *(Update 2026-07-07: fsck gained
chunk-delta chain validation — cycles / missing bases / depth — with 3 unit tests in
`src/fsck.rs`; the 4 gated integration tests remain `#[ignore]`d.)*

### `mediagit-protocol`

**`crates/mediagit-protocol/src/client.rs`** *(→ item 2 — ✅ DONE)*

Pack negotiation implemented in v0.2.6-beta.1. `collect_local_have(refdb)` computes the
have-set; server prunes the want-walk via `collect_objects_recursive(stop_at)`.
Incremental fetch validated in deep tests (all backends). Bitmap index remains a future
optimization (see §2b).

---

## Known Limitations

| # | Priority | Area | Description | Source |
|---|----------|------|-------------|--------|
| 1 | ~~P1~~ ✅ | **`.mediagitignore`** | **DONE** — v0.2.6-beta.1. `ignore` crate integration in `add` + `status` | `add.md:43` |
| 2 | ~~P1~~ ✅ | **Pack negotiation** | **DONE** — v0.2.6-beta.1. `collect_local_have` + `WantRequest{want,have}` + server `walk_reachable`; incremental fetch validated in deep tests | `client.rs` |
| 3 | P1 | **Parallel checkout I/O** | Checkout reads blobs sequentially; no parallel fetch | `checkout.rs` |
| 4 | ~~P1~~ ✅ | **TB-scale chunking** | **DONE** — Streaming format-aware chunking via mmap for all file sizes | v0.2.6-beta.1–3 |
| 6 | P1 | **`mediagit download` CLI** | Server endpoints shipped (`browse.rs`); CLI subcommand (`commands/download.rs`) not yet implemented | R&D 2026-03 |
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
| 21 | P3 | **FSCK test coverage** | Integration tests marked as potentially failing (fsck itself gained chunk-delta cycle/missing-base validation 2026-07-07) | `fsck_test.rs:37` |
| 23 | P2 | **Checkout re-materialization** | `checkout` doesn't restore a manually deleted working-tree file when switching to a branch with identical tree | found 2026-07-07 |
| 24 | P3 | **FBX structure chunking** | Top-level EndOffset walker ≈ CDC (Objects node holds ~98% of bytes); beating CDC needs an Objects-descending walker | fair trial 2026-07-07 |
| 25 | P3 | **EXR chunking** | No structure-aware chunking; blocked on real EXR fixtures | plan 2026-07-07 |
| 26 | P3 | **.sketch/.fig chunking** | ZIP containers get generic fixed chunking; entry-aware cuts unexplored | plan 2026-07-07 |
| 27 | P3 | **Video pHash** | No perceptual delta-base nomination for video; no viable crate | R&D 2026-07-07 |
| 28 | P3 | **Cross-process delta lock** | Chunk-delta cycle guard is per-process; concurrent multi-process writers to one local repo could still race | fix 2026-07-07 |
| 30 | P3 | **PSD spot-color channels** | `psd` crate 0.3.5 errors `"invalid channel id 3"` on PSDs with a spot-color channel; falls back to generic chunking, no crash/data-loss | found 2026-07-10, M5b |

# FUTURE_TODOS.md

Consolidated and **priority-ordered** registry of planned features, code-level TODOs, and
known limitations for MediaGit. Items are sourced from documentation, source code, and
historical claudedocs analyses.

> Last updated: 2026-08-25 (backlog reconciliation pass) | v0.3.0-rc.4 | Completed work is recorded in CHANGELOG.md and git history

**Priority levels:**
- **P0** — Quick win or active blocker — ≤1 day effort, implement immediately
- **P1** — High impact, near-term — 1-2 weeks, next milestone target
- **P2** — Medium impact or complex — 2-6 weeks, planned but not urgent
- **P3** — Low priority / long-term — deferred until triggered by demand

> **Two unrelated P0–P3 batches share this file and the same priority labels —
> don't conflate them.** The section immediately below is a closed, dated
> incident-response sprint (2026-07-18, leaked secrets) with its own 3-item P0
> checklist. The **Quick Reference — Priority Matrix** further down is the
> forward-looking product backlog (21 items, P1–P3, no P0s in it) that a
> "100% of P0–P3 implemented" claim would actually be about. Both reuse
> P0/P1/P2/P3 as effort/urgency labels; neither implies the other is done.

---

## ⚠ P0 — Security Remediation: leaked secrets in git history (2026-07-18, closed incident sprint)

Secrets (`.mcp.json` Morph key, `enc_key`/`enc_key.pub` SSH pair, an Anthropic API key) were
scrubbed from git history and force-pushed on all branches; a fresh-clone verification
confirmed the remote is clean. Both API keys were revoked regardless of the cleanup.

- [ ] Close/regenerate dependabot PRs (all branch hashes changed by the history rewrite) —
      **unverified 2026-08-25**: no `gh` CLI / GitHub API access in this environment to check
      PR state, and no related commits in `git log`. Status unknown, not confirmed done.
- [ ] Request cached-view purge via GitHub Support — public repo: old commits (e.g. `82295e8`,
      `7177269`) may remain servable by direct SHA URL until purged — **unverified 2026-08-25**,
      same reason (no GitHub API access; this is an external support request, not something
      `git log` would show either way).
- [ ] Delete the backup mirror `D:\own\saas\mediagit-core-backup-20260718.git` — it is now the
      **only remaining copy of the secrets on disk**; remote verified clean 2026-07-18 so its
      rollback purpose is served. **CONFIRMED STILL PRESENT 2026-08-25** — the directory still
      exists on disk (`D:\own\saas\mediagit-core-backup-20260718.git\config` etc., last
      modified 2026-07-18). Still open, not a false positive.

---

## Quick Reference — Priority Matrix

| # | Item | Priority | Effort | Blocks / Enables |
|---|------|----------|--------|-----------------|
| 1 | ~~Bitmap index (pack negotiation follow-up)~~ | **DONE** | — | Shipped: `crates/mediagit-versioning/src/bitmap.rs` (roaring-bitmap reachability index), wired into `gc` (`crates/mediagit-cli/src/commands/gc.rs`) and pack negotiation (`crates/mediagit-server/src/handlers/repo.rs`), covered by `crates/mediagit-server/tests/e2e_bitmap_negotiation.rs`, gated behind `MEDIAGIT_BITMAP` (default on) |
| 2 | HTTP/3 via reqwest feature flag | **P3** | 1 day | When reqwest `http3` stabilizes (~2026 Q4) |
| 3 | Git migration tooling (re-add filter/install/track) | **P3** | 1-2 wk | When user base requests migration |
| 4 | `mediagit://` URL scheme | **P3** | 1 day | Post-HTTP/3 adoption |
| 5 | Differential checkout (only changed files) | **P3** | 1-2 wk | 70% branch switch latency reduction |
| 6 | Incremental status scan (inode/mtime cache) | **P3** | 1-2 wk | Repeated `status` performance |
| 7 | Pack file format documentation | **P3** | 0.5 day | Book completeness |
| 8 | TOML-configurable similarity thresholds | **P3** | 0.5 day | User tunability |
| 9 | Windows ARM64 native binaries | **P3** | — | Blocked on GitHub runner availability |
| 10 | macOS Metal GPU acceleration | **P3** | 2-3 wk | Apple Silicon image processing |
| 11 | Security / Audit enhancements (v0.3.0+) | **P3** | — | Compliance, SIEM |
| 12 | GA knob-policy execution (remove/keep each `MEDIAGIT_*` knob) | **P1** (at GA) | 1 day | `docs/next-set/knob-policy.md` is the decision record |
| 13 | SSO integration, multi-region, Web UI (v1.0.0) | **P3** | — | Enterprise features |
| 14 | FBX Objects-descending walker (or delete walker at GA) | **P3** | 1-2 wk | Fair trial closed 2026-07-07: top-level cuts ≈ CDC (+0.003pp) |
| 15 | EXR structure-aware chunking | **P3** | 3-5 days | Needs real EXR fixtures first (creating them requires the `exr` crate) |
| 16 | .sketch/.fig ZIP-entry-aware chunking | **P3** | 3-5 days | No fixtures in corpus yet |
| 17 | Video pHash (keyframe extract + image_hasher) | **P3** | 1-2 wk | No viable video-phash crate (verified 2026-07-07) |
| 18 | Cross-process chunk-delta write lock | **P3** | 2-3 days | In-process race fixed 2026-07-07; multi-process writers to one local repo could still race (CLI never does this) |
| 19 | phash.idx compaction | **P3** | 0.5 day | Append-only today; only matters >1M entries (~16 MB) |
| 20 | PSD spot-color channel parse failure | **P3** | Unscoped (needs upstream fix or crate swap) | `psd` crate 0.3.5 errors "invalid channel id 3" on PSDs with a spot-color channel; found 2026-07-10, M5b |
| 21 | ~~FSCK integration test coverage~~ | **DONE** | — | `crates/mediagit-versioning/tests/fsck_integration_test.rs` now has 10 active `#[tokio::test]`s and zero `#[ignore]` markers; the gating FIXME comment cited below is gone from the file |

---

## P2 — Medium Priority (Planned, Not Urgent)

### 1. Bitmap Index (Pack Negotiation Follow-up) — DONE

**Shipped.** `crates/mediagit-versioning/src/bitmap.rs` persists a commit's full
object closure as a roaring-bitmap-backed `ReachabilityBitmap`, so pack negotiation
can skip the BFS walk when a valid bitmap exists for the client's `have` tip. Per
the module's own correctness contract, it is derived data only: any miss, staleness,
or format-version mismatch falls back to `walk_reachable` silently, never errors.
Generation is wired into `gc` (`crates/mediagit-cli/src/commands/gc.rs`); consumption
into pack negotiation (`crates/mediagit-server/src/handlers/repo.rs`). Gated behind
`MEDIAGIT_BITMAP` (default on; `=0` reproduces pre-bitmap BFS-only behavior byte
for byte — see `docs/next-set/knob-policy.md`, local-only). Covered end-to-end by
`crates/mediagit-server/tests/e2e_bitmap_negotiation.rs`.

---

## P3 — Low Priority / Long-Term

### 2. HTTP/3 Support via reqwest Feature Flag
*Source: protocol R&D analysis 2026-03; `crates/mediagit-protocol/src/streaming.rs`*

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

### 3. Git Migration Tooling (Re-add `filter`/`install`/`track`)
*Source: `CHANGELOG.md` §Unreleased → Removed*

The `mediagit-git` crate remains in the workspace and compiles independently. Re-integration
as a first-class migration CLI flow is deferred until there is user demand.

Trigger: user requests for git/git-LFS → MediaGit migration tooling. Effort: **1-2 weeks**.

---

### 4. `mediagit://` URL Scheme
*Source: protocol R&D analysis 2026-03*

A native `mediagit://` URL scheme for brand identity, post-HTTP/3 adoption. Maps to
`https://` or `quic://` internally. Effort: **1 day** (low value until HTTP/3 is live).

---

### 5. Differential Checkout (Only Changed Files)
*Source: `claudedocs/` optimization roadmap*

Branch switching currently rewrites all files even if only a subset changed. Diffing the
source and target trees and only updating changed paths targets **~70% latency reduction**
(estimated 496ms → ~150ms for medium repos).

Requires tree diff engine in `mediagit-versioning`. Effort: **1-2 weeks**.

---

### 6. Incremental Status Scan (inode / mtime Cache)
*Source: `claudedocs/` optimization roadmap*

Full-tree scan on every `status` invocation. An inode cache / mtime-based incremental scan
(similar to git's index) would reduce repeated-status overhead significantly for repos with
large working trees.

Effort: **1-2 weeks**.

---

### 7. Pack File Format Documentation
*Source: `book/src/reference/file-formats.md:15`*

`.mediagit/objects/pack/` is reserved for future pack-file storage. The directory layout
and on-disk format are not documented in the book. Effort: **0.5 day**.

---

### 8. TOML-Configurable Similarity Thresholds
*Source: `book/src/guides/performance.md:60-65`*

Similarity thresholds (controlling when delta encoding is triggered) are hardcoded in
`smart_compressor.rs`. Planned config keys:
- `[performance] ai_pdf_similarity_threshold = 0.15`
- `[performance] office_similarity_threshold = 0.20`
- `[performance] default_similarity_threshold = 0.80`

Effort: **0.5 day** (config schema + read + pass-through).

---

### 9. Windows ARM64 Native Binaries
*Source: `book/src/installation/windows-arm64.md`*

Blocked on GitHub Actions native ARM64 Windows runner availability. Currently, x64 binary
runs via Windows ARM emulation at reduced performance.

---

### 10. macOS Metal GPU Acceleration
*Source: `book/src/installation/macos-arm64.md:88`*

GPU-accelerated image processing via Apple Metal for Apple Silicon builds. No concrete
implementation plan yet. Effort: **2-3 weeks** (research + implementation).

---

### 11. Security / Audit Enhancements (v0.3.0+)
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
- `mediagit migrate` — import from Git-LFS repositories *(see item 3)*
- Chocolatey and Homebrew package managers
- Official VS Code extension (file status, staging UI)

### v1.0.0 — Production-Grade Enterprise Features
- Stable API and wire protocol (v1 guarantee)
- SSO integration (OIDC/SAML) for enterprise auth
- Multi-region active-active replication
- Audit log export (compliance — SOC 2, GDPR)
- Plugin system for custom media type handlers
- Web UI for repository browsing and review workflows
- Commercial support tiers

> See [FUTURE_TODOS.md](./FUTURE_TODOS.md) for individual item details (this file).

---

## Code TODOs (from source — grouped by crate)

### `mediagit-versioning`

**`crates/mediagit-versioning/tests/fsck_integration_test.rs`** *(→ item 21)* — **DONE,
verified 2026-08-25.** The FIXME marker quoted here in past revisions of this file
(`// FIXME: FSCK functionality is under development...`) is no longer present in the
file. All 10 tests (`test_fsck_clean_repository`, `test_fsck_detect_corrupted_object`,
`test_fsck_detect_missing_object`, `test_fsck_detect_broken_reference`,
`test_fsck_quick_mode`, `test_fsck_full_mode`, `test_fsck_repair_broken_reference`,
`test_fsck_repair_dry_run`, `test_fsck_connectivity_check`,
`test_fsck_max_objects_limit`) are active `#[tokio::test]`s with zero `#[ignore]`
attributes in the file.

---

## Known Limitations

| # | Priority | Area | Description | Source |
|---|----------|------|-------------|--------|
| 2 | P3 | **HTTP/3** | reqwest `http3` feature not yet stable | R&D 2026-03 |
| 3 | P3 | **Git migration CLI** | `mediagit-git` crate exists; `filter/install/track` removed from binary | CHANGELOG |
| 4 | P3 | **`mediagit://` scheme** | No native URL scheme; uses `http://` | R&D 2026-03 |
| 5 | P3 | **Differential checkout** | Full tree rewritten on branch switch; ~70% latency reduction possible | claudedocs |
| 6 | P3 | **Incremental status** | Full-tree scan on every `status` invocation | claudedocs |
| 7 | P3 | **Pack file docs** | `.mediagit/objects/pack/` format not documented | `file-formats.md` |
| 8 | P3 | **Similarity thresholds** | Delta thresholds hardcoded, not configurable via `config.toml` | `performance.md` |
| 9 | P3 | **Windows ARM64** | No native pre-built binary; x64 emulation works but slower | `windows-arm64.md` |
| 10 | P3 | **Metal GPU** | No GPU-accelerated image processing on Apple Silicon | `macos-arm64.md:88` |
| 11 | P3 | **SIEM / audit** | No Splunk/ELK connectors; SOC 2/GDPR export is v1.0.0 | claudedocs |
| 14 | P3 | **FBX structure chunking** | Top-level EndOffset walker ≈ CDC (Objects node holds ~98% of bytes); beating CDC needs an Objects-descending walker | fair trial 2026-07-07 |
| 15 | P3 | **EXR chunking** | No structure-aware chunking; blocked on real EXR fixtures | plan 2026-07-07 |
| 16 | P3 | **.sketch/.fig chunking** | ZIP containers get generic fixed chunking; entry-aware cuts unexplored | plan 2026-07-07 |
| 17 | P3 | **Video pHash** | No perceptual delta-base nomination for video; no viable crate | R&D 2026-07-07 |
| 18 | P3 | **Cross-process delta lock** | Chunk-delta cycle guard is per-process; concurrent multi-process writers to one local repo could still race | fix 2026-07-07 |
| 20 | P3 | **PSD spot-color channels** | `psd` crate 0.3.5 errors `"invalid channel id 3"` on PSDs with a spot-color channel; falls back to generic chunking, no crash/data-loss | found 2026-07-10, M5b |
| 21 | — | ~~**FSCK test coverage**~~ | **DONE, verified 2026-08-25** — 10 active integration tests, 0 `#[ignore]`d, gating FIXME removed | `fsck_integration_test.rs` |

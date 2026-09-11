# FUTURE_TODOS.md

Consolidated and **priority-ordered** registry of planned features, code-level TODOs, and
known limitations for MediaGit. Items are sourced from documentation and source code;
each entry cites the file it was verified against.

> Last updated: 2026-09-11 (items 21-24 and 26 closed; 25 partially closed) | v0.4.0-rc.1 | Completed work is recorded in CHANGELOG.md and git history

**Priority levels:**
- **P0** — Quick win or active blocker — ≤1 day effort, implement immediately
- **P1** — High impact, near-term — 1-2 weeks, next milestone target
- **P2** — Medium impact or complex — 2-6 weeks, planned but not urgent
- **P3** — Low priority / long-term — deferred until triggered by demand

> **Two unrelated P0–P3 batches share this file and the same priority labels —
> don't conflate them.** The section immediately below is a closed, dated
> incident-response sprint (2026-07-18, leaked secrets) with its own 3-item P0
> checklist. The **Quick Reference — Priority Matrix** further down is the
> forward-looking product backlog (19 items, P1–P3, no P0s in it) that a
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
| 2 | HTTP/3 via reqwest feature flag | **P3** | 1 day | When reqwest `http3` stabilizes (~2026 Q4) |
| 3 | Git migration tooling (rewrite filter/install/track) | **P3** | 2-3 wk | Crate deleted in `c5d0a23`; rewrite, not re-integration |
| 4 | `mediagit://` URL scheme | **P3** | 1 day | Post-HTTP/3 adoption |
| 5 | Differential checkout (only changed files) | **P3** | 1-2 wk | 70% branch switch latency reduction |
| 6 | Incremental status scan (inode/mtime cache) | **P3** | 1-2 wk | Repeated `status` performance |
| 7 | Pack file format documentation | **P3** | 0.5 day | Book completeness |
| 8 | TOML-configurable similarity thresholds | **P3** | 0.5-3 day | User tunability; ~32 values across 2 tables, not 3 |
| 9 | Windows ARM64 native binaries | **P3** | — | Blocked on GitHub runner availability |
| 10 | macOS Metal GPU acceleration | **P3** | 2-3 wk | Apple Silicon image processing |
| 11 | Security / Audit enhancements (v0.3.0+) | **P3** | — | Compliance, SIEM |
| 12 | GA knob-policy execution (remove/keep each `MEDIAGIT_*` knob) | **P1** (at GA) | 1 day | `docs/next-set/knob-policy.md` is the decision record (gitignored, local-only) |
| 13 | SSO integration, multi-region, Web UI (v1.0.0) | **P3** | — | Enterprise features |
| 14 | FBX Objects-descending walker (or delete walker at GA) | **P3** | 1-2 wk | Fair trial closed 2026-07-07: top-level cuts ≈ CDC (+0.003pp) |
| 15 | EXR structure-aware chunking | **P3** | 3-5 days | Needs real EXR fixtures first (creating them requires the `exr` crate) |
| 16 | .sketch/.fig ZIP-entry-aware chunking | **P3** | 3-5 days | No fixtures in corpus yet |
| 17 | Video pHash (keyframe extract + image_hasher) | **P3** | 1-2 wk | No viable video-phash crate (verified 2026-07-07) |
| 18 | Cross-process chunk-delta write lock | **P3** | 2-3 days | In-process race fixed 2026-07-07; multi-process writers to one local repo could still race (CLI never does this) |
| 19 | phash.idx compaction | **P3** | 0.5 day | Append-only today; only matters >1M entries (~16 MB) |
| 20 | PSD spot-color channel parse failure | **P3** | Unscoped (needs upstream fix or crate swap) | `psd` crate 0.3.5 errors "invalid channel id 3" on PSDs with a spot-color channel; found 2026-07-10, M5b |
| ~~21~~ | ~~`remote add` accepts URL schemes with no transport~~ | **DONE** v0.4.0 | — | Both gates narrowed to HTTP(S): `validate_url` and, the one that mattered, `Config::resolve_remote_url` (9 callers). Tests red-verified |
| ~~22~~ | ~~Wire `load_with_overrides` into the real config path (or delete it)~~ | **DONE** v0.4.0 | — | Deleted, with `apply_env_overrides`. 14 inert `MEDIAGIT_*` names retired; `MEDIAGIT_API_KEY` and `MEDIAGIT_CHUNK_WRITE_CONCURRENCY` stay live via their own read sites |
| ~~23~~ | ~~Dead config keys: decide wire-up vs removal~~ | **DONE** v0.4.0 | — | Removed: `[compression]`, `max_concurrency`, `chunk_write_concurrency` (TOML key only), `[performance.connection_pool]`, `[performance.timeouts]`. Breaking change recorded in CHANGELOG |
| ~~24~~ | ~~`s3.rs` module doc describes a credential chain that does not exist~~ | **DONE** v0.4.0 | — | Now documents both paths: plain-AWS uses the SDK chain, custom-endpoint (MinIO) uses explicit `S3Config` creds only |
| 25 | Truth-up the prose in `book/src/cli/` (39 pages, not 29) | **P3** (partial) | 1-2 days | Mechanical claims swept v0.4.0: all 12 cited `MEDIAGIT_*` vars verified live in code, config-section and path claims checked, one stale `[compression]` claim fixed in `init.md`. **Narrative behavioural prose across the 39 pages is still unswept.** |
| ~~26~~ | ~~Re-verify `CHANGELOG.md:314`~~ | **DONE** v0.4.0 | — | Rewritten as reported-at-the-time and unverifiable; the run's `summary.json` is gone, and neighbouring artifacts record 234 gates, not 237 |

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

### 3. Git Migration Tooling (rewrite `filter`/`install`/`track` from scratch)
*Source: `c5d0a23` (2026-07-29) — corrected 2026-08-26*

**The `mediagit-git` crate no longer exists.** Earlier revisions of this entry said it
"remains in the workspace and compiles independently" — it was deleted in `c5d0a23`, and
not for being merely unused. `FilterDriver::clean` read a file from stdin, hashed it, wrote
a pointer to stdout and **never stored the content anywhere** (a "NOTE: Object storage
integration pending" comment sat in the gap), then logged success. Installed as the git
filter its own README documented, it would have destroyed every file it touched while
reporting that it had worked.

So this is a **from-scratch rewrite, not a re-integration**, and the deleted code must not
be resurrected as a starting point. Two things also depend on it staying gone: deleting it
removed `git2` and `openssl-sys` from the dependency graph entirely (it was the only path
in, via git2's vendored-openssl), which retired RUSTSEC-2026-0183/-0184 from the
`.cargo/audit.toml` ignore list and made `deny.toml`'s `openssl-sys` ban enforceable
instead of pre-broken. Re-adding a git2-based importer re-opens all of that.

Git/git-LFS import is currently documented as **unsupported**, which is a decision rather
than an omission — MediaGit is a standalone VCS for media.

Trigger: user requests for git/git-LFS → MediaGit migration tooling. Effort: **2-3 weeks**
(rewrite + a dependency path that does not reintroduce openssl).

---

### 4. `mediagit://` URL Scheme
*Source: protocol R&D analysis 2026-03*

A native `mediagit://` URL scheme for brand identity, post-HTTP/3 adoption. Maps to
`https://` or `quic://` internally. Effort: **1 day** (low value until HTTP/3 is live).

---

### 5. Differential Checkout (Tree-Diff, Not Per-File Hashing)
*Source: `crates/mediagit-versioning/src/checkout.rs` — verified 2026-08-26*

**Partly done already.** Earlier revisions said branch switching "rewrites all files even
if only a subset changed" — it does not. `checkout_entry_differential`
(`checkout.rs:161`) stats the on-disk file, compares size against
`odb.get_object_size`, then compares a full `Oid::from_file` hash, and skips the write when
they match. `branch.rs:650` reaches it via `checkout_commit` → `checkout_tree_optimized`
→ `checkout.rs:577`, so an ordinary branch switch already avoids rewriting unchanged files.

What remains is the cost the skip does **not** avoid: every path in the target tree is still
visited, stat'd and fully hashed. A real tree diff would compare the source and target trees
and never visit an unchanged path at all. The **~70% reduction** (496ms → ~150ms, medium
repos) claimed here was measured against the pre-skip behaviour, so it is **stale as an
estimate** — re-baseline before scoping.

Requires a tree diff engine in `mediagit-versioning`. Effort: **1-2 weeks**.

---

### 6. Incremental Status Scan (inode / mtime Cache)
*Source: `crates/mediagit-cli/src/commands/status.rs` (`scan_working_directory`, called at
`status.rs:383`)*

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
*Source: `book/src/guides/performance.md` §Delta Encoding — pointer corrected 2026-08-26*

**Not in `smart_compressor.rs`** — earlier revisions of this entry (and the book passage it
cites) named that file; there is no such file, and the `smart_compressor/` module that
replaced it holds no similarity threshold at all. The thresholds live in **two independent
tables**:

- `crates/mediagit-versioning/src/similarity.rs` — `get_similarity_threshold(filename)`,
  a 19-arm extension match (`ai`/`pdf`/`psd` → 0.15, office → 0.20, text → 0.85, config →
  0.95, images → 0.70, video → 0.50, `blend` → 0.40, `hip` → 0.35, NLE projects → 0.25, …),
  defaulting to `MIN_SIMILARITY_THRESHOLD`.
- `crates/mediagit-versioning/src/odb/mod.rs` — `delta_ratio_threshold(codec, chunk_type)`,
  keyed on codec rather than extension (ProRes/DNxHR/J2K/raw → 0.60, subtitles/metadata →
  0.90, default 0.80).

The three config keys previously planned here cover only 3 of those ~32 values, so exposing
them alone would leave most of the surface still hardcoded and split the source of truth
across a config file and two match arms. Decide first whether the config surface is
per-extension, per-codec, or a single global scale factor.

Effort: **0.5 day** for the three-key subset as originally scoped; **2-3 days** to expose
both tables coherently.

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
*Source: `crates/mediagit-security/src/audit.rs`*

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

## Known Limitations

| # | Priority | Area | Description | Source |
|---|----------|------|-------------|--------|
| 2 | P3 | **HTTP/3** | reqwest `http3` feature not yet stable | R&D 2026-03 |
| 3 | P3 | **Git migration CLI** | `mediagit-git` deleted in `c5d0a23` (its clean filter destroyed file content); git/git-LFS import documented unsupported | `c5d0a23` |
| 4 | P3 | **`mediagit://` scheme** | No native URL scheme; uses `http://` | R&D 2026-03 |
| 5 | P3 | **Differential checkout** | Unchanged files already skipped by hash; every path still visited + hashed, no tree diff | `checkout.rs:161` |
| 6 | P3 | **Incremental status** | Full-tree scan on every `status` invocation | `status.rs:383` |
| 7 | P3 | **Pack file docs** | `.mediagit/objects/pack/` format not documented | `file-formats.md` |
| 8 | P3 | **Similarity thresholds** | Hardcoded in `similarity.rs` (27 arms) + `odb/mod.rs` (`delta_ratio_threshold`), not configurable via `config.toml` | `similarity.rs` |
| 9 | P3 | **Windows ARM64** | No native pre-built binary; x64 emulation works but slower | `windows-arm64.md` |
| 10 | P3 | **Metal GPU** | No GPU-accelerated image processing on Apple Silicon | `macos-arm64.md:88` |
| 11 | P3 | **SIEM / audit** | No Splunk/ELK connectors; SOC 2/GDPR export is v1.0.0 | `audit.rs` |
| 14 | P3 | **FBX structure chunking** | Top-level EndOffset walker ≈ CDC (Objects node holds ~98% of bytes); beating CDC needs an Objects-descending walker | fair trial 2026-07-07 |
| 15 | P3 | **EXR chunking** | No structure-aware chunking; blocked on real EXR fixtures | plan 2026-07-07 |
| 16 | P3 | **.sketch/.fig chunking** | ZIP containers get generic fixed chunking; entry-aware cuts unexplored | plan 2026-07-07 |
| 17 | P3 | **Video pHash** | No perceptual delta-base nomination for video; no viable crate | R&D 2026-07-07 |
| 18 | P3 | **Cross-process delta lock** | Chunk-delta cycle guard is per-process; concurrent multi-process writers to one local repo could still race | fix 2026-07-07 |
| 20 | P3 | **PSD spot-color channels** | `psd` crate 0.3.5 errors `"invalid channel id 3"` on PSDs with a spot-color channel; falls back to generic chunking, no crash/data-loss | found 2026-07-10, M5b |
| 25 | P3 | **`book/src/cli/` prose partly unswept** | The mechanically-checkable surface was swept in v0.4.0: all 12 `MEDIAGIT_*` variables cited across the 39 pages were verified to exist in code, config-section and repo-path claims were checked, and one stale `[compression]` claim in `init.md` was corrected. What remains unswept is narrative behavioural prose — descriptions of what a command *does* — which `13_docs` cannot gate because it only compares `--flag` tokens against `--help`. | partial, v0.4.0 |

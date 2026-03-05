# Codebase Refactor & Consistency Plan — mediagit-core

> **Branch**: `chore/ci_cd_pipeline`
> **Scope**: 14 crates, ~92K lines of Rust
> **Nature**: Non-functional — consistency, cleanliness, structure only. No API changes.
> **Verified**: All claims independently validated against codebase on 2026-02-28.

---

## About This Project

MediaGit is its **own version control system** purpose-built for media files. It uses git-like concepts (commits, branches, refs, trees) but is **not git-compatible**. The `mediagit-git` crate exists solely for **migration from git** — it provides filter drivers and pointer file support to help users move media projects from git/git-lfs into MediaGit.

---

## Why This Plan

The mediagit-core workspace has accumulated quality debt across dependency versions, metadata, dead code, error handling patterns, CLI output consistency, and build configuration. This plan systematically brings all 14 crates to a consistent baseline before further feature work.

**Analysis basis**: 3 parallel Explore agents + 2 verification agents + 1 dependency audit. Every claim below has been validated against the actual codebase with file paths and line numbers.

---

## Findings Overview (Verified)

| # | Category | Severity | Details | Verified |
|---|----------|----------|---------|----------|
| 1 | Cargo.toml metadata gaps | 🟡 Medium | 10/14 crates missing `repository`/`homepage`/`documentation`; 9/14 missing `description` | ✅ exact |
| 2 | Copyright header drift | 🟡 Medium | **142 files** have `winnyboy5` headers; canonical is `MediaGit Contributors` | ✅ 142 files confirmed |
| 3 | Dead code accumulation | 🟡 Medium | 7 `#[allow(dead_code)]` in `progress.rs`; module-level suppress in `output.rs` | ✅ lines 64,119,140,162,204,210,235 |
| 4 | TODO/FIXME debt | 🟢 Low | `bisect.rs`, `tag.rs` TODOs; `fsck` FIXME in test; deprecated fn missing `#[deprecated]` attr | ✅ |
| 5 | Error type inconsistency | 🟢 Low | `MediaError` + `GitError` lack `is_*()` predicates; `StorageError` + `CompressionError` have them | ✅ no impl blocks |
| 6 | Progress bar duplication | 🟡 Medium | 8 near-identical progress bar factory methods (only template/color differ) | ✅ |
| 7 | Output formatting drift | 🟡 Medium | **23/32** command files import `console::style` directly instead of using `output::*` module | ✅ 23 files confirmed |
| 8 | CLI flag duplication | 🟡 Medium | Multiple commands re-declare `-q`/`-v` as `#[arg(short, long)]` — duplicates global flags | ✅ add.rs:86, branch.rs:113, etc. |
| 9 | Utility scatter | 🟢 Low | `format_duration_ago` (stats.rs:26), `validate_ref_name` (push.rs:27), `categorize_extension` (stats.rs:40) | ✅ exact lines |
| 10 | Clippy `unwrap_used` suppressions | 🟢 Low | 1 in prod code (cert.rs:186), 22 in test modules — **test suppressions are standard practice** | ✅ corrected from "30+" |
| 11 | Cloud deps always-compiled | 🟡 Medium | Azure/GCS deps in `mediagit-storage` not feature-gated | ✅ |
| 12 | Non-workspace dep versions | 🟢 Low | `reqwest = "0.12"` in protocol (prod), metrics (dev), **and server** (dev) — not via workspace | ✅ 3 crates, not 2 |
| 13 | Test coverage gap | 🔴 High | All 32 CLI commands have zero unit tests — **deferred to separate PR** | ✅ |

---

## Findings Overview (Continued — New Additions)

| # | Category | Severity | Details | Verified |
|---|----------|----------|---------|----------|
| 14 | Progress bar outlier in `add.rs` | 🟡 Medium | `add.rs` creates standalone `ProgressBar` + `ProgressStyle` directly, bypassing `ProgressTracker` | ✅ add.rs:214–222 |
| 15 | Unused progress bar factories | 🟡 Medium | 4 of 8 factory methods unused: `upload_bar`, `verify_bar`, `merge_bar`, `io_bar` | ✅ progress.rs:64,119,140,162 |
| 16 | Missing `missing_docs` lint | 🟡 Medium | No `missing_docs` in workspace lints — doc coverage entirely voluntary | ✅ root Cargo.toml |
| 17 | Missing crate-level `//!` docs | 🟡 Medium | `mediagit-security` and `mediagit-server` have **no** `//!` at all; `mediagit-media` has only 6 bullets | ✅ lib.rs files |
| 18 | Documentation coverage variance | 🟡 Medium | Storage/versioning/compression at ~95–100%; security/media/metrics at ~55–65% | ✅ agent analysis |
| 19 | Inline comment tag inconsistency | 🟢 Low | `// NOTE:` used for both "rationale" and "pending work"; `// TODO` missing colons | ✅ 17 occurrences |
| 20 | Stale GitHub Actions versions | 🟡 Medium | `actions/checkout@v4` (latest v6), `codecov-action@v4` (v5), `upload-pages-artifact@v3` (v4) | ✅ workflow YAMLs |
| 21 | Stale Cargo dependencies | 🟡 Medium | brotli 7→8, dialoguer 0.11→0.12, criterion 0.5→0.8 (MSRV ok), tempfile minor bump | ✅ Cargo.toml files |
| 22 | secrecy API breaking change | 🔴 High | `secrecy 0.8→0.10`: `SecretVec<T>` removed → `SecretBox<Vec<T>>`; `SecretString::new()` → `::from()` | ✅ encryption.rs, kdf.rs |
| 23 | google-cloud deps stale | 🔴 High | `google-cloud-storage 0.24→1.8` + `google-cloud-auth 0.17→1.6` (must bump together) | ✅ mediagit-storage/Cargo.toml |

---

## Implementation — 8 Phases

Each phase is independently committable and passes CI before the next begins.

**Execution order note**: Phase 0 must run first — `encryption.rs` is touched by both Phase 0 (secrecy migration) and Phase 7 (documentation); `mediagit-storage/Cargo.toml` + `gcs.rs` are touched by both Phase 0 (GCS bump) and Phase 5 (feature gating). Doing Phase 0 first means later phases work on already-updated code.

---

### Phase 0 — Dependency Upgrades

**Effort**: ~1.5 hrs | **Risk**: Low–High (tiered)

Bring all dependencies current before the consistency work begins. Executed in 4 tiers from zero-risk to code-migration.

#### Tier 1 — GitHub Actions (YAML only, zero code risk)

Update version tags in workflow files:

| File | Change |
|------|--------|
| `.github/workflows/ci.yml` | `actions/checkout@v4` → `@v6`; `codecov/codecov-action@v4` → `@v5` |
| `.github/workflows/docs.yml` | `actions/checkout@v4` → `@v6`; `actions/upload-pages-artifact@v3` → `@v4` |
| `.github/workflows/bench.yml` | `actions/checkout@v4` → `@v6` |
| `.github/workflows/release.yml` | `actions/checkout@v4` → `@v6` (if present) |

#### Tier 2 — Cargo version bumps only (no code changes)

All semver-compatible bumps. Apply to workspace `Cargo.toml` unless noted:

| Dependency | From | To | Notes |
|-----------|------|----|-------|
| `brotli` | `7.0` | `8.0` | workspace dep |
| `criterion` | `0.5` | `0.8` | workspace dep; MSRV bumped to 1.80 in 0.6 — project at 1.92 ✅; API unchanged |
| `dialoguer` | `0.11` | `0.12` | `mediagit-cli/Cargo.toml` only |
| `tempfile` | `3` | `3.25` | `cargo update -p tempfile` (already `"3"` in workspace, semver-compatible) |

#### Tier 3a — secrecy 0.8 → 0.10 (code migration, medium risk)

> `SecretVec<T>` removed in 0.10. `SecretString::new(s)` renamed to `SecretString::from(s)`.

**`crates/mediagit-security/Cargo.toml`**:
```diff
-secrecy = { version = "0.8", features = ["serde"] }
+secrecy = { version = "0.10", features = ["serde"] }
```

**`crates/mediagit-security/src/encryption.rs`**:
```diff
-use secrecy::{ExposeSecret, SecretVec};
+use secrecy::{ExposeSecret, SecretBox};

 pub struct EncryptionKey {
-    key: SecretVec<u8>,
+    key: SecretBox<Vec<u8>>,
 }

 // Clone impl (line ~103):
-    key: SecretVec::new(self.key.expose_secret().to_vec()),
+    key: SecretBox::new(Box::new(self.key.expose_secret().clone())),

 // from_bytes (line ~120):
-    key: SecretVec::new(bytes),
+    key: SecretBox::new(Box::new(bytes)),

 // generate (line ~134):
-    key: SecretVec::new(key_bytes),
+    key: SecretBox::new(Box::new(key_bytes)),

 // expose_key (line ~145):
-    self.key.expose_secret()
+    self.key.expose_secret().as_slice()
```

**`crates/mediagit-security/src/kdf.rs`** — ~8 occurrences in tests (lines 185, 393, 403, 404, 418, 429, 430, 442, 443, 463):
```diff
-SecretString::new(...)
+SecretString::from(...)
```

Same change in `crates/mediagit-security/tests/security_test.rs` and `crates/mediagit-security/benches/encryption_benchmark.rs`.

#### Tier 3b — google-cloud-storage 0.24 → 1.8 + google-cloud-auth 0.17 → 1.6 (high risk, compile-driven)

> Must upgrade together. `google-cloud-storage 1.8` requires `google-cloud-auth ^1.5`.
> The 1.x API preserves the same auth patterns (`ClientConfig::with_credentials`, `with_auth()`, `CredentialsFile`) — 1.0 was a maturity/GA release, not a redesign.

**`crates/mediagit-storage/Cargo.toml`**:
```diff
-google-cloud-storage = "0.24"
-google-cloud-auth = "0.17"
+google-cloud-storage = "1.8"
+google-cloud-auth = "1.6"
```

**`crates/mediagit-storage/src/gcs.rs`** — compile-driven:
1. `cargo check -p mediagit-storage`
2. Fix any changed import paths (HTTP request/response types may have reorganized)
3. Core logic (auth, retry, upload/download) should compile with minimal changes

#### Phase 0 Verification

```bash
cargo check --workspace
cargo test --workspace
cargo bench --workspace --no-run   # benches must compile
cargo clippy --workspace -- -D warnings
```

---

### Phase 1 — Cargo.toml Metadata Normalization

**Effort**: ~30 min | **Risk**: None

Add the three missing workspace-inherited fields and `description` to all crates that lack them.

**Pattern to apply** in `[package]` section (after `authors.workspace = true`):
```toml
repository.workspace    = true
homepage.workspace      = true
documentation.workspace = true
description             = "<short description>"
```

**Crates needing repo/homepage/docs + description** (5 crates):

| Crate | Description to add |
|-------|--------------------|
| `mediagit-cli` | "Version control CLI purpose-built for media files" |
| `mediagit-compression` | "Content-aware compression engine for media objects" |
| `mediagit-media` | "Media metadata extraction and merge strategies" |
| `mediagit-storage` | "Unified async storage backend trait and implementations" |
| `mediagit-versioning` | "Core VCS engine: ODB, refs, commits, trees, packs" |

**Crates needing repo/homepage/docs only** (have description already) (5 crates):

| Crate | Existing description |
|-------|---------------------|
| `mediagit-config` | "Configuration management system for MediaGit Core" |
| `mediagit-metrics` | "Prometheus metrics and performance monitoring for MediaGit" |
| `mediagit-migration` | "Storage backend migration tool for MediaGit" |
| `mediagit-observability` | "Structured logging and observability for MediaGit" |
| `mediagit-test-utils` | "Shared test utilities for MediaGit crates" (publish=false) |

**Crates needing description only** (already have repo/homepage/docs) (4 crates):

| Crate | Description to add |
|-------|--------------------|
| `mediagit-git` | "Git migration support: filter drivers and pointer files" |
| `mediagit-protocol` | "Network push/pull/clone protocol implementation" |
| `mediagit-security` | "Authentication, encryption, and audit trail" |
| `mediagit-server` | "Axum REST API server for MediaGit repositories" |

**Also fix — workspace reqwest**:
- Add to root `Cargo.toml` `[workspace.dependencies]`:
  ```toml
  reqwest = { version = "0.12", default-features = false, features = ["json", "stream", "rustls-tls"] }
  ```
- `crates/mediagit-protocol/Cargo.toml` line 21: change to `reqwest = { workspace = true }`
- `crates/mediagit-metrics/Cargo.toml` line 25 (dev-dep): change to `reqwest = { workspace = true }`
- `crates/mediagit-server/Cargo.toml` line 60 (dev-dep): change to `reqwest = { workspace = true }`

---

### Phase 2 — Copyright Headers & Dead Code Cleanup

**Effort**: ~1.5 hrs | **Risk**: Low

#### 2a. Normalize copyright headers (**142 files**)

**Scope**: `grep -rl "winnyboy5" crates/ --include="*.rs"` returns **142 files**. This is the largest single change by file count.

All files must use the canonical form:
```rust
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
```

**Special case**: `crates/mediagit-git/src/error.rs` has **two** copyright blocks concatenated (winnyboy5 lines 1–15, then MediaGit Contributors on line 16). Remove the winnyboy5 block entirely.

**Strategy**: Use `sed` or morphllm for bulk replacement across 142 files. The winnyboy5 block is always lines 1–14 (`// Copyright (C) 2026  winnyboy5` through `// along with this program.  If not, see <https://www.gnu.org/licenses/>.`). Replace with the canonical form.

#### 2b. Remove/activate dead code

| File | Line(s) | Issue | Action |
|------|---------|-------|--------|
| `progress.rs` | 64 | `#[allow(dead_code)]` on `upload_bar()` | Wire to `push.rs` or remove |
| `progress.rs` | 119 | `#[allow(dead_code)]` on `verify_bar()` | Wire to `verify.rs` or remove |
| `progress.rs` | 140 | `#[allow(dead_code)]` on `merge_bar()` | Wire to `merge.rs` or remove |
| `progress.rs` | 162 | `#[allow(dead_code)]` on `io_bar()` | Remove if unused |
| `progress.rs` | 204 | `#[allow(dead_code)]` on `is_quiet()` | Remove if unused |
| `progress.rs` | 210 | `#[allow(dead_code)]` on `format_count()` | Remove if unused |
| `progress.rs` | 235 | `#[allow(dead_code)]` on `OperationStats::new()` | Remove if unused |
| `output.rs` | 39 | `#![allow(dead_code)]` module-level suppress | Remove; delete truly unused functions |
| `odb.rs` | 3170 | `parse_oid_from_path()` — "DEPRECATED" comment, no attr | Add `#[deprecated(since = "0.1.0", note = "use Oid::from_path()")]` |
| `rebase_state.rs` | 158, 174 | `set_conflicts()`, `is_complete()` marked dead_code | Remove (re-add when interactive rebase lands) |

**Note**: The `println!("DEBUG summary: '{}'", summary)` at `progress.rs:419` is inside `#[cfg(test)]` — it does NOT appear in production binaries. Leave as-is or clean up as cosmetic.

---

### Phase 3 — Error Type & Clippy Consistency

**Effort**: ~45 min | **Risk**: Low

#### 3a. Add `is_*()` predicate methods to MediaError and GitError

`StorageError` has: `is_not_found()` (line 90), `is_permission_denied()` (95), `is_invalid_key()` (100).
`CompressionError` has: `is_compression_failed()` (89), `is_decompression_failed()` (94), `is_io()` (99).
`MediaError` has: **no impl block at all** (68-line file, only enum + type alias).
`GitError` has: **no impl block at all** (72-line file, only enum).

**`crates/mediagit-media/src/error.rs`** — add `impl MediaError`:
```rust
impl MediaError {
    pub fn is_unsupported(&self) -> bool {
        matches!(self, Self::UnsupportedFormat(_))
    }
    pub fn is_io_error(&self) -> bool {
        matches!(self, Self::IoError(_))
    }
    pub fn is_parse_error(&self) -> bool {
        matches!(self, Self::ImageError(_) | Self::PsdError(_) | Self::VideoError(_) | Self::AudioError(_))
    }
}
```

**`crates/mediagit-git/src/error.rs`** — add `impl GitError`:
```rust
impl GitError {
    pub fn is_repo_not_found(&self) -> bool {
        matches!(self, Self::RepositoryNotFound(_))
    }
    pub fn is_invalid_oid(&self) -> bool {
        matches!(self, Self::InvalidOid(_))
    }
    pub fn is_filter_error(&self) -> bool {
        matches!(self, Self::FilterFailed(_) | Self::FilterNotConfigured(_))
    }
}
```

#### 3b. Clippy `unwrap_used` — targeted cleanup only

**Corrected assessment**: Only **1 non-test** occurrence exists: `crates/mediagit-security/src/tls/cert.rs:186`. The other 22 occurrences are all inside `#[cfg(test)]` modules — this is standard Rust practice and does not need changing.

**Action**:
- `cert.rs:186`: Evaluate whether the `.unwrap()` can be replaced with `?` or `.expect()` with a descriptive message.
- **Do NOT** mass-convert test module `#[allow(clippy::unwrap_used)]` — these are intentional and idiomatic. Leave them.

---

### Phase 4 — CLI Consistency Refactor

**Effort**: ~2.5 hrs | **Risk**: Low–Medium

#### 4a. Progress bar factory extraction

**`crates/mediagit-cli/src/progress.rs`**

Extract private factory to eliminate 8 near-identical methods:
```rust
fn make_bytes_bar(&self, msg: &str, template: &str) -> ProgressBar {
    if self.quiet { return ProgressBar::hidden(); }
    let pb = self.multi.add(ProgressBar::new(0));
    pb.set_style(
        ProgressStyle::default_bar()
            .template(template)
            .expect("valid progress template")
            .progress_chars("█▓░"),
    );
    pb.set_message(msg.to_string());
    pb.enable_steady_tick(Duration::from_millis(100));
    pb
}
```

Each public method delegates to this with its specific template string. Replace all `.unwrap()` on `ProgressStyle::template()` with `.expect("valid template")`.

#### 4b. Output formatting consolidation (**23 command files**)

**Verified**: 23 out of 32 command files import `use console::style;` directly.

Files using `console::style`:
```
bisect.rs, branch.rs, cherrypick.rs, clone.rs, diff.rs, fetch.rs,
fsck.rs, gc.rs, install.rs, log.rs, merge.rs, pull.rs, push.rs,
rebase.rs, reflog.rs, remote.rs, reset.rs, revert.rs, show.rs,
stash.rs, stats.rs, track.rs, verify.rs
```

Files NOT using `console::style` (already clean):
```
add.rs, commit.rs, filter.rs, init.rs, mod.rs, rebase_state.rs,
status.rs, tag.rs
```

**Decision needed**: Two valid approaches:
1. **Replace all** `console::style()` with `output::*` calls — ensures single output API, but `output.rs` may need new functions (e.g., for colored refs, diff coloring)
2. **Keep `console::style`** for formatting-heavy commands (diff, log, show) and only consolidate simple status messages — less churn, pragmatic

**Recommendation**: Option 2. The `output` module handles success/error/info/warning/detail messages. Commands like `diff.rs` and `log.rs` need fine-grained ANSI coloring that doesn't fit the `output::*` API. Leave `console::style` for those; migrate simple status messages.

#### 4c. Audit duplicate CLI flags

**Verified**: Multiple commands declare `#[arg(short, long)] pub quiet: bool` and/or `pub verbose: bool` as clap arguments, duplicating the `global = true` flags on the top-level `Cli` struct.

**Files confirmed** (partial list — run full audit during implementation):
- `add.rs:86` (`quiet`), `add.rs:90` (`verbose`)
- `branch.rs`: `quiet` at lines 113, 145, 169, 193, 213, 236, 280; `verbose` at 109, 248
- `cherrypick.rs:58` (`quiet`)
- `clone.rs:61` (`quiet`), `clone.rs:65` (`verbose`)
- `commit.rs:91` (`quiet`), `commit.rs:95` (`verbose`)
- `diff.rs:86` (`quiet`)
- `fetch.rs:69` (`quiet`), `fetch.rs:73` (`verbose`)
- `fsck.rs:94` (`quiet`)

**Decision needed**: clap `global = true` flags propagate to subcommands automatically. However, removing per-command flags changes the CLI interface (users who pass `mediagit add -q file.txt` would still work via global propagation, but help text changes).

**Recommendation**: Keep per-command flags for now — they document intent in `--help` output. Add a comment `// Mirrors Cli::quiet (global)` for clarity. This is cosmetic, not a bug.

#### 4d. Extract shared CLI utilities

Create `crates/mediagit-cli/src/commands/utils.rs` and move:

| Function | Current Location | Line |
|----------|-----------------|------|
| `format_duration_ago(duration: Duration) -> String` | `commands/stats.rs` | 26 |
| `validate_ref_name(name: &str) -> Result<()>` | `commands/push.rs` | 27 |
| `categorize_extension(ext: &str) -> &'static str` | `commands/stats.rs` | 40 |

Declare in `crates/mediagit-cli/src/commands/mod.rs`: `pub(crate) mod utils;`

Update imports in `stats.rs` and `push.rs`.

---

### Phase 5 — Feature Gates for Cloud Storage Backends

**Effort**: ~1.5 hrs | **Risk**: Medium (compile/feature matrix)

**`crates/mediagit-storage/Cargo.toml`**

Introduce optional features so users can opt out of cloud SDKs:
```toml
[features]
default = ["local", "s3", "minio", "b2"]
local   = []
s3      = ["dep:aws-config", "dep:aws-sdk-s3"]
minio   = ["s3"]           # MinIO reuses the S3 client
azure   = ["dep:azure_storage", "dep:azure_storage_blobs", "dep:azure_core"]
gcs     = ["dep:google-cloud-storage", "dep:google-cloud-auth"]
b2      = []               # Uses HTTP directly, no additional dep
all     = ["local", "s3", "azure", "gcs", "minio", "b2"]
```

Mark cloud deps as `optional = true` in `[dependencies]`.

Gate module declarations in `crates/mediagit-storage/src/lib.rs`:
```rust
#[cfg(feature = "azure")]
pub mod azure;
#[cfg(feature = "azure")]
pub use azure::AzureBackend;

#[cfg(feature = "gcs")]
pub mod gcs;
#[cfg(feature = "gcs")]
pub use gcs::GcsBackend;
```

**Downstream impact**: Update these Cargo.toml files to declare the features they need:
- `crates/mediagit-server/Cargo.toml`: `mediagit-storage = { path = "...", features = ["all"] }`
- `crates/mediagit-cli/Cargo.toml`: `mediagit-storage = { path = "...", features = ["all"] }` (or default)
- `crates/mediagit-migration/Cargo.toml`: already only depends on the trait, no feature change needed

---

### Phase 6 — Progress Bar Standardization

**Effort**: ~1 hr | **Risk**: Low

#### 6a. Migrate `add.rs` to use `ProgressTracker`

**Problem**: `crates/mediagit-cli/src/commands/add.rs` is the **sole outlier** — it imports `indicatif::{ProgressBar, ProgressStyle}` directly (line 22) and constructs a standalone progress bar (lines 214–222):

```rust
// Current (outlier pattern in add.rs):
let pb = ProgressBar::new(total_bytes);
pb.set_style(
    ProgressStyle::default_bar()
        .template("{spinner:.green} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({percent}%) {msg}")
        .unwrap()
        .progress_chars("█▓░"),
);
```

**Action**: Replace with `ProgressTracker::download_bar()` or add a new `add_bar()` method to `ProgressTracker` if the template needs to differ. The `pb` is wrapped in `Arc<ProgressBar>` for sharing across parallel Tokio tasks — this is compatible with `ProgressTracker` since `download_bar()` returns a `ProgressBar` that can be wrapped in `Arc`.

**Specific steps**:
1. Remove `use indicatif::{ProgressBar, ProgressStyle};` from `add.rs`
2. Accept `ProgressTracker` (already passed via `quiet` flag) and call its factory method
3. Wrap returned `ProgressBar` in `Arc` as before

#### 6b. Clean up dead factory methods

From the progress bar analysis, only **4 of 8** factory methods are actively used:

| Method | Status | Used By |
|--------|--------|---------|
| `download_bar()` | ✅ Used | `clone.rs`, `fetch.rs`, `pull.rs` |
| `object_bar()` | ✅ Used | `clone.rs`, `fetch.rs`, `pull.rs`, `push.rs` |
| `file_bar()` | ✅ Used | `add.rs`, `gc.rs`, `stats.rs`, `verify.rs` |
| `spinner()` | ✅ Used | `commit.rs`, `gc.rs`, `merge.rs`, `rebase.rs` |
| `upload_bar()` | ❌ Unused | `#[allow(dead_code)]` |
| `verify_bar()` | ❌ Unused | `#[allow(dead_code)]` |
| `merge_bar()` | ❌ Unused | `#[allow(dead_code)]` |
| `io_bar()` | ❌ Unused | `#[allow(dead_code)]` |

**Action for unused methods**: Evaluate each:
- `upload_bar()` → Wire to `push.rs` for upload progress (currently uses `object_bar`). If push already works well with `object_bar`, remove `upload_bar`.
- `verify_bar()` → Wire to `verify.rs` or `fsck.rs`. If they already use `file_bar`/`spinner`, remove.
- `merge_bar()` → Wire to `merge.rs`. If merge uses `spinner`, remove.
- `io_bar()` → Generic I/O bar with no specific consumer. Remove.

**Default action**: Remove all 4 unused methods unless wiring them provides clear UX improvement over the currently-used alternatives.

#### 6c. Extract shared progress bar factory

As described in Phase 4a, extract a private `make_bytes_bar()` helper to DRY the remaining 4 active factory methods. All share:
- `if self.quiet { return ProgressBar::hidden(); }`
- `self.multi.add(ProgressBar::new(0))`
- `.progress_chars("█▓░")`
- `.enable_steady_tick(Duration::from_millis(100))`

Only the template string and color differ.

#### 6d. Define standard progress bar templates

Establish named constants for the 4 standard templates:

```rust
/// Standard templates for progress bars across all CLI commands.
/// All use 40-char width, "█▓░" characters, 100ms tick, stderr output.
mod templates {
    /// Bytes-based progress (downloads, uploads, file I/O)
    pub const BYTES: &str =
        "{spinner:.cyan} {msg} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})";

    /// Object/item count progress (objects, chunks)
    pub const OBJECTS: &str =
        "{spinner:.green} {msg} [{bar:40.green/white}] {pos}/{len} ({per_sec}, {eta})";

    /// File count progress (add, verify, gc)
    pub const FILES: &str =
        "{spinner:.yellow} {msg} [{bar:40.yellow/white}] {pos}/{len} ({per_sec})";

    /// Indeterminate spinner (commit, merge, rebase)
    pub const SPINNER: &str =
        "{spinner:.blue} {msg} {elapsed_precise}";
}
```

**Architecture note**: `indicatif` is and should remain a **CLI-only** dependency. The protocol layer (`mediagit-protocol`) correctly uses generic callbacks (`Fn(PushProgress)`) — no changes needed there. `mediagit-versioning` has no progress callbacks at all and doesn't need them for this refactor.

---

### Phase 7 — Code Documentation Convention

**Effort**: ~2 hrs | **Risk**: None

#### 7a. Enable `missing_docs` lint (gradual)

Add to root `Cargo.toml` workspace lints:

```toml
[workspace.lints.rust]
missing_docs = "warn"  # warn, not deny — allows gradual adoption
```

Exempt internal crates that are not public API:

```rust
// In crates/mediagit-cli/src/main.rs (binary, not library):
#![allow(missing_docs)]

// In crates/mediagit-test-utils/src/lib.rs:
#![allow(missing_docs)]
```

#### 7b. Add missing crate-level `//!` documentation

Three crates need crate-level docs added to their `lib.rs`:

**`crates/mediagit-security/src/lib.rs`** (Priority 1 — zero `//!` docs):
```rust
//! Authentication, encryption, and audit trail for MediaGit.
//!
//! Provides security primitives for protecting repository data at rest
//! and in transit, plus authentication for remote operations.
//!
//! # Components
//!
//! - **Encryption**: AES-256-GCM symmetric encryption with Argon2id key derivation
//! - **Authentication**: JWT tokens and API key verification
//! - **TLS**: Certificate management for secure transport
//! - **Audit**: Structured audit trail for security-sensitive operations
//!
//! # Security Model
//!
//! Keys are never logged or serialized in plaintext. The [`EncryptionKey`]
//! type wraps raw key material and only exposes it through [`expose_key()`]
//! to make accidental leakage difficult.
```

**`crates/mediagit-server/src/lib.rs`** (Priority 2 — no `//!` docs):
```rust
//! Axum REST API server for MediaGit repositories.
//!
//! Provides HTTP endpoints for push, pull, clone, and repository management.
//! Includes rate limiting, authentication middleware, and CORS support.
//!
//! # Quick Start
//!
//! ```no_run
//! use mediagit_server::create_router;
//!
//! let app = create_router("/data/repos");
//! // Serve with: axum::serve(listener, app).await
//! ```
```

**`crates/mediagit-media/src/lib.rs`** (Priority 3 — minimal `//!`):
Expand the existing 6-bullet list to include architecture overview and a usage example showing `MergeStrategy::from_extension()`.

#### 7c. Establish inline comment convention

Standardize all inline comment tags with consistent formatting:

| Tag | Usage | Format |
|-----|-------|--------|
| `// NOTE:` | Design rationale not obvious from code | Always with colon |
| `// TODO:` | Known incomplete work | Always with colon, add issue ref when possible |
| `// FIXME:` | Known incorrect/broken code | Always with colon |
| `// SAFETY:` | Invariants upheld before `unsafe` | Always with colon (Rust convention) |

**Actions**:
- Fix 2 `// TODO` (no colon) in `tag.rs` and `bisect.rs` → `// TODO:`
- Convert 1 `// IMPORTANT:` in `chunking.rs:584` → promote to `///` doc comment on enclosing function
- Audit `// NOTE:` usages: keep for design rationale, convert to `// TODO:` where they describe pending work

#### 7d. Document public items in high-priority gaps

Focus on the 3 crates with lowest coverage. **Do not** add docs to already-well-documented crates (storage, versioning, compression are at ~95–100%).

| Crate | Gap | Action |
|-------|-----|--------|
| `mediagit-security` | No `//!`, `EncryptionError` variants undocumented | Add `//!` (7b), add `///` to enum variants |
| `mediagit-media` | `MediaType` enum + variants undocumented, `SupportedImageFormat` undocumented | Add `///` to each variant |
| `mediagit-server` | No `//!`, middleware ordering undocumented | Add `//!` (7b), add `///` to middleware fns |

**Gold standard references** to follow:
- `crates/mediagit-storage/src/lib.rs` — best `//!` + trait docs with `# Arguments`, `# Returns`, `# Errors`, `# Examples`
- `crates/mediagit-versioning/src/oid.rs` — best per-method `///` with `# Examples` on every public fn

---

## Critical Files Reference

```
Phase 0 (GitHub Actions YAMLs + Cargo files + Rust sources):
  .github/workflows/ci.yml          (checkout v4→v6, codecov v4→v5)
  .github/workflows/docs.yml        (checkout v4→v6, upload-pages v3→v4)
  .github/workflows/bench.yml       (checkout v4→v6)
  .github/workflows/release.yml     (checkout v4→v6 if present)
  Cargo.toml                         (brotli 7→8, criterion 0.5→0.8)
  crates/mediagit-cli/Cargo.toml     (dialoguer 0.11→0.12)
  crates/mediagit-storage/Cargo.toml (google-cloud-storage 0.24→1.8, google-cloud-auth 0.17→1.6)
  crates/mediagit-security/Cargo.toml (secrecy 0.8→0.10)
  crates/mediagit-security/src/encryption.rs  (SecretVec→SecretBox, expose_key fix)
  crates/mediagit-security/src/kdf.rs          (SecretString::new→::from, ~8 occurrences)
  crates/mediagit-security/tests/security_test.rs   (SecretString::new→::from)
  crates/mediagit-security/benches/encryption_benchmark.rs  (SecretString::new→::from)
  crates/mediagit-storage/src/gcs.rs           (compile-driven fixes after GCS bump)

Phase 1 (14 Cargo.toml files + root Cargo.toml):
  Cargo.toml                               (add reqwest workspace dep)
  crates/mediagit-{cli,compression,config,media,metrics,migration,
    observability,storage,test-utils,versioning}/Cargo.toml  (add repo/homepage/docs)
  crates/mediagit-{git,protocol,security,server}/Cargo.toml  (add description)
  crates/mediagit-{protocol,metrics,server}/Cargo.toml       (reqwest → workspace)

Phase 2 (142 .rs files + 5 specific files):
  142 files: grep -rl "winnyboy5" crates/ --include="*.rs"
  crates/mediagit-cli/src/progress.rs              (dead code cleanup)
  crates/mediagit-cli/src/output.rs                (remove module-level allow)
  crates/mediagit-versioning/src/odb.rs            (#[deprecated] attr)
  crates/mediagit-cli/src/commands/rebase_state.rs (remove dead methods)
  crates/mediagit-git/src/error.rs                 (duplicate header)

Phase 3 (3 files):
  crates/mediagit-media/src/error.rs     (add impl block with is_*())
  crates/mediagit-git/src/error.rs       (add impl block with is_*())
  crates/mediagit-security/src/tls/cert.rs  (1 prod unwrap → expect)

Phase 4 (~25 files):
  crates/mediagit-cli/src/progress.rs              (factory refactor)
  crates/mediagit-cli/src/commands/utils.rs        (new file)
  crates/mediagit-cli/src/commands/mod.rs           (declare utils)
  crates/mediagit-cli/src/commands/stats.rs         (move utils out)
  crates/mediagit-cli/src/commands/push.rs          (move validate_ref_name)
  23 command files with console::style              (selective migration)

Phase 5 (4 files):
  crates/mediagit-storage/Cargo.toml
  crates/mediagit-storage/src/lib.rs
  crates/mediagit-server/Cargo.toml      (add features = ["all"])
  crates/mediagit-cli/Cargo.toml         (add features = ["all"])

Phase 6 (3 files):
  crates/mediagit-cli/src/progress.rs              (factory refactor + template constants + dead method removal)
  crates/mediagit-cli/src/commands/add.rs           (migrate to ProgressTracker)
  crates/mediagit-cli/src/commands/mod.rs            (if add.rs needs ProgressTracker import)

Phase 7 (~10 files):
  Cargo.toml                                        (add missing_docs = "warn")
  crates/mediagit-cli/src/main.rs                   (add #![allow(missing_docs)])
  crates/mediagit-test-utils/src/lib.rs              (add #![allow(missing_docs)])
  crates/mediagit-security/src/lib.rs                (add //! crate doc)
  crates/mediagit-server/src/lib.rs                  (add //! crate doc)
  crates/mediagit-media/src/lib.rs                   (expand //! crate doc)
  crates/mediagit-media/src/strategy.rs              (add /// to MediaType variants)
  crates/mediagit-media/src/image.rs                 (add /// to SupportedImageFormat)
  crates/mediagit-security/src/encryption.rs         (add /// to EncryptionError variants)
  crates/mediagit-cli/src/commands/tag.rs             (fix // TODO → // TODO:)
  crates/mediagit-cli/src/commands/bisect.rs          (fix // TODO → // TODO:)
  crates/mediagit-versioning/src/chunking.rs          (// IMPORTANT: → /// doc comment)
```

---

## Existing Patterns to Reuse

| Pattern | Source File | Used By |
|---------|-----------|---------|
| Error predicates (`is_*()`) | `crates/mediagit-storage/src/error.rs` lines 90–100 | Phase 3a |
| Feature flags (`optional = true`) | `crates/mediagit-security/Cargo.toml` | Phase 5 |
| Progress bar output | `crates/mediagit-cli/src/progress.rs` | Phase 4a, 6 |
| Output module | `crates/mediagit-cli/src/output.rs` | Phase 4b |
| Workspace deps | Root `Cargo.toml` `[workspace.dependencies]` | Phase 1 |
| Crate-level `//!` docs | `crates/mediagit-storage/src/lib.rs` | Phase 7b (gold standard) |
| Per-method `///` docs | `crates/mediagit-versioning/src/oid.rs` | Phase 7d (gold standard) |
| Workspace lints | Root `Cargo.toml` `[workspace.lints.rust]` | Phase 7a |

---

## Verification

After each phase commit:

```bash
# 1. Type-check
cargo check --workspace

# 2. Lint (must be clean)
cargo clippy --workspace --all-targets --all-features -- -D warnings

# 3. Tests (must all pass)
cargo test --workspace

# 4. Format check
cargo fmt --check --all
```

Phase 5 additional checks:
```bash
cargo check -p mediagit-storage --no-default-features
cargo check -p mediagit-storage --features azure
cargo check -p mediagit-storage --features gcs
cargo check -p mediagit-storage --all-features
```

---

## Out of Scope

- New functionality or feature additions
- Breaking public API changes (e.g., `StorageBackend` trait error type)
- Test coverage additions for CLI commands — high value but large scope, separate PR
- Performance optimizations
- Mass-converting test `#[allow(clippy::unwrap_used)]` — these are idiomatic Rust test practice

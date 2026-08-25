# MediaGit-Core Development Guide
**Version**: 0.3.0-rc.3
**Last Updated**: July 18, 2026

A guide for people building MediaGit itself: repo layout, local build/test loop,
debugging client-server flows, and where to look when something breaks in dev.

**Looking for install/deploy or the config reference instead?**
- Installing binaries, provisioning a server, backend setup (S3/Azure/GCS/MinIO), production deployment → **SETUP.md**
- Full client/server config file and env-var reference → **CONFIGURATION.md**

---

## 📋 Table of Contents

1. [Understanding MediaGit Architecture](#understanding-mediagit-architecture)
2. [Prerequisites](#prerequisites)
3. [Quick Start](#quick-start)
4. [Local Development Setup](#local-development-setup)
5. [Project Structure](#project-structure)
6. [Client-Server Workflows](#client-server-workflows)
7. [File Locking](#file-locking)
8. [Auth State](#auth-state)
9. [Testing](#testing)
10. [Troubleshooting](#troubleshooting)
11. [Performance Tuning](#performance-tuning)
12. [Quick Reference](#quick-reference)
13. [Project Maintenance](#project-maintenance)

---

## Understanding MediaGit Architecture

### Two Usage Modes

MediaGit can be used in **two different modes** depending on your needs:

#### Mode 1: Standalone (Local-Only)
**Perfect for**: Single developer, local versioning, experimenting

```
┌─────────────────────────────────┐
│   Your Computer                 │
│                                 │
│  mediagit CLI                   │
│       ↓                         │
│  .mediagit/        (metadata)   │
│  mediagit-data/    (objects)    │
└─────────────────────────────────┘
```

**What you need**: `mediagit` binary only — no server, no network.

**What you can do**: `init`, `add`, `commit`, `status`, `log`, and all other
local-only commands.

#### Mode 2: Client-Server (Collaborative)
**Perfect for**: Teams, remote backups, collaboration

```
┌──────────────────┐         ┌──────────────────┐
│  Your Computer   │         │   Server         │
│                  │         │                  │
│  mediagit CLI    │ ←────→  │ mediagit-server  │
│       ↓          │  push/  │       ↓          │
│  .mediagit/      │  pull   │  repos/          │
└──────────────────┘         │  S3/Azure/etc    │
                             └──────────────────┘
```

**What you need**: `mediagit` (client) + a running `mediagit-server` + network.

**What you can do**: Everything from Mode 1, plus `push`, `pull`, `clone`, `fetch`.

### Storage Architecture

MediaGit uses **two separate storage locations**:

1. **Repository metadata** (`.mediagit/`) — commits, branches, refs, config.
   Always local to the machine running the CLI.
2. **Object storage backend** (configurable) — the actual chunked/compressed/
   deduplicated file content. Local filesystem by default; can be S3, Azure
   Blob, GCS, or MinIO on the server side. See **SETUP.md** for backend setup.

### Key Differences from Git

| Aspect | Git | MediaGit |
|--------|-----|----------|
| **Optimized for** | Text/code | Large media files |
| **Metadata** | `.git/` directory | `.mediagit/` directory |
| **Object storage** | Inside `.git/objects/` | Separate backend (configurable) |
| **Deduplication** | File-level | Chunk-level (CDC) |
| **Compression** | zlib | zstd/brotli/zlib (content-aware) |
| **Hashing** | SHA-1/SHA-256 | BLAKE3 |
| **Max file size** | ~100MB practical | Multi-GB supported |

---

## Prerequisites

### System Requirements
- **OS**: Linux, macOS, Windows (native or WSL2)
- **Rust**: 1.97+ (pinned in `Cargo.toml` `[workspace.package].rust-version` — check with `rustc --version`)
- **CPU**: 2+ cores
- **RAM**: 8GB minimum (use `RUST_TEST_THREADS=2`), 16GB+ recommended (default parallel tests)
- **Disk**: 10GB+ free space

### Required Tools

```bash
# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env

# Verify installation
rustc --version  # Must be 1.97 or higher
cargo --version

# Ubuntu/Debian build essentials
sudo apt-get update && sudo apt-get install -y build-essential pkg-config libssl-dev

# macOS
xcode-select --install

# Dev tools
cargo install cargo-watch    # Auto-rebuild on file changes
cargo install cargo-nextest  # Fast test runner (optional, better memory isolation)
```

Docker (for local MinIO), AWS/Azure/GCS CLIs are only needed if you're testing
against a specific cloud backend — see **SETUP.md**.

---

## Quick Start

**New to the codebase?** Get a working build and a local commit in a few minutes:

```bash
git clone https://github.com/winnyboy5/mediagit-core.git
cd mediagit-core
cargo build   # debug build, 5-10 minutes on first build

# Verify both binaries exist
ls -lh target/debug/mediagit
ls -lh target/debug/mediagit-server

# Local-only workflow (no server needed)
cd /path/to/your/project
../mediagit-core/target/debug/mediagit init
../mediagit-core/target/debug/mediagit add your-file.psd
../mediagit-core/target/debug/mediagit commit -m "First commit"
```

For a client-server loop against a local server, see [Client-Server
Workflows](#client-server-workflows) below. For provisioning a real backend
(S3/Azure/GCS/MinIO) or a production deployment, see **SETUP.md**.

---

## Local Development Setup

### 1. Clone and Build

```bash
git clone https://github.com/winnyboy5/mediagit-core.git
cd mediagit-core

# Development build (recommended for day-to-day work)
cargo build
# Creates target/debug/mediagit and target/debug/mediagit-server

# Release build (optimized, for perf work or production)
cargo build --release
```

**TLS feature flag**: TLS support is enabled by default via the
`mediagit-server` crate's `tls` feature (`default = ["tls"]` in
`crates/mediagit-server/Cargo.toml`, which pulls in `mediagit-security/tls`).
To build without it:

```bash
cargo build --no-default-features
```

### 2. Verify Build

```bash
./target/debug/mediagit --version
# Should output: mediagit 0.3.0-rc.3

./target/debug/mediagit-server --help
```

### 3. Run Tests

```bash
# Memory-constrained machines: limit threads (see Troubleshooting below)
RUST_TEST_THREADS=2 cargo test --workspace

# High-RAM machines (default: one thread per CPU)
cargo test --workspace

# One suite
cargo test --test cli_add_test

# With output
cargo test --workspace -- --nocapture

# nextest (if installed)
cargo nextest run
```

> **Memory note**: `cargo test` defaults to one thread per logical CPU. Each
> thread spawns a full mediagit process with its own Moka cache, parallel
> chunk workers, and file buffers — on 8-core machines this can peak at
> **8–16 GB RAM**. Use `RUST_TEST_THREADS=2` to stay under 4 GB. See
> [Troubleshooting → Tests consuming too much RAM](#tests-consuming-too-much-ram).

#### Low-Memory Build Tips (≤ 8 GB RAM)

```bash
export CARGO_BUILD_JOBS=1   # One linker at a time — safest
export RUST_TEST_THREADS=1  # Or =2 for a middle ground

# Build/test only the crate you're touching
cargo build -p mediagit-cli
cargo test  -p mediagit-config
```

On Windows, `.cargo/config.toml` already sets `/INCREMENTAL:NO`, which halves
peak link-step RAM.

### 4. Local Backend Services (Docker Compose)

Three compose files at the repo root cover different jobs — none of them run
in CI except the third:

- **`docker-compose.yml`** — full local dev stack (MinIO/Silo, Azurite, a
  fake-GCS server, optionally LocalStack). Driven by
  `scripts/start-test-services.sh` and `scripts/stop-test-services.sh`.
  Use this when you want all four cloud backends available locally at once,
  e.g. to run `cargo test test_s3_backend test_azure_backend
  test_gcs_backend` against real emulators instead of mocks.
  ```bash
  scripts/start-test-services.sh
  # ...run tests...
  scripts/stop-test-services.sh --clean   # also drops volumes
  ```
- **`docker-compose.minio.yml`** — a single pinned MinIO-compatible backend
  (see [SETUP.md → Local MinIO for backend testing](SETUP.md)). Exists
  specifically for the QA suite's A7 backend-outage drill
  (`dev-tests/qa-suite/scripts/07_abuse.ps1`), which needs a stable,
  version-pinned MinIO it can stop and restart by container name mid-run.
- **`docker-compose.test.yml`** — CI's integration-test services; started and
  torn down by `.github/workflows/ci.yml`. Not meant to be run manually for
  day-to-day dev work — use `docker-compose.yml` for that instead.

### 5. Local Dev Server Harness

`dev-tests/dev-server/` is a ready-made server working directory for manual
client-server testing — `mediagit-server.toml` (port `5000`), a `repos/` dir,
an `auth/` store, and per-backend configs (`config.aws.toml`,
`config.azure.toml`, `config.gcs.toml`) you can swap in. `dev-tests/dev-client/`
is the matching client-side scratch dir. Start it with:

```bash
./target/debug/mediagit-server --config dev-tests/dev-server/mediagit-server.toml
```

### 6. Pre-Commit Hooks

We use **[husky-rs](https://github.com/pplmx/husky-rs)** (pure Rust) for Git
hooks: `cargo fmt --check`, `cargo clippy`, license header check, a >5MB file
guard, conflict-marker check, and conventional-commit message validation.
Hooks auto-install on `cargo build`.

```bash
git commit --no-verify     # bypass hooks for a WIP commit
NO_HUSKY_HOOKS=1 cargo build  # skip hook installation (e.g. in CI)
```

If hooks fail to execute at all (`cannot exec '.husky/pre-commit'`), see
[Troubleshooting → Git hooks not executing](#git-hooks-not-executing).

---

## Project Structure

```
mediagit-core/
├── crates/                    # 14 workspace members (see below)
├── dev-tests/                 # Manual/integration test harnesses (qa-suite, dev-server, deep-tests, compat-fixture)
├── docs/                      # env-knobs.md, ROADMAP, ARCHITECTURE notes
├── book/                      # mdBook documentation source
├── target/                    # Build artifacts
│   ├── debug/                 # mediagit, mediagit-server (dev builds)
│   └── release/               # Optimized binaries
└── Cargo.toml                 # Root workspace config
```

There is no top-level `tests/` directory of integration tests — integration
and manual test harnesses live under `dev-tests/`, and each crate has its own
`crates/<crate>/tests/` for `cargo test` integration tests.

### Workspace Crates

| Crate | Purpose |
|-------|---------|
| `mediagit-cli` | Version control CLI purpose-built for media files (the `mediagit` binary) |
| `mediagit-server` | Axum REST API server for MediaGit repositories (the `mediagit-server` binary) |
| `mediagit-versioning` | Core VCS engine: ODB, refs, commits, trees, and packs |
| `mediagit-storage` | Unified async storage backend trait and implementations (filesystem/S3/Azure/GCS) |
| `mediagit-protocol` | Network push/pull/clone protocol implementation |
| `mediagit-security` | Authentication, encryption, and audit trail |
| `mediagit-compression` | Content-aware compression engine for media objects |
| `mediagit-media` | Media metadata extraction and merge strategies |
| `mediagit-config` | Configuration management system for MediaGit Core |
| `mediagit-observability` | Structured logging and observability for MediaGit |
| `mediagit-git` | Git migration support: filter drivers and pointer files |
| `mediagit-metrics` | Prometheus metrics and performance monitoring for MediaGit |
| `mediagit-migration` | Storage backend migration tool for MediaGit |
| `mediagit-test-utils` | Shared test utilities for MediaGit crates |

---

## Client-Server Workflows

Use this section for local debugging loops against a server. For provisioning
a real backend or a production server, see **SETUP.md**; for the full config
reference, see **CONFIGURATION.md**.

### Start a Local Server

```bash
# Terminal 1
cd mediagit-core
./target/debug/mediagit-server
# Wait for "Server listening on http://127.0.0.1:3000"

# Or against the dev-server harness (port 5000, pre-wired repos/auth dirs):
./target/debug/mediagit-server --config dev-tests/dev-server/mediagit-server.toml
```

### Basic Push/Pull/Clone Loop

```bash
# Terminal 2
cd /path/to/your/project
../mediagit-core/target/debug/mediagit init
../mediagit-core/target/debug/mediagit remote add origin http://localhost:3000/repos/my-project
../mediagit-core/target/debug/mediagit add your-file.psd
../mediagit-core/target/debug/mediagit commit -m "First commit"
../mediagit-core/target/debug/mediagit push origin main

# Pull changes
../mediagit-core/target/debug/mediagit pull origin main

# Clone
../mediagit-core/target/debug/mediagit clone http://localhost:3000/repos/my-project
```

### Debugging Transfers

Push and pull use a pipelined transfer engine with two advanced modes worth
knowing when debugging slow or failing transfers:

- **Presigned-URL transfer**: for S3/MinIO/Azure backends the server issues
  short-lived presigned URLs so chunk data travels direct client ↔ cloud
  without proxying through the server process. GCS falls back to
  server-proxy when SA-key signing is unavailable (see **SETUP.md** GCS
  section for why).
- **Cloud packs**: chunks are bundled into pack objects on push, cutting S3
  API call count drastically and speeding up clone on small-chunk repos.

To see which path is active, run the server with storage-level logs:

```bash
RUST_LOG=mediagit_server=info,mediagit_storage=info ./target/debug/mediagit-server
```

Design details live in `ARCHITECTURE.md`.

### Branch Lifecycle & Maintenance

```bash
# Create and push a feature branch
mediagit branch create feature/new-asset
mediagit push -u origin feature/new-asset

# Work, commit, push
mediagit add *.psd
mediagit commit -m "Add new assets"
mediagit push

# After merge, delete the remote branch
mediagit push origin --delete feature/new-asset
mediagit branch delete -r origin/feature/new-asset

# Reclaim storage from orphaned chunks/manifests
mediagit gc
mediagit gc --dry-run --verbose  # Preview
```

> `push --delete` cannot delete the branch currently checked out on the
> remote (HEAD protection) — deleting `main` while it's active is rejected.

---

## File Locking

MediaGit supports server-enforced exclusive file locking so two people can't
both edit the same binary asset unnoticed. Implementation:
`crates/mediagit-server/src/locks.rs` (lock records, path normalization,
push-side enforcement) and `crates/mediagit-cli/src/commands/lock.rs`
(`lock create` / `lock unlock` / `lock list` subcommands).

Enforcement compares the tree paths touched by a push against active locks
held by other users; a push touching a path locked by someone else is
rejected. Debugging tips:

- `MEDIAGIT_LOCKS_ENFORCE=0` disables enforcement entirely (fail-open) — set
  this if you need to reproduce pre-locking behavior or unblock a stuck test.
- Lock state is server-side; check `locks.rs` for how paths are normalized
  before comparison if a lock isn't matching the push you expect it to.

---

## Auth State

For debugging auth issues on a dev server, know where the state lives:

- **Location**: `ServerConfig::auth_store_dir` if explicitly set, otherwise a
  sibling `auth/` directory next to `repos_dir` (see
  `crates/mediagit-server/src/config.rs::resolved_auth_store_dir`).
- **Files**: `users.jsonl`, `api_keys.jsonl`, `grants.jsonl` — one JSON object
  per line, loaded on server start via `AuthService`, `ApiKeyAuth`, and
  `GrantsStore` respectively (`crates/mediagit-server/src/state.rs`).
- **Ephemeral test servers**: set `MEDIAGIT_AUTH_PERSIST=0` to skip writing
  these files to disk entirely — useful in `cargo test` and CI where you
  don't want auth state to leak between runs (see
  `crates/mediagit-security/src/auth/apikey.rs`).

### Client Credential Resolution

Client-side credential lookup for talking to a remote (used by `fetch`,
`pull`, `push`, `clone`, `download`, `lock`) follows this precedence, in
`crates/mediagit-cli/src/repo.rs::resolve_credentials` (repo.rs:136-177):

1. Env var `MEDIAGIT_TOKEN` (bearer token)
2. Env var `MEDIAGIT_API_KEY`
3. OS keychain (skipped if `MEDIAGIT_NO_KEYRING` is set)
4. Per-remote config: `remotes.<name>.token` / `remotes.<name>.api_key` in
   `.mediagit/config.toml` (`token` wins if both are set)

There is no `MEDIAGIT_AUTH_TOKEN` env var or top-level `auth_token` config
key — those don't exist in the codebase; use the precedence above instead.

---

## Testing

```bash
# Full workspace (limit threads to avoid OOM — see Troubleshooting)
RUST_TEST_THREADS=2 cargo test --workspace

# Specific backend tests
cargo test test_s3_backend
cargo test test_azure_backend
cargo test test_gcs_backend

# With logging
RUST_LOG=debug cargo test --workspace -- --nocapture

# Lint (must pass with zero warnings — CI runs this exact command)
cargo clippy --workspace --all-targets --all-features -- -D warnings

# Format check
cargo fmt --check
```

### QA Suite

For end-to-end scenario testing beyond `cargo test` (personas, economics,
branching, remote, abuse, perf), use the harness at `dev-tests/qa-suite/`:

```powershell
powershell -ExecutionPolicy Bypass -File dev-tests\qa-suite\scripts\run_all.ps1
```

It's organized as numbered phases (`01_preflight`, `02_matrix`,
`03_persona_*`, `04_economics`, `05_branching`, `06_remote`, `07_abuse`,
`08_perf`, `09_report`) with env knobs in `campaign_env.ps1`.

### Benchmarks

```bash
cargo bench
cargo bench --bench storage_benchmarks
cargo bench --bench odb_bench
```

---

## Troubleshooting

### "Binary not found" error

**Problem**: `./target/debug/mediagit: No such file or directory`

**Solution**: `cargo build` first, then `ls -lh target/debug/mediagit` to confirm.

### "Rust version too old" error

**Solution**: `rustup update` — MediaGit requires 1.97+ (see `rust-version`
in root `Cargo.toml`).

### Tests consuming too much RAM

**Problem**: `cargo test --workspace` triggers OOM or swap spikes.

**Root causes**:

| Cause | Impact |
|-------|--------|
| Default thread count = CPU count | Multiplies all other costs |
| Moka cache: 1000-object count limit (not byte-bounded) | Up to GB per ODB on large chunks |
| 3-layer nested parallelism (test × files × chunk workers) | Hundreds of concurrent async tasks |
| `write_chunked_parallel` reads entire file into RAM | 50–500 MB per concurrent test |

**Quick fix**:
```bash
RUST_TEST_THREADS=2 cargo test --workspace
# or
cargo nextest run --workspace --test-threads 2
```

**Memory budget by thread count** (approximate, 8-core machine):

| `--test-threads` | Peak RAM | Suitable for |
|-----------------|----------|--------------|
| 1 | ~1–2 GB | CI, 4 GB machines |
| 2 | ~2–4 GB | 8 GB machines (recommended) |
| 4 | ~4–8 GB | 16 GB machines |
| default (8+) | ~8–16 GB | 32 GB+ machines |

Large/ignored tests are excluded by default (`#[ignore]`); run explicitly
with `cargo test --workspace -- --ignored` (high memory).

### Git hooks not executing

**Problem**: `git commit` fails with `fatal: cannot exec '.husky/pre-commit': No such file or directory`.

Two independent causes, either or both can apply:

**Cause 1 — CRLF line endings.** `core.autocrlf` (Windows default) or
unzipping on Windows turns the shebang into `#!/bin/sh\r`, which the kernel
can't resolve.

```bash
file .husky/pre-commit   # "with CRLF line terminators" if affected
sed -i 's/\r//' .husky/pre-commit .husky/pre-push .husky/commit-msg
```

**Cause 2 — Missing execute bit.** NTFS/FAT32/some SMB shares/CI artifact
extracts don't preserve the `+x` bit.

```bash
ls -la .husky/   # should show -rwxr-xr-x
chmod +x .husky/pre-commit .husky/pre-push .husky/commit-msg
git update-index --chmod=+x .husky/pre-commit .husky/pre-push .husky/commit-msg
```

**Combined one-shot fix**:
```bash
sed -i 's/\r//' .husky/pre-commit .husky/pre-push .husky/commit-msg
chmod +x        .husky/pre-commit .husky/pre-push .husky/commit-msg
git update-index --chmod=+x .husky/pre-commit .husky/pre-push .husky/commit-msg
```

`.gitattributes` enforces `eol=lf` for `.husky/*` to prevent recurrence.

### "Server not responding" error

**Problem**: `mediagit push` fails with connection refused or timeout.

**Solution**: Start `mediagit-server` first — push/pull need a running
server; `init`/`add`/`commit`/`status` don't.

```bash
curl http://localhost:3000/health   # should return {"status":"healthy"}
```

### Client credentials rejected / auth confusion

See [Auth State](#auth-state) above for where server-side auth state lives
and the client credential precedence — there is no `MEDIAGIT_AUTH_TOKEN` env
var or `auth_token` config key.

### Debug Logging

```bash
export RUST_LOG=debug
./target/debug/mediagit add large-file.mp4

export RUST_LOG=trace   # very verbose
./target/debug/mediagit commit -m "Debug commit"

export RUST_LOG=mediagit_storage=debug,mediagit_versioning=info   # per-module
```

### Getting Help

```bash
./target/debug/mediagit --help
./target/debug/mediagit add --help
./target/debug/mediagit-server --help
```

For cloud-backend-specific issues (MinIO connection, S3 access denied, Azure
auth, GCS permissions), see the Troubleshooting section in **SETUP.md**.

---

## Performance Tuning

For local dev, the defaults are fine. These are the knobs worth knowing when
profiling or debugging throughput locally; the full catalog (all env knobs,
every config key) lives in **env-knobs.md** and **CONFIGURATION.md**.

### Compression (dev iteration speed vs. ratio)

```toml
# Fast (dev loop)
[compression]
algorithm = "zstd"
level = 1

# Balanced (default)
[compression]
algorithm = "zstd"
level = 3
```

### Benchmark Instrumentation

```bash
MEDIAGIT_BENCH=1 mediagit push origin main   # emits [bench] throughput summary
```

### Frequently-Used Knobs

| Variable | Default | Tunes |
|----------|---------|-------|
| `MEDIAGIT_UPLOAD_CONCURRENCY` | `32` | Total upload semaphore slots per push |
| `MEDIAGIT_DOWNLOAD_CONCURRENCY` | `32` | Total chunk downloads during pull/clone |
| `MEDIAGIT_BENCH` | `0` | Set `1` to emit throughput summary after push/pull |
| `MEDIAGIT_HASH_PARALLEL` | `0` | Set `1` for BLAKE3 tree-parallel hashing (~2.6× faster) |
| `RUST_LOG` | — | Module-scoped log levels, see Debug Logging above |

See **env-knobs.md** for the complete list (30+ knobs covering upload/
download concurrency, MPU, range-parallel GET, GCS-specific tuning, etc.).

---

## Quick Reference

### Compression Ratios (observed)

| File Type | Expected Compression |
|-----------|---------------------|
| PNG | 0-5% (already compressed) |
| PSD | 30-40% |
| Text/CSV | 85-95% |
| Video (MP4) | 0% (already compressed) |

### Where to Look

| Question | Look here |
|----------|-----------|
| How do I install/deploy this? | **SETUP.md** |
| What does config key X do? | **CONFIGURATION.md** |
| What does env knob X do? | `env-knobs.md` |
| How does push/pull/clone work internally? | `ARCHITECTURE.md` |
| How does file locking work? | [File Locking](#file-locking) above, `crates/mediagit-server/src/locks.rs` |
| Where does server auth state live? | [Auth State](#auth-state) above |

---

## Project Maintenance

### Test Archives

Test artifacts from validation sessions are archived in `test-archives/` with
dated folders (e.g. `2025-12-27-option-b-validation/`, 7.3GB). Each includes
a README.md with results and metadata. Retention: 30 days from creation.

### Cleanup Commands

```bash
# Remove old archives after retention period
rm -rf test-archives/YYYY-MM-DD-*/

# Clean build artifacts
cargo clean

# Find temporary files
find . -name "*.tmp" -o -name "*.log" -o -name "*~"
```

---

**Version**: 0.3.0-rc.3
**Last Updated**: July 18, 2026
**Maintained by**: MediaGit Core Team

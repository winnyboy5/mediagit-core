# Development Setup

A complete guide for setting up MediaGit for development and contribution.

## Prerequisites

| Tool | Version | Purpose |
|------|---------|---------|
| Rust | 1.91.0+ | Language toolchain (MSRV) |
| Docker | 20.10+ | Integration test emulators |
| Git | 2.x | Source code management |

### Install Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://rustup.rs | sh
source ~/.cargo/env

# Install the exact MSRV toolchain
rustup toolchain install 1.91.0
rustup component add rustfmt clippy
```

## Clone and Build

```bash
git clone https://github.com/mediagit/mediagit-core.git
cd mediagit-core

# Build (debug)
cargo build

# Build release binaries
cargo build --release --bin mediagit --bin mediagit-server

# Install locally
cargo install --path crates/mediagit-cli
```

## Project Structure

```
mediagit-core/
├── crates/
│   ├── mediagit-cli/          # CLI binary (clap commands)
│   ├── mediagit-versioning/   # ODB, refs, commits, trees, pack files, chunking
│   ├── mediagit-storage/      # LocalBackend + cloud StorageBackend trait
│   ├── mediagit-compression/  # zstd/brotli, smart compression, ObjectType
│   ├── mediagit-media/        # PSD/video/audio/3D format parsers
│   ├── mediagit-config/       # TOML config schema, branch protection
│   ├── mediagit-protocol/     # HTTP push/pull/clone protocol
│   ├── mediagit-server/       # mediagit-server binary
│   ├── mediagit-security/     # AES-GCM encryption, argon2 key derivation
│   ├── mediagit-observability/ # tracing, structured logging
│   ├── mediagit-metrics/      # Prometheus metrics
│   ├── mediagit-migration/    # Repository migration utilities
│   ├── mediagit-git/          # Git interop (smudge/clean filters)
│   └── mediagit-test-utils/   # Shared test helpers (publish = false)
├── book/                      # mdBook documentation source
├── docker/                    # Dockerfiles
├── docker-compose.test.yml    # Storage emulators for integration tests
├── Cargo.lock                 # Committed lockfile (binary workspace)
└── Cargo.toml                 # Workspace root with [workspace.lints]
```

## Running Tests

### Unit Tests

```bash
# All workspace crates
cargo test --workspace --all-features

# Single crate
cargo test -p mediagit-versioning

# With output (for debugging)
cargo test --workspace -- --nocapture
```

### Integration Tests (requires Docker)

Integration tests are marked `#[ignore]` and need real storage emulators:

```bash
# Start emulators
docker compose -f docker-compose.test.yml up -d

# Run integration tests
export AWS_ACCESS_KEY_ID=minioadmin
export AWS_SECRET_ACCESS_KEY=minioadmin
export AWS_ENDPOINT_URL=http://localhost:9000
export AWS_REGION=us-east-1
export AZURE_STORAGE_CONNECTION_STRING="DefaultEndpointsProtocol=http;AccountName=devstoreaccount1;AccountKey=Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/KBHBeksoGMGw==;BlobEndpoint=http://localhost:10000/devstoreaccount1;"
export GCS_EMULATOR_HOST=http://localhost:4443

cargo test --ignored -p mediagit-storage -p mediagit-server --verbose

# Cleanup
docker compose -f docker-compose.test.yml down -v
```

### MSRV Check

```bash
cargo +1.91.0 check --workspace --all-features
```

## Code Quality

### Formatting

```bash
cargo fmt --all
```

### Linting

```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Workspace lint policy is in `Cargo.toml` under `[workspace.lints]`. Three crates
(config, security, compression) already inherit these. When adding `[lints] workspace = true`
to other crates, fix any new clippy::all warnings first.

### Unused Dependencies

```bash
cargo install cargo-machete --locked
cargo machete
```

### License Headers

All `.rs` files must include an AGPL-3.0 header. Check with:

```bash
while IFS= read -r file; do
  grep -q "GNU Affero General Public License" "$file" || echo "MISSING: $file"
done < <(git ls-files 'crates/**/*.rs')
```

### Security Audit

```bash
cargo install cargo-audit --locked
cargo audit
```

## Key Coding Patterns

### Repository Discovery

```rust
use mediagit_cli::repo::find_repo_root;
let root = find_repo_root().await?;
// Respects MEDIAGIT_REPO env var and -C flag
```

### ObjectDatabase Construction

```rust
// Always prefer with_smart_compression()
let odb = ObjectDatabase::with_smart_compression(root).await?;
```

### Config Loading

```rust
use mediagit_config::schema::Config;
let config = Config::load(&repo_root).await?;
// Author priority: --author CLI > MEDIAGIT_AUTHOR_NAME env > config.toml [author] > $USER
```

### Cross-Platform Paths

```rust
use dunce::canonicalize;
// NOT: std::fs::canonicalize (adds \\?\ prefix on Windows)
let path = dunce::canonicalize(&path)?;
```

### Progress Bars

```rust
// Use ProgressTracker from src/progress.rs, not raw indicatif
use mediagit_cli::progress::ProgressTracker;
```

## Adding a New CLI Command

1. Create `crates/mediagit-cli/src/commands/mycommand.rs`
2. Derive `clap::Parser` on your args struct
3. Add `pub mod mycommand;` to `commands/mod.rs`
4. Add a variant to the `Commands` enum in `main.rs`
5. Wire up execution in `main.rs` match arm

## Adding a New Storage Backend

1. Implement the `Backend` trait in `mediagit-storage`
2. Add a variant to `StorageConfig` enum in `mediagit-config/src/schema.rs`
3. Wire up construction in the storage factory
4. Add integration tests with `#[ignore]` and an emulator
5. Document in `book/src/architecture/backend-*.md`

## Benchmarks

```bash
cargo bench --workspace
```

Benchmarks use [criterion](https://docs.rs/criterion). Results are stored in `target/criterion/`.

## Documentation

```bash
# Install mdbook
cargo install mdbook
# Install mdbook-mermaid (pre-built binary, much faster)
MERMAID_VERSION="0.14.0"
curl -fsSL "https://github.com/badboy/mdbook-mermaid/releases/download/v${MERMAID_VERSION}/mdbook-mermaid-v${MERMAID_VERSION}-x86_64-unknown-linux-gnu.tar.gz" \
  | tar xz -C ~/.cargo/bin/

# Serve docs locally with live reload
cd book
mdbook serve
# Open http://localhost:3000
```

## WSL2 Notes

When developing on Windows via WSL2 with the repository on an NTFS mount (`/mnt/d/...`):
- Cargo's fingerprinting may not detect file changes reliably
- Prefer cloning to a WSL2-native path (`~/projects/mediagit-core`) for reliable incremental builds
- The Linux ELF binary at `target/release/mediagit` works from WSL2
- The Windows PE binary (`mediagit.exe`) requires building from Windows `cmd` or PowerShell

## Getting Help

- Check existing issues: [GitHub Issues](https://github.com/mediagit/mediagit-core/issues)
- Review [CONTRIBUTING.md](https://github.com/mediagit/mediagit-core/blob/main/CONTRIBUTING.md)
- Read the [Architecture Overview](../architecture/README.md)

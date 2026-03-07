# MediaGit-Core 🎬

> High-performance version control for large media files and binary assets

[![CI](https://github.com/winnyboy5/mediagit-core/workflows/CI/badge.svg)](https://github.com/winnyboy5/mediagit-core/actions)
[![License: AGPL-3.0](https://img.shields.io/badge/License-AGPL%203.0-blue.svg)](https://www.gnu.org/licenses/agpl-3.0)
[![Rust Version](https://img.shields.io/badge/rust-1.92+-orange.svg)](https://www.rust-lang.org)
[![Features](https://img.shields.io/badge/features-100%25%20complete-success.svg)](claudedocs/2026-02-27/UNIMPLEMENTED_FEATURES.md)

## 🎯 Status

**Version**: v0.2.1-beta.2
**Status**: 🚧 **BETA**
**Features**: 100% complete (all P0–P3 items implemented)
**Last Validated**: March 5, 2026 

### Validation Results (March 5, 2026)

✅ **194/195 Tests Passing — 0 Failures**
- 27+ real media files tested (58G total dataset)
- 32 CLI commands validated end-to-end
- 22 file types: video, audio, 3D, image, vector, design
- 0 crashes, 0 data corruption, 0 command failures

✅ **Comprehensive Testing (release build)**
- Large files: MOV 398MB (153 MB/s), MP4 264MB (174.4 MB/s)
- Design files: PSD 181MB (119.3 MB/s), PSD 72MB (81 MB/s)
- Cloud backend: MinIO S3 — all operations working
- Server: push / pull / clone / fetch all validated

✅ **All Core Features Validated**
- Content-addressable storage with CAS deduplication (~30% avg storage savings)
- Smart compression — 70+ file type classifications
- Chunking + delta encoding: STL text meshes up to 65% reduction; GLB ~30–50%
- MediaAware chunking: MP4, WAV, GLB, FLAC
- PSD layer preservation
- S3-compatible cloud storage (MinIO confirmed working)

---

## Overview

MediaGit is a Git-like version control system optimized for large media files. Built in Rust for maximum performance, security, and reliability.

### Why MediaGit?

Traditional Git struggles with large binary files. MediaGit solves this with:

- **Intelligent Chunking**: Split large files for efficient storage and transfer
- **Smart Compression**: Type-aware compression — lossless audio/RAW up to 40%, text/JSON up to 70%, pre-compressed video/JPEG stored as-is
- **Cloud-Native**: AWS S3, Azure Blob, Google Cloud Storage, MinIO
- **Media Intelligence**: PSD layer merging, video timeline parsing, audio track handling
- **High Performance**: 80–240 MB/s staging throughput for large files (release build)

### Key Features

🚀 **Performance** (release build, March 2026)
- **Large video (398 MB MOV)**: 153 MB/s staging
- **Large video (264 MB MP4)**: 174.4 MB/s staging
- **PSD design (181 MB)**: 119.3 MB/s staging
- **PSD design (72 MB)**: 81 MB/s staging
- **Compression**: Content-aware, 0% overhead for pre-compressed formats (MP4, JPEG, ZIP)
- **Deduplication**: CAS dedup — 50%+ savings when same content re-stored across branches/versions
- **Delta encoding**: STL text meshes 40–65% delta savings; GLB binary 20–45%

🎨 **Media-Aware Intelligence**
- **PSD Files**: Layer metadata extraction, auto-merge, conflict detection
- **Video**: Timeline parsing, non-overlapping edit merge
- **Audio**: Track-level merge, format metadata
- **3D Models**: OBJ, FBX, Blend, GLTF support

☁️ **Cloud Storage**
- **AWS S3**: Production-ready with encryption, lifecycle policies
- **Azure Blob**: Managed identity support, access tiers
- **Google Cloud Storage**: Service account auth, storage classes
- **MinIO**: S3-compatible local/private cloud (validated at 100+ MB/s)
- **Others**: Backblaze B2, DigitalOcean Spaces

🔒 **Security**
- AES-256-GCM encryption at rest
- JWT + API key authentication
- TLS 1.3 with certificate management
- Rate limiting and DoS protection

📁 **Supported File Formats (70+ extensions)**

| Category | MediaAware Chunking | Other Formats |
|----------|---------------------|---------------|
| **Video** | MP4, MOV, AVI, MKV, WebM | FLV, WMV, MPG |
| **Audio** | WAV (RIFF) | MP3, FLAC, AAC, OGG |
| **3D Models** | GLB, glTF | OBJ, FBX, Blend, STL |
| **Images** | — | JPEG, PNG, PSD, TIFF, RAW, EXR |
| **Documents** | — | PDF, SVG, EPS, AI |
| **Archives** | — | ZIP, TAR, 7Z |

---

## Quick Start

### Installation

#### Pre-built Binaries (Recommended)

Download the latest release for your platform from [GitHub Releases](https://github.com/winnyboy5/mediagit-core/releases).

**Linux / macOS — one-liner install:**
```bash
curl -fsSL https://raw.githubusercontent.com/winnyboy5/mediagit-core/main/install.sh | sh
```

**Linux x86_64 — manual:**
```bash
curl -fsSL https://github.com/winnyboy5/mediagit-core/releases/download/v0.2.1-beta.2/mediagit-0.2.1-beta.2-x86_64-linux.tar.gz \
  | tar xz -C /usr/local/bin
```

**macOS Apple Silicon — manual:**
```bash
curl -fsSL https://github.com/winnyboy5/mediagit-core/releases/download/v0.2.1-beta.2/mediagit-0.2.1-beta.2-aarch64-macos.tar.gz \
  | tar xz -C /usr/local/bin
```

**Windows x86_64 (PowerShell):**
```powershell
Invoke-WebRequest -Uri "https://github.com/winnyboy5/mediagit-core/releases/download/v0.2.1-beta.2/mediagit-0.2.1-beta.2-x86_64-windows.zip" -OutFile mediagit.zip
Expand-Archive mediagit.zip -DestinationPath "$env:LOCALAPPDATA\MediaGit\bin"
# Add to PATH:
[Environment]::SetEnvironmentVariable("Path", "$env:Path;$env:LOCALAPPDATA\MediaGit\bin", "User")
```

#### Docker

```bash
docker pull ghcr.io/winnyboy5/mediagit-core:0.2.1-beta.2
docker run --rm ghcr.io/winnyboy5/mediagit-core:0.2.1-beta.2 mediagit --version
```

#### From Source

```bash
# Requires Rust 1.92+
git clone https://github.com/winnyboy5/mediagit-core.git
cd mediagit-core
cargo build --release

# Binaries at:
# ./target/release/mediagit
# ./target/release/mediagit-server
```

#### All Available Archives

| Platform | Archive |
|----------|---------|
| Linux x86_64 | `mediagit-0.2.1-beta.2-x86_64-linux.tar.gz` |
| Linux ARM64 | `mediagit-0.2.1-beta.2-aarch64-linux.tar.gz` |
| macOS Intel | `mediagit-0.2.1-beta.2-x86_64-macos.tar.gz` |
| macOS Apple Silicon | `mediagit-0.2.1-beta.2-aarch64-macos.tar.gz` |
| Windows x86_64 | `mediagit-0.2.1-beta.2-x86_64-windows.zip` |

Each archive includes `mediagit` (CLI) and `mediagit-server` binaries, plus a `.sha256` checksum file.

### Basic Usage

```bash
# Initialize repository
mediagit init

# Add files
mediagit add *.psd
mediagit add large-video.mp4

# Commit
mediagit commit -m "Initial commit"

# Check status
mediagit status

# View log
mediagit log
```

### Server Setup

```bash
# Run server (default: http://localhost:3000)
mediagit-server

# Or with custom config
mediagit-server --config server.toml
```

**See [DEVELOPMENT_GUIDE.md](DEVELOPMENT_GUIDE.md) for complete setup instructions.**

---

## CLI Reference

All 32 MediaGit commands, grouped by workflow:

### Repository Setup
| Command | Description |
|---------|-------------|
| `mediagit init` | Initialize a new MediaGit repository in the current directory |
| `mediagit clone <url>` | Clone a remote repository into a new directory |

### Staging & Committing
| Command | Description |
|---------|-------------|
| `mediagit add <path>...` | Stage files for the next commit (supports globs, `--all`) |
| `mediagit commit -m <msg>` | Record staged changes as a new commit |
| `mediagit status` | Show working tree status — staged, unstaged, untracked files |
| `mediagit diff [commit]` | Show changes between working tree and commits |

### History & Inspection
| Command | Description |
|---------|-------------|
| `mediagit log [-n <N>]` | Show commit history (supports git-style `-N` shorthand, e.g. `-5`) |
| `mediagit show <object>` | Show detailed info for a commit, blob, or tree |
| `mediagit reflog` | Show reference history (HEAD movement log) |

### Branching & Merging
| Command | Description |
|---------|-------------|
| `mediagit branch` | List, create, rename, or delete branches |
| `mediagit merge <branch>` | Merge a branch into the current branch |
| `mediagit rebase <upstream>` | Rebase current branch onto upstream |
| `mediagit cherry-pick <commit>` | Apply changes from an existing commit |
| `mediagit stash` | Stash uncommitted changes; restore with `stash pop` |
| `mediagit bisect` | Binary search through history to find a bug-introducing commit |

### Tags
| Command | Description |
|---------|-------------|
| `mediagit tag <name>` | Create, list, or delete tags |

### Remote Operations
| Command | Description |
|---------|-------------|
| `mediagit remote` | Add, remove, rename, or list remote connections |
| `mediagit fetch [remote]` | Download remote changes without merging |
| `mediagit pull [remote]` | Fetch and integrate remote changes into current branch |
| `mediagit push [remote]` | Upload local commits to the remote repository |

### Undoing Changes
| Command | Description |
|---------|-------------|
| `mediagit reset [--soft\|--mixed\|--hard] <ref>` | Move HEAD to a previous state |
| `mediagit revert <commit>` | Create a new commit that undoes a previous commit |

### Storage & Integrity
| Command | Description |
|---------|-------------|
| `mediagit stats` | Show repository storage statistics (compression, dedup, delta ratios) |
| `mediagit gc` | Garbage-collect loose objects and repack for efficiency |
| `mediagit fsck` | Check repository integrity — detect corruption or missing objects |
| `mediagit verify <commit>` | Verify commit signatures and data integrity |

### Git Interop (Migration)
| Command | Description |
|---------|-------------|
| `mediagit filter clean` | Filter driver: stage files through MediaGit on `git add` |
| `mediagit filter smudge` | Filter driver: restore files through MediaGit on `git checkout` |
| `mediagit install` | Install MediaGit as a Git filter driver in `.gitattributes` |
| `mediagit track <pattern>` | Configure a file pattern to be managed by MediaGit |
| `mediagit untrack <pattern>` | Stop managing a file pattern with MediaGit |

### Utility
| Command | Description |
|---------|-------------|
| `mediagit version` | Show MediaGit version and build info |
| `mediagit completions <shell>` | Generate shell completion script (bash, zsh, fish, powershell) |

### Global Flags

```bash
mediagit [--verbose] [--quiet] [--color always|auto|never] [-C <path>] <command>
```

| Flag | Description |
|------|-------------|
| `-v, --verbose` | Enable verbose output |
| `-q, --quiet` | Suppress non-essential output |
| `--color <when>` | Colored output: `always`, `auto` (default), or `never` |
| `-C <path>` | Run as if started in `<path>` (like `git -C`) |

---

## Architecture

MediaGit is organized as a Cargo workspace with specialized crates:

```
mediagit-core/
├── crates/
│   ├── mediagit-cli/          # CLI client
│   ├── mediagit-server/       # HTTP server
│   ├── mediagit-storage/      # Storage backends (S3, Azure, GCS, MinIO, Local)
│   ├── mediagit-versioning/   # Object database & version control
│   ├── mediagit-compression/  # Smart compression (zstd, brotli)
│   ├── mediagit-media/        # Media-aware merge intelligence
│   ├── mediagit-config/       # Configuration management
│   ├── mediagit-security/     # Auth, encryption, TLS
│   └── ...
├── tests/                     # Integration tests
├── docker/                    # Docker configurations
├── DEVELOPMENT_GUIDE.md       # Complete setup guide
└── Cargo.toml                 # Workspace configuration
```

### Storage Backends

| Backend | Status | Use Case | Performance |
|---------|--------|----------|-------------|
| **Local Filesystem** | ✅ Ready | Development, testing | Fast |
| **MinIO** | ✅ Validated | Local S3 testing, private cloud | 108 MB/s up, 263 MB/s down |
| **AWS S3** | ✅ Ready | Production, global scale | High |
| **Azure Blob** | ✅ Ready | Azure-centric deployments | High |
| **Google Cloud Storage** | ✅ Ready | GCP-centric deployments | High |
| **Backblaze B2** | ✅ Ready | Cost-effective storage | Good |
| **DigitalOcean Spaces** | ✅ Ready | Simple cloud storage | Good |

---

## Industry Use Cases

MediaGit is designed for **enterprise-scale media workflows**:

### VFX Studio: 50TB Shot Library
| Feature | Capability |
|---------|------------|
| **Deduplication** | CDC + Delta = typically 25–50% savings |
| **Fast Clone** | Differential checkout (<1s for unchanged) |
| **Branching** | Instant branch creation |
| **Cost** | $0 (AGPL) vs $50k/year Perforce |

### Game Dev: 10TB Texture Library
| Feature | Capability |
|---------|------------|
| **Cross-platform dedup** | Same source art deduped |
| **Smart compression** | Skip GPU formats, compress PSD |
| **Platform checkout** | Pull only needed assets |

### Virtual Production: 20TB HDRI Library
| Feature | Capability |
|---------|------------|
| **Multi-backend** | Local NAS + S3 cloud sync |
| **Differential** | Pull only changed environments |
| **Offline** | Full DVCS, work without internet |

### ML/Datasets: 100TB Training Data
| Feature | Capability |
|---------|------------|
| **Chunking** | CDC finds duplicates across versions |
| **Differential** | Pull only new chunks (incremental) |
| **Storage** | S3 + Glacier lifecycle support |

---

## Performance

### Validated Throughput (March 2026 — release build)

| File | Size | Throughput | Strategy | Status |
|------|------|------------|----------|--------|
| **MP4 (large)** | 264 MB | 174.4 MB/s | Store | ✅ Pass |
| **MOV (large)** | 398 MB | 153.0 MB/s | Store | ✅ Pass |
| **PSD (large)** | 181 MB | 119.3 MB/s | Zstd Best | ✅ Pass |
| **PSD (72 MB)** | 72 MB | 81.0 MB/s | Zstd Best | ✅ Pass |
| **USDZ (8 MB)** | 8 MB | 34.3 MB/s | Store | ✅ Pass |
| **MP4 (5 MB)** | 5 MB | 30.4 MB/s | Store | ✅ Pass |
| **FLAC (38 MB)** | 38 MB | 2.2 MB/s | Store | ✅ Pass |
| **WAV (55 MB)** | 55 MB | 2.1 MB/s | Zstd Best* | ✅ Pass |
| **GLB (14 MB)** | 14 MB | 3.0 MB/s | Zstd Best | ✅ Pass |
| **MinIO PSD (72 MB)** | 72 MB | 72.8 MB/s | Cloud | ✅ Pass |
| **MinIO MP4 (5 MB)** | 5 MB | 15.1 MB/s | Cloud | ✅ Pass |

*WAV is CPU-bound due to Zstd Best compression on uncompressed PCM audio.

### Compression Efficiency

Compression strategy is selected automatically per file type. Pre-compressed formats are stored as-is to avoid CPU waste and size expansion.

| Category | Extensions | Strategy | Size Reduction | Notes |
|----------|------------|----------|---------------|-------|
| Video (encoded) | MP4, MOV, AVI, MKV, WebM, FLV | Store | ~0% | H.264/H.265 codec; recompression expands |
| Audio (lossy) | MP3, AAC, OGG, Opus | Store | ~0% | Already compressed |
| Images (lossy/compressed) | JPEG, PNG, GIF, WebP, AVIF | Store | ~0% | Pre-compressed |
| GPU textures | DDS, KTX, KTX2, ASTC | Store | ~0% | Hardware-compressed formats |
| Archives | ZIP, GZ, 7Z, RAR, USDZ | Store | ~0% | Pre-compressed containers |
| Office documents | DOCX, XLSX, PPTX, ODT | Store | ~0% | ZIP containers with compressed XML |
| Creative (PDF containers) | AI, INDD | Store | ~0% | PDF-based; recompression expands |
| ML columnar data | Parquet, Arrow, Feather, ORC | Store | ~0% | Already columnar-compressed |
| Audio (lossless) | WAV, AIFF, ALAC | Zstd Best | 20–40% | Uncompressed PCM; content-dependent |
| Audio (FLAC) | FLAC | Zstd Best | 5–15% | FLAC already compressed; limited gain |
| Raw images | TIFF, BMP, EXR, HDR, RAW | Zstd Best | 30–60% | Uncompressed raster; compresses well |
| 3D models (mesh) | STL, OBJ, PLY | Zstd Best | 40–65% | Triangle soup; float data compresses well |
| 3D models (binary) | FBX, GLB, GLTF, DAE | Zstd Best | 20–45% | Mixed binary+metadata |
| PSD / PSB | PSD, PSB | Zstd Best | 15–35% | Layer data + compressed internal streams |
| Documents | PDF, SVG, EPS | Zstd Default | 20–50% | Mixed binary/text |
| DCC project files | AEP, PRPROJ, BLEND, MA, MB, C4D | Zstd Default | 10–35% | Binary project data |
| Audio projects | .als, .ptx, .logic, .flp | Zstd Default | 10–30% | DAW project files |
| Game projects | .unity, .uasset, .tscn | Zstd Default | 10–35% | Game engine formats |
| ML models | .safetensors, .pkl, .joblib | Zstd Fast | 5–25% | Large float arrays; limited compressibility |
| ML checkpoints | .ckpt, .pt, .pth | Zstd Fast | 5–20% | Training weights |
| Text / Code / Data | TXT, JSON, XML, YAML, TOML, CSV | Brotli Default | 50–75% | Best for structured text |

> **Average across a mixed media project: ~30% storage reduction.** Results vary by content — text-heavy projects save more, video-heavy projects less.

### Deduplication (Content-Addressed Storage)

MediaGit uses CDC chunking + CAS: identical chunks are stored once regardless of how many files or commits reference them.

| Scenario | Description | Typical Savings |
|----------|-------------|-----------------|
| Same file in multiple branches | Re-committing unchanged assets | 50–66% per duplicate |
| Identical files across team members | Same texture/audio stored by N people | ~(N-1)/N savings |
| Small edits to large files | Only changed chunks stored | 70–95% chunk reuse |
| Completely different content | No shared chunks | 0% dedup (compression only) |

> Validated: 3× identical 5MB MP4 → 66% savings; 2× identical 72MB PSD → 68% savings.

### Scalability (TB+ Architecture)

MediaGit is **designed for terabyte-scale files**:

| Component | Limit | Evidence |
|-----------|-------|----------|
| **File Size** | 18 exabytes | u64 offset addressing |
| **Chunk Count** | ~2.25 billion | u32 chunk index |
| **Memory** | O(chunk_size) | Streaming I/O |
| **Storage** | Unlimited | S3/cloud backends |

**Tested**: Up to 6GB single file (1,541 chunks)
**Designed for**: TB+ with adaptive 8MB chunks for >100GB files

---

## Configuration

MediaGit supports multiple configuration methods:

### 1. TOML Configuration

```toml
# config.toml
[storage]
backend = "s3"

[storage.s3]
bucket = "my-mediagit-bucket"
region = "us-east-1"
encryption = true

[compression]
enabled = true
algorithm = "zstd"
level = 3
```

### 2. Environment Variables

```bash
export MEDIAGIT_S3_BUCKET=my-bucket
export MEDIAGIT_S3_REGION=us-east-1
export MEDIAGIT_S3_ACCESS_KEY_ID=...
export MEDIAGIT_S3_SECRET_ACCESS_KEY=...
```

### 3. Cloud Provider Credentials

```bash
# AWS (auto-detected)
aws configure

# Azure
az login

# GCP
gcloud auth login
```

**See [DEVELOPMENT_GUIDE.md](DEVELOPMENT_GUIDE.md) for complete configuration examples.**

---

## Documentation

### Guides
- **[DEVELOPMENT_GUIDE.md](DEVELOPMENT_GUIDE.md)** - Complete setup for local, MinIO, AWS, Azure, GCS
- **[ARCHITECTURE.md](ARCHITECTURE.md)** - Project Architecture


### Examples
- Configuration examples: `crates/mediagit-config/examples/`
- Docker configs: `docker/`
- Test scripts: `tests/`

---

## Development

### Prerequisites

- **Rust**: 1.92+ (MSRV — check with `rustc --version`)
- **OS**: Linux, macOS, or WSL2 (Windows)
- **Tools**: cargo, git

### Building

```bash
# Debug build (faster compilation)
cargo build

# Release build (optimized)
cargo build --release

# Build with specific features
cargo build --features tls
```

### Testing

```bash
# Run all tests (uses memory-optimized settings via .cargo/config.toml)
cargo test --workspace

# Limit threads for memory-constrained systems
cargo test --workspace -- --test-threads=2

# Run tests with logging
RUST_LOG=debug cargo test -- --nocapture

# Run ignored tests (large file tests, memory-intensive)
cargo test --workspace -- --ignored
```

#### Test Organization

| Crate | Test File | Coverage |
|-------|-----------|----------|
| **mediagit-cli** | `tests/*.rs` | 20+ test files: init, add, commit, branch, merge, etc. |
| **mediagit-metrics** | `tests/metrics_test.rs` | Registry, dedup, compression, cache metrics |
| **mediagit-migration** | `tests/migration_test.rs` | State, progress, integrity verification |
| **mediagit-security** | `tests/security_test.rs` | Encryption, KDF, audit logging |
| **mediagit-compression** | `tests/proptest_compression.rs` | Property-based compression roundtrip |
| **mediagit-versioning** | `tests/proptest_odb.rs` | Property-based ODB operations |
| **mediagit-storage** | `tests/*.rs` | S3, Azure, GCS, MinIO backends |

#### E2E Tests

```bash
# Run comprehensive E2E suite
cargo test -p mediagit-cli --test comprehensive_e2e_tests

# Run large file tests (requires test files)
cargo test -p mediagit-cli --test large_file_test -- --ignored

# Run performance benchmarks
cargo test -p mediagit-cli --test performance_benchmark_test -- --ignored
```

#### Crate-Specific Tests

```bash
# Test individual crates
cargo test -p mediagit-metrics
cargo test -p mediagit-security
cargo test -p mediagit-migration
cargo test -p mediagit-compression
cargo test -p mediagit-versioning
cargo test -p mediagit-storage
```


### Code Quality

```bash
# Format code
cargo fmt

# Lint
cargo clippy

# Check compilation
cargo check
```

---

## Platform Support

| Platform | Architecture | Status | Notes |
|----------|--------------|--------|-------|
| **Linux** | x86_64 | ✅ Supported | Primary development platform |
| **Linux** | aarch64 | ✅ Supported | ARM64 support |
| **macOS** | x86_64 | ✅ Supported | Intel Macs |
| **macOS** | Apple Silicon | ✅ Supported | M1/M2/M3 |
| **Windows** | x86_64 | ✅ Supported | Via WSL2 recommended |
| **Windows** | ARM64 | ✅ Supported | Surface Pro X, etc. |

---

## Production Deployment

### Server Deployment

```bash
# Build release binary
cargo build --release

# Copy binary to production
scp target/release/mediagit-server user@server:/opt/mediagit/

# Run as systemd service
sudo systemctl enable mediagit-server
sudo systemctl start mediagit-server
```

### Configuration Checklist

- [ ] Choose storage backend (S3, Azure, GCS, MinIO)
- [ ] Configure credentials (environment variables or config file)
- [ ] Enable HTTPS/TLS for production
- [ ] Set up authentication (JWT or API keys)
- [ ] Configure rate limiting
- [ ] Set up monitoring and logging
- [ ] Test backup and recovery procedures

**See [DEVELOPMENT_GUIDE.md § Production Deployment](DEVELOPMENT_GUIDE.md#production-deployment-checklist) for complete checklist.**

---

## Contributing

We welcome contributions! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for details.

### Development Workflow

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Make your changes
4. Run tests (`cargo test`)
5. Commit changes (`git commit -m 'Add amazing feature'`)
6. Push to branch (`git push origin feature/amazing-feature`)
7. Open a Pull Request

### Code Standards

- Follow Rust best practices (rustfmt, clippy)
- Write tests for new features
- Update documentation
- Maintain backward compatibility
- Add entries to CHANGELOG.md

---

## Roadmap

### v0.1.0 ✅ — February 27, 2026
*Initial public release — core infrastructure*

- [x] Core CLI: `init`, `add`, `commit`, `status`, `log`, `branch`, `merge`, `push`, `pull`
- [x] Content-addressed object database (SHA-256, CDC chunking)
- [x] Intelligent compression — Zstd, Brotli, per-type strategy (70+ file types)
- [x] PSD layer-aware merge intelligence
- [x] Multi-cloud storage: AWS S3, Azure Blob, GCS, MinIO, Backblaze B2, DO Spaces
- [x] Security: AES-256-GCM encryption at rest, Argon2id key derivation
- [x] Observability: structured logging, Prometheus metrics
- [x] 960 unit tests, 80%+ coverage
- [x] Multi-platform binaries: Linux, macOS, Windows (x86_64 + ARM64)

### v0.2.0 ✅ — March 5, 2026
*Major features — storage efficiency and security*

- [x] Dual-layer delta encoding (bsdiff + sliding-window)
- [x] Delta chain depth cap (MAX_DELTA_DEPTH=10) — prevents read-amplification
- [x] Adaptive chunk sizes (1–8 MB) — replaces fixed 64 MB chunks
- [x] Per-type similarity thresholds for delta compression
- [x] AES-256-GCM client-side encryption with Argon2id KDF
- [x] TLS 1.3 for all network operations
- [x] JWT + API key authentication (server mode)
- [x] Video timeline and audio track-based merging
- [x] Automated multi-platform release CI (Linux, macOS, Windows, Docker, crates.io)
- [x] S3/MinIO bucket auto-create on first use
- [x] 194 tests passing on release build (0 failures)

### v0.2.1 (Current Beta) — March 2026
*Stability and distribution*

- [x] Pre-built release binaries on GitHub Releases (5 platforms)
- [x] Docker multi-arch images on GHCR
- [x] PowerShell installer (`install.ps1`) with `-UseBasicParsing`
- [x] Install scripts with pre-release fallback (fetch `/releases` when no stable exists)
- [x] Automated version bumping (`scripts/bump-version.sh`)
- [x] Full documentation sync: book, architecture, CLI reference
- [x] Security audit clean (`cargo audit`)

### v0.3.0 (Planned)
*Developer experience and ecosystem*

- [ ] `mediagit diff` with media-aware visual diffing (image pixel diff, audio waveform)
- [ ] Conflict markers for PSD/Blend/FBX with editor integrations
- [ ] Shallow clone (`--depth N`) for large repositories
- [ ] Partial/sparse checkout (pull only specific asset subdirectories)
- [ ] `mediagit migrate` — import from Git-LFS repositories
- [ ] Chocolatey and Homebrew package managers
- [ ] Official VS Code extension (file status, staging UI)

### v1.0.0 (Stable Release)
*Production-grade enterprise features*

- [ ] Stable API and wire protocol (v1 guarantee)
- [ ] SSO integration (OIDC/SAML) for enterprise auth
- [ ] Multi-region active-active replication
- [ ] Audit log export (compliance — SOC 2, GDPR)
- [ ] Plugin system for custom media type handlers
- [ ] Web UI for repository browsing and review workflows
- [ ] Commercial support tiers

---

## Troubleshooting

### Common Issues

**"Binary not found"**
```bash
# Solution: Build the project
cargo build --release
ls -lh target/release/mediagit
```

**"MinIO connection failed"**
```bash
# Check MinIO status
docker ps | grep minio
curl http://localhost:9000/minio/health/live

# Restart if needed
docker restart mediagit-minio
```

**"AWS S3 access denied"**
```bash
# Verify credentials
aws sts get-caller-identity
aws s3 ls s3://my-bucket/

# Check IAM permissions
aws iam get-user-policy --user-name mediagit-user --policy-name MediaGitS3Policy
```

**"Could not fetch latest version" during install**
```bash
# The /releases/latest API returns 404 when only pre-releases exist.
# Pass the version explicitly:
VERSION=0.2.1-beta.1 curl -fsSL https://raw.githubusercontent.com/winnyboy5/mediagit-core/main/install.sh | sh

# Or on Windows PowerShell:
iwr -UseBasicParsing https://raw.githubusercontent.com/winnyboy5/mediagit-core/main/install.ps1 | iex
```

**See [DEVELOPMENT_GUIDE.md § Troubleshooting](DEVELOPMENT_GUIDE.md#troubleshooting) for complete guide.**

---

## License

This project is licensed under the **GNU Affero General Public License v3.0 (AGPL-3.0)**.

Key points:
- ✅ Free to use, modify, and distribute
- ✅ Source code must be made available
- ✅ Network use requires source disclosure (AGPL provision)
- ✅ Commercial use allowed with license compliance

See [LICENSE](LICENSE) for complete terms.

---

## Acknowledgments

Built with the modern Rust ecosystem:

- [Tokio](https://tokio.rs/) - Async runtime
- [Clap](https://docs.rs/clap/) - CLI framework
- [Serde](https://serde.rs/) - Serialization
- [Tracing](https://tokio.rs/tokio/topics/tracing) - Observability
- [AWS SDK](https://github.com/awslabs/aws-sdk-rust) - S3 integration
- [Azure SDK](https://github.com/azure/azure-sdk-for-rust) - Blob storage
- [zstd](https://github.com/facebook/zstd) - Fast compression

Special thanks to:
- Rust community for excellent tooling
- Contributors and testers
- Open-source maintainers

---

## Support

- **Documentation**: [DEVELOPMENT_GUIDE.md](DEVELOPMENT_GUIDE.md)
- **Issues**: [GitHub Issues](https://github.com/winnyboy5/mediagit-core/issues)
- **Discussions**: [GitHub Discussions](https://github.com/winnyboy5/mediagit-core/discussions)

---

## Statistics

- **Lines of Code**: 78,000+ (Rust)
- **Features**: 100% complete (all P0–P3 items)
- **Test Coverage**: 960 unit tests; 194 E2E tests validated on 22+ file types (58 GB dataset)
- **Staging Throughput**: 80–240 MB/s (release build, local storage); 100+ MB/s on MinIO
- **Storage Savings**: ~30% average across mixed media projects (compression + dedup + delta)
- **Stability**: 0 crashes, 0 data corruption across all validated test runs
- **File Formats**: 70+ extensions (video, audio, image, 3D, DCC, ML, game engines, office)
- **Platforms**: Linux, macOS, Windows — x86_64 + ARM64

---

**Made with 🦀 and ❤️ by the MediaGit Contributors**

**Status**: Beta | **Version**: v0.2.1-beta.2 | **Updated**: March 7, 2026

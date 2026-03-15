# MediaGit-Core 🎬

> High-performance version control for large media files and binary assets

[![CI](https://github.com/winnyboy5/mediagit-core/workflows/CI/badge.svg)](https://github.com/winnyboy5/mediagit-core/actions)
[![License: AGPL-3.0](https://img.shields.io/badge/License-AGPL%203.0-blue.svg)](https://www.gnu.org/licenses/agpl-3.0)
[![Rust Version](https://img.shields.io/badge/rust-1.92+-orange.svg)](https://www.rust-lang.org)
[![Features](https://img.shields.io/badge/features-100%25%20complete-success.svg)](claudedocs/2026-02-27/UNIMPLEMENTED_FEATURES.md)

## 🎯 Status

**Version**: v0.2.3-beta.1
**Status**: 🚧 **BETA**
**Features**: 100% complete (all P0–P3 items implemented)
**Last Validated**: March 2026 — Linux & Windows, release build

✅ **32 CLI commands validated end-to-end** — 0 crashes, 0 data corruption
✅ **27+ file types tested** (58 GB dataset) across video, audio, 3D, image, design, ML
✅ **All storage backends validated** — local, MinIO S3, push / pull / clone / fetch
✅ **Files up to 398 MB** staged and transferred; single-file scalability to 6 GB tested

| Metric | Result |
|--------|--------|
| **Staging (small files < 5 MB)** | 25–182 MB/s |
| **Staging (large video/PSD, no chunking)** | 80–240 MB/s |
| **Staging (chunked files 5–60 MB)** | 3.6–5.2 MB/s |
| **Network push** | 167 MB/s (150 MB over local server) |
| **Network clone** | 100 MB/s (150 MB over local server) |
| **Commit latency** | 30–52 ms (constant regardless of file size) |
| **Average storage savings** | ~30% (compression + dedup + delta, mixed media) |
| **Exact dedup (CAS): exact duplicate** | 99.9% savings (CAS hit, 0.7 KB overhead) |
| **Exact dedup (CAS): 3× identical MP4** | 66% savings |
| **Exact dedup (CAS): small edit to large file** | 70–95% chunk reuse via CDC + CAS |
| **Similarity delta: STL/OBJ text mesh** | 40–65% savings |
| **Similarity delta: GLB/FBX binary** | 20–45% savings |
| **Similarity delta: PSD/WAV** | 15–40% savings |

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
- **Pre-compressed files** (MP4, MOV, JPEG, USDZ): 25–240 MB/s — store-mode, zero CPU overhead
- **Compressible files** (PSD, TIFF, WAV): 2–120 MB/s — Zstd compression + optional chunking
- **Chunked large files** (GLB, FLAC, AI): 1.9–5.2 MB/s — CDC chunking + delta encoding
- **Network**: 167 MB/s push · 100 MB/s clone (local server, 150 MB dataset)
- **Commit latency**: 30–52 ms constant regardless of file size
- **Compression**: ~30% average storage savings across mixed media projects
- **Exact dedup (CAS)**: 66–99.9% savings when identical content is re-stored; CDC ensures chunk-level granularity
- **Similarity delta**: 15–65% savings for similar-but-changed chunks; FNV-1a sampler + type-aware thresholds

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
curl -fsSL https://github.com/winnyboy5/mediagit-core/releases/download/v0.2.3-beta.1/mediagit-0.2.3-beta.1-x86_64-linux.tar.gz \
  | tar xz -C /usr/local/bin
```

**macOS Apple Silicon — manual:**
```bash
curl -fsSL https://github.com/winnyboy5/mediagit-core/releases/download/v0.2.3-beta.1/mediagit-0.2.3-beta.1-aarch64-macos.tar.gz \
  | tar xz -C /usr/local/bin
```

**Windows x86_64 (PowerShell):**
```powershell
Invoke-WebRequest -Uri "https://github.com/winnyboy5/mediagit-core/releases/download/v0.2.3-beta.1/mediagit-0.2.3-beta.1-x86_64-windows.zip" -OutFile mediagit.zip
Expand-Archive mediagit.zip -DestinationPath "$env:LOCALAPPDATA\MediaGit\bin"
# Add to PATH:
[Environment]::SetEnvironmentVariable("Path", "$env:Path;$env:LOCALAPPDATA\MediaGit\bin", "User")
```

#### Docker

```bash
docker pull ghcr.io/winnyboy5/mediagit-core:0.2.3-beta.1
docker run --rm ghcr.io/winnyboy5/mediagit-core:0.2.3-beta.1 mediagit --version
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
| Linux x86_64 | `mediagit-0.2.3-beta.1-x86_64-linux.tar.gz` |
| Linux ARM64 | `mediagit-0.2.3-beta.1-aarch64-linux.tar.gz` |
| macOS Intel | `mediagit-0.2.3-beta.1-x86_64-macos.tar.gz` |
| macOS Apple Silicon | `mediagit-0.2.3-beta.1-aarch64-macos.tar.gz` |
| Windows x86_64 | `mediagit-0.2.3-beta.1-x86_64-windows.zip` |

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

### Validated Staging Throughput (release build)

| Format | Size | Throughput | Strategy | Notes |
|--------|------|------------|----------|-------|
| JPEG | 506 KB | 25 MB/s | Store | Direct write, no chunking |
| PNG | 1.8 MB | 72 MB/s | Store | Direct write, no chunking |
| USDZ | 8.2 MB | 182 MB/s | Store | Direct write, no chunking |
| MP4 | 5.1 MB | 146 MB/s | Store | Direct write, no chunking |
| MP4 (large) | 264 MB | 174 MB/s | Store | Pre-compressed; store-mode |
| MOV (large) | 398 MB | 153 MB/s | Store | Pre-compressed; store-mode |
| PSD | 181 MB | 119 MB/s | Zstd Best | Layer data compresses well |
| PSD | 72 MB | 72–81 MB/s | Zstd Best | Layer data compresses well |
| GLB | 13.8 MB | 3.0–4.2 MB/s | Zstd Best | GLB parser + CDC chunking |
| GLB | 25.4 MB | 5.2 MB/s | Zstd Best | GLB parser + CDC chunking |
| FLAC | 38–39 MB | 2.2–4.1 MB/s | Zstd Best | FastCDC chunking |
| WAV | 55–57 MB | 2.1–3.6 MB/s | Zstd Best | RIFF parser + chunking (CPU-bound) |
| AI (large) | 129 MB | 1.9 MB/s | Zstd Best | Deep delta + chunking |
| AI (very large) | 216 MB | 2.4 MB/s | Zstd Best | Deep delta + chunking |
| MinIO PSD | 72 MB | 72.8 MB/s | Cloud upload | S3-compatible backend |
| Push (150 MB) | — | 167 MB/s | Network | Local server |
| Clone (150 MB) | — | 100 MB/s | Network | Local server |

> WAV is CPU-bound: RIFF chunking + Zstd Best on uncompressed PCM. Throughput scales with CPU core count.

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

### Comparison with Git LFS and Perforce

| Feature | **MediaGit** | **Git LFS** | **Perforce (Helix Core)** |
|---------|:----------:|:---------:|:-----------------------:|
| **Architecture** | Native VCS with built-in chunking | Git extension + external store | Centralized VCS |
| **Install Complexity** | Single binary | Git + LFS extension + server | Server + client + license |
| **Deduplication** | ✅ Content-addressable (SHA-256) | ❌ None | ✅ Server-side |
| **Delta Compression** | ✅ Cross-version via similarity | ❌ None | ✅ RCS-style deltas |
| **Chunking** | ✅ Content-defined (CDC) | ❌ Whole-file | ❌ Whole-file |
| **Storage (100MB × 2 versions)** | ~75–80 MB (with delta) | ~200 MB (2 full copies) | ~110–120 MB |
| **Small file add** (< 5 MB) | **20–45 ms** | ~50–100 ms | ~100–200 ms |
| **Large file add** (129 MB) | **69 s** (local, with delta) | 5–30 s (HTTP upload) | 10–60 s |
| **Clone 150 MB** | **1.5 s** (local server) | 5–20 s (HTTP chunked) | 10–30 s |
| **Offline Commits** | ✅ Full local history | ✅ (Git handles) | ❌ Requires server |
| **Branching Cost** | ✅ Instant (ref-based) | ✅ (Git handles) | ⚠️ Copy-based (expensive) |
| **Lock Support** | ❌ Not yet | ✅ File locking | ✅ Exclusive checkout |
| **Max File Size** | 16 GB+ (u64 offset) | Varies by server | Unlimited |
| **Cost** | Free (AGPL-3.0) | Free + server costs | $$$ per-seat |

### Storage Reduction: Two Complementary Mechanisms

MediaGit achieves storage savings through two distinct layers that work together on every chunk:

#### Layer 1 — Exact Deduplication (CAS)

SHA-256 content-addressing means identical chunks are stored only once, no matter how many files, commits, or branches reference them. Before storing any chunk, the ODB checks `storage.exists(sha256_key)` — a hit skips the write entirely.

| Scenario | Validated Result | How |
|----------|-----------------|-----|
| Exact duplicate file (506 KB JPEG) | **99.9% savings** (0.7 KB stored vs 506 KB) | Full-object CAS hit |
| 3× identical 5 MB MP4 | **66% savings** | 1 copy stored, 2 zero-cost refs |
| 2× identical 72 MB PSD | **68% savings** | Chunk-level CAS across both files |
| Small edit to large file | **70–95% chunk reuse** | Unchanged CDC chunks → CAS hit; only edited chunks are new |
| Same asset across N team members | ~(N−1)/N savings | Single stored object, N refs |
| Completely different content | 0% dedup | No shared chunks; compression only |

> **CDC + CAS synergy**: Content-defined chunking (FastCDC) splits files at natural boundaries. When you version a large file, only the chunks that actually changed produce new SHA-256 hashes — all unchanged chunks are free CAS hits.

#### Layer 2 — Similarity-Based Delta Compression

For chunks that are new (no CAS hit) but *similar* to a previously stored chunk, the `SimilarityDetector` samples 10 × 1 KB windows per object using FNV-1a hashing and scores candidates. If the similarity score meets the type-aware threshold, the chunk is stored as a **delta** (base OID + sliding-window diff instructions) rather than a full copy.

```
New chunk → CAS check → miss → SimilarityDetector.find_similar_with_size_ratio()
               ↓                        ↓                          ↓
          hit: free dedup       score ≥ threshold           score < threshold
                              DeltaEncoder.encode()       compress + store full
                              store delta (base + diff)
```

| Format | Eligible? | Similarity Threshold | Size Ratio Threshold | Validated Savings |
|--------|-----------|---------------------|---------------------|-------------------|
| Text / Code / JSON | ✅ Always | 0.85–0.95 | 0.80 | **50–75%** |
| SVG / EPS (vector) | ✅ Always | 0.30 | 0.80 | **20–50%** |
| PSD / PSB | ✅ Always | 0.70 | 0.80 | **15–35%** |
| WAV / AIFF (lossless audio) | ✅ Always | 0.65 | 0.80 | **20–40%** |
| STL / OBJ / PLY (text 3D) | ✅ Always | 0.30 | 0.80 | **40–65%** |
| GLB / FBX (binary 3D) | ✅ If > 1 MB | 0.70 | 0.80 | **20–45%** |
| MP4 / MKV (video) | ✅ If > 100 MB | 0.50 | 0.70 | Variable |
| AI / InDesign (PDF containers) | ✅ If > 50 MB | 0.15 | 0.50 | ~0% (pre-compressed internals) |
| JPEG / PNG / ZIP | ❌ Never | — | — | Not eligible |

**Delta chain cap**: depth 10 maximum. Prevents read-amplification — at depth 10 the object is re-stored as a full compressed copy.

**Real-world storage observations:**
- 329 MB AI (2 versions) → 248 MB stored — **25% saved**, 27 delta + 62 full chunks
- 144 MB mixed dataset → 100 MB stored — **31% saved** (69% storage ratio)
- 3 versioned 506 KB JPEGs: v1 = 496 KB, v2 = +536 KB new, v3 (exact dup of v1) = **+0.7 KB only** (CAS hit)

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
# .mediagit/config.toml
[storage]
backend = "s3"
bucket = "my-mediagit-bucket"
region = "us-east-1"
encryption = true
encryption_algorithm = "AES256"   # Options: AES256, aws:kms

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
- [x] `branch rename` argument order aligned with git semantics (`OLD NEW`)
- [x] Validated on Linux + Windows; all 32 commands stable across both platforms

### v0.3.0 and beyond

See [FUTURE_TODOS.md](./FUTURE_TODOS.md) for planned features.

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
- **Test Coverage**: 960 unit tests; 194 E2E tests (Linux) + 84 scenarios (Windows) validated
- **Staging Throughput**: 25–240 MB/s for small files; 1.9–5.2 MB/s for chunked large files
- **Network Throughput**: 167 MB/s push, 100 MB/s clone (local server)
- **Storage Savings**: ~30% average across mixed media projects (compression + dedup + delta)
- **Stability**: 0 crashes, 0 data corruption across all validated test runs
- **File Formats**: 70+ extensions (video, audio, image, 3D, DCC, ML, game engines, office)
- **Platforms**: Linux, macOS, Windows — x86_64 + ARM64

---

**Made with 🦀 and ❤️ by the MediaGit Contributors**

**Status**: Beta | **Version**: v0.2.3-beta.1 | **Updated**: March 12, 2026

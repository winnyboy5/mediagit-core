# MediaGit-Core Development Guide
**Version**: 0.2.7-beta.1
**Last Updated**: May 25, 2026

Complete setup guide for MediaGit development - from beginner setup to production deployment.

---

## 📋 Table of Contents

1. [Understanding MediaGit Architecture](#understanding-mediagit-architecture)
2. [Prerequisites](#prerequisites)
3. [Quick Start Guide](#quick-start-guide)
4. [Local Development Setup](#local-development-setup)
5. [Backend Configurations](#backend-configurations)
   - [Local Filesystem](#1-local-filesystem-backend-default)
   - [MinIO (S3-Compatible)](#2-minio-s3-compatible-backend)
   - [AWS S3](#3-aws-s3-backend)
   - [Azure Blob Storage](#4-azure-blob-storage-backend)
   - [Google Cloud Storage](#5-google-cloud-storage-backend)
6. [Server Setup](#server-setup)
7. [Client-Server Workflows](#client-server-workflows)
8. [Complete Configuration Reference](#complete-configuration-reference)
9. [Testing Your Setup](#testing-your-setup)
10. [Troubleshooting](#troubleshooting)
11. [Performance Tuning](#performance-tuning)

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

**What you need**:
- ✅ `mediagit` binary only
- ❌ No server required
- ❌ No network needed

**What you can do**:
- `mediagit init` - Initialize repository
- `mediagit add` - Stage files
- `mediagit commit` - Save changes
- `mediagit status` - Check status
- `mediagit log` - View history

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

**What you need**:
- ✅ `mediagit` binary (client)
- ✅ `mediagit-server` running (locally or remote)
- ✅ Network connection

**What you can do**:
- Everything from Mode 1 **PLUS**:
- `mediagit push` - Upload to server
- `mediagit pull` - Download from server
- `mediagit clone` - Copy remote repository
- `mediagit fetch` - Get remote changes

### Storage Architecture

MediaGit uses **TWO separate storage locations**:

#### 1. Repository Metadata (`.mediagit/`)
```
your-project/
├── .mediagit/           ← Repository structure (like .git/)
│   ├── objects/         ← Compressed objects
│   ├── refs/            ← Branch/tag references
│   ├── HEAD             ← Current branch
│   └── config.toml      ← Local config
├── your-files.psd
└── config.toml          ← Optional: storage backend config
```

**Contains**: Commits, branches, refs, MediaGit metadata

**Always stored**: Locally on your machine

#### 2. Object Storage Backend (Configurable)
```
Default filesystem backend:
mediagit-data/           ← Actual file objects
├── objects/
│   ├── ab/
│   │   └── cd/
│   │       └── abcd1234...  ← Chunked file data
```

**Contains**: Actual file content (chunked, compressed, deduplicated)

**Can be stored**:
- Local filesystem (`./mediagit-data/`)
- AWS S3 bucket
- Azure Blob Storage
- Google Cloud Storage
- MinIO server

### Key Differences from Git

| Aspect | Git | MediaGit |
|--------|-----|----------|
| **Optimized for** | Text/code | Large media files |
| **Metadata** | `.git/` directory | `.mediagit/` directory |
| **Object storage** | Inside `.git/objects/` | Separate backend (configurable) |
| **Deduplication** | File-level | Chunk-level (CDC) |
| **Compression** | zlib | zstd/brotli (configurable) |
| **Max file size** | ~100MB practical | Multi-GB supported |

### Choosing Your Mode

**Answer these questions**:

1. Are you working alone? → **Standalone mode**
2. Do you need remote backups? → **Client-Server mode**
3. Do you need to collaborate with others? → **Client-Server mode**
4. Just experimenting with MediaGit? → **Standalone mode**

**Still unsure?** Start with Standalone mode (simpler setup, no server required). You can migrate to Client-Server mode later when you need collaboration or remote backups.

**Decision flowchart**:
```
Need collaboration OR remote backups?
├─ No  → Standalone mode (Quick Start guide)
└─ Yes → Client-Server mode (Server Setup section)
```

---

## Quick Start Guide

**New to MediaGit?** Follow these steps to get started in 5 minutes:

### Step 1: Install Rust (if not already installed)

```bash
curl --proto='=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
rustc --version  # Must show 1.92.0 or higher
```

### Step 2: Clone and Build

```bash
git clone https://github.com/winnyboy5/mediagit-core.git
cd mediagit-core
cargo build  # Takes 5-10 minutes on first build
```

### Step 3: Verify Binaries

```bash
# Check that both binaries were built successfully
ls -lh target/debug/mediagit
ls -lh target/debug/mediagit-server

# Both should show file sizes (not "No such file")
```

### Step 4: Choose Your Mode

**For local-only use (no server needed)**:

```bash
cd /path/to/your/project
../mediagit-core/target/debug/mediagit init
../mediagit-core/target/debug/mediagit add your-file.psd
../mediagit-core/target/debug/mediagit commit -m "First commit"
```

**For team collaboration (requires server)**:

```bash
# Terminal 1: Start server
cd mediagit-core
./target/debug/mediagit-server

# Terminal 2: Use client
cd /path/to/your/project
../mediagit-core/target/debug/mediagit init
../mediagit-core/target/debug/mediagit remote add origin http://localhost:3000/repos/my-project
../mediagit-core/target/debug/mediagit add your-file.psd
../mediagit-core/target/debug/mediagit commit -m "First commit"
../mediagit-core/target/debug/mediagit push origin main
```

### Next Steps

- **Local development**: Continue to [Local Development Setup](#local-development-setup)
- **Production deployment**: Skip to [Server Setup](#server-setup)
- **Cloud storage**: Check [Backend Configurations](#backend-configurations)

---

## Prerequisites

### System Requirements
- **OS**: Linux, macOS, Windows (native or WSL2)
- **Rust**: 1.92.0+ (required - check with `rustc --version`)
- **CPU**: 2+ cores
- **RAM**: 8GB minimum (for `RUST_TEST_THREADS=2`), 16GB+ recommended (for default parallel tests)
- **Disk**: 10GB+ free space

### Required Tools

```bash
# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env

# Verify installation
rustc --version  # Must be 1.92.0 or higher
cargo --version

# Install build essentials
# Ubuntu/Debian
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libssl-dev

# macOS
xcode-select --install

# Install development tools
cargo install cargo-watch  # Auto-rebuild on file changes
cargo install cargo-nextest  # Fast test runner (optional)
```

### Optional Tools

```bash
# Docker (for MinIO testing)
# https://docs.docker.com/get-docker/

# AWS CLI (for S3 backend)
curl "https://awscli.amazonaws.com/awscli-exe-linux-x86_64.zip" -o "awscliv2.zip"
unzip awscliv2.zip
sudo ./aws/install

# Azure CLI (for Azure Blob backend)
curl -sL https://aka.ms/InstallAzureCLIDeb | sudo bash

# Google Cloud SDK (for GCS backend)
# https://cloud.google.com/sdk/docs/install
```

---

## Local Development Setup

### 1. Clone Repository

```bash
git clone https://github.com/winnyboy5/mediagit-core.git
cd mediagit-core
```

### 2. Build Project

```bash
# Development build (recommended for testing)
cargo build
# Takes 5-10 minutes on first build
# Creates binaries in target/debug/

# Full release build (for production)
cargo build --release
# Takes 10-20 minutes, creates optimized binaries in target/release/
```

**Note**: TLS support is **enabled by default** in MediaGit. To disable TLS:
```bash
cargo build --no-default-features
```

### 3. Verify Build

```bash
# Confirm both binaries exist
ls -lh target/debug/mediagit
ls -lh target/debug/mediagit-server

# Test CLI
./target/debug/mediagit --version
# Should output: mediagit 0.2.7-beta.1

# Test server (optional)
./target/debug/mediagit-server --help
```

### 4. Run Tests

```bash
# Run all tests (memory-constrained: limit threads — see note below)
RUST_TEST_THREADS=2 cargo test --workspace

# Run all tests on high-RAM machines (default: one thread per CPU)
cargo test --workspace

# Run specific test suite
cargo test --test cli_add_test

# Run with output
cargo test --workspace -- --nocapture

# Run only ignored large-file / benchmark tests
cargo test --workspace -- --ignored --nocapture

# Fast testing with nextest (if installed)
cargo nextest run
```

> **Memory note**: `cargo test` defaults to one thread per logical CPU. Each thread spawns
> a full mediagit process with its own 1000-object Moka cache, parallel chunk workers, and
> file buffers. On 8-core machines this can peak at **8–16 GB RAM**. Use
> `RUST_TEST_THREADS=2` or `cargo test -- --test-threads=2` to stay under 4 GB.
> See [Troubleshooting → Tests consuming too much RAM](#tests-consuming-too-much-ram).

#### Low-Memory Build Tips (≤ 8 GB RAM)

If your machine has limited memory, use these settings to prevent OOM during builds and tests:

```bash
# Cap parallel compile/link jobs (default is 1 per CPU core)
export CARGO_BUILD_JOBS=1          # Safest — serialises all link steps

# Cap test parallelism separately
export RUST_TEST_THREADS=1         # Or =2 for a good middle-ground

# Build only the crate you're working on (skip heavy deps you're not touching)
cargo build -p mediagit-cli        # Just the CLI
cargo test  -p mediagit-config     # Just the config crate

# On Windows: the /INCREMENTAL:NO flag in .cargo/config.toml already
# disables incremental linking, which halves peak link-step RAM.
```

**Why these help**: Each parallel link job can consume 1–4 GB of RAM. With
`CARGO_BUILD_JOBS=1`, only one linker runs at a time, keeping peak memory
under 4 GB even for the largest test binary (`e2e_backends_test`).

> **Tip**: If you still see OOM errors, increase your swap/page file to 8 GB.
> On Linux: `sudo fallocate -l 8G /swapfile && sudo mkswap /swapfile && sudo swapon /swapfile`

### 5. Set Up Pre-Commit Hooks

We use **[husky-rs](https://github.com/pplmx/husky-rs)** (pure Rust) for Git hooks that enforce code quality:
- `cargo fmt --check` — formatting
- `cargo clippy` — lint warnings
- License header check — AGPL compliance
- Large file guard — blocks files > 5MB
- Conflict marker check — catches leftover merge markers
- Conventional commit message validation (`feat:`, `fix:`, `docs:`, etc.)

Hooks auto-install when you build:

```bash
cargo build    # hooks are automatically configured
```

> **Tip**: To bypass hooks for a WIP commit: `git commit --no-verify`
> To skip hook installation in CI: `NO_HUSKY_HOOKS=1 cargo build`

### 6. Project Structure

```
mediagit-core/
├── crates/
│   ├── mediagit-cli/          # CLI client
│   ├── mediagit-server/       # HTTP server
│   ├── mediagit-storage/      # Storage backends
│   ├── mediagit-versioning/   # Core versioning logic
│   ├── mediagit-config/       # Configuration management
│   └── ...
├── tests/                     # Integration tests
├── target/                    # Build artifacts
│   └── debug/                 # Debug binaries
│       ├── mediagit           # Client binary
│       └── mediagit-server    # Server binary
└── Cargo.toml                 # Root workspace config
```

---

## Backend Configurations

MediaGit supports multiple storage backends. Choose based on your deployment environment.

### Config File Locations

MediaGit looks for configuration files in this order (highest precedence first):

1. **Environment variables** - Highest precedence
   - Example: `MEDIAGIT_S3_BUCKET=my-bucket`
   - Overrides all config files

2. **`.mediagit/config.toml`** - Repository-specific config
   - Located in your project's `.mediagit/` directory
   - Recommended for project-specific settings

3. **`config.toml`** - Current directory config
   - Located in the directory where you run mediagit commands
   - Useful for workspace-level settings

4. **`~/.config/mediagit/config.toml`** - User-level config
   - Global defaults for all your projects
   - Lowest precedence

**Recommendation**: Use `.mediagit/config.toml` for repository-specific settings (remote URLs, storage backends) and environment variables for sensitive credentials (API keys, passwords).

**Example precedence**:
```bash
# If all three exist, values are merged with this priority:
# 1. MEDIAGIT_S3_BUCKET env var (wins)
# 2. .mediagit/config.toml [storage] bucket  (where backend = "s3")
# 3. config.toml [storage] bucket
# 4. ~/.config/mediagit/config.toml [storage] bucket (lowest)
```

### 1. Local Filesystem Backend (Default)

**Best for**: Local development, testing, small teams

#### Configuration

Create `config.toml` in your working directory:

```toml
[app]
name = "mediagit"
environment = "development"
port = 8080
host = "127.0.0.1"

[storage]
backend = "filesystem"
base_path = "./mediagit-data"
create_dirs = true
sync = false
file_permissions = "0644"

[compression]
enabled = true
algorithm = "zstd"
level = 3
min_size = 1024

[performance]
max_concurrency = 4
buffer_size = 65536
```

#### Usage

```bash
# Initialize repository
./target/debug/mediagit init

# Add files
./target/debug/mediagit add myfile.psd

# Commit
./target/debug/mediagit commit -m "Initial commit"

# Data stored in ./mediagit-data/
ls -la ./mediagit-data/
```

#### Pros & Cons

✅ **Pros**:
- Zero configuration
- No external dependencies
- Fast for local development
- Easy debugging

❌ **Cons**:
- Not distributed
- No built-in redundancy
- Limited scalability

---

### 2. MinIO (S3-Compatible) Backend

**Best for**: Local S3 testing, development, small deployments

#### Prerequisites

```bash
# Using Docker
docker run -d \
  --name mediagit-minio \
  -p 9000:9000 \
  -p 9001:9001 \
  -e MINIO_ROOT_USER=minioadmin \
  -e MINIO_ROOT_PASSWORD=minioadmin \
  -v minio_data:/data \
  minio/minio server /data --console-address ":9001"

# Verify MinIO is running
curl http://localhost:9000/minio/health/live
```

#### Install MinIO Client (mc)

```bash
wget https://dl.min.io/client/mc/release/linux-amd64/mc
chmod +x mc
sudo mv mc /usr/local/bin/

# Configure mc
mc alias set localminio http://localhost:9000 minioadmin minioadmin

# Create bucket
mc mb localminio/mediagit-bucket
```

#### Configuration

Create `config.toml`:

```toml
[app]
name = "mediagit"
environment = "development"
port = 8080
host = "127.0.0.1"

[storage]
backend = "s3"  # MinIO uses S3-compatible API
bucket = "mediagit-bucket"
region = "us-east-1"  # MinIO doesn't enforce regions
access_key_id = "minioadmin"
secret_access_key = "minioadmin"
endpoint = "http://localhost:9000"  # MinIO endpoint
prefix = "media/"
encryption = false

[compression]
enabled = true
algorithm = "zstd"
level = 3
```

#### Alternative: Environment Variables

```bash
# Set credentials via environment (more secure)
export MEDIAGIT_S3_ACCESS_KEY_ID=minioadmin
export MEDIAGIT_S3_SECRET_ACCESS_KEY=minioadmin
export MEDIAGIT_S3_ENDPOINT=http://localhost:9000
export MEDIAGIT_S3_BUCKET=mediagit-bucket
```

Then simplified config:

```toml
[storage]
backend = "s3"
bucket = "mediagit-bucket"
region = "us-east-1"
endpoint = "http://localhost:9000"
# Credentials from environment
```

#### Testing

```bash
# Initialize repo
./target/debug/mediagit init

# Add and commit files
./target/debug/mediagit add test.txt
./target/debug/mediagit commit -m "Test MinIO backend"

# Verify objects in MinIO
mc ls localminio/mediagit-bucket/media/

# Check upload/download performance
mc stat localminio/mediagit-bucket/media/objects/
```

#### MinIO Console

Access MinIO console at http://localhost:9001
- Username: `minioadmin`
- Password: `minioadmin`

---

### 3. AWS S3 Backend

**Best for**: Production deployments, scalability, global distribution

#### Prerequisites

1. **AWS Account**: https://aws.amazon.com/
2. **AWS CLI Installed**: See prerequisites section
3. **IAM Credentials**: Create IAM user with S3 access

#### Create S3 Bucket

```bash
# Configure AWS CLI
aws configure
# Enter: Access Key ID, Secret Access Key, Region (e.g., us-east-1), Output format (json)

# Create bucket
aws s3 mb s3://my-mediagit-bucket --region us-east-1

# Verify bucket
aws s3 ls s3://my-mediagit-bucket
```

#### IAM Policy for MediaGit

Create IAM policy with minimum required permissions:

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Action": [
        "s3:PutObject",
        "s3:GetObject",
        "s3:DeleteObject",
        "s3:ListBucket",
        "s3:HeadBucket",
        "s3:GetBucketLocation"
      ],
      "Resource": [
        "arn:aws:s3:::my-mediagit-bucket",
        "arn:aws:s3:::my-mediagit-bucket/*"
      ]
    }
  ]
}
```

Attach policy to IAM user or role.

#### Configuration

**Option 1: Credentials in Config (Development Only)**

```toml
[storage]
backend = "s3"
bucket = "my-mediagit-bucket"
region = "us-east-1"
access_key_id = "AKIAIOSFODNN7EXAMPLE"
secret_access_key = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
prefix = "media/"
encryption = true
encryption_algorithm = "AES256"
# Do NOT set endpoint for real AWS S3 — omitting it enables native AWS mode
# (correct SigV4 region signing + virtual-hosted-style addressing)
```

**Option 2: Environment Variables (Recommended)**

```bash
# Set AWS credentials
export MEDIAGIT_S3_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE
export MEDIAGIT_S3_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY
export MEDIAGIT_S3_BUCKET=my-mediagit-bucket
export MEDIAGIT_S3_REGION=us-east-1

# Or use AWS CLI credentials (auto-detected)
export AWS_PROFILE=mediagit-production
```

Simplified config:

```toml
[storage]
backend = "s3"
bucket = "my-mediagit-bucket"
region = "us-east-1"
encryption = true
encryption_algorithm = "AES256"
# Credentials from environment or AWS CLI config
```

**Option 3: EC2 Instance Role (Production)**

For EC2 deployments, use IAM instance roles (no credentials needed):

```toml
[storage]
backend = "s3"
bucket = "my-mediagit-bucket"
region = "us-east-1"
encryption = true
# Credentials auto-detected from instance metadata
```

#### Rust Code Integration

MediaGit uses `aws-sdk-rust` internally:

```rust
// Automatically handled by MediaGit storage layer
use aws_sdk_s3 as s3;

#[tokio::main]
async fn main() -> Result<(), s3::Error> {
    // Load config from environment
    let config = aws_config::load_from_env().await;
    let client = aws_sdk_s3::Client::new(&config);

    // MediaGit handles S3 operations internally
    Ok(())
}
```

#### Testing

```bash
# Test connection
aws s3 ls s3://my-mediagit-bucket/

# Initialize MediaGit repo
./target/debug/mediagit init

# Add large file
./target/debug/mediagit add large-video.mp4

# Commit (uploads to S3)
./target/debug/mediagit commit -m "Test S3 backend"

# Verify objects in S3
aws s3 ls s3://my-mediagit-bucket/media/objects/ --recursive
```

#### Encryption Options

```toml
# Server-side encryption (SSE-S3) — add to your [storage] block
[storage]
backend = "s3"
bucket = "my-mediagit-bucket"
region = "us-east-1"
encryption = true
encryption_algorithm = "AES256"   # Options: AES256, aws:kms
```

For KMS-managed keys, set `encryption_algorithm = "aws:kms"`. Key selection and rotation are configured in the AWS Console or via AWS CLI — not in MediaGit config.

#### Cost Optimization

S3 storage class (Intelligent-Tiering, Glacier, etc.) and lifecycle policies are configured via the AWS Console or AWS CLI, not in MediaGit config:

```bash
# Example: set lifecycle policy via AWS CLI
aws s3api put-bucket-lifecycle-configuration \
  --bucket my-mediagit-bucket \
  --lifecycle-configuration file://lifecycle-policy.json
```

---

### 4. Azure Blob Storage Backend

**Best for**: Azure-centric deployments, Microsoft ecosystem integration.

> The Azure backend is selected per **server-side** repository — the
> `[storage]` section in `<server-repos-dir>/<repo>/.mediagit/config.toml`
> picks the backend that `mediagit-server` will use for that repo's objects.
> Clients do not need any Azure credentials; they only talk to
> `mediagit-server` over HTTP.

#### Prerequisites

1. **Azure subscription**: https://azure.microsoft.com/
2. **Azure CLI**: `az login` already authenticated.
3. `mediagit-server` and `mediagit` built with `--release` (the workspace
   already enables `mediagit-storage/all`, which compiles in the Azure
   backend — no extra cargo flags are needed).

#### Provision Resource Group, Storage Account, and Container

```bash
# 1. Resource group
az group create --name mediagit-dev-rg --location eastus

# 2. Storage account (account names: 3-24 lowercase alphanumeric, globally unique)
az storage account create \
  --name mediagitdev$(openssl rand -hex 3) \
  --resource-group mediagit-dev-rg \
  --location eastus \
  --sku Standard_LRS \
  --kind StorageV2

# 3. Container — record the account name you got from step 2
ACCOUNT=mediagitdevXXXXXX   # replace with the actual name from step 2
az storage container create \
  --account-name "$ACCOUNT" \
  --name mediagit-server-repos \
  --auth-mode login

# 4. Account key (kept server-side; never commit this anywhere)
az storage account keys list \
  --account-name "$ACCOUNT" \
  --resource-group mediagit-dev-rg \
  --query "[0].value" -o tsv
```

#### Per-Repository Server Config

For each repo the server hosts, write its `.mediagit/config.toml` so the
`[storage]` block selects Azure. The simplest flow is `mediagit init` on the
server-side repo dir, then overwrite the `[storage]` section.

```toml
# <server-repos-dir>/<repo>/.mediagit/config.toml

[storage]
backend = "azure"
account_name = "mediagitdevXXXXXX"
container = "mediagit-server-repos"
account_key = "<KEY_FROM_STEP_4_ABOVE>"
# Optional: blob path prefix
# prefix = "repo-objects/"
```

Alternative: instead of `account_key`, use a connection string:

```toml
[storage]
backend = "azure"
account_name = "mediagitdevXXXXXX"
container = "mediagit-server-repos"
connection_string = "DefaultEndpointsProtocol=https;AccountName=...;AccountKey=...;EndpointSuffix=core.windows.net"
```

Validation rules (enforced by `mediagit-config`):

- `account_name`, `container` are required.
- `container` must be 3-63 chars (Azure rule).
- Either `account_key` **or** `connection_string` must be set (the current
  code path does not auto-resolve from environment variables — the chosen
  credential lives in the TOML on the server filesystem).

#### Verified Manual Dev Test

A working harness lives at `dev-tests/azure-manual-test/run_azure_dev_test.py`.
It provisions a test repo, starts `mediagit-server` against the Azure
container, pushes a 4 MiB blob from a fresh client, verifies the blobs
landed in Azure, and clones into a second working tree to confirm a
byte-identical roundtrip.

```bash
# All three vars are required by the harness
export MEDIAGIT_AZURE_ACCOUNT=mediagitdevXXXXXX
export MEDIAGIT_AZURE_CONTAINER=mediagit-server-repos
export MEDIAGIT_AZURE_KEY="<account_key>"

python dev-tests/azure-manual-test/run_azure_dev_test.py
```

Expected tail of output on success:

```text
Azure container blobs after push: total=3, chunks/=0, chunk-deltas/=0, bare-oid=3
$ mediagit clone http://127.0.0.1:8770/azuretest .../cloned
✅ Cloned into ...
=== PASS ===
blobs uploaded to Azure: 3
clone roundtrip byte-identical: yes
```

> **Note on object key layout:** for small / single-chunk objects (commits,
> trees, blobs that don't trigger the chunking pipeline) the server stores
> blobs at bare-OID keys (`<oid>`) via `ObjectDatabase::write_with_path`.
> The `chunks/<oid>` and `chunk-deltas/<oid>` prefixes only appear for
> multi-chunk media (e.g. PSDs, videos) that go through the chunked-write
> path. Both layouts are valid and clones reconstruct cleanly.

#### Production: Managed Identity

For Azure VM / App Service deployments you typically don't want long-lived
account keys on disk. The current `mediagit-storage::AzureBackend`
constructors (`with_account_key`, `with_connection_string`,
`with_sas_token`) require explicit credentials, so managed-identity
authentication is **not yet wired** end-to-end through `mediagit-server`.
For now, prefer rotating `account_key` and storing it in a secrets manager
that templates the per-repo `config.toml` at server start.

#### Backend API Reference

The server consumes the Azure backend via `mediagit_storage::AzureBackend`:

```rust
use mediagit_storage::{AzureBackend, StorageBackend};

let backend = AzureBackend::with_account_key(
    "mediagitdevXXXXXX",      // account_name
    "mediagit-server-repos",  // container
    "<account_key>",
).await?;

backend.put("commits/abc123", b"...").await?;
let bytes = backend.get("commits/abc123").await?;
```

The `StorageBackend` trait surface (`get` / `put` / `exists` / `delete` /
`list_objects`) is identical across all backends.

#### Access Tiers

Azure blob access tiers (Hot, Cool, Archive) and lifecycle policies are configured via the Azure Portal or Azure CLI — not in MediaGit config:

```bash
# Set lifecycle management via Azure CLI
az storage account management-policy create \
  --account-name mediagitstorage \
  --resource-group mediagit-rg \
  --policy @lifecycle-policy.json
```

---

### 5. Google Cloud Storage (GCS) Backend

**Best for**: Google Cloud deployments, GCP-centric infrastructure.

> The GCS backend is selected per **server-side** repository — the `[storage]`
> section in `<server-repos-dir>/<repo>/.mediagit/config.toml` picks the backend
> that `mediagit-server` will use for that repo's objects. Clients do not need
> any GCP credentials; they only talk to `mediagit-server` over HTTP.

#### Prerequisites

1. **Google Cloud account**: https://cloud.google.com/
2. **gcloud CLI installed and authenticated** (`gcloud auth login`)
3. `mediagit-server` and `mediagit` built with `--release` (the workspace
   already enables `mediagit-storage/all`, which compiles in the GCS backend
   — no extra cargo flags needed).

#### Provision Project, Bucket, and Service Account

```bash
# 1. Pick / create a project
gcloud config set project YOUR_PROJECT_ID

# 2. Create the bucket (pick a region close to where mediagit-server runs;
#    cross-region WAN bandwidth is the dominant push/pull bottleneck — see
#    the "Observed throughput" note below)
gcloud storage buckets create gs://my-mediagit-bucket \
  --location=us-east1 \
  --default-storage-class=STANDARD

# 3. Create a service account that the server will run as
gcloud iam service-accounts create mediagit-sa \
  --display-name="MediaGit Service Account"

# 4. Grant Storage Object Admin (project-level is fine; bucket-level also works)
gcloud projects add-iam-policy-binding YOUR_PROJECT_ID \
  --member="serviceAccount:mediagit-sa@YOUR_PROJECT_ID.iam.gserviceaccount.com" \
  --role="roles/storage.objectAdmin"
```

#### Authentication: ADC vs Service Account JSON Key

The GCS backend supports **two** auth modes. **Pick ADC (Option A) first** —
many GCP organisations enforce
`constraints/iam.disableServiceAccountKeyCreation`, which blocks the SA-key
flow described in older versions of this guide. The
`mediagit-storage::GcsBackend::with_default_credentials` code path already
handles ADC and is what `mediagit-server` falls back to when
`credentials_path` is empty (`crates/mediagit-server/src/handlers.rs` →
`with_default_credentials` branch).

**Option A — Application Default Credentials (recommended)**

Works on GCE / GKE / Cloud Run via instance metadata, and on a developer
laptop via `gcloud auth application-default login` (one-time interactive
browser sign-in that writes
`%APPDATA%\gcloud\application_default_credentials.json` on Windows or
`~/.config/gcloud/application_default_credentials.json` on Linux/macOS).

```bash
# One-time on the host that runs mediagit-server
gcloud auth application-default login
```

```toml
# <server-repos-dir>/<repo>/.mediagit/config.toml
[storage]
backend = "gcs"
bucket = "my-mediagit-bucket"
project_id = "your-project-id"
# credentials_path intentionally omitted -> server uses ADC
```

**Option B — Service Account JSON key file** (only when key creation is allowed)

```bash
gcloud iam service-accounts keys create ~/mediagit-credentials.json \
  --iam-account=mediagit-sa@YOUR_PROJECT_ID.iam.gserviceaccount.com
chmod 600 ~/mediagit-credentials.json
```

```toml
[storage]
backend = "gcs"
bucket = "my-mediagit-bucket"
project_id = "your-project-id"
credentials_path = "/home/user/mediagit-credentials.json"
```

If you hit `FAILED_PRECONDITION: Key creation is not allowed on this service
account`, your org has the `iam.disableServiceAccountKeyCreation` constraint
enforced. Switch to Option A.

#### Verified Manual Dev Test

A working harness lives at `dev-tests/gcs-manual-test/run_gcs_dev_test.py`.
It provisions a fresh server-side repo, boots `mediagit-server` against the
GCS bucket, pushes a media fixture from a clean client, lists the bucket via
`gcloud storage ls` to confirm the blobs landed, and clones into a second
working tree to verify a byte-identical sha256 roundtrip.

```bash
# Required environment variables
export MEDIAGIT_GCS_BUCKET=my-mediagit-bucket
export MEDIAGIT_GCS_PROJECT=your-project-id

# Optional: override the fixture (default is a 398 MiB video under test-files/)
# export MEDIAGIT_GCS_FIXTURE=/abs/path/to/your-fixture.bin

python dev-tests/gcs-manual-test/run_gcs_dev_test.py
```

Expected tail of output on success:

```text
GCS bucket blobs: total=33, chunks/=30, chunk-deltas/=0, manifests/=1, bare-oid=2

=== PASS ===
blobs uploaded to GCS: 33
clone roundtrip byte-identical: yes (sha256=b36f1d672d768c1a...)
push throughput: 0.66 MiB/s
```

A focused round-trip integration test that exercises the
`upload_resumable` (>5 MiB) code path directly is at
`crates/mediagit-storage/tests/gcs_integration_tests.rs::test_gcs_large_blob_roundtrip`.
Run it with:

```bash
export MEDIAGIT_GCS_BUCKET=my-mediagit-bucket
export MEDIAGIT_GCS_PROJECT=your-project-id
cargo test -p mediagit-storage --release --features gcs \
  --test gcs_integration_tests test_gcs_large_blob_roundtrip -- --ignored --nocapture
```

> **Note on object key layout:** `mediagit-server` writes commits, trees, and
> single-chunk blobs to bare-OID keys via `ObjectDatabase::write_with_path`.
> Multi-chunk media (PSDs, videos) lands at `chunks/<oid>` and possibly
> `chunk-deltas/<oid>` / `chunk-deltas/<oid>.meta`. Manifests sit at
> `manifests/<oid>`. All three layouts are valid and clones reconstruct
> cleanly.

#### SDK migration note (2026-04-29)

The `google-cloud-storage` crate name on crates.io was donated by `@yoshidan`
to Google. Versions ≤ `0.24` are yoshidan's pre-donation legacy releases;
versions ≥ `1.x` are Google's official Rust SDK
(`googleapis/google-cloud-rust`). MediaGit was previously pinned to
`0.24` and is now on `1.11`.

The migration also delivers the upload-correctness guarantee by construction:
the v1 SDK never slices the body across multiple `Multipart` calls, so the
`upload_resumable` truncation bug that affected v0.24 (a single repeated key
silently overwriting itself) cannot recur. Bucket data written by the old
code (≤ 2026-04-28) is still corrupted on disk — re-push any repos whose
chunked blobs (≥ 5 MiB) were uploaded against the legacy backend.

The migration also picks up:

- AIP-194 strict retries (provider-aware idempotency).
- CRC32C verification on every read/write (enabled by default).
- Caller-driven striped reads via the new `StorageBackend::get_with_size_hint`
  trait method (see "Tunable Knobs" below).

#### Known Caveats

- **`prefix` is silently ignored.** `GcsBackend::{put, get, exists, delete,
  list_objects}` does not honour the `[storage] prefix` field, so all blobs
  land at the bucket root. Use a dedicated bucket per repo / per developer
  if you need namespace isolation; do not co-tenant unrelated data inside one
  bucket via `prefix`.
- **CRC32C is computed per byte uploaded and downloaded.** This is a v1.11
  default. Throughput on emulator / LAN paths can be CPU-bound on the hash;
  WAN paths (the common case) are unaffected. There is no per-request knob
  to disable it; if a future emulator regression surfaces, look at the SDK's
  `with_resumable_upload_threshold`-style configuration.

#### Observed Throughput

Push throughput is dominated by WAN bandwidth between the host running
`mediagit-server` and the bucket region. Sample numbers from `ASIA-SOUTH1`:

| Fixture                        | Size   | SDK     | Push     | Throughput |
|--------------------------------|--------|---------|----------|------------|
| 7 MiB synthetic blob (direct)  | 7 MiB  | v0.24   | 25 s     | 0.28 MiB/s |
| 38 MiB FLAC (full client→push) | 38 MiB | v0.24   | 216 s    | 0.17 MiB/s |
| 398 MiB video (full pipeline)  | 398 MiB| v0.24   | 1155 s   | 0.34 MiB/s |
| 38 MiB FLAC (full client→push) | 38 MiB | v1.11   | 70 s     | 0.54 MiB/s |
| 38 MiB FLAC + tuned concurrency| 38 MiB | v1.11¹  | 57 s     | 0.66 MiB/s |
| 40 MiB striped roundtrip       | 40 MiB | v1.11²  | 202 s    | 0.20 MiB/s |
| 7 MiB single-shot roundtrip    | 7 MiB  | v1.11   | 22 s     | 0.32 MiB/s |

¹ With `[performance] pack_workers = 16`, `upload_concurrency = 64`.
² Striped via 8 MiB ranges × 8 concurrent reads (5 stripes); covers the
caller-driven `get_with_size_hint` path, not a typical push.

These are bandwidth-bound, not CPU- or backend-bound. Co-locate
`mediagit-server` with the bucket region for production.

#### Tunable Knobs

The following keys live under `[performance]` in the repo's
`mediagit-config.toml`. All are optional; absent values fall through to the
env override (if set) and then a hard-coded default.

| Key                  | Type           | Env override                  | Default | Effect                                                           |
|----------------------|----------------|-------------------------------|---------|------------------------------------------------------------------|
| `upload_concurrency` | `Option<usize>`| `MEDIAGIT_UPLOAD_CONCURRENCY` | `32`    | Client-side `buffer_unordered` width when pushing chunked blobs. |
| `pack_workers`       | `Option<usize>`| `MEDIAGIT_PACK_WORKERS`       | `8`     | Server-side parallel ODB writes during streaming pack ingest.    |

The original 32-stream default for `upload_concurrency` came from a 2 Mbps
Azure West-EU tuning run. On modern WAN paths to GCS, `pack_workers = 16` and
`upload_concurrency = 64` lift 38 MiB push from 0.54 → 0.66 MiB/s; bench in
your own environment before raising further.

The `StorageBackend::get_with_size_hint` trait method on the GCS backend
issues striped parallel `read_object` ranges when the caller passes a size
hint ≥ 32 MiB (`STRIPED_GET_THRESHOLD`). The default trait impl ignores the
hint and delegates to `get`, so non-GCS backends remain unchanged. The hint
must come from a caller that already knows the size (typically a manifest);
do NOT add a metadata RPC just to populate it — that probe was the
regression that got the prior striped-get attempt reverted.

#### Storage Classes

GCS storage class (STANDARD, NEARLINE, COLDLINE, ARCHIVE) is configured via lifecycle policies in the GCP Console or gcloud CLI — not in MediaGit config.

```bash
# Set lifecycle management
gcloud storage buckets update gs://my-mediagit-bucket \
  --lifecycle-file=lifecycle-config.json
```

Example `lifecycle-config.json`:

```json
{
  "lifecycle": {
    "rule": [
      {
        "action": {"type": "SetStorageClass", "storageClass": "NEARLINE"},
        "condition": {"age": 30}
      },
      {
        "action": {"type": "SetStorageClass", "storageClass": "COLDLINE"},
        "condition": {"age": 90}
      }
    ]
  }
}
```

---

## Server Setup

### Server Configuration

Create `mediagit-server.toml` in server working directory:

```toml
# Basic server configuration
port = 3000
repos_dir = "./repos"
host = "0.0.0.0"  # Listen on all interfaces

# TLS/HTTPS (optional)
enable_tls = false
tls_port = 3443
tls_cert_path = "/path/to/cert.pem"  # If enable_tls = true
tls_key_path = "/path/to/key.pem"    # If enable_tls = true
tls_self_signed = false              # Use for development only

# Authentication (optional)
enable_auth = false
jwt_secret = "your-secure-jwt-secret"  # If enable_auth = true

# Rate limiting (optional)
enable_rate_limiting = false
rate_limit_rps = 10     # Requests per second
rate_limit_burst = 20   # Burst size
```

### Running the Server

```bash
# Development mode (with config)
./target/debug/mediagit-server

# Production mode
./target/release/mediagit-server

# With custom config path
./target/release/mediagit-server --config /etc/mediagit/server.toml

# With environment variables
export MEDIAGIT_PORT=8080
export MEDIAGIT_REPOS_DIR=/var/mediagit/repos
./target/release/mediagit-server
```

### Server as systemd Service

Create `/etc/systemd/system/mediagit-server.service`:

```ini
[Unit]
Description=MediaGit Server
After=network.target

[Service]
Type=simple
User=mediagit
Group=mediagit
WorkingDirectory=/opt/mediagit
ExecStart=/opt/mediagit/mediagit-server
Restart=on-failure
RestartSec=10
Environment="MEDIAGIT_PORT=3000"
Environment="MEDIAGIT_REPOS_DIR=/var/mediagit/repos"

[Install]
WantedBy=multi-user.target
```

Enable and start:

```bash
sudo systemctl daemon-reload
sudo systemctl enable mediagit-server
sudo systemctl start mediagit-server
sudo systemctl status mediagit-server
```

### Server Health Check

```bash
# Check if server is running
curl http://localhost:3000/health

# Expected response: {"status": "healthy"}
```

---

## Client-Server Workflows

**When do you need this?** Only if you want to collaborate with others or backup to a server.

**For local-only use**, skip this section - you already have everything you need from the Quick Start.

### Starting the Server

#### Option A: Local Development Server

```bash
# Terminal 1: Start server
cd mediagit-core
./target/debug/mediagit-server

# Server runs on http://localhost:3000 by default
# Wait for "Server listening on http://127.0.0.1:3000" message
```

#### Option B: Production Server

See [Server Setup](#server-setup) for full production configuration with TLS, authentication, and systemd service setup.

### Using the Client

#### 1. Configure Remote

Create `.mediagit/config.toml` in your project:

```toml
[remote "origin"]
url = "http://localhost:3000/repos/my-project"
# Or for remote server:
# url = "https://mediagit.example.com/repos/my-project"

[user]
name = "Your Name"
email = "you@example.com"

[compression]
enabled = true
algorithm = "zstd"
level = 3
```

#### 2. Push to Server

```bash
# Initialize repository (if not already done)
./target/debug/mediagit init

# Add and commit files
./target/debug/mediagit add *.psd
./target/debug/mediagit commit -m "Initial commit"

# Push requires server running (see "Starting the Server" above)
./target/debug/mediagit push origin main
```

#### 3. Pull from Server

```bash
# Fetch and merge changes from server
./target/debug/mediagit pull origin main
```

#### 4. Clone Existing Repository

```bash
# Clone from server (server must be running)
./target/debug/mediagit clone http://localhost:3000/repos/my-project
cd my-project
```

### Client Authentication

If server has authentication enabled (`enable_auth = true` in server config):

```bash
# Set auth token via environment variable
export MEDIAGIT_AUTH_TOKEN=your-jwt-token

# Or store in config file
echo "auth_token = \"your-jwt-token\"" >> .mediagit/config.toml
```

---

### Branch Lifecycle & Maintenance

**Complete branch lifecycle**: create → push → merge → cleanup → gc

```bash
# 1. Create and push a feature branch
mediagit branch create feature/new-asset
mediagit push -u origin feature/new-asset

# 2. Work on the branch, commit, push
mediagit add *.psd
mediagit commit -m "Add new assets"
mediagit push

# 3. After merge, delete the remote branch
mediagit push origin --delete feature/new-asset

# 4. Clean up local remote-tracking ref (if needed)
mediagit branch delete -r origin/feature/new-asset

# 5. Reclaim storage from orphaned chunks/manifests
mediagit gc
mediagit gc --dry-run --verbose  # Preview what would be cleaned
```

> **Note**: `push --delete` cannot delete the branch that is currently checked out on the remote (HEAD protection). Deleting `main` while it's the active branch will be rejected.

---

## Complete Configuration Reference

This section provides a comprehensive reference for all configuration files used by MediaGit.

### Architecture Overview

```
┌──────────────────────────────────────────────────────────────────────────────┐
│                           MediaGit Architecture                              │
├──────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  ┌─────────────────┐                    ┌───────────────────┐                │
│  │   MediaGit CLI  │                    │  MediaGit Server  │                │
│  │ (mediagit.exe)  │                    │ (mediagit-server) │                │
│  │                 │                    │                   │                │
│  │ LOCAL STORAGE   │     HTTP/HTTPS     │  CONFIGURABLE     │                │
│  │ .mediagit/      │ ─────────────────► │  STORAGE BACKEND  │                │
│  │                 │  Push/Pull/Clone   │                   │                │
│  └─────────────────┘                    └─────────┬─────────┘                │
│                                                   │                          │
│                                                   │ S3 API                   │
│                                                   ▼                          │
│                                         ┌───────────────────┐                │
│                                         │  MinIO / S3 /     │                │
│                                         │  Azure / GCS      │                │
│                                         └───────────────────┘                │
│                                                                              │
└──────────────────────────────────────────────────────────────────────────────┘
```

**Key Points:**
- The **CLI never directly talks to cloud storage** - it always goes through the server
- The **server** is where you configure storage backends (S3, MinIO, Azure, etc.)
- The **CLI** only configures remote server URLs

### Configuration File Summary

| Config File | Location | Purpose |
|-------------|----------|---------|
| Client config | `my-project/.mediagit/config.toml` | Remotes, compression, performance |
| Server config | `./mediagit-server.toml` | Port, auth, rate limiting, TLS |
| Server repo config | `repos/<repo>/.mediagit/config.toml` | Storage backend (S3, MinIO, etc.) |

---

### Client Config (`.mediagit/config.toml`)

Located in: **your working repository** (e.g., `my-project/.mediagit/config.toml`)

```toml
# ============================================
# REMOTES - Where to push/pull from (REQUIRED for push/pull)
# ============================================
[remotes.origin]
url = "http://localhost:3000/my-repo"     # MediaGit server URL

[remotes.backup]                          # Optional: multiple remotes
url = "http://backup-server:3000/my-repo"

# ============================================
# OPTIONAL OVERRIDES (usually not needed)
# ============================================
# Compression is automatic - only override if needed:
# [compression]
# algorithm = "zstd"    # zstd (default), brotli, or none
# level = 3             # 1-22 for zstd

# Performance tuning (defaults work well):
# [performance]
# max_concurrency = 8
```

> **Note**: Compression is **automatic** - MediaGit detects file types and applies optimal compression. Pre-compressed files (JPEG, PNG, MP4, etc.) are stored as-is.

---

### Server Config (`mediagit-server.toml`)

Located in: **same directory where you run the server**

```toml
# ============================================
# SERVER SETTINGS
# ============================================
port = 3000                   # HTTP port
host = "127.0.0.1"            # Bind address (use 0.0.0.0 for all interfaces)
repos_dir = "./repos"         # Where server repos are stored

# ============================================
# AUTHENTICATION (optional)
# ============================================
enable_auth = false           # Set true for production
jwt_secret = "your-secret"    # Required if enable_auth = true

# ============================================
# RATE LIMITING (optional)
# ============================================
enable_rate_limiting = false
rate_limit_rps = 10           # Requests per second
rate_limit_burst = 20         # Burst size

# ============================================
# TLS/HTTPS (optional)
# ============================================
enable_tls = false
tls_port = 3443
tls_cert_path = "/path/to/cert.pem"
tls_key_path = "/path/to/key.pem"
tls_self_signed = false       # Use self-signed for dev
```

---

### Server Repository Config (`repos/<repo-name>/.mediagit/config.toml`)

Located in: **each repository on the server**

This is where you configure the **storage backend** (S3, MinIO, Azure, etc.).

#### Option 1: Local Filesystem (Default)

```toml
[storage]
backend = "filesystem"
base_path = "./data"
```

#### Option 2: MinIO / S3-Compatible

```toml
[storage]
backend = "s3"
bucket = "mediagit-test"
region = "us-east-1"
endpoint = "http://localhost:9000"    # MinIO endpoint
access_key_id = "minioadmin"
secret_access_key = "minioadmin"
# prefix = "objects/"                 # Optional path prefix
# encryption = true                   # Optional SSE
```

#### Option 3: AWS S3

```toml
[storage]
backend = "s3"
bucket = "my-mediagit-bucket"
region = "us-west-2"
# access_key_id = "..."              # Or use AWS env vars/IAM role
# encryption = true
# encryption_algorithm = "AES256"
```

#### Option 4: Azure Blob Storage

```toml
[storage]
backend = "azure"
account_name = "mystorageaccount"
container = "mediagit"
account_key = "..."
# connection_string = "..."          # Alternative to account_key
```

#### Option 5: Google Cloud Storage

```toml
[storage]
backend = "gcs"
bucket = "my-mediagit-bucket"
project_id = "my-project"
# credentials_path = "/path/to/credentials.json"
```

---

### Data Flow Example: Push with MinIO

```
1. CLI: mediagit push origin main
   │
   ▼
2. CLI reads local objects from .mediagit/objects/
   │
   ▼
3. CLI packs objects and sends HTTP POST to server
   │   POST http://localhost:3000/repo-name/objects/pack
   ▼
4. Server receives pack, unpacks objects
   │
   ▼
5. Server reads its config → sees backend="s3"
   │
   ▼
6. Server writes objects to MinIO via S3 API
   └──► PUT http://localhost:9000/mediagit-test/objects/abc123...
```

---

## Testing Your Setup

### Test Suite

```bash
# Run all integration tests (limit threads to avoid OOM — see Troubleshooting)
RUST_TEST_THREADS=2 cargo test --workspace

# Run specific backend test
cargo test test_s3_backend
cargo test test_azure_backend
cargo test test_gcs_backend

# Run with logging
RUST_LOG=debug cargo test --workspace -- --nocapture
```

### Manual Testing

```bash
# Test local backend
./tests/psd_layer_preservation_test.sh

# Test MinIO backend
./tests/minio_cloud_backend_test.sh

# Test extreme scale (6GB file)
./tests/extreme_scale_test.sh
```

### Performance Benchmarking

```bash
# Run benchmarks
cargo bench

# Storage backend benchmarks
cargo bench --bench storage_benchmarks

# Versioning benchmarks
cargo bench --bench odb_bench
```

---

## Troubleshooting

### Common Setup Issues

#### "Binary not found" error

**Problem**: `./target/debug/mediagit: No such file or directory`

**Solution**: Build the project first
```bash
cargo build
ls -lh target/debug/mediagit  # Verify it exists
```

**Why this happens**: You're trying to run binaries before building them.

#### "Rust version too old" error

**Problem**: Build fails with compiler errors or feature compatibility issues

**Solution**: Update Rust to 1.92.0+
```bash
rustup update
rustc --version  # Must show 1.92.0 or higher
```

**Why this happens**: MediaGit uses features from Rust 1.92.0+ that aren't in older versions.

#### Tests consuming too much RAM

**Problem**: `cargo test --workspace` triggers OOM, swap usage spikes, or system becomes
unresponsive during the test run.

**Root causes** (identified 2026-02-27):

| Cause | Impact |
|-------|--------|
| Default thread count = CPU count | Multiplies all other costs |
| Moka cache: 1000-object count limit (not byte-bounded) | Up to GB per ODB on large chunks |
| 3-layer nested parallelism (test × files × chunk workers) | Hundreds of concurrent async tasks |
| `write_chunked_parallel` reads entire file into RAM | 50–500 MB per concurrent test |

**Quick fix — limit test threads**:
```bash
# Option 1: environment variable (persists for the shell session)
RUST_TEST_THREADS=2 cargo test --workspace

# Option 2: explicit flag
cargo test --workspace -- --test-threads=2

# Option 3: nextest (better memory isolation, parallel-safe)
cargo nextest run --workspace --test-threads 2
```

**Memory budget by thread count** (approximate, 8-core machine):

| `--test-threads` | Peak RAM | Suitable for |
|-----------------|----------|--------------|
| 1 | ~1–2 GB | CI, 4 GB machines |
| 2 | ~2–4 GB | 8 GB machines (recommended) |
| 4 | ~4–8 GB | 16 GB machines |
| default (8+) | ~8–16 GB | 32 GB+ machines |

**Skip large/ignored tests** (they are all `#[ignore]` and excluded by default):
```bash
cargo test --workspace  # already excludes #[ignore] tests
# To run them explicitly (high memory):
cargo test --workspace -- --ignored
```

**Why this happens**: Each test thread spawns a separate `mediagit` subprocess containing:
- A `ObjectDatabase` with Moka cache (1000 objects by count — not bounded by bytes)
- `num_cpus::get().clamp(2, 16)` async chunk workers per large file
- `num_cpus::get().min(8)` concurrent file tasks during `add`
- The entire file content loaded into RAM before chunking for files under 100 MB

#### Git hooks not executing

**Problem**: `git commit` (or `git push`) fails with:
```
fatal: cannot exec '.husky/pre-commit': No such file or directory
```

This failure has two independent causes that can occur on any platform (Windows, WSL2,
Linux, macOS, and CI runners that unzip archives instead of using `git clone`).

**Cause 1 — CRLF line endings**

When `core.autocrlf` is enabled (the Windows git default) or the repo is unzipped on
Windows, git converts `\n` → `\r\n` in text files. The hook shebang becomes
`#!/bin/sh\r`, and the kernel cannot find an interpreter named `sh` + carriage-return.

Diagnose:
```bash
file .husky/pre-commit   # reports "with CRLF line terminators" if affected
```

Fix:
```bash
sed -i 's/\r//' .husky/pre-commit .husky/pre-push .husky/commit-msg
```

**Cause 2 — Missing execute bit**

On filesystems that do not store Unix permissions (NTFS, FAT32, some SMB shares,
CI artifact extracts), the `+x` bit may be absent after cloning or extraction.

Diagnose:
```bash
ls -la .husky/   # pre-commit should show -rwxr-xr-x, not -rw-r--r--
```

Fix:
```bash
chmod +x .husky/pre-commit .husky/pre-push .husky/commit-msg
git update-index --chmod=+x .husky/pre-commit .husky/pre-push .husky/commit-msg
```

`git update-index --chmod=+x` records mode `100755` in the index so the bit is
preserved for future `git checkout` operations.

**Combined one-shot fix** (safe to run on any platform after a fresh clone):
```bash
sed -i 's/\r//' .husky/pre-commit .husky/pre-push .husky/commit-msg
chmod +x        .husky/pre-commit .husky/pre-push .husky/commit-msg
git update-index --chmod=+x .husky/pre-commit .husky/pre-push .husky/commit-msg
```

The `.gitattributes` file in the repo root enforces `eol=lf` for `.husky/*`, which
prevents CRLF recurrence on subsequent checkouts once applied.

#### "Server not responding" error

**Problem**: `mediagit push` fails with connection refused or timeout

**Solution**: Start mediagit-server first
```bash
# Terminal 1: Start server
./target/debug/mediagit-server

# Terminal 2: Wait for "Server listening on..." message, then:
./target/debug/mediagit push origin main
```

**Why this happens**: Push/pull operations require a running server. Local operations (init, add, commit, status) don't.

**Quick check**: `curl http://localhost:3000/health` should return `{"status":"healthy"}`

### Cloud Backend Issues

#### 2. MinIO Connection Failed

```bash
# Check MinIO status
docker ps | grep minio

# Check MinIO health
curl http://localhost:9000/minio/health/live

# Restart MinIO
docker restart mediagit-minio

# Check logs
docker logs mediagit-minio
```

#### 3. AWS S3 Access Denied

```bash
# Verify credentials
aws sts get-caller-identity

# Test bucket access
aws s3 ls s3://my-mediagit-bucket/

# Check IAM policy
aws iam get-user-policy --user-name mediagit-user --policy-name MediaGitS3Policy
```

#### 4. Azure Blob Authentication Failed

```bash
# Verify login
az account show

# Test storage account access
az storage account show --name mediagitstorage

# Regenerate access key if needed
az storage account keys renew \
  --account-name mediagitstorage \
  --resource-group mediagit-rg \
  --key primary
```

#### 5. GCS Permission Denied

```bash
# Verify authentication
gcloud auth list

# Check service account permissions
gcloud projects get-iam-policy YOUR_PROJECT_ID \
  --flatten="bindings[].members" \
  --filter="bindings.members:serviceAccount:mediagit-sa@*"

# Test bucket access
gcloud storage ls gs://my-mediagit-bucket/
```

#### 6. Slow Performance

```bash
# Check compression settings
# Reduce compression level for faster performance
[compression]
level = 1  # Lower = faster, larger files

# Increase concurrency
[performance]
max_concurrency = 8  # Match CPU cores

# Increase buffer size
buffer_size = 131072  # 128KB
```

#### 7. Out of Memory

```bash
# Reduce buffer size
[performance]
buffer_size = 32768  # 32KB

# Limit concurrent operations
max_concurrency = 2
```

### Debug Logging

```bash
# Enable debug logging
export RUST_LOG=debug
./target/debug/mediagit add large-file.mp4

# Enable trace logging (very verbose)
export RUST_LOG=trace
./target/debug/mediagit commit -m "Debug commit"

# Filter by module
export RUST_LOG=mediagit_storage=debug,mediagit_versioning=info
```

### Getting Help

```bash
# CLI help
./target/debug/mediagit --help
./target/debug/mediagit add --help
./target/debug/mediagit commit --help

# Server help
./target/debug/mediagit-server --help
```

---

## Performance Tuning

### Compression Settings

```toml
# Fast compression (good for development)
[compression]
algorithm = "zstd"
level = 1
min_size = 4096

# Balanced (recommended for production)
[compression]
algorithm = "zstd"
level = 3
min_size = 1024

# Maximum compression (slow, best ratio)
[compression]
algorithm = "brotli"
level = 11
min_size = 512
```

### Concurrency Tuning

```toml
# Match CPU cores
[performance]
max_concurrency = 4  # 4-core CPU

# For I/O-bound workloads (cloud storage)
max_concurrency = 8  # 2x CPU cores

# For CPU-bound workloads (compression)
max_concurrency = 4  # = CPU cores
```

### Buffer Size Optimization

```toml
# Small files, low memory
[performance]
buffer_size = 32768  # 32KB

# Balanced (default)
buffer_size = 65536  # 64KB

# Large files, high memory
buffer_size = 262144  # 256KB
```

### Caching Configuration

```toml
# In-memory cache (fastest)
[performance.cache]
enabled = true
cache_type = "memory"
max_size = 536870912  # 512MB
ttl = 3600  # 1 hour

# Disable cache (lowest memory)
[performance.cache]
enabled = false
```

### Network Timeouts

```toml
# Slow networks
[performance.timeouts]
connection = 60  # seconds
read = 60
write = 60

# Fast networks (default)
[performance.timeouts]
connection = 30
read = 30
write = 30
```

### Environment Variable Knobs (v0.2.7+)

Fine-grained runtime tuning without rebuilding. All knobs are read at startup; restart the client or server to pick up changes.

| Variable | Default | Tunes |
|----------|---------|-------|
| `MEDIAGIT_CONCURRENT_UPLOADS` | `32` | Total upload semaphore slots per push |
| `MEDIAGIT_PUSH_OBJECT_CONCURRENCY` | `8` | Objects uploaded concurrently (B2 pipeline) |
| `MEDIAGIT_PUSH_CHUNK_CONCURRENCY` | `(64/obj_conc).max(4)` | Per-object chunk concurrency; targets 64 total in-flight PUTs |
| `MEDIAGIT_DOWNLOAD_CONCURRENCY` | `32` | Total chunk downloads during pull/clone |
| `MEDIAGIT_FETCH_BRANCH_CONCURRENCY` | `4` | Branches fetched concurrently in `fetch --all` |
| `MEDIAGIT_FETCH_DOWNLOAD_CONCURRENCY` | `max(32/br_conc,8)` | Per-branch download cap (prevents TCP pool exhaustion) |
| `MEDIAGIT_GCS_UPLOAD_CONCURRENCY` | `4` | GCS concurrent `write_object` calls (lower = fewer 500s) |
| `MEDIAGIT_HTTP_POOL_MAX` | `64` | Max idle TCP connections per host |
| `MEDIAGIT_RANGE_PARALLEL` | `4` | Parallel byte-range GETs per chunk ≥ 64 MiB |
| `MEDIAGIT_RANGE_PARALLEL_THRESHOLD` | `67108864` | Chunk size (bytes) triggering range-parallel GET |
| `MEDIAGIT_STAGED_UPLOAD` | `0` | Set to `1` to enable S3/MinIO multipart upload (MPU) |
| `MEDIAGIT_MPU_THRESHOLD_BYTES` | `16777216` | Min chunk size for MPU path (16 MiB) |
| `MEDIAGIT_MPU_PART_SIZE` | adaptive | Override MPU part size (bytes) |
| `MEDIAGIT_BENCH` | `0` | Set to `1` to emit `[bench]` throughput summary after push/pull |
| `MEDIAGIT_HASH_PARALLEL` | `0` | Set to `1` to enable BLAKE3 tree-parallel hashing (~2.6× faster) |
| `MEDIAGIT_PUSH_PIPELINE` | `1` | B2 parallel push pipeline (default ON) |
| `MEDIAGIT_STREAM_CHUNK_TO_DISK` | `1` | B4 stream-to-disk during clone (default ON; prevents heap spike) |
| `MEDIAGIT_STORAGE_STREAMING` | `1` | B7 backend streaming GET (default ON; 15.8% faster AWS clone) |

**Tip:** For WAN pushes that stall at 0 B/s, reduce `MEDIAGIT_PUSH_OBJECT_CONCURRENCY` to 4 (→ 16 in-flight PUTs total) or set `MEDIAGIT_PUSH_CHUNK_CONCURRENCY` explicitly. Run with `MEDIAGIT_BENCH=1` to measure before/after.

### Cloud Backend Optimization

#### AWS S3

For S3 transfer acceleration, set the accelerated endpoint directly in the MediaGit config:

```toml
[storage]
backend = "s3"
bucket = "my-mediagit-bucket"
region = "us-east-1"
endpoint = "https://my-mediagit-bucket.s3-accelerate.amazonaws.com"
```

Multipart upload thresholds and chunk sizes are handled automatically by the AWS SDK. No MediaGit config is needed.

#### Azure Blob

Azure performance tiers (Premium, Standard) are configured when creating the storage account in the Azure Portal. MediaGit uses whatever tier the account is configured with. Concurrent upload streams are controlled by the `[performance] max_concurrency` setting:

```toml
[performance]
max_concurrency = 4
```

#### GCS

The v1.11 SDK uploads bodies in a single resumable session — there is no
automatic parallel-composite split. Throughput is controlled by the two
tunables documented in the GCS Backend section:

```toml
[performance]
upload_concurrency = 64   # client-side buffer_unordered width for chunk pushes
pack_workers       = 16   # server-side parallel ODB writes during pack ingest
```

For large reads (≥ 32 MiB) that originate from a caller holding a size hint
(typically a chunk manifest), the GCS backend automatically stripes the
download across 8 MiB ranges with up to 8 concurrent requests. No config
knob — see `STRIPED_GET_THRESHOLD` / `STRIPE_SIZE` / `STRIPE_CONCURRENCY` in
`crates/mediagit-storage/src/gcs.rs` to retune.

---

## Production Deployment Checklist

### Security

- [ ] Enable HTTPS/TLS on server
- [ ] Use environment variables for credentials (never commit secrets)
- [ ] Enable authentication and rate limiting
- [ ] Configure firewall rules
- [ ] Enable encryption at rest (S3/Azure/GCS)
- [ ] Rotate access keys regularly
- [ ] Use IAM roles/managed identities when possible

### Performance

- [ ] Tune compression settings for your workload
- [ ] Configure appropriate buffer sizes
- [ ] Set concurrency based on server resources
- [ ] Enable caching if memory allows
- [ ] Configure lifecycle policies for cold storage

### Monitoring

- [ ] Enable metrics collection (port 9090)
- [ ] Set up log aggregation
- [ ] Configure alerts for errors and performance
- [ ] Monitor storage costs
- [ ] Track throughput and latency

### Backup & Recovery

- [ ] Enable versioning on cloud storage
- [ ] Configure cross-region replication (if needed)
- [ ] Test restore procedures
- [ ] Document recovery processes

---

## Quick Reference

### Backend Selection Matrix

| Backend | Use Case | Setup Complexity | Cost | Scalability |
|---------|----------|------------------|------|-------------|
| **Filesystem** | Local dev, testing | ⭐ Easy | 💰 Free | ⬆️ Low |
| **MinIO** | Dev, testing, small teams | ⭐⭐ Moderate | 💰 Low | ⬆️⬆️ Medium |
| **AWS S3** | Production, enterprise | ⭐⭐⭐ Complex | 💰💰 Medium | ⬆️⬆️⬆️ High |
| **Azure Blob** | Azure-centric | ⭐⭐⭐ Complex | 💰💰 Medium | ⬆️⬆️⬆️ High |
| **GCS** | GCP-centric | ⭐⭐⭐ Complex | 💰💰 Medium | ⬆️⬆️⬆️ High |

### Performance Targets

| Operation | Target | Acceptable |
|-----------|--------|------------|
| Small files (<1MB) | >10 MB/s | >5 MB/s |
| Large files (>100MB) | >15 MB/s | >10 MB/s |
| PSD files | >20 MB/s | >15 MB/s |
| Cloud upload (MinIO) | >100 MB/s | >50 MB/s |
| Cloud download (MinIO) | >200 MB/s | >100 MB/s |

### Compression Ratios

| File Type | Expected Compression |
|-----------|---------------------|
| PNG | 0-5% (already compressed) |
| PSD | 30-40% |
| Text/CSV | 85-95% |
| Video (MP4) | 0% (already compressed) |

---

## Additional Resources

- **Official Docs**: `book/src/` (mdBook documentation)
- **Examples**: `crates/mediagit-config/examples/`
- **Tests**: `tests/` directory
- **Docker Configs**: `docker-compose*.yml` files in project root
- **Test Archives**: `test-archives/` (validation artifacts with 30-day retention)
- **Cleanup Guide**: `CLEANUP_SUMMARY.md` (project organization and archival)
- **Issues**: https://github.com/winnyboy5/mediagit-core/issues

## Project Maintenance

### Test Archives

Test artifacts from validation sessions are archived in `test-archives/` with dated folders:
- `2025-12-27-option-b-validation/` - Option B validation tests (7.3GB)
- Each archive includes README.md with test results and metadata
- Retention: 30 days from creation date

### Workspace Management

Active test workspaces are kept in `tests/`:
- `smoke_test_workspace/` - Quick smoke tests
- `perf_workspace/` - Performance benchmarks
- `media_merge_workspace/` - Media merge tests
- `test_workspace/` - Comprehensive test suite
- `test_workspace_fix/` - Workspace utilities

These are actively used by test scripts and should not be deleted.

### Cleanup Commands

```bash
# Remove old archives after retention period
rm -rf test-archives/YYYY-MM-DD-*/

# Clean build artifacts
cargo clean

# Remove test workspaces (regenerated by tests)
rm -rf tests/*_workspace/*/.mediagit/

# Find temporary files
find . -name "*.tmp" -o -name "*.log" -o -name "*~"
```

---

**Version**: 0.2.7-beta.1
**Last Updated**: May 25, 2026
**Maintained by**: MediaGit Core Team

# MediaGit-Core Development Guide
**Version**: 0.1.0
**Last Updated**: December 30, 2025

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

**Contains**: Commits, branches, refs, Git-compatible metadata

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
rustc --version  # Must show 1.91.0 or higher
```

### Step 2: Clone and Build

```bash
git clone https://github.com/yourusername/mediagit-core.git
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
- **OS**: Linux, macOS, or WSL2 (Windows)
- **Rust**: 1.91.0+ (required - check with `rustc --version`)
- **CPU**: 2+ cores
- **RAM**: 4GB minimum, 8GB+ recommended
- **Disk**: 10GB+ free space

### Required Tools

```bash
# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env

# Verify installation
rustc --version  # Must be 1.91.0 or higher
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
git clone https://github.com/yourusername/mediagit-core.git
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
# Should output: mediagit 0.1.0

# Test server (optional)
./target/debug/mediagit-server --help
```

### 4. Run Tests

```bash
# Run all tests
cargo test

# Run specific test suite
cargo test --test medieval_village_test

# Run with output
cargo test -- --nocapture

# Fast testing with nextest (if installed)
cargo nextest run
```

### 5. Project Structure

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
# 2. .mediagit/config.toml [storage.s3] bucket
# 3. config.toml [storage.s3] bucket
# 4. ~/.config/mediagit/config.toml [storage.s3] bucket (lowest)
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

[storage.filesystem]
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
endpoint = "https://s3.amazonaws.com"
prefix = "media/"
encryption = true
encryption_algorithm = "AES256"
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
# Server-side encryption (SSE-S3)
[storage.s3]
encryption = true
encryption_algorithm = "AES256"

# Server-side encryption with KMS
[storage.s3]
encryption = true
encryption_algorithm = "aws:kms"
kms_key_id = "arn:aws:kms:us-east-1:123456789012:key/12345678-1234-1234-1234-123456789012"
```

#### Cost Optimization

```toml
# Use S3 Intelligent-Tiering storage class
[storage.s3]
storage_class = "INTELLIGENT_TIERING"

# Or use lifecycle policies
# Configure in AWS Console or via AWS CLI
```

```bash
# Example lifecycle policy
aws s3api put-bucket-lifecycle-configuration \
  --bucket my-mediagit-bucket \
  --lifecycle-configuration file://lifecycle-policy.json
```

---

### 4. Azure Blob Storage Backend

**Best for**: Azure-centric deployments, Microsoft ecosystem integration

#### Prerequisites

1. **Azure Account**: https://azure.microsoft.com/
2. **Azure CLI Installed**: See prerequisites section
3. **Storage Account**: Create in Azure Portal

#### Create Storage Account and Container

```bash
# Login to Azure
az login

# Create resource group
az group create --name mediagit-rg --location eastus

# Create storage account
az storage account create \
  --name mediagitstorage \
  --resource-group mediagit-rg \
  --location eastus \
  --sku Standard_LRS

# Get connection string
az storage account show-connection-string \
  --name mediagitstorage \
  --resource-group mediagit-rg \
  --output tsv

# Create container
az storage container create \
  --name mediagit-container \
  --account-name mediagitstorage
```

#### Configuration

**Option 1: Connection String (Development)**

```toml
[storage]
backend = "azure"

[storage.azure]
account_name = "mediagitstorage"
container = "mediagit-container"
connection_string = "DefaultEndpointsProtocol=https;AccountName=mediagitstorage;AccountKey=<YOUR_KEY>;EndpointSuffix=core.windows.net"
prefix = "files/"
```

**Option 2: Account Key (Recommended)**

```bash
# Get account key
az storage account keys list \
  --account-name mediagitstorage \
  --resource-group mediagit-rg \
  --output table

# Set environment variable
export MEDIAGIT_AZURE_ACCOUNT_KEY=<account_key>
```

```toml
[storage]
backend = "azure"

[storage.azure]
account_name = "mediagitstorage"
container = "mediagit-container"
# account_key from environment
prefix = "files/"
```

**Option 3: Managed Identity (Production)**

For Azure VM/App Service deployments:

```toml
[storage]
backend = "azure"

[storage.azure]
account_name = "mediagitstorage"
container = "mediagit-container"
use_managed_identity = true
# Credentials auto-detected from Azure AD
```

#### Rust Code Integration

MediaGit uses `azure-sdk-for-rust`:

```rust
// Automatically handled by MediaGit storage layer
use azure_storage_blob::{BlobClient, BlobClientOptions};
use azure_identity::DeveloperToolsCredential;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let credential = DeveloperToolsCredential::new(None)?;
    let blob_client = BlobClient::new(
        "https://mediagitstorage.blob.core.windows.net/",
        "mediagit-container".to_string(),
        "blob_name".to_string(),
        credential,
        Some(BlobClientOptions::default()),
    )?;

    // MediaGit handles blob operations
    Ok(())
}
```

#### Testing

```bash
# Test connection
az storage blob list \
  --account-name mediagitstorage \
  --container-name mediagit-container

# Initialize MediaGit repo
./target/debug/mediagit init

# Add and commit
./target/debug/mediagit add test-file.psd
./target/debug/mediagit commit -m "Test Azure backend"

# Verify blobs
az storage blob list \
  --account-name mediagitstorage \
  --container-name mediagit-container \
  --prefix files/
```

#### Access Tiers

```toml
# Configure blob access tier for cost optimization
[storage.azure]
access_tier = "Hot"  # Options: Hot, Cool, Archive
```

```bash
# Set lifecycle management
az storage account management-policy create \
  --account-name mediagitstorage \
  --resource-group mediagit-rg \
  --policy @lifecycle-policy.json
```

---

### 5. Google Cloud Storage (GCS) Backend

**Best for**: Google Cloud deployments, GCP-centric infrastructure

#### Prerequisites

1. **Google Cloud Account**: https://cloud.google.com/
2. **gcloud CLI Installed**: See prerequisites section
3. **GCS Bucket**: Create in GCP Console

#### Create GCS Bucket

```bash
# Login to GCP
gcloud auth login
gcloud config set project YOUR_PROJECT_ID

# Create bucket
gcloud storage buckets create gs://my-mediagit-bucket \
  --location=us-east1 \
  --default-storage-class=STANDARD

# Verify bucket
gcloud storage ls gs://my-mediagit-bucket/
```

#### Service Account Setup

```bash
# Create service account
gcloud iam service-accounts create mediagit-sa \
  --display-name="MediaGit Service Account"

# Grant Storage Object Admin role
gcloud projects add-iam-policy-binding YOUR_PROJECT_ID \
  --member="serviceAccount:mediagit-sa@YOUR_PROJECT_ID.iam.gserviceaccount.com" \
  --role="roles/storage.objectAdmin"

# Create and download key
gcloud iam service-accounts keys create ~/mediagit-credentials.json \
  --iam-account=mediagit-sa@YOUR_PROJECT_ID.iam.gserviceaccount.com

# Set permissions
chmod 600 ~/mediagit-credentials.json
```

#### Configuration

**Option 1: Service Account Key File**

```toml
[storage]
backend = "gcs"

[storage.gcs]
bucket = "my-mediagit-bucket"
project_id = "your-project-id"
credentials_path = "/home/user/mediagit-credentials.json"
prefix = "media/"
```

**Option 2: Environment Variable (Recommended)**

```bash
export MEDIAGIT_GCS_CREDENTIALS_PATH=/home/user/mediagit-credentials.json
export MEDIAGIT_GCS_PROJECT_ID=your-project-id
export MEDIAGIT_GCS_BUCKET=my-mediagit-bucket
```

```toml
[storage]
backend = "gcs"

[storage.gcs]
bucket = "my-mediagit-bucket"
project_id = "your-project-id"
# credentials_path from environment
```

**Option 3: Application Default Credentials (Production)**

For GCE/GKE deployments:

```bash
# On GCE/GKE, credentials auto-detected from instance metadata
gcloud auth application-default login  # For local testing
```

```toml
[storage]
backend = "gcs"

[storage.gcs]
bucket = "my-mediagit-bucket"
project_id = "your-project-id"
# Credentials auto-detected from environment
```

#### Testing

```bash
# Test connection
gcloud storage ls gs://my-mediagit-bucket/

# Initialize MediaGit repo
./target/debug/mediagit init

# Add and commit
./target/debug/mediagit add large-image.tif
./target/debug/mediagit commit -m "Test GCS backend"

# Verify objects
gcloud storage ls gs://my-mediagit-bucket/media/ --recursive
```

#### Storage Classes

```toml
# Configure storage class for cost optimization
[storage.gcs]
storage_class = "STANDARD"  # Options: STANDARD, NEARLINE, COLDLINE, ARCHIVE
```

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
# Run all integration tests
cargo test

# Run specific backend test
cargo test test_s3_backend
cargo test test_azure_backend
cargo test test_gcs_backend

# Run with logging
RUST_LOG=debug cargo test -- --nocapture
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

**Solution**: Update Rust to 1.91.0+
```bash
rustup update
rustc --version  # Must show 1.91.0 or higher
```

**Why this happens**: MediaGit uses features from Rust 1.91.0+ that aren't in older versions.

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

### Cloud Backend Optimization

#### AWS S3

```toml
[storage.s3]
# Use transfer acceleration for global uploads
endpoint = "https://my-mediagit-bucket.s3-accelerate.amazonaws.com"

# Use multipart upload for large files (handled automatically)
multipart_threshold = 8388608  # 8MB
multipart_chunk_size = 5242880  # 5MB
```

#### Azure Blob

```toml
[storage.azure]
# Use premium block blobs for best performance
performance_tier = "Premium"

# Concurrent upload streams
max_concurrency = 4
```

#### GCS

```toml
[storage.gcs]
# Use parallel composite uploads
parallel_composite_upload_threshold = 8388608  # 8MB
```

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
- **Docker Configs**: `docker/` directory (see `docker/README.md`)
- **Test Archives**: `test-archives/` (validation artifacts with 30-day retention)
- **Cleanup Guide**: `CLEANUP_SUMMARY.md` (project organization and archival)
- **Issues**: https://github.com/yourusername/mediagit-core/issues

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

**Version**: 0.1.0
**Last Updated**: December 27, 2025
**Maintained by**: MediaGit Core Team

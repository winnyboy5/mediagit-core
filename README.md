# MediaGit Core 🎬

> Git for Media Files - High-performance version control for large binary assets

[![CI](https://github.com/yourusername/mediagit-core/workflows/CI/badge.svg)](https://github.com/yourusername/mediagit-core/actions)
[![License: AGPL-3.0](https://img.shields.io/badge/License-AGPL%203.0-blue.svg)](https://www.gnu.org/licenses/agpl-3.0)
[![Rust Version](https://img.shields.io/badge/rust-1.91.0+-orange.svg)](https://www.rust-lang.org)

## Overview

MediaGit is a next-generation version control system optimized for media files (images, videos, audio, 3D models). Built in Rust for maximum performance and reliability.

### Key Features

- 🚀 **High Performance**: Content-addressable storage with intelligent caching
- 🗜️ **Smart Compression**: Zstd, Brotli, and XDelta3 for optimal space efficiency
- 🎨 **Media-Aware Merging**: Intelligent conflict resolution for PSD layers, video timelines, audio tracks
- ☁️ **Multi-Cloud Support**: AWS S3, Azure Blob, GCS, MinIO, B2, DigitalOcean Spaces
- 🔒 **Security**: AES-256-GCM encryption at rest with Argon2 key derivation
- 🔧 **Git Compatible**: Works with existing Git workflows via filter drivers

## Architecture

MediaGit is organized as a Cargo workspace with specialized crates:

```
mediagit-core/
├── crates/
│   ├── mediagit-cli/          # Command-line interface
│   ├── mediagit-storage/      # Storage abstraction layer
│   ├── mediagit-versioning/   # Object database & version control
│   ├── mediagit-compression/  # Intelligent compression
│   └── mediagit-media/        # Media-aware merge intelligence
```

## Quick Start

### Installation

```bash
# From source
git clone https://github.com/yourusername/mediagit-core.git
cd mediagit-core
cargo build --release

# The binary will be at target/release/mediagit
```

### Basic Usage

```bash
# Initialize a repository
mediagit init

# Check status
mediagit status

# Show version
mediagit version
```

## Development

### Prerequisites

- Rust 1.91.0 or later
- Cargo

### Building

```bash
# Debug build
cargo build

# Release build (optimized)
cargo build --release

# Run tests
cargo test --all

# Run with logging
RUST_LOG=debug cargo run
```

### Testing

```bash
# Unit tests
cargo test

# Integration tests
cargo test --test '*'

# With coverage (requires cargo-tarpaulin)
cargo tarpaulin --out Html
```

## Platform Support

MediaGit supports 6 platforms:

| Platform | Architecture | Status |
|----------|--------------|--------|
| Linux    | x86_64       | ✅ Supported |
| Linux    | aarch64      | ✅ Supported |
| macOS    | x86_64       | ✅ Supported |
| macOS    | Apple Silicon | ✅ Supported |
| Windows  | x86_64       | ✅ Supported |
| Windows  | ARM64        | ✅ Supported |

## Contributing

We welcome contributions! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for details.

### Development Workflow

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

## License

This project is licensed under the GNU Affero General Public License v3.0 (AGPL-3.0).

See [LICENSE](LICENSE) for details.

## Roadmap

- [x] Week 1: Project foundation and local storage
- [ ] Week 2: Object database and compression
- [ ] Week 3: Git integration and 3-way merge
- [ ] Week 4: Delta encoding and media-aware merge
- [ ] Week 5: FSCK and integrity verification
- [ ] Week 6: Cloud storage backends
- [ ] Week 7: Metrics, GC, and encryption
- [ ] Week 8: Testing, documentation, and release

## Performance Targets

- Object store: <50ms for <100MB files
- Branch switching: <100ms
- Compression: <100ms for 10MB files
- Deduplication check: <10ms
- Cache hit: <5ms

## Acknowledgments

Built with modern Rust ecosystem:
- [Tokio](https://tokio.rs/) - Async runtime
- [Clap](https://docs.rs/clap/) - CLI framework
- [Serde](https://serde.rs/) - Serialization
- [Tracing](https://tokio.rs/tokio/topics/tracing) - Observability

---

**Made with 🦀 and ❤️ by the MediaGit Contributors**

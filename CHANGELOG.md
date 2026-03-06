# Changelog

All notable changes to MediaGit will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.1-beta.1] - 2026-03-06

### Changed
- Automated version extraction from Cargo.toml in release workflow dry-run mode
- Updated all documentation to reflect correct version, URLs, and archive names
- Added `scripts/bump-version.sh` for automated version bumping across the project

## [0.2.0] - 2026-03-05

### Added
- Dual-layer delta encoding (bsdiff + sliding-window)
- AES-256-GCM client-side encryption with Argon2id key derivation
- TLS 1.3 for all network operations
- JWT + API key authentication for server mode
- Video and audio track-based merging (fully implemented)
- Multi-platform distribution (Linux, macOS, Windows, Docker, crates.io)
- Automated release pipeline with cross-compilation

### Changed
- Delta max chain depth reduced from 50 to 10 for faster reads
- Chunk sizes now adaptive (1-8 MB) instead of fixed 64 MB
- Similarity thresholds tuned per file type for better delta compression
- macOS Intel CI runner updated to macos-15-intel

### Fixed
- macOS Intel (x86_64-apple-darwin) build failure due to retired macos-13 runner
- Docker push to GHCR (added packages:write permission)
- Comprehensive documentation sync with codebase (book, architecture docs, CLI reference)

## [0.1.0] - 2026-02-27

### Added
- Core MediaGit CLI implementation
- Object database with SHA-256 content addressing
- Intelligent compression (Zstd, Brotli)
- Branch management system
- 3-way merge algorithm
- Media-aware merge intelligence (PSD layer-aware)
- Git integration layer
- Multi-cloud storage backends:
  - Local filesystem
  - AWS S3
  - Azure Blob Storage
  - Google Cloud Storage
  - MinIO (S3-compatible)
  - Backblaze B2
  - DigitalOcean Spaces
- Security: AES-256-GCM encryption at rest
- Observability: Structured logging with Tracing
- Metrics: Prometheus metrics endpoint
- Operations: Garbage collection, FSCK, storage migration
- Comprehensive test suite (960 tests, 80%+ coverage)
- Documentation and user guide
- Multi-platform binaries (Linux, macOS, Windows on x86_64 and ARM64)

### Security
- AGPL-3.0 license enforcement
- Dependency security audits in CI
- Encryption at rest with Argon2 key derivation

[Unreleased]: https://github.com/winnyboy5/mediagit-core/compare/v0.2.1-beta.1...HEAD
[0.2.0]: https://github.com/winnyboy5/mediagit-core/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/winnyboy5/mediagit-core/releases/tag/v0.1.0

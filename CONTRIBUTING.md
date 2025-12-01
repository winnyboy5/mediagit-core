# Contributing to MediaGit-Core

Thank you for your interest in contributing to MediaGit-Core! This document provides guidelines and instructions for contributing.

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [Getting Started](#getting-started)
- [Development Setup](#development-setup)
- [Project Structure](#project-structure)
- [Building and Testing](#building-and-testing)
- [Making Changes](#making-changes)
- [Submitting Pull Requests](#submitting-pull-requests)
- [Code Style Guidelines](#code-style-guidelines)
- [Documentation](#documentation)
- [Release Process](#release-process)

## Code of Conduct

This project adheres to a Code of Conduct that all contributors are expected to follow. Please read [CODE_OF_CONDUCT.md](./CODE_OF_CONDUCT.md) before contributing.

## Getting Started

### Prerequisites

- **Rust**: 1.91.0 or later
- **Cargo**: Comes with Rust
- **Git**: For version control
- **Docker**: For integration tests (optional)

### Fork and Clone

```bash
# Fork the repository on GitHub, then clone your fork
git clone https://github.com/YOUR_USERNAME/mediagit-core.git
cd mediagit-core

# Add upstream remote
git remote add upstream https://github.com/mediagit/mediagit-core.git
```

## Development Setup

### Install Dependencies

```bash
# Install Rust toolchain (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Update to latest Rust version
rustup update

# Install development tools
cargo install cargo-tarpaulin  # Code coverage
cargo install cargo-audit      # Security audits
cargo install mdbook           # Documentation
cargo install mdbook-mermaid   # Diagrams in documentation
```

### Build the Project

```bash
# Build in debug mode
cargo build

# Build in release mode
cargo build --release

# Build specific crate
cargo build -p mediagit-cli
```

## Project Structure

```
mediagit-core/
├── crates/
│   ├── mediagit-cli/          # Command-line interface
│   ├── mediagit-storage/       # Storage abstraction and backends
│   ├── mediagit-versioning/    # Object database, commits, branches
│   ├── mediagit-compression/   # Compression algorithms
│   ├── mediagit-media/         # Media-aware merging
│   ├── mediagit-config/        # Configuration management
│   ├── mediagit-observability/ # Logging and tracing
│   ├── mediagit-git/           # Git integration layer
│   ├── mediagit-security/      # Encryption and security
│   ├── mediagit-metrics/       # Prometheus metrics
│   └── mediagit-migration/     # Storage backend migration
├── benches/                    # Performance benchmarks
├── book/                       # User documentation (mdBook)
├── docs/                       # Developer documentation
├── .github/                    # GitHub Actions CI/CD
├── Cargo.toml                  # Workspace configuration
└── README.md
```

## Building and Testing

### Run Tests

```bash
# Run all tests
cargo test --workspace

# Run tests for specific crate
cargo test -p mediagit-versioning

# Run integration tests only
cargo test --workspace --test '*'

# Run with output
cargo test --workspace -- --nocapture
```

### Run Benchmarks

```bash
# Run all benchmarks
cargo bench --workspace

# Run specific benchmark
cargo bench --bench odb_benchmarks

# Compare with baseline
cargo bench --workspace -- --save-baseline main
```

### Check Code Quality

```bash
# Run clippy (linter)
cargo clippy --workspace --all-targets --all-features -- -D warnings

# Run rustfmt (formatter)
cargo fmt --all -- --check

# Check for security vulnerabilities
cargo audit

# Generate code coverage
cargo tarpaulin --workspace --out Html
```

## Making Changes

### Create a Feature Branch

```bash
# Update your fork
git checkout main
git fetch upstream
git merge upstream/main

# Create feature branch
git checkout -b feature/your-feature-name
```

### Make Your Changes

1. **Write tests first** (TDD approach recommended)
2. **Implement your changes**
3. **Ensure tests pass**
4. **Add documentation**
5. **Run quality checks**

### Commit Guidelines

Use conventional commit format:

```
<type>(<scope>): <description>

[optional body]

[optional footer]
```

**Types**:
- `feat`: New feature
- `fix`: Bug fix
- `docs`: Documentation changes
- `test`: Test additions or updates
- `refactor`: Code refactoring
- `perf`: Performance improvements
- `chore`: Maintenance tasks
- `ci`: CI/CD changes

**Examples**:

```bash
git commit -m "feat(storage): Add Azure Blob Storage backend"
git commit -m "fix(merge): Handle edge case in 3-way merge algorithm"
git commit -m "docs: Update quickstart guide with S3 configuration"
git commit -m "test(odb): Add property-based tests for deduplication"
```

## Submitting Pull Requests

### Before Submitting

- ✅ All tests pass (`cargo test --workspace`)
- ✅ No clippy warnings (`cargo clippy --workspace`)
- ✅ Code is formatted (`cargo fmt --all`)
- ✅ Documentation is updated
- ✅ Changelog is updated (for significant changes)
- ✅ Commit messages follow conventions

### Pull Request Process

1. **Push your branch**:
   ```bash
   git push origin feature/your-feature-name
   ```

2. **Create Pull Request** on GitHub with:
   - Clear title following conventional commit format
   - Description of changes and motivation
   - Link to related issues (if any)
   - Test plan and verification steps

3. **Address Review Comments**:
   - Respond to all feedback
   - Make requested changes
   - Push updates to the same branch

4. **Merge**:
   - Maintainers will merge when approved
   - Squash commits if requested

### Pull Request Template

```markdown
## Summary
Brief description of changes

## Motivation
Why are these changes needed?

## Changes
- Change 1
- Change 2
- Change 3

## Test Plan
How have you verified these changes work?

## Related Issues
Closes #123
Related to #456

## Checklist
- [ ] Tests pass locally
- [ ] Code follows project style guidelines
- [ ] Documentation updated
- [ ] Changelog updated (if applicable)
```

## Code Style Guidelines

### Rust Code Style

- **Follow Rust conventions**: Use `rustfmt` defaults
- **Idiomatic Rust**: Prefer Rust idioms and patterns
- **Error Handling**: Use `Result` and `anyhow` for errors
- **Comments**: Use `///` for doc comments, `//` for inline comments
- **Async**: Use `tokio` for async runtime, `async-trait` for traits

### Module Organization

```rust
// Module structure
mod submodule;

use std::...;
use external_crate::...;
use crate::...;

pub struct MyStruct { ... }

impl MyStruct { ... }

#[cfg(test)]
mod tests { ... }
```

### Documentation

- **Public APIs**: Always document with `///` comments
- **Examples**: Include examples in doc comments
- **Safety**: Document `unsafe` code thoroughly
- **Panics**: Document when functions can panic

```rust
/// Compresses data using Zstd algorithm
///
/// # Arguments
///
/// * `data` - The raw data to compress
///
/// # Returns
///
/// Compressed data as `Vec<u8>`
///
/// # Examples
///
/// ```
/// use mediagit_compression::ZstdCompressor;
///
/// let compressor = ZstdCompressor::new();
/// let compressed = compressor.compress(b"Hello, World!").unwrap();
/// ```
///
/// # Errors
///
/// Returns error if compression fails
pub fn compress(&self, data: &[u8]) -> Result<Vec<u8>> {
    // Implementation
}
```

## Documentation

### User Documentation

User documentation lives in `book/` and is built with mdBook:

```bash
# Install mdbook
cargo install mdbook mdbook-mermaid

# Serve documentation locally
cd book
mdbook serve
# Open http://localhost:3000
```

### API Documentation

Generate API documentation with:

```bash
# Generate and open API docs
cargo doc --workspace --no-deps --open

# Check for documentation warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

## Release Process

Releases are managed by maintainers. The process is:

1. **Version Bump**: Update version in `Cargo.toml`
2. **Changelog**: Update `CHANGELOG.md`
3. **Tag Release**: Create git tag (e.g., `v0.1.0`)
4. **CI/CD**: GitHub Actions builds and publishes binaries
5. **Crates.io**: Publish to crates.io
6. **Documentation**: Deploy docs to docs.mediagit.dev
7. **Announcement**: Announce release

## Getting Help

- **Discord**: Join our [Discord server](https://discord.gg/mediagit)
- **GitHub Discussions**: Ask questions in [Discussions](https://github.com/mediagit/mediagit-core/discussions)
- **Issue Tracker**: Report bugs in [Issues](https://github.com/mediagit/mediagit-core/issues)

## Recognition

Contributors are recognized in:
- `CONTRIBUTORS.md` file
- Release notes
- Project README

Thank you for contributing to MediaGit-Core! 🎉

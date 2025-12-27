# Building from Source

Build MediaGit from source code.

## Prerequisites
- Rust 1.91.0 or later
- Git

## Steps

```bash
git clone https://github.com/yourusername/mediagit-core.git
cd mediagit-core
cargo build --release
cargo install --path crates/mediagit-cli
```

## Verify Installation

```bash
mediagit --version
```

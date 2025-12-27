# Summary

[Introduction](./introduction.md)

# Getting Started

- [Installation](./installation/README.md)
  - [Linux x64](./installation/linux-x64.md)
  - [Linux ARM64](./installation/linux-arm64.md)
  - [macOS Intel](./installation/macos-intel.md)
  - [macOS ARM64 (M1/M2/M3)](./installation/macos-arm64.md)
  - [Windows x64](./installation/windows-x64.md)
  - [Windows ARM64](./installation/windows-arm64.md)
  - [Building from Source](./installation/from-source.md)
- [Quickstart Guide](./quickstart.md)
- [Configuration](./configuration.md)

# CLI Reference

- [CLI Overview](./cli/README.md)
- [Core Commands](./cli/core-commands.md)
  - [init](./cli/init.md)
  - [add](./cli/add.md)
  - [commit](./cli/commit.md)
  - [status](./cli/status.md)
  - [log](./cli/log.md)
  - [diff](./cli/diff.md)
  - [show](./cli/show.md)
- [Branch Management](./cli/branch-management.md)
  - [branch](./cli/branch.md)
  - [merge](./cli/merge.md)
  - [rebase](./cli/rebase.md)
- [Remote Operations](./cli/remote-operations.md)
  - [push](./cli/push.md)
  - [pull](./cli/pull.md)
- [Maintenance](./cli/maintenance.md)
  - [gc](./cli/gc.md)
  - [fsck](./cli/fsck.md)
  - [verify](./cli/verify.md)
  - [stats](./cli/stats.md)

# Architecture

- [Architecture Overview](./architecture/README.md)
- [Core Concepts](./architecture/concepts.md)
  - [Object Database (ODB)](./architecture/odb.md)
  - [Content-Addressable Storage](./architecture/cas.md)
  - [Delta Encoding](./architecture/delta-encoding.md)
  - [Compression Strategy](./architecture/compression.md)
- [Storage Backends](./architecture/storage-backends.md)
  - [Local Storage](./architecture/backend-local.md)
  - [S3 Storage](./architecture/backend-s3.md)
  - [Azure Blob Storage](./architecture/backend-azure.md)
  - [Backblaze B2](./architecture/backend-b2.md)
  - [Google Cloud Storage](./architecture/backend-gcs.md)
  - [MinIO](./architecture/backend-minio.md)
  - [DigitalOcean Spaces](./architecture/backend-do.md)
- [Media-Aware Merging](./architecture/media-merging.md)
- [Branch Model](./architecture/branching.md)
- [Security](./architecture/security.md)

# User Guides

- [Basic Workflow](./guides/basic-workflow.md)
- [Branching Strategies](./guides/branching-strategies.md)
- [Merging Media Files](./guides/merging-media.md)
- [Working with Remote Repositories](./guides/remote-repos.md)
- [Storage Backend Configuration](./guides/storage-config.md)
- [Delta Compression Guide](./guides/delta-compression.md)
- [Performance Optimization](./guides/performance.md)
- [Troubleshooting](./guides/troubleshooting.md)

# Advanced Topics

- [Custom Merge Strategies](./advanced/custom-merge.md)
- [Backup and Recovery](./advanced/backup-recovery.md)
- [Repository Migration](./advanced/migration.md)
- [CI/CD Integration](./advanced/cicd.md)
- [Large File Optimization](./advanced/large-files.md)

# Reference

- [Configuration Reference](./reference/config.md)
- [Environment Variables](./reference/environment.md)
- [File Formats](./reference/file-formats.md)
- [API Documentation](./reference/api.md)
- [Comparison with Git-LFS](./reference/vs-git-lfs.md)
- [FAQ](./reference/faq.md)

# Contributing

- [Contributing Guide](./contributing/README.md)
- [Development Setup](./contributing/development.md)
- [Code of Conduct](./contributing/code-of-conduct.md)
- [Release Process](./contributing/releases.md)

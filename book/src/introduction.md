# Introduction

Welcome to MediaGit-Core, a next-generation media versioning system built with Rust that replaces Git-LFS with intelligent compression, multi-backend storage, full branching support, and media-aware merging capabilities.

## What is MediaGit-Core?

MediaGit-Core is an open-source version control system designed specifically for managing large binary files (media assets) with the same efficiency and flexibility that Git provides for source code. It solves the fundamental limitations of Git-LFS while providing a familiar Git-like interface.

## Key Features

### High Performance
- **Fast staging**: 25–240 MB/s for large files (release build)
- **Efficient storage**: Up to 81% savings via compression + dedup + delta encoding
- **Parallel operations**: Optimized for modern multi-core systems

### Multi-Backend Storage
Support for 7 storage backends with zero vendor lock-in:
- Local filesystem
- Amazon S3
- Azure Blob Storage
- Backblaze B2
- Google Cloud Storage
- MinIO
- DigitalOcean Spaces

### Media-Aware Merging

**Status: not yet available.** MediaGit can *analyse* images (PSD, PNG, JPEG,
WebP), video (MP4, MOV, AVI), audio (WAV, MP3, FLAC) and 3D assets to determine
whether two sets of edits overlap. It cannot yet **write** a merged file back
in those formats, so there is no auto-merge to wire up: merging a binary file
detects the conflict and checks out one side for you to resolve.

Format *inspection* is available now via `mediagit media`, which parses PSD
layers, video/audio streams and 3D model metadata.

### Full Branching Support
- Create, merge, and rebase branches just like Git
- Protected branches with review requirements
- Branch-specific storage optimization

### Enterprise-Ready
- BUSL-1.1 source-available license + commercial licensing
- Audit trails and security features
- Self-hosted or cloud deployment options

## Who Should Use MediaGit-Core?

MediaGit-Core is designed for teams and individuals working with large binary files:

- **Game Developers**: Manage textures, models, and game assets
- **VFX Artists**: Version video files, composites, and renders
- **ML Engineers**: Track datasets and model files
- **Design Teams**: Collaborate on PSD files and design assets
- **Media Production**: Manage video, audio, and multimedia projects

## How Does It Compare?

| Feature | MediaGit-Core | Git-LFS | Perforce |
|---------|--------------|---------|----------|
| Architecture | Standalone native VCS | Git extension + server | Centralized VCS |
| Branch Switch Speed | Instant (ref-based) | Instant (Git handles) | ⚠️ Copy-based |
| Storage Savings (avg) | Up to 81% (compression + dedup + delta) | ~0% (no dedup/delta) | ~10-20% (RCS deltas) |
| Deduplication | ✅ Content-addressable | ❌ None | ✅ Server-side |
| Multi-Backend Support | 7 backends | Server-dependent | Proprietary |
| Media-Aware Merging | ⚠️ Conflict *detection* only — see above; no auto-merge yet | ❌ No | ⚠️ Limited |
| Offline Commits | ✅ Full DVCS | ✅ (Git handles) | ❌ Server required |
| Source available | ✅ BUSL-1.1 | ✅ MIT (open source) | ❌ No |

## Quick Example

```bash
# Initialize a repository
mediagit init my-project
cd my-project

# Add and commit media files
mediagit add textures/*.png
mediagit commit -m "Add game textures"

# Create a feature branch
mediagit branch create feature/new-assets

# Work on the branch
mediagit add models/*.fbx
mediagit commit -m "Add 3D models"

# Merge back to main
mediagit branch switch main
mediagit merge feature/new-assets
```

## Getting Started

Ready to get started? Head to the [Installation](./installation/README.md) guide to install MediaGit-Core on your platform, then follow the [Quickstart Guide](./quickstart.md) for a 5-minute tutorial.

## License

MediaGit-Core is **source available** under the **Business Source License 1.1
(BUSL-1.1)**.

- ✅ **Free for production use at any scale**, by any organisation — no seat cap,
  no company-size cap
- ✅ Read, modify, fork and self-host freely
- ✅ Each release converts to **AGPL-3.0-or-later four years after publication**
- ⚠️ Offering MediaGit *to third parties* as a hosted, managed or
  software-as-a-service offering requires a commercial licence

Not open source in the OSI sense — the competing-service restriction is a
field-of-use limit, which the Open Source Definition does not permit.

See [LICENSE](https://github.com/winnyboy5/mediagit-core/blob/main/LICENSE) for the
governing terms and
[LICENSE-COMMERCIAL.md](https://github.com/winnyboy5/mediagit-core/blob/main/LICENSE-COMMERCIAL.md)
for what needs a commercial licence.

## Community and Support

- **GitHub**: [winnyboy5/mediagit-core](https://github.com/winnyboy5/mediagit-core)
- **Documentation**: [https://winnyboy5.github.io/mediagit-core](https://winnyboy5.github.io/mediagit-core)
- **Issues**: [GitHub Issues](https://github.com/winnyboy5/mediagit-core/issues)

## What's Next?

- [Installation Guide](./installation/README.md) - Install MediaGit-Core on your platform
- [Quickstart Guide](./quickstart.md) - Get up and running in 5 minutes
- [CLI Reference](./cli/README.md) - Comprehensive command reference
- [Architecture](./architecture/README.md) - Learn how MediaGit-Core works

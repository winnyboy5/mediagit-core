# Introduction

Welcome to MediaGit-Core, a next-generation media versioning system built with Rust that replaces Git-LFS with intelligent compression, multi-backend storage, full branching support, and media-aware merging capabilities.

## What is MediaGit-Core?

MediaGit-Core is an open-source version control system designed specifically for managing large binary files (media assets) with the same efficiency and flexibility that Git provides for source code. It solves the fundamental limitations of Git-LFS while providing a familiar Git-like interface.

## Key Features

### Lightning-Fast Performance
- **Instant branch switching**: <100ms vs 30+ minutes with Git-LFS
- **Efficient storage**: 70-90% compression with intelligent delta encoding
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
Intelligent conflict detection and resolution for:
- Images (PSD, PNG, JPEG, WebP)
- Video files (MP4, MOV, AVI)
- Audio files (WAV, MP3, FLAC)
- 3D models and game assets

### Full Branching Support
- Create, merge, and rebase branches just like Git
- Protected branches with review requirements
- Branch-specific storage optimization

### Enterprise-Ready
- AGPL-3.0 community license + commercial licensing
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
| Branch Switch Speed | <100ms | 30-60 min | 5-10 min |
| Storage Compression | 70-90% | 0% | 0% |
| Multi-Backend Support | 7 backends | GitHub only | Proprietary |
| Media-Aware Merging | ✅ Yes | ❌ No | ⚠️ Limited |
| Open Source | ✅ AGPL-3.0 | ✅ MIT | ❌ No |
| Cost (1TB storage) | ~$60/mo | ~$500/mo | ~$1000/mo |

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

MediaGit-Core is dual-licensed:
- **AGPL-3.0** for community use (individuals, open-source projects, education)
- **Commercial License** for enterprise use requiring proprietary modifications

See the [LICENSE](https://github.com/mediagit/mediagit-core/blob/main/LICENSE) file for details.

## Community and Support

- **GitHub**: [mediagit/mediagit-core](https://github.com/mediagit/mediagit-core)
- **Documentation**: [https://mediagit.dev/docs](https://mediagit.dev/docs)
- **Discord**: [Join our community](https://discord.gg/mediagit)
- **Issues**: [GitHub Issues](https://github.com/mediagit/mediagit-core/issues)

## What's Next?

- [Installation Guide](./installation/README.md) - Install MediaGit-Core on your platform
- [Quickstart Guide](./quickstart.md) - Get up and running in 5 minutes
- [CLI Reference](./cli/README.md) - Comprehensive command reference
- [Architecture](./architecture/README.md) - Learn how MediaGit-Core works

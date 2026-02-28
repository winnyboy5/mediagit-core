# MediaGit vs Git-LFS

Detailed comparison of MediaGit and Git Large File Storage (Git-LFS).

## Overview

**Git-LFS** is a Git extension developed by GitHub that replaces large files in a Git repository with text pointers. The actual file content is stored on an LFS server. It requires Git to be installed and a Git host with LFS support (GitHub, GitLab, Bitbucket).

**MediaGit** is an independent version control system designed specifically for large media files. It does not depend on Git and uses its own object database, chunking engine, delta encoder, and wire protocol.

---

## Feature Comparison

| Feature | MediaGit | Git-LFS |
|---------|----------|---------|
| **Git dependency** | None — standalone | Required |
| **Server requirement** | MediaGit server or cloud backend | Git host with LFS support |
| **Content-aware chunking** | Yes (FastCDC, StreamCDC, per-format) | No |
| **Cross-file deduplication** | Yes — shared chunk pool | No |
| **Delta encoding** | Yes — chunk-level deltas | No |
| **Parallel ingestion** | Yes — multi-core (`--jobs`) | No |
| **Cloud backends** | S3, Azure, GCS, MinIO, B2, DO Spaces | LFS server (varies by host) |
| **No file size limit** | Practical limit: disk/network | Host-specific (e.g., 2 GB on GitHub free) |
| **Format-aware compression** | Yes — JPEG/MP4 stored as-is | No — always uploads raw bytes |
| **Offline history** | Full history locally | Pointer only; content fetched on demand |
| **Git compatibility** | Not compatible | Full Git compatibility |
| **Windows ARM64** | Build from source | Part of Git for Windows |

---

## Storage Efficiency

### Deduplication

Git-LFS stores each version of a file as a complete copy on the LFS server. If you have 100 versions of a 500 MB video, Git-LFS stores 50 GB.

MediaGit splits files into content-addressable chunks. Chunks shared across versions or files are stored only once. For a video with 80% unchanged content between versions, MediaGit stores approximately:

```
500 MB base + (19 × 100 MB deltas) = 2.4 GB  vs  Git-LFS: 50 GB
```

### Delta Encoding

Git-LFS has no delta encoding. MediaGit stores only the diff between similar chunk versions, which is especially effective for:

- PSD files with layer changes (5–20% of file size per edit)
- 3D model files with incremental geometry changes
- Audio stems with minor edits

---

## Performance

### Ingestion

| File | Git-LFS | MediaGit (16 cores) |
|------|---------|---------------------|
| PSD 71 MB | ~2 MB/s | ~35 MB/s |
| MP4 500 MB | ~3 MB/s | ~20 MB/s |
| Pre-compressed (JPEG) | ~80 MB/s | ~200 MB/s |

MediaGit's parallel chunking pipeline (`--jobs N`) saturates multi-core machines.

### Transfer

Git-LFS uploads each file as a single HTTP request. Interrupted transfers must restart from the beginning. MediaGit transfers individual chunks, so an interrupted upload can resume from the last successful chunk.

---

## Workflow Comparison

### Git-LFS

```bash
# Setup (once per repo)
git lfs install
git lfs track "*.psd" "*.mp4"
git add .gitattributes

# Normal workflow
git add scene.psd
git commit -m "Update scene"
git push origin main
```

### MediaGit

```bash
# Setup (once per repo)
mediagit init
mediagit remote add origin http://server/my-project

# Normal workflow
mediagit add scene.psd
mediagit commit -m "Update scene"
mediagit push origin main
```

---

## Advantages of MediaGit

1. **No Git required** — deploy MediaGit independently of your code repository
2. **Content-aware chunking** — PSD layers, video frames, and audio segments are split at format boundaries for better deduplication
3. **Cross-file deduplication** — identical frames in different videos share storage
4. **Delta encoding** — only differences are stored for similar versions
5. **Parallel ingestion** — 10–20× faster than sequential on multi-core hardware
6. **Format-aware compression** — pre-compressed formats (JPEG, MP4) are not re-compressed, saving CPU cycles
7. **No host file size limits** — no GitHub-imposed 2 GB limit
8. **Self-hosted options** — MinIO, local filesystem, or any S3-compatible service

## Advantages of Git-LFS

1. **Git compatible** — works with GitHub, GitLab, Bitbucket, and any Git host
2. **Familiar workflow** — same `git add / commit / push` commands
3. **Mature ecosystem** — LFS support is built into all major Git hosting platforms
4. **Selective fetching** — pull only specific files with `git lfs pull --include`

---

## Trade-offs

MediaGit is optimized for media-heavy workflows where storage efficiency and ingestion speed matter more than Git compatibility. If your team is already using GitHub and needs code and assets in one repository, Git-LFS integrates with less friction.

| Situation | Recommendation |
|-----------|---------------|
| Code + small assets (<100 MB total) | Git-LFS or plain Git |
| Media-first pipeline (game art, VFX, video) | MediaGit |
| Existing GitHub workflow, want LFS | Git-LFS |
| Self-hosted, storage cost matters | MediaGit |
| 1TB+ media repositories | MediaGit |
| Need Git compatibility | Git-LFS |

---

## Migration Guide

### From Git-LFS to MediaGit

```bash
# 1. Pull all LFS content locally
git lfs pull

# 2. Initialize a MediaGit repository
mediagit init

# 3. Add all LFS files to MediaGit
mediagit add --jobs $(nproc) --all

# 4. Create initial commit
mediagit commit -m "Migrate from Git-LFS"

# 5. Configure remote and push
mediagit remote add origin http://media-server.example.com/my-project
mediagit push origin main
```

### Running Both Side-by-Side

Many teams use Git for source code and MediaGit for assets:

```
project/
├── .git/          # Git repo for code
├── .mediagit/     # MediaGit repo for assets
├── src/           # Tracked by Git
└── assets/        # Tracked by MediaGit
```

```bash
# Code workflow
git add src/feature.py && git commit -m "Add feature"

# Asset workflow
mediagit add assets/scene.psd && mediagit commit -m "Update scene"
```

---

## See Also

- [FAQ](./faq.md)
- [Performance Guide](../guides/performance.md)
- [Delta Compression Guide](../guides/delta-compression.md)
- [Storage Backend Configuration](../guides/storage-config.md)

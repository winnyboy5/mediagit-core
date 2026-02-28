# Frequently Asked Questions

## How is MediaGit different from Git-LFS?

Git-LFS is an extension to Git that replaces large files with text pointers and stores the actual content on a separate LFS server. It requires a Git repository and a Git-LFS-compatible server.

MediaGit is a standalone version control system purpose-built for large media files. Key differences:

| | MediaGit | Git-LFS |
|---|---|---|
| **Git required** | No — fully standalone | Yes — wraps Git |
| **Chunking** | Content-aware chunking (FastCDC, StreamCDC) | No chunking |
| **Delta encoding** | Yes — chunk-level deltas | No |
| **Deduplication** | Cross-file chunk deduplication | No deduplication |
| **Parallel ingestion** | Yes — multi-core | No |
| **Cloud backends** | S3, Azure, GCS, MinIO, B2 | Server-specific |
| **File size limit** | No practical limit | Depends on server |

For a detailed comparison, see [MediaGit vs Git-LFS](./vs-git-lfs.md).

---

## What file sizes are supported?

MediaGit has no hard file size limit. In practice:

- Files under 10 MB: stored as single objects with FastCDC chunking
- Files 10–100 MB: split into 10–100 chunks with content-aware FastCDC
- Files over 100 MB: split with StreamCDC into 100–2000 chunks
- Files over 10 GB: supported; tested with 100 GB+ video files

Performance for large files benefits from the `--jobs` flag:

```bash
mediagit add --jobs 16 huge-video.mp4
```

---

## Which cloud storage is best?

All supported backends (local, S3, Azure Blob, GCS) are functionally equivalent. Choose based on your infrastructure:

| Backend | Best for |
|---------|----------|
| **Local filesystem** | Development, single-machine use |
| **Amazon S3** | AWS-hosted projects, widest ecosystem |
| **MinIO** | Self-hosted S3-compatible, on-premise |
| **Azure Blob** | Azure-hosted projects |
| **Google Cloud Storage** | GCP-hosted projects |

For CI/CD environments, use the same region as your runners to minimize latency and transfer costs.

---

## How does delta encoding work?

When you add a new version of a file that already exists in the repository, MediaGit computes a similarity score between the new file's chunks and the stored chunks. If the chunks are sufficiently similar, it stores only the difference (delta) rather than a full copy.

Similarity thresholds vary by file type:
- AI/PDF files: 15% similarity required
- Office documents (docx, xlsx): 20% similarity required
- General files: 80% similarity required

Delta chains are capped at depth 10 to prevent slow reads.

---

## Does MediaGit work with Git?

No. MediaGit is an independent version control system, not a Git extension or plugin. It uses its own object database, ref format, and wire protocol. You cannot push a MediaGit repository to GitHub/GitLab.

Use MediaGit alongside Git: keep source code in Git, keep large media assets in MediaGit.

---

## How do I migrate from Git-LFS?

The migration process:

1. Export your Git-LFS files: `git lfs pull`
2. Initialize a MediaGit repository: `mediagit init`
3. Add the exported files: `mediagit add --all`
4. Commit: `mediagit commit -m "Initial import from Git-LFS"`

For large repositories, use the parallel add flag to speed up ingestion:

```bash
mediagit add --jobs $(nproc) --all
```

---

## How do I undo a commit?

MediaGit does not yet have a built-in revert or reset command. To recover from a bad commit, check out an earlier commit:

```bash
mediagit log           # find the target commit hash
mediagit checkout <hash>
```

---

## Can multiple people work on the same repository?

Yes. Push your changes to a shared remote:

```bash
mediagit push origin main
```

Others pull updates:

```bash
mediagit pull origin main
```

Concurrent writes to the same branch follow a push/pull model similar to Git.

---

## What compression algorithm does MediaGit use?

Zstd (level 3 by default) for compressible formats, and Store (no compression) for already-compressed formats like JPEG, MP4, ZIP, PDF, and AI files. The algorithm is selected automatically per file type.

You can tune the global level in `.mediagit/config.toml`:

```toml
[compression]
algorithm = "zstd"
level = 3   # 1 (fast) to 22 (best)
```

---

## How do I configure author information for commits?

Priority (highest to lowest):

1. `--author "Name <email>"` CLI flag
2. `MEDIAGIT_AUTHOR_NAME` / `MEDIAGIT_AUTHOR_EMAIL` environment variables
3. `[author]` section in `.mediagit/config.toml`
4. `$USER` environment variable (name only)

```toml
[author]
name = "Alice Smith"
email = "alice@example.com"
```

---

## Is Windows ARM64 supported?

Windows ARM64 pre-built binaries are not included in official releases because the cross-compilation tool (`cross-rs`) does not support Windows targets. Windows ARM64 users must [build from source](../installation/windows-arm64.md). Alternatively, the x64 binary runs via Windows ARM64 emulation.

---

## See Also

- [MediaGit vs Git-LFS](./vs-git-lfs.md)
- [Configuration Reference](./config.md)
- [Environment Variables](./environment.md)
- [Performance Guide](../guides/performance.md)
- [Troubleshooting](../guides/troubleshooting.md)

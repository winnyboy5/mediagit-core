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

MediaGit has no hard file size limit. Content-defined chunking (FastCDC)
scales its average chunk size with the file size so chunk *count* stays
manageable for very large files:

- Under 100 MB: ~1 MB average chunk (512 KB–4 MB range)
- 100 MB – 10 GB: ~2 MB average chunk (1–8 MB range)
- 10 GB – 100 GB: ~4 MB average chunk (1–16 MB range)
- Over 100 GB: ~8 MB average chunk (1–32 MB range)

Format-aware chunkers (MP4, MKV, GLB, STL, PLY, FBX, Blender, ...) use their
own structure-based split instead of these generic tiers, up to a size cap
(`MEDIAGIT_CONTAINER_CHUNK_CAP_MB`, default 100 MB) above which they fall
back to plain content-defined chunking.

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
- AI/PDF/PSD files: 15% similarity required
- Office documents (docx, xlsx): 20% similarity required
- Text/code files (txt, py, rs, js, ...): 85% similarity required
- Unknown/general files: 30% similarity required

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

MediaGit provides two commands for undoing commits:

**Revert** — creates a new commit that undoes a previous commit (preserves history):
```bash
mediagit revert <commit-hash>   # undo a specific commit
```

**Reset** — moves the current branch pointer backward (rewrites history):
```bash
mediagit reset --soft HEAD~1    # undo commit but keep changes staged
mediagit reset --mixed HEAD~1   # undo commit and unstage changes (default)
mediagit reset --hard HEAD~1    # undo commit and discard changes
```

For recovering specific files from an earlier commit, use `mediagit show`:
```bash
mediagit log --oneline          # find the target commit hash
mediagit show <hash>:<path>     # inspect a file from that commit
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

MediaGit's `SmartCompressor` picks one of four strategies automatically per file type — you don't choose an algorithm yourself:

- **Store** (no recompression) for already-compressed formats: JPEG, PNG, MP4, ZIP, AI/InDesign, Office documents
- **Zstd** (Best or Default level, depending on format) for uncompressed images (TIFF, RAW, EXR), PSD/3D models/creative project files, PDF/SVG, and as the safe fallback for unknown binary data
- **Brotli** (Default level) for text/code formats (TXT, JSON, XML, YAML, TOML, CSV) — falls back to Zstd above 500 MB, where Brotli's encode cost stops paying off
- **Zlib** only for internal Git-compatible objects (not used on media files)

You cannot tune this. Algorithm and level are chosen per file type by
`SmartCompressor` and there is no config key for either - the `[compression]`
section that used to appear in `config.toml` was never read at runtime and was
removed in v0.4.0.

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

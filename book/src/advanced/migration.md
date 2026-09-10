# Repository Migration

Migrating from other version control systems to MediaGit.

## From Git-LFS

If you currently use Git-LFS to store large media files, you can migrate your assets to MediaGit while keeping source code in Git.

### Step 1: Pull all LFS content locally

```bash
git lfs pull
```

This downloads all LFS-tracked files to your working tree.

### Step 2: Initialize a MediaGit repository

```bash
mediagit init
```

### Step 3: Add all media files

```bash
# For parallel ingestion (recommended)
mediagit add --jobs $(nproc) --all
```

This processes all files in the current directory. For large collections (hundreds of GB), expect several minutes to several hours depending on file count, file types, and hardware.

### Step 4: Commit

```bash
mediagit commit -m "Initial import from Git-LFS"
```

### Step 5: Configure a remote and push

```bash
# In .mediagit/config.toml
[remotes.origin]
url = "http://media-server.example.com/my-project"
```

```bash
mediagit push origin main
```

### Ongoing workflow

After migration, continue tracking code with Git and assets with MediaGit:

```
project/
├── .git/       ← Git repository for code
├── .mediagit/  ← MediaGit repository for assets
├── src/        ← tracked by Git
└── assets/     ← tracked by MediaGit
```

Remove large files from Git-LFS tracking in `.gitattributes` and stop using `git lfs track`.

---

## From Plain Git (Large Files in History)

If your Git repository has large binary files committed directly:

### Step 1: Export the current state of large files

```bash
# Identify large files in Git history
git rev-list --objects --all | \
  git cat-file --batch-check='%(objecttype) %(objectname) %(objectsize) %(rest)' | \
  awk '/^blob/ && $3 > 10485760 {print $4, $3}' | sort -k2 -rn
```

### Step 2: Copy large files to a separate directory

```bash
mkdir ../media-export
# Copy each identified large file:
cp assets/video.mp4 ../media-export/
cp assets/textures/ ../media-export/ -r
```

### Step 3: Import into MediaGit

```bash
mkdir ../my-media-repo
cd ../my-media-repo
mediagit init
mediagit add --jobs $(nproc) ../media-export/
mediagit commit -m "Initial import from Git"
```

### Step 4: Remove large files from Git history (optional)

Use [`git filter-repo`](https://github.com/newren/git-filter-repo) to remove large files from Git history and reduce repository size:

```bash
pip install git-filter-repo
git filter-repo --strip-blobs-bigger-than 10M
```

---

## From File System (No VCS)

If your media assets are on a file system with no version control:

```bash
mediagit init
mediagit add --jobs $(nproc) /path/to/media/
mediagit commit -m "Initial import"
```

For very large collections (TB-scale), run add in batches by directory:

```bash
for dir in /media/project/*/; do
  mediagit add --jobs 16 "$dir"
  mediagit commit -m "Import $(basename "$dir")"
done
```

---

## Verifying the Migration

After migration, verify all files were ingested correctly:

```bash
# Check object integrity
mediagit fsck

# Review statistics
mediagit stats
```

`mediagit verify` has no per-file spot-check: its `--path` flag selects the
*repository* root (same as `-C`), not a file inside it, and `[COMMIT]` takes a
commit-ish, not a file path (`mediagit verify --help`). For a fuller report use
`mediagit verify --detailed`; there is no flag to verify a single file's
checksum in isolation.

---

## Storage Efficiency After Migration

How much you save depends almost entirely on whether your bytes are already
compressed. The figures below are **measured**, from the per-family economics of
QA campaign `20260909-ga52` and the chain fixtures in
[BENCHMARKS.md](../../../BENCHMARKS.md) — not estimates.

| Content type | Measured savings vs raw files | Why |
|---|---|---|
| Layered design files (PSD) | **67%** | Layer data is stored less aggressively compressed, so unchanged layers dedup across edits |
| Vector / page containers (SVG) | **67%** | Structured text deltas well |
| Uncompressed audio (WAV) | **95%** | Raw PCM; both compression and delta apply |
| 3D meshes (GLB, STL, PLY, DAE) | **80%** on a GLB version chain (39.5 → 7.8 MB); 33–83% per-edit | Float-heavy geometry compresses and deltas extremely well |
| Illustrator / PDF containers (AI) | **26%** | Embedded streams are already compressed; only structure deltas |
| ML weights (safetensors, NPZ) | **45–77%** | Large float arrays; content-dependent |
| Photos (JPEG, PNG) | **0%** | Already compressed — stored as-is, by design |
| Video, including master + proxy variants | **0%** | Already compressed. Two encodes of the same footage share no *bytes*, so chunk dedup cannot see the shared frames |
| FLAC | **0%** | Already losslessly compressed |

**Aggregate on a mixed real-file corpus: ~26.5%.** That number is lower than most
rows above because real repositories are dominated by pre-compressed bytes —
video and JPEG pull the average down no matter how well PSD and WAV do. Estimate
your own result from your *format mix*, not from the best row in this table.

Run `mediagit stats` after committing multiple versions to see your actual
compression and deduplication ratios.

---

## See Also

- [MediaGit vs Git-LFS](../reference/vs-git-lfs.md)
- [Performance Optimization](../guides/performance.md)
- [Large File Optimization](./large-files.md)
- [Storage Backend Configuration](../guides/storage-config.md)

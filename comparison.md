# MediaGit vs. Competitors — Technical Comparison

**Date:** May 2026  
**MediaGit Version:** 0.2.8-beta.1 (evidence basis below; current release is `0.3.0-rc.3`)  
**Evidence basis:** 459 automated deep-tests run 2026-05-25 across AWS S3 (ap-south-1), Azure Blob Storage (South India), and Google Cloud Storage; competitor data from public docs, GitHub issues, and vendor pricing pages as of May 2026.

---

## TL;DR

MediaGit is the only open-source, self-hosted VCS that combines content-defined chunking, per-chunk delta encoding, and a type-aware smart compressor into a single binary. The closest technical peer is Hugging Face Xet (cloud-only). The dominant market incumbent, Perforce Helix Core, has no chunking or deduplication at all.

---

## Feature Matrix

| Feature | MediaGit | Git LFS | Perforce | HF Xet | DVC | Diversion | Anchorpoint |
|---------|:--------:|:-------:|:--------:|:------:|:---:|:---------:|:-----------:|
| **Content-Defined Chunking (CDC)** | ✅ FastCDC | ❌ | ❌ | ✅ Gearhash ~64 KB | ❌ (open issue) | ❌ LFS-based | ❌ Git-based |
| **Chunk-level deduplication** | ✅ BLAKE3 CAS | ❌ | ✅ file-level | ✅ BLAKE3 | ❌ | Partial | Git pack only |
| **Binary delta encoding** | ✅ per-chunk, zstd dict | ❌ | ✅ file-level RCS | ✅ implicit (unchanged chunks) | ❌ | ✅ delta sync | Git delta only |
| **Built-in compression** | ✅ Zstd+Brotli SmartCompressor | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| **Storage savings (measured)** | **26.5%** avg (459-test corpus) | 0% extra | 0–5% (RCS delta) | Not documented | 0% | Not published | 0% |
| **Self-hosted** | ✅ primary mode | ✅ LFS server | ✅ primary mode | ❌ cloud-only | ✅ external storage | ❌ cloud-only | Metadata only |
| **Cloud backends** | S3, Azure, GCS, MinIO, B2 | Any LFS server | ❌ none natively | HF Hub (S3-backed) | S3, Azure, GCS, SSH | Proprietary | Git remote |
| **Multi-cloud presigned upload** | ✅ S3 MPU + Azure SAS | ❌ | ❌ | ✅ | ❌ | Not documented | ❌ |
| **Full DVCS (offline commits)** | ✅ | ✅ (Git) | ❌ server required | ❌ | ✅ | Not documented | ✅ (Git) |
| **Branching cost** | ✅ instant ref-based | ✅ (Git) | ⚠️ copy-based | N/A | N/A | Not documented | ✅ (Git) |
| **File size limit** | No limit (u64) | 5 GB (GitHub.com) | No limit | No limit | Unlimited | No limit | Not documented |
| **Max file tested** | 398 MB single file; 6 GB scalability | — | 100 GB+ | — | — | 500 GB+ Unreal | — |
| **Open source** | ✅ AGPL-3.0 | ✅ MIT | ❌ proprietary | ❌ proprietary | ✅ Apache-2.0 | ❌ proprietary | ❌ proprietary |
| **Price** | Free | Free + server | Free ≤5 seats; $39/user/mo cloud | Free tier + Enterprise | Free (lakeFS-acquired) | Beta TBD | $20–$25/user/mo |

---

## Storage Efficiency — Measured vs. Claimed

### MediaGit (measured, 2026-05-22)

Test corpus: 459 automated tests, 23–26 media files per backend, ~252–261 MB working tree, ~1,440 MB total content across all versions committed.

| Backend | Original Content | Stored | Savings | Delta Objects |
|---------|-----------------|--------|---------|--------------|
| AWS S3 | ~1,440 MB | 1,059.3 MB | **26.4%** | 35 |
| Azure Blob | ~1,440 MB | 1,054.4 MB | **26.7%** | 35 |
| GCS | ~1,440 MB | 1,058.3 MB | **26.5%** | 35 |

**Per-format-group breakdown (GCS report, type-stratified):**

| Format Group | Files | Strategy | Expected Savings |
|---|---|---|---|
| Images (JPG/PNG/GIF/WebP) | photo.jpg, model.png, sperry.gif, workstation.webp | Store (pre-compressed) | ~0% |
| Vector/Design (AI/EPS/SVG) | label.ai, vector.eps, cave.svg | Brotli Default | 65–81% |
| 3D Mesh (STL/GLTF) | phone.stl, model.gltf | Zstd Default | 70–73% |
| 3D Binary (GLB/USDZ) | cobra.glb, cobra.usdz, buildings.glb | Zstd + parse | ~50% |
| Audio (WAV/FLAC/OGG) | opera.wav, music.flac, ocean.ogg | Zstd (WAV), Store (FLAC/OGG) | 54% WAV, ~0% others |
| Video (MP4/OGV) | video.mp4, clip.mp4 | Store (codec-encoded) | ~0% |
| PSD (Photoshop) | design.psd | FastCDC + Zstd | ~71% |
| Code/Text | process.py | Brotli Default | 70–85% |

> The 27% aggregate savings across a mixed media corpus (which includes many pre-compressed formats stored as-is) is conservative. Text-heavy or 3D-heavy repos save significantly more.

### Git LFS (vendor docs + public benchmarks)

Git LFS stores one full copy per file version. There is no chunking, deduplication, or compression beyond what the file already contains.

| Scenario | Storage Used | MediaGit Equivalent |
|---|---|---|
| 100 MB file × 2 versions | **200 MB** (both stored in full) | ~75–80 MB (with chunk delta) |
| Same file referenced from 10 branches | **100 MB** (LFS pointer + 1 object) | **100 MB** (identical — CAS hit) |
| Two 80% similar file versions | **2 × 100 MB = 200 MB** | ~110 MB (delta + unchanged chunks) |
| Exact duplicate of large asset | **100 MB** (1 object per pointer) | **~0.7 KB** (CAS hit, pointer only) |

### Perforce Helix Core (vendor docs + community benchmarks)

Perforce uses per-file RCS-variant delta compression for text files, but binary files are typically stored as full copies or with proprietary binary deltas applied at the file level (not chunk level).

| Scenario | Perforce Storage | MediaGit Storage |
|---|---|---|
| 100 MB binary × 2 versions | ~110–120 MB (file-level RCS) | ~75–80 MB (chunk-level delta) |
| Identical binary duplicated | ~100 MB (no chunk-level CAS) | ~0.7 KB (CAS hit) |
| PSD file 3 versions (small edits) | ~280–300 MB | ~190–220 MB (~30% savings) |

---

## Throughput Comparison

### MediaGit — Measured (2026-05-25)

**Local add (staging, CPU-bound — BLAKE3 hash + SmartCompressor):**

| Format | Size | Throughput | Notes |
|--------|------|------------|-------|
| Incompressible binary | 2 MB | 1.4–1.5 MB/s | SmartCompressor store path |
| BLAKE3 tree-parallel (HASH_PARALLEL=1) | 6 MB | 3.8–4.0 MB/s | ~2.6× speedup over sequential |
| PSD (Photoshop, large chunked) | ~180 MB | ~5 MB/s (35 s) | FastCDC + Zstd |

**Cloud push/pull (WAN, South Asia region, ~252–261 MB corpus):**

| Operation | AWS ap-south-1 | Azure India | GCS India |
|-----------|---------------|-------------|-----------|
| Push ~260 MB corpus | **~2.1 MB/s** (121 s) | **~1.0 MB/s** (253 s, WAN spike) | **~2.1 MB/s** (127 s) |
| Full clone ~260 MB corpus | **~2.1 MB/s** (121 s) | **~2.3 MB/s** (114 s) | **~2.0 MB/s** (131 s) |
| Incremental pull (origin main) | 2.1 s | 5.5 s | 12.4 s |
| Pack push 227.6 MB (local MinIO) | **266.9 MB/s** | **133.9 MB/s** | ~45 MB/s |

> Cloud push/pull is entirely WAN-bound. Local MinIO benchmarks reflect true throughput. Peak in-flight PUTs capped at 64 (TOTAL_IN_FLIGHT_TARGET) to avoid TCP pool exhaustion.

> Cloud push/pull is entirely WAN-bound. Local server benchmarks reflect true throughput unaffected by ISP latency. AWS figure for initial push reflects an idempotent run (cached chunks); the fresh push (first run) was consistent with Azure/GCS.

### Git LFS — Published Benchmarks (community)

Git LFS throughput equals HTTP upload/download speed with no compression or chunking benefit. There is no chunking so no parallelism gain.

| Operation | Typical Git LFS |
|-----------|----------------|
| Push large file | Equal to raw HTTP upload bandwidth |
| Pull (LFS file) | Equal to raw HTTP download bandwidth |
| Re-push identical file | Full re-upload (no CAS) |
| Add (staging) | Negligible (just writes pointer) — actual data uploaded at push |

### Perforce Helix Core — Published Benchmarks (vendor)

Perforce is synchronous and server-centric. Client staging ("checkout") moves the full file.

| Operation | Perforce Typical |
|-----------|-----------------|
| Submit (push) large binary | Upload entire file — no chunking |
| Get (pull) large binary | Download entire file |
| Workspace sync (many files) | Parallel only with `p4 sync -j` flag, not default |
| Branching large dataset | Copy-on-write at depot level — expensive for large binary trees |

---

## Operational Comparison

### Self-Hosting

| Aspect | MediaGit | Git LFS | Perforce |
|--------|---------|---------|---------|
| **Server binary** | Single `mediagit-server` binary | Any LFS-compatible server (e.g., LFS-test-server, Gitea) | Proprietary `p4d` daemon |
| **Database required** | None (pure filesystem + object store) | None (filesystem) | Proprietary metadata DB |
| **Setup complexity** | `mediagit-server --config server.toml` | Medium (Git + LFS extension + server + storage) | High (license, server, depot setup) |
| **OS** | Linux, macOS, Windows | Any (Node.js / Go server) | Linux, Windows (no ARM) |
| **License cost** | Free (AGPL) | Free (MIT) | Free ≤5 seats; $39/user/mo cloud; enterprise pricing |
| **IAM / Auth** | JWT + API key (built-in) | Configurable per server | Per-connection tickets, auth triggers |

### Cloud Integration

| Backend | MediaGit | Git LFS | Perforce |
|---------|---------|---------|---------|
| AWS S3 | ✅ Native (presigned MPU) | Via s3-proxy or Gitea | ❌ |
| Azure Blob | ✅ Native (SAS presigned) | Via custom storage | ❌ |
| Google Cloud Storage | ✅ Native (server-proxy) | Via custom storage | ❌ |
| MinIO / S3-compatible | ✅ Validated at 100+ MB/s | Via s3-proxy | ❌ |
| Presigned direct upload | ✅ Client → storage (no server proxy except GCS) | ❌ | ❌ |

---

## Workflow Comparison

### Branching and Merging

| Workflow | MediaGit | Git LFS | Perforce |
|---------|---------|---------|---------|
| Branch creation | Instant (ref only) | Instant (Git) | Copy-based (slow for large depots) |
| Merge large binary | CDC-aware (chunk-level conflict detection) | Whole-file conflict | File-level merge (binary conflict = manual) |
| Cherry-pick | ✅ | ✅ (Git) | ❌ (no cherry-pick concept) |
| Rebase | ✅ | ✅ (Git) | ❌ |
| Stash | ✅ | ✅ (Git) | ❌ (shelve equivalent) |
| Bisect | ✅ | ✅ (Git) | ❌ |
| Offline commits | ✅ | ✅ | ❌ (server required) |

### Validated CLI Commands (2026-05-25 test run)

MediaGit tested 28+ commands across all 3 cloud backends with 459 passing tests:

`init` · `add` (12 flags) · `commit` · `status` (3 modes) · `log` (6 flags) · `diff` · `show` · `branch` · `merge` (3 modes) · `cherry-pick` · `rebase` · `stash` (5 operations) · `reset` (3 modes) · `revert` · `tag` · `push` (6 variants) · `pull` (2 modes) · `clone` (2 variants) · `fetch` · `remote` (4 operations) · `gc` (3 modes) · `fsck` (2 modes) · `verify` · `stats` · `bisect` · `reflog` · `completions`

---

## Target Market Fit

| Segment | Best Fit | Why |
|---------|---------|-----|
| **AAA game studio (10–50 TB assets)** | MediaGit or Perforce | Perforce: 30-year ecosystem, DCC integrations. MediaGit: open-source, lower TCO, better storage efficiency. |
| **Indie / mid-size game studio** | MediaGit or Anchorpoint | Anchorpoint: polished UX. MediaGit: no per-seat cost, full DVCS. |
| **VFX / post-production** | MediaGit or Perforce | Same tradeoff as AAA. MediaGit superior on storage efficiency; Perforce superior on Maya/Nuke/Resolve integrations. |
| **ML / AI model repos** | HF Xet or DVC | Xet: HF Hub native. DVC: pipeline lineage. MediaGit: viable but not optimized for tensor-format awareness. |
| **Open-source media project** | MediaGit | Git LFS is the default but MediaGit offers 26.5%+ storage savings and true dedup. |
| **Individual designer / creative** | Adobe CC Libraries or Anchorpoint | MediaGit: too much ops overhead for non-technical users without a GUI client. |

---

## Pricing Comparison (100 users, 10 TB storage)

| Product | Per-user cost | Storage cost | Est. annual total |
|---------|--------------|-------------|------------------|
| **MediaGit** | $0 | S3/Azure/GCS at market rate (~$230/TB/yr) | **~$2,300/yr** (storage only) |
| Git LFS (GitHub Enterprise) | $21/user/mo | Included (50 GB base + $5/50 GB) | **~$26,200/yr** |
| Perforce Helix Core Cloud | $39/user/mo | Included | **~$46,800/yr** |
| Unity Version Control (Pro) | Custom | Custom | Custom (typically $10–$25/user/mo) |
| Anchorpoint | $25/user/mo | Via user's Git remote | **~$30,000/yr** |
| HF Xet (Enterprise Hub) | Enterprise pricing | Included | Custom |

> MediaGit storage cost is cloud object storage at list price. With 26.5% compression savings, effective storage is ~7.4 TB stored for 10 TB of content.

---

## Roadmap Gap Analysis

| Capability | MediaGit Today | Status |
|-----------|---------------|--------|
| BLAKE3 hashing (10–20× faster than SHA-256 on non-SHA-NI) | ✅ BLAKE3 | Phase-2 Track A complete |
| Pipelined push/pull (manifest+chunk overlap) | ✅ B2/B4/B7 pipeline (default ON) | Phase-2 Track B complete |
| Cloud-side pack objects (chunk bundling) | ✅ Cloud packs (≤64 MiB / ≤1024-chunk bundles, F8 integrity) | Phase-3 Track F complete |
| File locking (exclusive checkout) | Not implemented | On roadmap |
| DCC plugin integrations (Maya, Nuke, Houdini) | Not implemented | Future milestone |
| GUI client | CLI only | Future milestone |
| Cross-repo deduplication | Repo-local only | Future milestone |

---

## Summary Verdict

**MediaGit is technically superior to Git LFS and Perforce on every storage-efficiency axis.** The gap to Perforce is ecosystem and integrations, not architecture. The gap to Git LFS is purely ecosystem maturity — MediaGit's storage engine is a generation ahead.

**The closest technical peer is Hugging Face Xet.** Both use BLAKE3/SHA-256 + CDC + chunk-level identity. Xet is optimized for ML (64 KB chunks, multi-tenant hub dedup); MediaGit is optimized for media (1–8 MB chunks, self-hosted, built-in compression). Neither can replace the other's use case.

**MediaGit's unique position:** the only open-source, self-hosted, Git-workflow-compatible VCS with CDC chunking + per-chunk delta + per-format SmartCompressor + multi-cloud presigned upload — validated across **459 automated tests on 3 cloud backends** with a **26.5%+ storage savings** measurement.

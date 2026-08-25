# Delta Compression Guide

Complete guide to understanding and optimizing delta compression in MediaGit.

## What is Delta Compression?

Delta compression stores only the differences between file versions instead of complete copies:

```
Traditional Storage:
  v1.psd: 500 MB
  v2.psd: 500 MB (full copy)
  v3.psd: 500 MB (full copy)
  Total: 1,500 MB

Delta Compression:
  v1.psd: 500 MB (base)
  v2.psd: 15 MB (delta from v1)
  v3.psd: 8 MB (delta from v2)
  Total: 523 MB (65% savings!)
```

## How MediaGit Applies Delta Compression

### Automatic Detection

Delta is automatic. There is no flag that turns it on, and the decision runs
in three stages — a candidate has to clear all three:

1. **Is this file type worth trying?** (`should_use_delta`) Decided by
   extension, not by a global size floor. Uncompressed raster and audio
   (`psd`, `tif`, `bmp`, `wav`, `aiff`), text and code, and text-based 3D
   formats (`obj`, `gltf`, `stl`, `step`) are always attempted. Already-
   compressed formats — `jpg`, `png`, `webp`, `gif`, `zip`, `gz`, `7z` — are
   always skipped, because there is nothing left to find. A few types are
   size-conditional: compressed video (`mp4`, `mkv`, `flv`, `wmv`) only above
   100 MB, and PDF containers (`ai`, `indd`, `idml`, `pdf`) only above 50 MB,
   where even partial similarity in unchanged embedded images is worth the CPU.
2. **Is a similar enough base available?** Content similarity must clear the
   per-type threshold in the table below.
3. **Did it actually help?** The delta must come out **below 80%** of the full
   object. At 80% or above it is discarded and the object is stored whole — a
   delta that saves a fifth is not worth the reconstruction cost.

### Similarity Thresholds by File Type

| File Type | Threshold | Behavior |
|-----------|-----------|----------|
| **AI/PDF/InDesign** | 0.15 | Very aggressive (compressed streams, structural similarity) |
| **DOCX/XLSX/PPTX** (Office) | 0.20 | Aggressive (ZIP containers, shared structure) |
| **MP4/MOV** (Video) | 0.50 | Moderate (metadata/timeline changes) |
| **WAV/AIF** (Audio) | 0.65 | Medium (clip edits) |
| **JPG/PNG** (Compressed images) | 0.70 | Moderate — but see the note below |
| **OBJ/FBX/GLTF/GLB** (3D interchange) | 0.70 | Moderate (geometry changes) |
| **MA/MB** (Maya) | 0.50 | Moderate |
| **BLEND/C4D** (3D scenes) | 0.40 | Aggressive (heavy per-edit diffs) |
| **HIP** (Houdini) | 0.35 | Aggressive |
| **DRP/FCPBUNDLE/AVB** (NLE projects) | 0.25 | Very aggressive |
| **PTX/ALS/FLP** (DAW projects) | 0.55 | Medium |
| **DWG/DXF** (CAD) | 0.45 | Moderate |
| **RVT/RFA** (Revit) | 0.30 | Aggressive |
| **TXT/Code** | 0.85 | Conservative (small changes matter) |
| **JSON/YAML/TOML/XML** (Config) | 0.95 | Very conservative (exact matches preferred) |
| **Default** | 0.30 | Global minimum (`MIN_SIMILARITY_THRESHOLD`) |

**Lower threshold** = more files use delta compression
**Higher threshold** = only very similar files use delta

Note **PSD sits with the AI/PDF group at 0.15, not with the images at 0.70**:
it is a layered container of embedded compressed streams, structurally much
closer to InDesign than to a flat JPEG. And the 0.70 on JPG/PNG never applies
in practice — stage 1 skips those types outright.

These thresholds are compile-time constants in
`crates/mediagit-versioning/src/similarity.rs`. **They are not configurable**,
by config file or by environment variable; making them so is a tracked backlog
item, not a current feature.

## Checking Delta Status

MediaGit has no per-object delta inspector — no `show --similarity`, no
`stats --delta-report`, no `verify --check-deltas`. What exists is aggregate
and repository-wide:

```bash
# Compression metrics across the repository
$ mediagit stats --compression

# Everything stats knows, as JSON
$ mediagit stats --all --json
```

To watch the decision for a single file, turn on the log for the add path:

```bash
$ MEDIAGIT_LOG=warn,mediagit=debug mediagit add large-file.psd
```

Keep the leading `warn,` — a bare target directive silences every other
target, which has produced more than one confusing debugging session in this
repository.

### Overriding for a single file

One override exists, and it only goes one way:

```bash
# Skip delta for this file
$ mediagit add --no-delta huge-video.mp4
```

There is no force-on counterpart. If the type gate, the similarity threshold,
or the 80% benefit gate rejects a file, nothing on the command line overrides
that — the object is stored whole.

## Delta Chains

Delta chains form when multiple versions are stored:

```
Base (v1) -> d2 -> d3 -> d4 -> d5
```

Reconstruction applies the deltas in sequence, so a deep chain costs more to
read than a shallow one. MediaGit bounds this for you: `MAX_DELTA_DEPTH` is
**10**, enforced on both the write and the read side
(`crates/mediagit-versioning/src/odb/mod.rs`). A chain that would exceed it
gets a fresh base instead, and `get_chunk` refuses to reconstruct past it.

Two consequences:

- **You cannot end up with a depth-50 chain.** Guidance elsewhere about
  "optimizing chains over 20 deep" describes a situation this cap makes
  unreachable.
- **There is no chain-optimization command**, because there is no unbounded
  chain to optimize. `mediagit gc --repack` consolidates loose objects into
  packs, which is a storage-layout operation, not a chain one.

To check that chains are actually intact, use the integrity checker:

```bash
# Full check, including delta chain reconstruction
$ mediagit fsck --full
```

## Performance Tuning

Delta has exactly two knobs, both environment variables. There is no
`[compression.delta]` table — earlier revisions of this guide showed
`[compression.delta.thresholds]`, `[compression.delta.performance]` and
`[compression.delta.memory]`, and none of them have ever existed in the config
schema.

| Variable | Default | Effect |
|---|---|---|
| `MEDIAGIT_DELTA_LEVEL` | `19` | zstd dictionary compression level for delta encoding. Valid `1`-`22`; outside that range is rejected with a warning and the default used. |
| `MEDIAGIT_DELTA_ENABLED` | — | Legacy toggle; set to `0`/`false` to force delta off for the whole repo, overriding repo config. Candidacy is otherwise decided per file. |

Parallelism is not delta-specific — it is `add`'s file-level parallelism, and
the only control is `mediagit add --no-parallel` to turn it off.

## Troubleshooting

### Delta wasn't applied

Work the three stages in order — the first one that rejects is the answer.

**Is the type eligible at all?** `jpg`, `png`, `webp`, `gif`, `zip`, `gz`,
`7z` and `rar` are skipped unconditionally. This is correct behavior, not a
failure: those bytes are already compressed, and a delta of compressed data
finds nothing.

**Is it under a size gate?** `mp4`/`mkv`/`flv`/`wmv` are only attempted above
100 MB; `ai`/`indd`/`idml`/`pdf` only above 50 MB.

**Did it clear similarity and benefit?** Run the add with logging on:

```bash
$ MEDIAGIT_LOG=warn,mediagit=debug mediagit add large-file.psd
```

A file that was genuinely rewritten between versions has low similarity, and a
delta that lands at 80% or more of the full size is discarded by design.

### Reconstruction feels slow

Chain depth is capped at 10, so it is very unlikely to be the cause. Confirm
the repository is actually healthy first:

```bash
$ mediagit fsck --full
```

If fsck is clean, look at storage-layout and transfer concurrency rather than
at delta — see [Performance Optimization](./performance.md).

### High memory during add

There is no delta-specific memory knob. Reduce concurrency instead:

```bash
$ mediagit add --no-parallel large-file.psd
```

## Best Practices

1. **Let it decide.** The type gate, similarity thresholds and the 80% benefit
   gate are tuned per format. The one useful manual override is `--no-delta`
   for a file you know was fully rewritten.
2. **Run `mediagit gc` periodically** to reclaim unreachable objects, and
   `gc --repack` to consolidate loose objects into packs.
3. **Run `mediagit fsck --full` after bulk imports**, which is the check that
   actually reconstructs delta chains.
4. **Measure with `mediagit stats --compression`** rather than reasoning about
   what the thresholds should produce.

## Related Documentation

- [Delta Encoding Architecture](../architecture/delta-encoding.md)
- [Compression Strategy](../architecture/compression.md)
- [Garbage Collection](../cli/gc.md)
- [Performance Optimization](./performance.md)

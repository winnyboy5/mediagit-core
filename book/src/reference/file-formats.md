# File Formats

Internal file format reference for MediaGit's on-disk data structures.

## Repository Layout

```
<repo-root>/
└── .mediagit/
    ├── HEAD              # Current branch or commit pointer
    ├── config.toml       # Repository configuration (TOML)
    ├── objects/          # Content-addressable object database
    │   └── <h0:2>/<h2:4>/<hash>  # Two-level hash-fanout sharding (layout v2 — see below)
    ├── refs/             # Reference storage
    │   └── heads/        # Branch refs
    │       └── main      # Branch pointer files
    ├── manifests/        # Chunk manifests per committed file
    │   └── <h0:2>/<h2:4>/<hash>  # Envelope-wrapped postcard ChunkManifest (no extension)
    └── stats/            # Operation statistics (non-critical)
        └── <timestamp>.json
```

---

## Storage Layout v2 (namespace + true hash fanout)

As of the smart-media-handling M1 milestone, the *storage backend's* physical
layout (local disk or a cloud bucket, as opposed to the always-local
`.mediagit/` control plane above) is versioned:

```
<storage root>/
└── <repo_namespace>/        # sanitized [a-z0-9._-]; default = repo dir basename
    ├── LAYOUT                # plain-text "<version> <repo_id>" — version marker
    ├── objects/<h0:2>/<h2:4>/<oid>              # bare OIDs (commits/trees/blobs)
    ├── chunks/<h0:2>/<h2:4>/<hash>
    ├── chunk-deltas/<h0:2>/<h2:4>/<hash>[.meta]
    ├── manifests/<h0:2>/<h2:4>/<hash>
    ├── deltas/<h0:2>/<h2:4>/<hash>[.meta]
    ├── packs/<p0:2>/<pack_oid>
    └── bitmaps/<commit_oid>.bitmap              # reachability index
```

### LAYOUT Marker Format

The `LAYOUT` file is plain text containing the storage layout version and a
repository identity (collision detection):

```
2 <16-hex-char-repo-id>
```

The repo_id is a random 16-char hex string generated at `init`/`clone` time
(via `getrandom`). It is checked against the `repo_id` field in the repository's
`.mediagit/config.toml`. A mismatch indicates namespace collision: two
independently-created repos computed the same default namespace against one
shared storage root/bucket. This is a hard error, with a hint to use
`MEDIAGIT_REPO_NAMESPACE` to disambiguate.

Every physical path is sharded on the **hash/OID itself** (v1 sharded on the
key string's own prefix, which collapsed all chunk/chunk-delta traffic into
one `objects/ch/un/` directory). The `repo_namespace` prefix lets one
storage root or bucket safely host multiple repositories; `layout_version`
in `config.toml` mirrors the `LAYOUT` marker. Both are checked on every
open: a missing marker on a fresh/empty store is written automatically; a
version mismatch, or a missing marker alongside pre-existing data, is a
hard error (MediaGit is beta and does not migrate layouts — re-init or
re-clone). The **logical** key space (`chunks/<hash>`, `manifests/<hash>`,
bare OIDs, ...) is unchanged by layout v2 — only the physical path mapping
changed.

---

## Object Format

All objects are stored content-addressably. The object's BLAKE3 hash (over the uncompressed content) is used as the key. Under layout v2 (the default), the key is sharded two levels deep — the first 2 hex characters, then the next 2 — with the full hash as the filename:

```
objects/ab/cd/abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890
```

### Object Types

| Type | Description |
|------|-------------|
| `blob` | File content (full or delta) |
| `tree` | Directory listing: maps filenames to object hashes |
| `commit` | Commit metadata: tree hash, parent hashes, author, message |
| `chunk` | A content chunk from a chunked large file |

### Object Storage

Objects are stored via one of four strategies — Store, Zstd, Brotli, or (internal-only) Zlib — selected automatically per file type by `SmartCompressor`:

| Format class | Strategy |
|---|---|
| JPEG, PNG, GIF, WebP, AVIF, HEIC, MP4, MOV, MKV, MP3, AAC, Opus, ZIP, docx/xlsx/pptx, AI/InDesign | Store (already compressed) |
| TIFF, BMP, RAW, EXR, HDR, DPX, WAV, AIFF, 3D models (OBJ, FBX, GLB, STL, PLY) | Zstd Best |
| FLAC, ALAC, PDF, SVG, PSD, After Effects/Premiere/Resolve/Blender/Maya/Houdini project files | Zstd Default |
| Text, JSON, XML, YAML, TOML, CSV | Brotli Default (Zstd Default above 500 MB) |
| Unknown/binary | Zstd Default (safe fallback) |

See [`CompressionStrategy::for_object_type`](../architecture/compression.md) for the full per-format mapping. Delta objects reference a base object and store only the difference.

---

## Chunk Manifests

Large files are split into content-addressable chunks. The mapping of file → ordered list of chunks is stored as a `ChunkManifest`, on disk as an envelope — `MAGIC ("MGCM", 4 bytes) | VERSION (1 byte) | postcard body` — rather than a bare [postcard](https://docs.rs/postcard) payload, so a future format change can be detected before postcard (non-self-describing) attempts to parse it:

```
.mediagit/manifests/<content-hash>.bin
```

The manifest contains:

- `chunks` — ordered `Vec<ChunkRef>`, each with:
  - `id` — chunk identifier (BLAKE3 hash)
  - `offset` — offset in the original file
  - `size` — chunk size in bytes
  - `chunk_type` — media classification (Generic, VideoStream, AudioStream, Metadata, Subtitle, ...)
  - `codec_hint` — per-chunk compression/delta hint
- `total_size` — total size of the reconstructed object
- `filename` — original filename, optional (used for type detection, not a full path)

Whether a given chunk is stored full or as a delta is not recorded in the manifest itself — deltas live as separate sidecar objects keyed by chunk hash (`chunk-deltas/<hash>[.meta]`, see [Storage Layout v2](#storage-layout-v2-namespace--true-hash-fanout) above).

---

## Reference Format

Refs are plain text files containing a 64-character hex BLAKE3 hash followed by a newline:

```
.mediagit/refs/heads/main
```

```
a3c8f9d2e1b4f6a8c5d7e9f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1
```

### HEAD

`HEAD` contains either:

- A symbolic ref (pointing to a branch): `ref: refs/heads/main`
- A detached commit hash: `a3c8f9d2e1b4f6a8c5d7e9f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1`

---

## Configuration Format

`.mediagit/config.toml` is a standard [TOML](https://toml.io) file. See [Configuration Reference](./config.md) for all supported keys.

```toml
[author]
name = "Alice Smith"
email = "alice@example.com"

[storage]
backend = "filesystem"
base_path = "./data"
```

---

## Commit Object Format

Commits are postcard-serialized structs (see [Chunk Manifests](#chunk-manifests) — the whole object database uses postcard, not bincode) containing:

| Field | Type | Description |
|-------|------|-------------|
| `tree` | `Oid` | BLAKE3 hash of the root tree object |
| `parents` | `Vec<Oid>` | Parent commit hashes (0 for initial, 1+ for merges) |
| `author` | `Signature` | Author `{name, email, timestamp}` |
| `committer` | `Signature` | Committer `{name, email, timestamp}` — separate from `author` |
| `message` | `string` | Commit message |

A `Signature` is `{name: string, email: string, timestamp: DateTime<Utc>}`.

---

## Tree Object Format

A tree object is a postcard-serialized `BTreeMap<String, TreeEntry>` (keyed
by filename, so entries are already in canonical sorted order). Each
`TreeEntry`:

| Field | Type | Description |
|-------|------|-------------|
| `name` | `string` | Filename (not full path) |
| `mode` | `FileMode` | `Regular` (100644), `Executable` (100755), `Symlink` (120000), or `Directory` (040000) |
| `oid` | `Oid` | BLAKE3 hash of the blob or subtree |

There is no separate `size` field on a tree entry — file size lives on the blob/chunk manifest, not the tree.

---

## Tag Object Format

Annotated tags are stored as real objects in the object database, serialized
with postcard (compact binary format). Each Tag contains:

| Field | Type | Description |
|-------|------|-------------|
| `target` | `Oid` | BLAKE3 hash of the tagged object (commit, tree, or blob) |
| `target_type` | `ObjectType` | Type of the tagged object |
| `name` | `string` | Tag name (e.g., `v1.0.0`), without the `refs/tags/` prefix |
| `tagger` | `Signature` | Who created the tag: `{name, email, timestamp}` |
| `message` | `string` | Tag message |
| `signature` | `Option<Vec<u8>>` | Optional OpenSSH-armored `SSH SIGNATURE` PEM block (embedded, TOFU model) |

Tags are serialized with postcard so every field except `signature` forms the
deterministic signing payload. The `signature` field (if present) contains
an OpenSSH-armored `SshSig` blob, including the signer's public key fingerprint
for trust-on-first-use verification.

---

## Reachability Bitmap Format

Reachability bitmaps index commit reachability for fast negotiation during
fetch/pull. One bitmap file per commit, stored at:

```
.mediagit/objects/bitmaps/<commit_oid>.bitmap
```

(Note: as a bitmap file under `bitmaps/`, it is *not* itself an ODB object.)

Format (binary):

```
[1 byte: version] [postcard-serialized(Vec<Oid>, RoaringBitmap)]
```

| Field | Description |
|-------|-------------|
| **version** | Format version byte (currently `1`). Unknown version → treat as absent, rebuild over time |
| **id_array** | `Vec<Oid>` — ordered array of all object IDs reachable from this commit |
| **bitmap** | `RoaringBitmap` — compact set membership encoded in portable binary format |

Lookup semantics: bitwise `id → index in id_array → set_membership(index, bitmap)`.
Bitmaps are pure-speedup derived data (never queried for correctness); if a
bitmap is missing or corrupted, the system falls back to a full `walk_reachable`
BFS traversal (safe, slower).

Retention: when a push advances a ref, the server writes the new tip's bitmap
and deletes the previous tip's — steady state is roughly one bitmap per ref.
`gc` regenerates bitmaps for current branch tips and prunes unreachable ones
(the backstop for deleted branches and forced moves). Deleting a bitmap shared
by two refs at the same tip is safe: the next negotiation falls back to BFS
and the bitmap is regenerated.

---

## Working-Tree Temporary Files

Checkout writes each file to a sibling temporary file named
`<file_name>.mgtmp` in the same directory, then atomically renames it into
place — a crash mid-checkout can never leave a truncated file at a final
path. A stale `.mgtmp` file after a crash or aborted checkout is harmless
residue: it is overwritten the next time that file is checked out, and can
be deleted freely.

---

## Sparse Checkout Pattern File

Sparse checkout filters are stored in a plain-text file:

```
.mediagit/info/sparse-checkout
```

One pattern per line. Lines starting with `#` are comments. Empty lines are
ignored. Absent file or empty file = sparse checkout disabled (full checkout).

**Format:**
- **Cone mode** (default): each line is a directory prefix (e.g., `assets/textures/`),
  included recursively.
- **Pattern mode** (when set with `--patterns`): gitignore-style globs (e.g., `*.png`),
  where a match means **include** (inverse of `.mediagitignore`).

---

## Statistics Format

Per-operation transfer statistics (push/pull/fetch/clone) are written as JSON
to `.mediagit/stats/<timestamp>_<operation_name>.json`:

```json
{
  "operation_name": "push",
  "timestamp": "2026-02-20T10:30:00Z",
  "bytes_downloaded": 0,
  "bytes_uploaded": 1073741824,
  "objects_received": 0,
  "objects_sent": 512,
  "files_updated": 0,
  "duration_ms": 4320
}
```

Only the most recent 100 stats files are kept; older ones are pruned automatically. These files are informational only and can be deleted without affecting repository integrity.

---

## Hashing

MediaGit uses **BLAKE3** for all content hashing (32-byte digest, displayed as 64 hex chars; tree-parallel for large media):

- Object identity: BLAKE3 of the uncompressed object content
- Chunk identity: BLAKE3 of the uncompressed chunk content
- Commit hash: BLAKE3 of the serialized commit object

---

## See Also

- [Architecture — Object Database](../architecture/odb.md)
- [Architecture — Content-Addressable Storage](../architecture/cas.md)
- [Architecture — Compression Strategy](../architecture/compression.md)
- [Configuration Reference](./config.md)

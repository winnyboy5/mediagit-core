# File Formats

Internal file format reference for MediaGit's on-disk data structures.

## Repository Layout

```
<repo-root>/
└── .mediagit/
    ├── HEAD              # Current branch or commit pointer
    ├── config.toml       # Repository configuration (TOML)
    ├── objects/          # Content-addressable object database
    │   ├── <xx>/         # Two-character prefix directories
    │   │   └── <hash>    # Object files (remaining 62 hex chars of SHA-256)
    │   └── pack/         # Pack files (future)
    ├── refs/             # Reference storage
    │   └── heads/        # Branch refs
    │       └── main      # Branch pointer files
    ├── manifests/        # Chunk manifests per committed file
    │   └── <hash>.bin    # Bincode-serialized ChunkManifest
    └── stats/            # Operation statistics (non-critical)
        └── <timestamp>.json
```

---

## Object Format

All objects are stored content-addressably. The object's SHA-256 hash (over the uncompressed content) is used as the key. The key is split into a 2-character directory prefix and 62-character filename:

```
objects/ab/cdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890cd
```

### Object Types

| Type | Description |
|------|-------------|
| `blob` | File content (full or delta) |
| `tree` | Directory listing: maps filenames to object hashes |
| `commit` | Commit metadata: tree hash, parent hashes, author, message |
| `chunk` | A content chunk from a chunked large file |

### Object Storage

Objects are stored compressed (Zstd) or uncompressed (Store), depending on the file type. The compression strategy is selected automatically per file extension:

| Format class | Strategy |
|---|---|
| JPEG, PNG, WebP, MP4, MOV, ZIP, docx, PDF, AI | Store (no compression) |
| PSD, 3D models (OBJ, FBX, GLB, STL, PLY) | Zstd Best |
| WAV, FLAC | Zstd Default |
| Text, JSON, TOML, CSV | Zstd Default |

Delta objects reference a base object and store only the difference.

---

## Chunk Manifests

Large files are split into content-addressable chunks. The mapping of file → ordered list of chunks is stored as a `ChunkManifest` serialized with [bincode](https://github.com/bincode-org/bincode):

```
.mediagit/manifests/<content-hash>.bin
```

The manifest contains:

- File path
- Total file size
- Ordered list of chunks, each with:
  - Chunk hash (SHA-256 of chunk content)
  - Chunk offset in the original file
  - Chunk size (uncompressed)
  - Whether the chunk is stored as full or delta

---

## Reference Format

Refs are plain text files containing a 64-character hex SHA-256 hash followed by a newline:

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

[compression]
enabled = true
algorithm = "zstd"
level = 3
```

---

## Commit Object Format

Commits are stored as Bincode-serialized structs containing:

| Field | Type | Description |
|-------|------|-------------|
| `tree` | `[u8; 32]` | SHA-256 hash of the root tree object |
| `parents` | `Vec<[u8; 32]>` | Parent commit hashes (0 for initial, 1+ for merges) |
| `author` | `string` | Author name |
| `email` | `string` | Author email |
| `timestamp` | `i64` | Unix timestamp (seconds) |
| `message` | `string` | Commit message |

---

## Tree Object Format

Trees are stored as Bincode-serialized ordered lists of entries:

| Field | Type | Description |
|-------|------|-------------|
| `name` | `string` | Filename (not full path) |
| `hash` | `[u8; 32]` | SHA-256 hash of the blob or subtree |
| `is_tree` | `bool` | `true` for subdirectory, `false` for file |
| `size` | `u64` | Uncompressed size in bytes |

---

## Statistics Format

Operation statistics are written as JSON to `.mediagit/stats/`:

```json
{
  "operation": "add",
  "timestamp": "2026-02-20T10:30:00Z",
  "files_processed": 42,
  "bytes_input": 1073741824,
  "bytes_stored": 157286400,
  "chunks_created": 512,
  "chunks_deduplicated": 87,
  "duration_ms": 4320
}
```

These files are informational only and can be deleted without affecting repository integrity.

---

## Hashing

MediaGit uses **SHA-256** for all content hashing:

- Object identity: SHA-256 of the uncompressed object content
- Chunk identity: SHA-256 of the uncompressed chunk content
- Commit hash: SHA-256 of the serialized commit object

---

## See Also

- [Architecture — Object Database](../architecture/odb.md)
- [Architecture — Content-Addressable Storage](../architecture/cas.md)
- [Architecture — Compression Strategy](../architecture/compression.md)
- [Configuration Reference](./config.md)

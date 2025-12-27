# Content-Addressable Storage

Content-Addressable Storage (CAS) is the foundation of MediaGit's deduplication and integrity verification.

## Concept

In CAS, data is retrieved by its content (hash) rather than by name or location:
- **Traditional FS**: `path/to/file.txt` → content
- **CAS**: `SHA-256(content)` → content

## SHA-256 Hashing

MediaGit uses SHA-256 for all objects:
```rust
use sha2::{Sha256, Digest};

let content = b"hello world";
let mut hasher = Sha256::new();
hasher.update(content);
let oid = hasher.finalize();
// oid = b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9
```

## Benefits

### 1. Automatic Deduplication
Identical files stored only once:
```
Branch A: large-file.psd (100 MB) → 5891b5b522...
Branch B: large-file.psd (100 MB) → 5891b5b522... (same hash, no duplication)
```

### 2. Data Integrity
Hash mismatch immediately detected:
```rust
let stored_oid = "5891b5b522...";
let content = read_object(stored_oid);
let actual_oid = sha256(&content);

if actual_oid != stored_oid {
    panic!("Corruption detected!");
}
```

### 3. Distributed Synchronization
Objects identifiable across repositories without central server.

## Object Identification

### Full OID
```
5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03
```

### Short OID
Abbreviated to 7-12 characters (Git-style):
```
5891b5b  // Unique prefix
```

MediaGit accepts short OIDs if unambiguous:
```bash
mediagit show 5891b5b
mediagit show 5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03
```

## Storage Layout

### Directory Sharding
Objects stored with 2-character prefix:
```
objects/
  58/
    91b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03
  a3/
    c5d3e8f2a1b7c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2d3e4f5a6b7c8d9
```

**Rationale**: Prevents millions of files in single directory (filesystem optimization).

## Collision Resistance

SHA-256 has 2^256 possible outputs (approximately 10^77):
- **Probability of collision**: Negligible (< 10^-60 for millions of objects)
- **Comparison**: More atoms in observable universe than SHA-256 outputs

### Collision Handling
If collision detected (theoretical):
1. Verify content matches
2. If content differs, abort (catastrophic error)
3. If content identical, continue (deduplication worked)

## Performance Characteristics

### Hashing Speed
- **Small files** (<1 MB): <1ms
- **Medium files** (10-100 MB): 10-100ms
- **Large files** (>1 GB): Streaming hash (chunked)

### Memory Usage
- Fixed memory (streaming hash): 32 bytes (hash state)
- No need to load entire file into memory

## Implementation Details

### Rust Code
```rust
use sha2::{Sha256, Digest};
use std::io::Read;

pub fn hash_object(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().into()
}

pub fn hash_stream<R: Read>(reader: &mut R) -> std::io::Result<[u8; 32]> {
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];

    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }

    Ok(hasher.finalize().into())
}
```

## Related Documentation

- [Object Database (ODB)](./odb.md)
- [Core Concepts](./concepts.md)

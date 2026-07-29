# Architecture Overview

MediaGit-Core is designed as a modular, extensible version control system optimized for large media files. The architecture follows a layered approach with clear separation of concerns.

## System Architecture

```mermaid
graph TD
    subgraph CLI["mediagit-cli (32 commands)"]
        ADD["add"]
        COMMIT["commit"]
        PUSH["push"]
        PULL["pull"]
        CLONE["clone"]
        OTHER["23+ more..."]
    end

    subgraph Core["Core Libraries"]
        VER["mediagit-versioning<br/>ODB · Index · Refs<br/>Chunks · Delta · Cloud Packs"]
        COMP["mediagit-compression<br/>Zstd · Brotli · Zlib<br/>SmartCompressor"]
        MEDIA["mediagit-media<br/>Image · PSD · Video<br/>Audio · 3D · VFX"]
    end

    subgraph Infra["Infrastructure"]
        STORE["mediagit-storage<br/>Local · S3 · Azure<br/>GCS · B2 · MinIO"]
        SEC["mediagit-security<br/>AES-256-GCM · JWT<br/>TLS · Audit · KDF"]
        PROTO["mediagit-protocol<br/>Client · Packs<br/>Chunk Transfer"]
    end

    subgraph Support["Support"]
        CFG["mediagit-config"]
        OBS["mediagit-observability"]
        MET["mediagit-metrics"]
        TEST["mediagit-test-utils"]
    end

    subgraph Server["mediagit-server"]
        AXUM["Axum REST API<br/>Auth · Rate Limit<br/>Security Middleware"]
    end

    CLI --> Core
    Core --> Infra
    Server --> Core
    Server --> Infra
    CLI --> Support
    Server --> Support
    VER -.->|"cloud packs<br/>(few large objects)"| STORE
```

## Core Components

### 1. CLI Layer
- **Technology**: Clap derive macros for type-safe command parsing
- **Responsibility**: User interface, command validation, help system
- **Location**: `crates/mediagit-cli/`

### 2. Core Logic
- **Object Database (ODB)**: Content-addressable storage with BLAKE3 hashing
- **Versioning Engine**: Branch management, merge strategies, LCA algorithms
- **Media Intelligence**: Format-aware parsing and merging for PSD, video, audio
- **Location**: `crates/mediagit-versioning/`, `crates/mediagit-media/`

### 3. Storage Abstraction
- **Design**: Trait-based abstraction (`StorageBackend` trait)
- **Implementations**: 7 storage backends (local, S3, Azure, GCS, B2, MinIO, Spaces)
- **Benefits**: Easy backend switching, testability, cloud-agnostic design
- **Location**: `crates/mediagit-storage/`

### 4. Compression Layer
- **Algorithms**: zstd (default), brotli, delta (zstd dictionary)
- **Strategy**: Automatic algorithm selection based on file type
- **Performance**: Async compression with tokio runtime
- **Location**: `crates/mediagit-compression/`

## Data Flow

```mermaid
sequenceDiagram
    participant User
    participant CLI
    participant ODB
    participant Compression
    participant Storage

    User->>CLI: mediagit add large-file.psd
    CLI->>ODB: Store object
    ODB->>ODB: Calculate BLAKE3 hash
    ODB->>Compression: Compress with zstd
    Compression->>Storage: Write to backend
    Storage-->>User: ✓ Object stored

    User->>CLI: mediagit commit -m "Update"
    CLI->>ODB: Create commit object
    ODB->>ODB: Link to tree & parent
    ODB->>Storage: Store commit metadata
    Storage-->>User: ✓ Commit created
```

## Design Principles

### Content-Addressable Storage
- **Why**: Automatic deduplication, data integrity verification
- **How**: BLAKE3 hashing of all objects (blobs, trees, commits)
- **Benefit**: Identical files stored only once across all branches

### Async-First Architecture
- **Why**: Handle large file I/O without blocking
- **How**: Tokio runtime with async/await throughout
- **Benefit**: Concurrent operations, better resource utilization

### Media-Aware Intelligence
- **Why**: Generic byte-level merging fails for structured media
- **How**: Format parsers for PSD layers, video tracks, audio channels
- **Benefit**: Preserve layer hierarchies, avoid corruption

### Trait-Based Abstraction
- **Why**: Decouple logic from storage implementation
- **How**: `StorageBackend` trait with 7 implementations
- **Benefit**: Easy testing (mock backends), cloud provider flexibility

## Performance Characteristics

| Operation | Local Backend | S3 Backend | Optimization |
|-----------|---------------|------------|--------------|
| Add 1GB file | ~2-3 seconds | ~15-20 seconds | Delta encoding for updates |
| Commit | <100ms | ~200-500ms | Metadata-only operation |
| Checkout | ~3-5 seconds | ~20-30 seconds | Parallel object fetch |
| Merge | ~1-2 seconds | ~5-10 seconds | Media-aware strategies |

## Security Model

### Authentication
- Local: File system permissions
- S3/Azure/GCS: IAM roles, service principals, service accounts
- B2/MinIO/Spaces: Application keys with bucket-level permissions

### Integrity
- BLAKE3 content verification on all read operations
- Cryptographic hashing prevents data tampering
- `mediagit verify` for repository health checks

### Encryption
- At-rest: AES-256-GCM client-side encryption + cloud provider encryption (SSE-S3, Azure SSE)
- In-transit: TLS 1.3 only when the server's TLS listener is enabled (`min_tls_version`, 1.3 by default). mTLS is not wired.
- Client-side encryption: Fully implemented with Argon2id key derivation

## Scalability

### Repository Size
- Tested with repositories up to 500GB
- Object count: Millions of objects supported
- Recommendation: Use cloud backends for >100GB repos

### Concurrency
- Parallel object fetch during checkout (configurable workers)
- Async I/O prevents thread pool exhaustion
- Lock-free read operations for status/log/diff

### Network Optimization
- HTTP/2 support for cloud backends
- Connection pooling (reqwest library)
- Automatic retry with exponential backoff

## Monitoring and Observability

### Metrics
- **Crate**: `mediagit-metrics`
- **Export**: Prometheus format
- **Metrics**: Operation latency, object size, cache hit rate, error rate

### Logging
- **Crate**: `mediagit-observability`
- **Levels**: ERROR, WARN, INFO, DEBUG, TRACE
- **Format**: Structured JSON logging for production

### Health Checks
- `mediagit fsck`: Repository integrity verification
- `mediagit verify`: Cryptographic object verification
- `mediagit stats`: Repository statistics and health metrics

## Extension Points

### Custom Merge Strategies
- Implement `MergeStrategy` trait
- Register strategy in `MergeEngine`
- Example: Custom video frame merging

### Storage Backend Development
- Implement `StorageBackend` trait
- Provide `get`, `put`, `exists`, `delete`, `list_objects` operations
- Example: IPFS backend, SFTP backend

### Media Format Support
- Implement format parser (e.g., FBX, Blender, CAD)
- Register with `MediaIntelligence` module
- Example: 3D model layer-aware merging

## Technology Stack

- **Language**: Rust 1.97.1
- **Async Runtime**: Tokio 1.40+
- **CLI Framework**: Clap 4.5+
- **Compression**: zstd, brotli, delta (zstd dictionary)
- **Cloud SDKs**: aws-sdk-s3, azure_storage, google-cloud-storage
- **Testing**: proptest (property-based), criterion (benchmarking)

## Related Documentation

- [Core Concepts](./concepts.md)
- [Object Database (ODB)](./odb.md)
- [Storage Backends](./storage-backends.md)
- [Media-Aware Merging](./media-merging.md)
- [Security](./security.md)

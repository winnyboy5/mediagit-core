# Environment Variables

All environment variables recognized by MediaGit. Environment variables take precedence over `config.toml` values where both are supported.

## Core Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `MEDIAGIT_REPO` | Override repository root path. Used internally by `-C <path>`. | — |
| `MEDIAGIT_TOKEN` | Bearer token attached to requests against a matching configured remote. | — |
| `MEDIAGIT_API_KEY` | API key (`X-Api-Key` header) attached to requests against a matching configured remote. Mutually exclusive with `MEDIAGIT_TOKEN` (token wins if both set). | — |
| `MEDIAGIT_REPO_NAMESPACE` | Per-repo namespace prefix for object-store layout v2. No "off" value — always resolves to a namespace (falls back to `config.toml` `repo_namespace`, then the sanitized repo-dir basename). | — |
| `MEDIAGIT_NO_KEYRING` | Any value disables OS keychain credential storage/lookup. | unset (keyring used) |

## Author Identity

These override the `[author]` section of `.mediagit/config.toml`. Priority (highest first): `--author` CLI flag → `MEDIAGIT_AUTHOR_NAME`/`MEDIAGIT_AUTHOR_EMAIL` → `config.toml [author]` → `$USER`.

| Variable | Description |
|----------|-------------|
| `MEDIAGIT_AUTHOR_NAME` | Commit author name (e.g., `"Alice Smith"`) |
| `MEDIAGIT_AUTHOR_EMAIL` | Commit author email (e.g., `"alice@example.com"`) |

## AWS / S3 / S3-Compatible Storage

Standard AWS SDK environment variables. Used when `storage.backend = "s3"`.

| Variable | Description |
|----------|-------------|
| `AWS_ACCESS_KEY_ID` | AWS access key ID |
| `AWS_SECRET_ACCESS_KEY` | AWS secret access key |
| `AWS_SESSION_TOKEN` | AWS session token (for temporary credentials) |
| `AWS_REGION` | AWS region (e.g., `us-east-1`) |
| `AWS_ENDPOINT_URL` | Custom S3 endpoint URL (for MinIO, DigitalOcean Spaces, Backblaze B2, etc.) |
| `AWS_PROFILE` | AWS named profile from `~/.aws/credentials` |

## Azure Blob Storage

Used when `storage.backend = "azure"`.

| Variable | Description |
|----------|-------------|
| `AZURE_STORAGE_CONNECTION_STRING` | Full connection string (alternative to account_name + account_key) |
| `AZURE_STORAGE_ACCOUNT` | Storage account name |
| `AZURE_STORAGE_KEY` | Storage account key |

## Google Cloud Storage

Used when `storage.backend = "gcs"`.

| Variable | Description |
|----------|-------------|
| `GOOGLE_APPLICATION_CREDENTIALS` | Path to service account JSON key file |
| `GCS_EMULATOR_HOST` | GCS emulator URL for testing (e.g., `http://localhost:4443`) |

## Performance Tuning

MediaGit exposes a large set of `MEDIAGIT_*` knobs for tuning push/pull concurrency, chunking, cloud-pack bundling, and storage-backend behavior. The table below is a summary grouped by area; **[`env-knobs.md`](https://github.com/mediagit/mediagit/blob/main/env-knobs.md) in the repository root is the canonical, exhaustive reference** with per-knob stability status and the release each knob was introduced in.

### Push / Pull / Fetch Concurrency

| Variable | Description | Default |
|----------|-------------|---------|
| `MEDIAGIT_UPLOAD_CONCURRENCY` | Max concurrent chunk PUT requests. | `32` |
| `MEDIAGIT_DOWNLOAD_CONCURRENCY` | Max concurrent chunk GET requests. Two internal defaults, deliberately: `32` on the per-chunk path, `24` on the pack range-GET path. Setting the variable makes both agree. | `32` / `24` |
| `MEDIAGIT_CLONE_OVERLAP` | Write the working tree while chunked media is still downloading, instead of waiting for the last byte. `0` restores the serial path. | `1` (ON) |
| `MEDIAGIT_PUSH_PIPELINE` | Cross-object push pipeline. `0` reverts to the sequential path. | `1` (ON) |
| `MEDIAGIT_PUSH_OBJECT_CONCURRENCY` | Max concurrent object uploads when `PUSH_PIPELINE=1`. | `8` |
| `MEDIAGIT_PUSH_CHUNK_CONCURRENCY` | Per-object chunk upload concurrency; targets ~64 total in-flight PUTs. | computed |
| `MEDIAGIT_PUSH_DEADLINE_SECS` | Overall wall-clock deadline for a single push. | `3600` |
| `MEDIAGIT_PULL_DEADLINE_SECS` | Overall wall-clock deadline for a single pull, fetch or clone, applied to the pack phase and the chunk phase separately. | `3600` |
| `MEDIAGIT_RATE_LIMIT_RETRIES` | Max HTTP 429 retries for a single control-plane request. Relevant when a large push outruns a server-side rate limiter. | `10` |
| `MEDIAGIT_RATE_LIMIT_MAX_WAIT_SECS` | Hard ceiling on a single 429 backoff, however large the server's `Retry-After` is. Bounds the worst case to retries x this value; without it an unbounded `Retry-After` could park a client for hours against a healthy server. | `60` |
| `MEDIAGIT_CONTROL_READ_TIMEOUT_SECS` | Max seconds with no bytes received on a control-plane request before it fails. A READ (inter-byte) timeout, not a total one, so slow-but-progressing transfers are unaffected. Catches a peer that is alive but has stopped answering, which `tcp_keepalive` cannot. `0` disables. | `300` |
| `MEDIAGIT_SHORT_REQUEST_DEADLINE_SECS` | Seconds a short, bodyless control-plane request (e.g. `GET /info/refs`) gets to produce response headers before that attempt is abandoned and retried. Two bounded attempts, then one unbounded attempt, so a genuinely slow server still succeeds. `0` disables. | `30` |
| `MEDIAGIT_LOG` | Tracing filter for the CLI (e.g. `debug`). Unset = silent; logs go to stderr. Takes precedence over `RUST_LOG`. Set this first when a client appears to hang. | unset |
| `MEDIAGIT_STRONG_VERIFY` | `1` runs a full BLAKE3 re-hash verification of pushed chunks after transfer. | `0` (OFF) |
| `MEDIAGIT_PULL_PIPELINE` | Overlap manifest fetches and chunk downloads. | `1` (ON) |
| `MEDIAGIT_PULL_MANIFEST_CONCURRENCY` | Max concurrent manifest fetches when `PULL_PIPELINE=1`. | `8` |
| `MEDIAGIT_RANGE_PARALLEL` | Parallel ranged-GET fan-out for large single-chunk downloads. | `4` |
| `MEDIAGIT_RANGE_PARALLEL_THRESHOLD` | Chunk size above which `RANGE_PARALLEL` kicks in. | `4194304` (4 MiB) |
| `MEDIAGIT_DOWNLOAD_DIRECT_DISABLE` | `1` disables direct presigned-URL downloads, forcing the server proxy. | `0` (OFF) |
| `MEDIAGIT_FETCH_BRANCH_CONCURRENCY` | Max branches fetched in parallel for `fetch --all`. | `4` |
| `MEDIAGIT_FETCH_DOWNLOAD_CONCURRENCY` | Per-branch chunk download concurrency when fetching multiple branches. | computed |
| `MEDIAGIT_STREAM_CHUNK_TO_DISK` | Stream chunk bodies directly to the ODB file instead of buffering in RAM. | `1` (ON) |
| `MEDIAGIT_STORAGE_STREAMING` | Use streaming GET on S3/MinIO instead of a `Vec<u8>` round-trip. Applies to the server's chunk-download handler as well as the client. | `1` (ON) |
| `MEDIAGIT_HTTP_POOL_MAX` | Max idle HTTP connections per host. Keep ≤128 on Windows. | `64` |
| `MEDIAGIT_DECOMPRESS_BLOCKING` | Offload decompression to `spawn_blocking` for chunks ≥ threshold. | `1` (ON) |
| `MEDIAGIT_DECOMPRESS_BLOCKING_THRESHOLD` | Byte size above which decompression is offloaded. | `262144` (256 KiB) |

### Add / Chunking / Delta

| Variable | Description | Default |
|----------|-------------|---------|
| `MEDIAGIT_HASH_PARALLEL` | Parallel mmap-based hashing for files ≥ the streaming threshold. `0` forces sequential. | `1` (ON) |
| `MEDIAGIT_ADD_MAX_INFLIGHT_BYTES` | Global in-flight byte budget for parallel file processing during `add`. | `536870912` (512 MiB) |
| `MEDIAGIT_ADD_COMPRESS_BLOCKING` | Offload add-time chunk compression to `spawn_blocking`. | `1` (ON) |
| `MEDIAGIT_ADD_COMPRESS_BLOCKING_THRESHOLD` | Byte size above which add-time compression is offloaded. | `262144` (256 KiB) |
| `MEDIAGIT_CDC_SEED` | Per-repo CDC chunk-boundary seed. Overrides `config.toml` `cdc_seed`. | repo config (`0` legacy) |
| `MEDIAGIT_CODEC_DETECT` | Container-aware codec detection for compression routing. `0` forces `CodecHint::Unknown`. | `1` (ON) |
| `MEDIAGIT_CHUNK_FBX` | Opt-in structure-aware FBX chunking walker (measured net-negative vs. generic CDC; off by design). | unset/`0` (OFF) |
| `MEDIAGIT_CHUNK_BLEND` / `MEDIAGIT_CHUNK_STL` / `MEDIAGIT_CHUNK_PLY` | Structure-aware chunking walkers for `.blend`/STL/PLY. `0` disables each. | `1` (ON) |
| `MEDIAGIT_AUDIO_TIER` | Audio-format chunk tier. `0` disables. | `1` (ON) |
| `MEDIAGIT_CONTAINER_CHUNK_CAP_MB` | Ceiling (MiB) above which container-format chunking is skipped for plain `StreamCDC`. `0` disables the cap. | `100` |
| `MEDIAGIT_PHASH` | Perceptual image hashing for delta candidacy. `0` disables hashing and image delta. | `1` (ON) |
| `MEDIAGIT_PHASH_MAX_MB` | Max image size (MiB) perceptual-hashed for delta candidacy. | `64` |
| `MEDIAGIT_SIMILARITY_SEED_MAX_CHUNKS` | Max chunks scanned when seeding delta-candidate similarity search. | `256` |
| `MEDIAGIT_CHUNK_CACHE_BYTES` | In-process chunk read cache size. | `268435456` (256 MiB) |
| `MEDIAGIT_CHUNK_WRITE_CONCURRENCY` | Worker threads for parallel chunk writes during `add`. | `num_cpus` (clamped 2–16) |
| `MEDIAGIT_MEDIA_META` | Show `media: ...` summary lines in `status`. `0` disables. | `1` (ON) |
| `MEDIAGIT_CHECKOUT_PARALLELISM` | Worker threads for parallel checkout. `1` forces the sequential loop. | `num_cpus` (capped at 8) |

### Cloud Packs & Transfer

| Variable | Description | Default |
|----------|-------------|---------|
| `MEDIAGIT_CLOUD_PACKS` | Bundle chunks into cloud packs for push/pull instead of per-chunk transfer. | `1` (ON) |
| `MEDIAGIT_PACK_BYTES` / `MEDIAGIT_PACK_CHUNKS` | Byte and chunk-count caps for cloud pack building. | `64 MiB` / `1024` |
| `MEDIAGIT_PACK_BUILDER_CONCURRENCY` | Concurrent pack-build workers. | `2` |
| `MEDIAGIT_PACK_WORKERS` | Concurrent ODB writes while unpacking an incoming push pack on the server. | `8` |
| `MEDIAGIT_PACK_UPLOAD_CONCURRENCY` | Concurrent pack uploads from the client pack builder. | `8` |
| `MEDIAGIT_PACK_PROXY_BATCH` | Batch small chunk fetches through the server proxy. `0` disables. | `1` (ON) |
| `MEDIAGIT_PACK_RANGE_COALESCE_MAX_GAP` / `MEDIAGIT_PACK_RANGE_COALESCE_MAX_BYTES` | Coalescing thresholds for ranged pack GETs. | `1 MiB` / `8 MiB` |
| `MEDIAGIT_REPACK_CHUNKS` | Whether `gc --repack` bundles loose chunks into cloud packs. | `1` (ON) |
| `MEDIAGIT_BATCH_GET_CONCURRENCY` | Server-side semaphore capacity for batched chunk GET requests. | `4` |
| `MEDIAGIT_DISABLE_BATCH_GET` | `1` disables the batch-GET endpoint, forcing per-chunk fallback. | `0` (OFF) |
| `MEDIAGIT_BFS_PARALLELISM` | Parallel BFS width for reachability walks. | `16` |
| `MEDIAGIT_PRESIGN_BATCH` / `MEDIAGIT_PRESIGN_CONCURRENCY` | Batch size and concurrency for presigned-URL requests. | `512` / `64` |
| `MEDIAGIT_REPAIR_EVICT_PACK_ENTRIES` | During `push --repair`, evict stale pack-index entries for repaired objects. | `1` (ON) |
| `MEDIAGIT_BITMAP` | Roaring-bitmap reachability index for `gc`/pack negotiation. `0`/`false`/`off` disables it. | `1` (ON) |

### Cloud Storage Backends

| Variable | Description | Default |
|----------|-------------|---------|
| `MEDIAGIT_MPU_THRESHOLD_BYTES` | Minimum chunk size to use multipart upload. | `10485760` (10 MiB) |
| `MEDIAGIT_MPU_PART_SIZE` | Override the MPU part size for S3/MinIO (must be within [5 MiB, 5 GiB]). | auto-computed |
| `MEDIAGIT_STAGED_UPLOAD` | Use multipart upload for chunks ≥ the threshold. `0` forces single-PUT uploads. | `1` (ON) |
| `MEDIAGIT_AWS_CONNECT_TIMEOUT_SECS` / `MEDIAGIT_AWS_MAX_ATTEMPTS` / `MEDIAGIT_AWS_POOL_IDLE_SECS` / `MEDIAGIT_AWS_POOL_WARM` | AWS SDK connect timeout, retry attempts, pool idle timeout, and startup pool warm count. | `10` / `5` / `90` / `16` |
| `MEDIAGIT_MINIO_MPU_CONCURRENCY` / `MEDIAGIT_MINIO_OP_CONCURRENCY` | Concurrent MinIO MPU creations, and concurrent in-flight `with_retry` operations. | `16` / `64` |
| `MEDIAGIT_AZURE_PUT_BLOCK_CONCURRENCY` | Concurrent block uploads per blob for Azure. | `8` |
| `MEDIAGIT_GCS_UPLOAD_CONCURRENCY` | Upload semaphore capacity for the GCS proxy-path uploader. | `4` |
| `MEDIAGIT_GCS_DISABLE_PRESIGN` | Any value forces GCS transfers through the server proxy, skipping presigned URLs. | unset |

### Server (mediagit-server)

| Variable | Description | Default |
|----------|-------------|---------|
| `MEDIAGIT_METRICS_ADDR` | `host:port` to bind the Prometheus metrics endpoint. | unset (metrics server not started) |
| `MEDIAGIT_ALLOW_INSECURE_BIND` | Bypasses the startup refusal to bind a non-loopback host when auth is disabled. | `0` (OFF) |
| `MEDIAGIT_JWT_SECRET` | JWT signing secret. Env wins over config file `jwt_secret` if both set. | none |
| `MEDIAGIT_STARTUP_PROBE` | Server scans `repos_dir` for repo health at startup. `0` skips it. | `1` (ON) |
| `MEDIAGIT_AUTH_PERSIST` | Persist auth state (users/grants) to disk. `0` keeps auth in-memory only. | `1` (ON) |
| `MEDIAGIT_GRANTS_ENFORCE` | Per-repo permission grants. `0` = off everywhere (flat roles only); `strict` = enforce on every repo, so one with no grants denies instead of falling back; unset = per repo (enforced only on repos that have grants). | unset (per repo) |
| `MEDIAGIT_LOCKS_ENFORCE` | Server rejects pushes that touch paths locked by another user. | `1` (ON) |

> **Removed from this table (DC-5).** `MEDIAGIT_APP_*`, `MEDIAGIT_LOG_LEVEL`,
> `MEDIAGIT_METRICS_ENABLED` / `_PORT`, `MEDIAGIT_COMPRESSION_ENABLED` /
> `_LEVEL`, `MEDIAGIT_MAX_CONCURRENCY`, `MEDIAGIT_BUFFER_SIZE` and
> `MEDIAGIT_HTTPS_ENABLED` / `MEDIAGIT_AUTH_ENABLED` were documented here but
> have never done anything. Two independent reasons, either sufficient:
> `ConfigLoader::apply_env_overrides` — the only code that reads them — is
> never called on any load path, and the `[app]`, `[observability]`,
> `[compression]` and `[performance] max_concurrency` fields it writes are
> read nowhere outside `mediagit-config`'s own tests. The server's real
> settings live in `ServerConfig` (`mediagit-server.toml`), a different type
> these variables do not touch.
>
> Set the corresponding key in `mediagit-server.toml` instead. Every variable
> still listed above is read directly by the code that acts on it.
| `MEDIAGIT_LOCKS_MAX_COMMITS` | Max commits walked when computing touched paths for lock enforcement. | `1000` |

### GC / Housekeeping

| Variable | Description | Default |
|----------|-------------|---------|
| `MEDIAGIT_GC_REFLOG_HORIZON_DAYS` | Reflog entries older than this stop being GC roots. `0` disables reflog roots. | `90` |
| `MEDIAGIT_NO_AUTO_GC` | Any value disables auto-gc for the current invocation. | unset (auto-gc ON) |
| `MEDIAGIT_REFLOG_MAX` | Max reflog entries retained per ref. | `1000` |
| `MEDIAGIT_SIGN` / `MEDIAGIT_SIGN_KEY` | Sign tags with SSH/ed25519 (`tag -a`), and the key path to sign/verify with. | `0` (OFF) / `~/.ssh/id_ed25519` |

## Observability

| Variable | Description | Default |
|----------|-------------|---------|
| `RUST_LOG` | Log filter directive (e.g., `mediagit=debug`, `info`) | `info` |
| `RUST_LOG_FORMAT` | Log output format: `json` or `text` | `json` |

### Log Filter Examples

```bash
# Show all debug logs
export RUST_LOG=debug

# Show debug for mediagit only, info for everything else
export RUST_LOG=mediagit=debug,info

# Show trace for a specific crate
export RUST_LOG=mediagit_versioning=trace

# Human-readable logs (development)
export RUST_LOG_FORMAT=text mediagit add file.psd
```

## Cargo / Build (Development)

| Variable | Description |
|----------|-------------|
| `CARGO_TERM_COLOR` | Force color output: `always`, `never`, `auto` |
| `RUST_BACKTRACE` | Enable Rust backtraces: `1` or `full` |

## Integration Test Variables

Used by the CI integration test job and local integration testing:

| Variable | Value for local testing |
|----------|------------------------|
| `AWS_ACCESS_KEY_ID` | `minioadmin` |
| `AWS_SECRET_ACCESS_KEY` | `minioadmin` |
| `AWS_ENDPOINT_URL` | `http://localhost:9000` |
| `AWS_REGION` | `us-east-1` |
| `AZURE_STORAGE_CONNECTION_STRING` | `DefaultEndpointsProtocol=http;AccountName=devstoreaccount1;AccountKey=Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/KBHBeksoGMGw==;BlobEndpoint=http://localhost:10000/devstoreaccount1;` |
| `GCS_EMULATOR_HOST` | `http://localhost:4443` |

See [Development Setup](../contributing/development.md#integration-tests-requires-docker) for running integration tests locally.

## Precedence Summary

For each setting, MediaGit resolves values in this order (first match wins):

1. CLI flag (e.g., `--author "Name <email>"`)
2. Environment variable (e.g., `MEDIAGIT_AUTHOR_NAME`)
3. Repository config (`.mediagit/config.toml`)
4. Built-in default

## See Also

- [Configuration Reference](./config.md) — `config.toml` file format
- [Storage Backend Configuration](../guides/storage-config.md) — backend-specific setup

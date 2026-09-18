# MediaGit Environment Knobs

All tunables are read from environment variables at process start.
Boolean knobs: `"1"` = on, `"0"` = off, absent = default.

Lifecycle: **experimental** → **stable** (2 clean deep-test releases) → **default ON** (1 more release) → **removed** (1 more release if nobody flips it OFF).

| Knob | Default | Introduced | Scope | Status | Description |
|------|---------|-----------|-------|--------|-------------|
| `MEDIAGIT_PULL_PIPELINE` | `1` (ON) | 0.2.6-beta | pull/clone | stable | Overlap manifest fetches and chunk downloads (B1). Cap with `MEDIAGIT_PULL_MANIFEST_CONCURRENCY`. |
| `MEDIAGIT_PULL_MANIFEST_CONCURRENCY` | `8` | 0.2.6-beta | pull/clone | stable | Max concurrent manifest fetches when `PULL_PIPELINE=1`. Caps aggregate in-flight against `DOWNLOAD_CONCURRENCY`. |
| `MEDIAGIT_DOWNLOAD_CONCURRENCY` | `32` / `24` | 0.2.5 | pull/clone | stable | Max concurrent chunk GET requests. **Two defaults, deliberately:** `32` on the per-chunk path (`pull.rs::pull_streaming`), `24` on the pack range-GET path (`packs.rs`). Setting the variable makes both agree; leaving it unset does not. Measured 2026-08-25 over loopback — 3.67s vs 3.57s, inside noise, and `util_pct=2%` says this path is not concurrency-limited at all. Unifying would be churn. |
| `MEDIAGIT_CLONE_OVERLAP` | `1` (ON) | 0.3.0-rc.4 | clone | stable | Write the working tree *while* chunked media is still downloading, instead of waiting for the last byte (C4). Small blobs are already in the ODB when `pull_streaming` returns, so only chunk-backed files wait. Set `=0` for the old serial path — that is both the revert switch and the parity oracle the clone-parity test diffs against. Reported explicitly as `overlap=on\|off` on the `[bench] op=checkout` line. |
| `MEDIAGIT_FETCH_BRANCH_CONCURRENCY` | `4` | 0.2.6-beta | fetch | stable | Max branches fetched in parallel (B3). Ref-write barrier still serialized at fs layer. |
| `MEDIAGIT_DECOMPRESS_BLOCKING` | `1` (ON) | 0.2.6-beta | pull/clone | stable | Offload decompression to `spawn_blocking` for chunks ≥ threshold (B6). Prevents blocking async executor. |
| `MEDIAGIT_DECOMPRESS_BLOCKING_THRESHOLD` | `262144` | 0.2.6-beta | pull/clone | stable | Byte size above which decompression is offloaded (B6). Avoids net-negative dispatch cost on tiny chunks. |
| `MEDIAGIT_PUSH_PIPELINE` | `1` (ON) | 0.2.6-beta | push | stable | Cross-object push pipeline (B2). Combine with `PUSH_OBJECT_CONCURRENCY`. Set `=0` to revert. Parity matrix 2/2 on MinIO/AWS/Azure. |
| `MEDIAGIT_PUSH_OBJECT_CONCURRENCY` | `8` | 0.2.6-beta | push | stable | Max concurrent object uploads when `PUSH_PIPELINE=1` (B2). Pool-blowout risk at `UPLOAD_CONCURRENCY=32`. |
| `MEDIAGIT_UPLOAD_CONCURRENCY` | `32` | 0.2.5 | push | stable | Max concurrent chunk PUT requests. |
| `MEDIAGIT_STREAM_CHUNK_TO_DISK` | `1` (ON) | 0.2.6-beta | pull/clone | stable | Stream chunk body directly to ODB file instead of buffering in RAM (B4). Set `=0` to revert. Windows stress + MinIO/AWS/Azure 148/148 PASS. |
| `MEDIAGIT_STORAGE_STREAMING` | `1` (ON) | 0.2.6-beta | pull/clone/server | stable | Use streaming GET on S3/MinIO instead of `Vec<u8>` round-trip (B7). Set `=0` to revert. AWS clone 15.8% faster (159.8s→134.5s). **Correction (0.3.0-rc.4):** between B7 and C2 this knob was DEAD on the server — `get_streaming` was implemented and overridden, but no request-serving code called it, so the server buffered every whole chunk in RAM regardless of the setting. C2 wired it into `download_chunk`, which is what makes the knob load-bearing. A streamed chunk response carries no `Content-Length` (chunked transfer-encoding); the client reads it with `resp.bytes()`, which handles that. **Backends that actually stream:** S3, MinIO, Azure, and B2/Spaces (delegated as part of C2 — before that the wrapper inherited the buffering default and cancelled the fix for those providers); the `namespaced` layout-v2 wrapper delegates and so preserves it. **Local and GCS use the buffering default** (GCS implements only `get_streaming_range`), so a local-backend server shows no memory change — do not use one to test this knob. |
| `MEDIAGIT_HTTP_POOL_MAX` | `64` | 0.2.6-beta | all | stable | Max idle HTTP connections per host (B8). Single source of truth; previously inconsistent across 108/1386/2122. |
| `MEDIAGIT_STAGED_UPLOAD` | `1` (ON) | 0.2.5 | push | stable | Use multipart upload (MPU) for chunks ≥ `MPU_THRESHOLD_BYTES` on S3/MinIO. Set `=0` to force single-PUT uploads. **Correction (0.3.0-rc.4 audit):** code default is ON (`!= "0"`); in-code comments still say "active when `=1`" — comment is stale, behavior is ground truth. |
| `MEDIAGIT_MPU_THRESHOLD_BYTES` | `10485760` | 0.2.5 | push | stable | Minimum chunk size (bytes) to use multipart upload. Below this threshold uses single PUT. |
| `MEDIAGIT_AWS_POOL_IDLE_SECS` | `90` | 0.2.5 | push/pull | stable | Pool idle timeout in seconds for AWS SDK HTTP connections. |
| `MEDIAGIT_AWS_POOL_WARM` | `16` (ON) | 0.2.5 | startup | experimental | Pre-warm the shared HTTP pool at backend startup so the first MPU burst finds warm TCP sessions. Fire-and-forget; failures are harmless. `0` disables. |
| `MEDIAGIT_AWS_CONNECT_TIMEOUT_SECS` | `5` | 0.2.5 | all | stable | TCP connect timeout for AWS SDK calls. |
| `MEDIAGIT_AWS_MAX_ATTEMPTS` | `5` | 0.2.5 | all | stable | Max retry attempts for AWS SDK calls (exponential backoff). |
| `MEDIAGIT_MINIO_MPU_CONCURRENCY` | `16` | 0.2.5 | push | stable | Semaphore capacity for concurrent MinIO MPU operations. |
| `MEDIAGIT_BENCH` | `0` (OFF) | 0.2.6-beta | all | stable | Emit `[bench]` lines to stderr with schema_version=2 (A9/B9). Capture with `2>file.tsv`; compare with `diff_bench.ps1` (`dev-tests/deep-tests/diff_bench.ps1` — gitignored, local-only, not in a fresh clone). |
| `MEDIAGIT_CDC_SEED` | repo config `cdc_seed` (`0` for legacy repos) | 0.2.8-beta | add/chunking | stable | Seeds CDC chunk boundaries per-repo. `0` forces legacy unseeded boundaries. |
| `MEDIAGIT_CODEC_DETECT` | `1` (ON) | 0.2.8-beta | add/chunking | stable | Detects chunk codec for compression routing. `0` forces all chunks to `CodecHint::Unknown` (pre-detection behavior). |
| `MEDIAGIT_PHASH` | `1` (ON) | 0.2.8-beta | add | stable | Computes perceptual image hashes for delta candidacy. `0` disables hashing, `phash.idx`, and image delta. |
| `MEDIAGIT_REPO_NAMESPACE` | repo config `repo_namespace` (else sanitized repo-dir basename) | 0.2.8-beta | all | stable | Per-repo namespace prefix for object-store layout v2. No "off" value — always resolves to a namespace. |
| `MEDIAGIT_TOKEN` | none | 0.2.8-beta | all | stable | Bearer token for client auth (`Authorization` header). Absent ⇒ no header sent. **Highest** precedence tier: env beats per-remote config, which beats the OS keychain (repo.rs `resolve_credentials_tiered`). |
| `MEDIAGIT_API_KEY` | none | 0.2.8-beta | all | stable | API key for client auth (`X-Api-Key` header). Absent ⇒ no header sent. Mutually exclusive with `MEDIAGIT_TOKEN` (token wins if both set). |
| `MEDIAGIT_CHECKOUT_PARALLELISM` | `num_cpus` capped at 8 | 0.2.8-beta | checkout | stable | Worker threads for parallel checkout. Set to `1` for the old fully-sequential per-file loop. |
| `MEDIAGIT_BITMAP` | `1` (ON) | 0.2.8-beta | push/pull | stable | Roaring-bitmap reachability index for `gc`/pack negotiation. `0`/`false`/`off` disables generation and consumption (falls back to BFS walk). |
| `MEDIAGIT_SIGN` | `0` (OFF) | 0.2.8-beta | tag | experimental | Sign tags with SSH/ed25519 (`tag -a`). Opt-in — off leaves `Tag.signature` `None` and `tag verify` reports "unsigned". |
| `MEDIAGIT_SIGN_KEY` | `~/.ssh/id_ed25519` | 0.2.8-beta | tag | stable | SSH key path used for both signing and self-verification. |
| `MEDIAGIT_REFLOG_MAX` | `1000` | 0.2.8-beta | reflog | stable | Max reflog entries retained per ref. No "unlimited" value — set a large number instead. |

## Add / Chunking / Delta

| Knob | Default | Introduced | Scope | Status | Description |
|------|---------|-----------|-------|--------|-------------|
| `MEDIAGIT_ADD_MAX_INFLIGHT_BYTES` | `536870912` (512 MiB) | 0.3.0-rc.4 | add | stable | Global in-flight byte budget for parallel file processing during `add`. |
| `MEDIAGIT_HASH_PARALLEL` | `1` (ON) | 0.3.0-rc.4 | add | stable | Parallel mmap-based hashing for files ≥ the streaming threshold (5 MiB). `0` forces sequential hashing. |
| `MEDIAGIT_ADD_COMPRESS_BLOCKING` | `1` (ON) | 0.3.0-rc.4 | add/chunking | stable | Offload add-time chunk compression to `spawn_blocking` for chunks ≥ threshold. `0` reverts to inline compression. |
| `MEDIAGIT_ADD_COMPRESS_BLOCKING_THRESHOLD` | `262144` (256 KiB) | 0.3.0-rc.4 | add/chunking | stable | Byte size above which add-time compression is offloaded to `spawn_blocking`. |
| `MEDIAGIT_CHUNK_FBX` | unset/`0` (OFF) | 0.3.0-rc.4 | add/chunking | experimental | Opt-in structure-aware FBX chunking walker. Measured net-negative vs. generic CDC on real files ([[project_smart_media_execution_2026_07_07]]) — set `=1` to opt in. |
| `MEDIAGIT_CHUNK_BLEND` | `1` (ON) | 0.3.0-rc.4 | add/chunking | stable | Structure-aware Blender `.blend` chunking walker. `0` disables (falls back to generic CDC). |
| `MEDIAGIT_CHUNK_STL` | `1` (ON) | 0.3.0-rc.4 | add/chunking | stable | Structure-aware STL chunking walker. `0` disables. |
| `MEDIAGIT_CHUNK_PLY` | `1` (ON) | 0.3.0-rc.4 | add/chunking | stable | Structure-aware PLY chunking walker. `0` disables. |
| `MEDIAGIT_AUDIO_TIER` | `1` (ON) | 0.3.0-rc.4 | add/chunking | stable | Audio-format chunk tier. `0` disables (falls back to generic CDC). |
| `MEDIAGIT_CONTAINER_CHUNK_CAP_MB` | `100` | 0.3.0-rc.4 | add/chunking | stable | Byte-size ceiling (MiB) above which container-format chunking (mmap walker) is skipped in favor of plain `StreamCDC`, to bound peak heap use. `0` disables the cap. |
| `MEDIAGIT_SIMILARITY_SEED_MAX_CHUNKS` | `256` | 0.3.0-rc.4 | add/delta | stable | Max chunks scanned when seeding delta-candidate similarity search. |
| `MEDIAGIT_PHASH_MAX_MB` | `64` | 0.3.0-rc.4 | add | stable | Max image size (MiB) perceptual-hashed for delta candidacy; larger images skip pHash. |
| `MEDIAGIT_MEDIA_META` | `1` (ON) | 0.3.0-rc.4 | status | stable | Show `media: ...` summary lines in `status` for recognized media files. `0` disables. |
| `MEDIAGIT_CHUNK_CACHE_BYTES` | `268435456` (256 MiB) | 0.3.0-rc.4 | odb | stable | In-process chunk read cache size. |
| `MEDIAGIT_CHECKOUT_READAHEAD_BYTES` | `16777216` (16 MiB) | 0.4.0-rc.1 | checkout/odb | stable | Per-file read-ahead budget during checkout, in bytes. Turned into a concurrent-read count for each file, so the worst case across 8 concurrent files is ~128 MB -- the same ceiling the pre-pipelining code had. `0` and unparseable values fall back to the default. |
| `MEDIAGIT_CHUNK_WRITE_CONCURRENCY` | `num_cpus` (clamped 2–16 at the call site) | 0.3.0-rc.4 | add/odb | stable | Worker threads for parallel chunk writes during `add`. There is no repo-config equivalent — `[performance] chunk_write_concurrency` was removed from `schema.rs` in v0.4.0 as a dead TOML key; this env var is the only way to set it. |
| `MEDIAGIT_DELTA_LEVEL` | `19` | 0.3.0-rc.4 | add/commit | stable | zstd dictionary compression level for delta encoding. Valid `1`-`22`; anything outside that range is rejected with a warning and the default used. |
| `MEDIAGIT_ODB_CACHE_MB` | `512` | 0.3.0-rc.4 | all | stable | Object-database in-memory cache size, in MiB. Unparseable values fall back to the default rather than to zero. |
| `MEDIAGIT_S3_ATTEST` | unset (on for `*.amazonaws.com`, off elsewhere) | 0.4.0-rc.1 | storage/s3 | stable | Escape hatch for provider upload attestation on the S3-compatible driver. `off`/`0`/`false` forces it off; `on`/`1` forces CRC64NVME on a non-AWS endpoint that supports it; `crc32c` selects that algorithm. NOT tuning: with attestation off the post-push read-back stays, so correctness is unchanged either way. |

## Cloud Packs (Track F)

| Knob | Default | Introduced | Scope | Status | Description |
|------|---------|-----------|-------|--------|-------------|
| `MEDIAGIT_CLOUD_PACKS` | `1` (ON) | 0.3.0-rc.4 | push/pull | stable | Use cloud-pack bundling for push/pull instead of per-chunk transfer. `0` reverts to per-chunk. |
| `MEDIAGIT_PACK_BYTES` | `67108864` (64 MiB) | 0.3.0-rc.4 | packs | stable | Byte-size cap per cloud pack (client pack builder and server-side chunk repacking). |
| `MEDIAGIT_PACK_CHUNKS` | `1024` | 0.3.0-rc.4 | packs | stable | Chunk-count cap per cloud pack. |
| `MEDIAGIT_PACK_BUILDER_CONCURRENCY` | `2` | 0.3.0-rc.4 | packs | stable | Concurrent pack-build workers in the streaming pack writer. |
| `MEDIAGIT_PACK_WORKERS` | `8` | 0.3.0-rc.4 | packs/server | stable | Concurrent ODB writes while unpacking an incoming push pack on the server. Priority: env > repo config `[performance] pack_workers` > default. |
| `MEDIAGIT_PACK_UPLOAD_CONCURRENCY` | `8` | 0.3.0-rc.4 | packs | stable | Concurrent pack uploads from the client pack builder. |
| `MEDIAGIT_PACK_PROXY_BATCH` | `1` (ON) | 0.3.0-rc.4 | packs | stable | Batch small chunk fetches through the server proxy instead of per-chunk presigned GETs. `0` disables. |
| `MEDIAGIT_PACK_RANGE_COALESCE_MAX_GAP` | `1048576` (1 MiB) | 0.3.0-rc.4 | packs | stable | Max byte gap between chunk ranges to coalesce into one ranged pack GET. |
| `MEDIAGIT_PACK_RANGE_COALESCE_MAX_BYTES` | `8388608` (8 MiB) | 0.3.0-rc.4 | packs | stable | Max coalesced-range size for a single ranged pack GET. |
| `MEDIAGIT_REPACK_CHUNKS` | `1` (ON) | 0.3.0-rc.4 | gc | stable | Whether `gc --repack` bundles loose chunks into cloud packs. `0` restores per-chunk-object repacking. |
| `MEDIAGIT_PACK_VERIFY_CONCURRENCY` | `16` | 0.3.0-rc.3 | packs/server | stable | Per-entry range-read concurrency for background content-verification INSIDE one pack (`pack_entries_failing_content_verification`, `handlers/repo.rs:1163`). Any positive integer; unset/unparseable/`0` falls back to `16`. Distinct axis from `MEDIAGIT_PACK_VERIFY_PACK_CONCURRENCY` below — until 2026-08-20 one name drove both, at two different defaults; setting this one now only logs a warning pointing at the other if the pack-level axis is what you meant to change. |
| `MEDIAGIT_PACK_VERIFY_PACK_CONCURRENCY` | `1` | 0.3.0-rc.3 | packs/server | stable | How many packs verify concurrently, global (not per-repo) semaphore (`pack_verify_semaphore`, `handlers/repo.rs:1485`). Any positive integer; unset/unparseable/`0` falls back to `1`. Deliberately serialised: raising it to `16` measured **4.5x worse** per-pack verification (mean 12.7s→56.6s over a 512 MB GCS payload) by thrashing the shared WAN link — more permits is the intuitive fix for a clone blocked on verification and it is the wrong one. |
| `MEDIAGIT_PACK_VERIFY_BUDGET_SECS` | `300` | 0.3.0-rc.3 | packs/server | stable | Wall-clock ceiling for ONE attempt at verifying ONE pack (`pack_verify_budget`, `handlers/repo.rs:1448`). Any positive integer (seconds); unset/unparseable/`0` falls back to `300`. Sized from measurement: healthy GCS per-pack verification on this link class ran mean 12.7s/worst 35.5s over 512 MB, worst-in-a-healthy-run 103s over 2 GB; the pathological case that motivated this was 2628s. Exceeding it is not an error — the pack is simply left unverified for a later attempt, same outcome as the pre-existing "unreadable after retries" path. |
| `MEDIAGIT_PACK_ATTEST_SKIP_READBACK` | `1` (ON) | 0.4.0 | packs/server | stable | Whether a pack the PROVIDER attested may skip the post-push read-back (`handlers/repo.rs`). `0` forces the read-back even for an attested pack, restoring pre-attestation behaviour exactly. Attestation means the backend validated a checksum of the assembled object at upload (S3 full-object CRC64NVME; GCS crc32c compared against the client's folded value), so re-reading the pack to confirm storage integrity is redundant. On a 16 GB push that read-back was 10.03 GB pulled back out of the bucket. Only a POSITIVE answer from the provider skips it: an unknown backend, a missing digest, a HEAD error or a permission gap all read as "not attested" and the read-back runs. This does NOT govern whether a pack's contents match its manifest -- that is enforced on the read path (`slice_verifies`, `put_compressed_chunk`) and is unaffected by this knob. |
| `MEDIAGIT_PACK_SCRUB_INTERVAL_SECS` | `300` | 0.4.0 | packs/server | experimental | Seconds between background content-scrub ticks; each tick verifies at most ONE provider-attested pack against its manifest (`main.rs`, `scrub_one_attested_pack`). `0` disables it. It costs bucket egress -- ~10 GB for a 16 GB repository -- but that is not an ADDED cost: before 0.4.0 the server read every pushed pack back at push time, the same bytes. The scrub relocates that read off the push's critical path rather than introducing it, which is why it is on by default: turning it off makes 0.4.0 verify LESS than 0.3 did, since an attested pack nobody reads would then never be content-checked. The scrub skips a tick while the data plane is busy or another pack verification holds the verify permit, so it yields to live transfers rather than competing with them. **Costs bucket egress**: scrubbing a 16 GB repository reads ~10 GB back out of the bucket once. Measured at a 20s interval over 217 packs it added no measurable push time (+2.9%, inside the control arm's own spread) and ~0 MB peak RSS -- so the reason it is off is egress, not throughput. **When to disable:** the read path catches a bad pack the moment anyone reads it, so the scrub buys EARLIER detection, not detection. If your repositories are read often and you would rather not pay the egress, `0` is a legitimate trade. Keep it on where the bucket is the only copy and reads are rare -- archival repositories, cold media -- because corruption found while a local source still exists is re-pushable, while the same corruption found years later at first read is data loss. |
| `MEDIAGIT_PACK_RANGE_GET_RETRY_BUDGET_SECS` | `120` | 0.4.0-rc.1 | packs/clone | stable | Wall-clock budget for retrying ONE ranged pack GET through transport errors, non-206 statuses and short bodies (`pack_range_get_retry_budget`, `client/packs.rs`). Elapsed-time, not an attempt count, and measured from the first attempt so a slow failing attempt spends its own time -- the same shape as `MEDIAGIT_PACK_PUT_RETRY_BUDGET_SECS`. Permanent outcomes (404, 403) bail immediately: falling back to per-chunk is correct for a genuinely absent or forbidden object. An expired signature also bails, because this path holds no server handle to re-presign with; the per-chunk fallback re-presigns naturally. Exhausting the budget now costs only THAT pack its fast path -- until 0.4.0 a single pack's failure dropped the whole clone to per-chunk. |

## Transfer / Server-side Concurrency

| Knob | Default | Introduced | Scope | Status | Description |
|------|---------|-----------|-------|--------|-------------|
| `MEDIAGIT_BATCH_GET_CONCURRENCY` | `4` | 0.3.0-rc.4 | server | stable | Server-side semaphore capacity for batched chunk GET requests. |
| `MEDIAGIT_DISABLE_BATCH_GET` | `0` (OFF) | 0.3.0-rc.4 | server | stable | `1` disables the batch-GET endpoint (returns 404), forcing per-chunk fallback. |
| `MEDIAGIT_BFS_PARALLELISM` | `16` | 0.3.0-rc.4 | server | stable | Level-by-level parallel BFS width for reachability walks (`/browse`, negotiation fallback). |
| `MEDIAGIT_PRESIGN_BATCH` | `512` | 0.3.0-rc.4 | push/pull | stable | Chunk IDs per batch when requesting presigned URLs. |
| `MEDIAGIT_PRESIGN_CONCURRENCY` | `64` | 0.3.0-rc.4 | server | stable | Concurrent presigned-URL generation calls to the storage backend. |
| `MEDIAGIT_404_FALLBACK_DELAY_MS` | `500` | 0.3.0-rc.4 | protocol | stable | Delay before falling back to the proxy path after a presigned-URL 404. |
| `MEDIAGIT_REPAIR_EVICT_PACK_ENTRIES` | `1` (ON) | 0.3.0-rc.4 | server | stable | During `push --repair`, evict stale pack-index entries for repaired objects. `0` disables. |
| `MEDIAGIT_RANGE_PARALLEL` | `4` | 0.3.0-rc.4 | pull | stable | Parallel ranged-GET fan-out for large single-chunk downloads. |
| `MEDIAGIT_RANGE_PARALLEL_THRESHOLD` | `4194304` (4 MiB) | 0.3.0-rc.4 | pull | stable | Chunk size above which `RANGE_PARALLEL` fan-out kicks in. |
| `MEDIAGIT_DOWNLOAD_DIRECT_DISABLE` | `0` (OFF) | 0.3.0-rc.4 | pull | stable | `1` disables direct presigned-URL downloads, forcing all chunk GETs through the server proxy. |
| `MEDIAGIT_PUSH_DEADLINE_SECS` | `3600` | 0.3.0-rc.4 | push | stable | Overall wall-clock deadline for a single push operation. |
| `MEDIAGIT_PULL_DEADLINE_SECS` | `3600` | 0.3.0-rc.4 | pull/fetch/clone | stable | Absolute wall-clock deadline for a download, applied per phase so the error names the phase that ran out. How you fail fast against a dead backend. `0` or unparseable falls back to the default. |
| `MEDIAGIT_GCS_DISABLE_MPU` | unset | 0.4.0-rc.1 | push, GCS | stable | Set to anything to make the GCS backend decline presigned multipart, sending every pack as a single all-or-nothing PUT instead. That is what GCS did before 0.4.0-rc.1, and it costs both retry granularity (a failure at 63 of 64 MiB re-sends all 64) and memory — measured 1,075.7 MB peak client working set on a 10.03 GB push against 288.9 MB with multipart on. An escape hatch for a bucket or emulator whose XML API misbehaves, not a tuning knob. `MEDIAGIT_GCS_DISABLE_PRESIGN` also disables it, since multipart is built on the same signer. |
| `MEDIAGIT_AZURE_DISABLE_MPU` | unset | 0.4.0-rc.1 | push, Azure | stable | Set to anything to make the Azure backend decline its block-blob staged upload (Put Block / Put Block List), falling back to a single PUT per pack. Same trade as the GCS knob: measured 1,075.2 MB peak client working set with it disabled against 293.0 MB with it on, plus all-or-nothing retries. An escape hatch, not a tuning knob. |
| `MEDIAGIT_CHUNK_OPEN_RETRIES` | `2` | 0.4.0-rc.1 | server, chunk serving | stable | How many times the SERVER re-opens a chunk whose read fails transiently, before answering 503. `get_streaming` resolves as soon as a backend hands back a stream handle — no byte has been read — so an upstream read that dies at offset 0 used to reach the client as a *truncated 200* it had to diagnose from broken chunked framing. `open_chunk_stream` now proves a byte is readable before committing to a status, and retries the open itself. Deliberately SMALL (2 retries, 250ms then 500ms): this path runs when the server is already degraded, and a generous loop ties up connections exactly when they are scarcest — the client supplies the patience via `MEDIAGIT_CHUNK_GET_SEND_RETRIES`. It also makes the client’s small 503 budget honest: that budget assumes "the server already exhausted its own storage retries", which was false before this knob existed, because no backend’s `get_streaming` retried anything. Not-found is never retried and never becomes 503 — it routes to the pack-index and chunk-delta fallbacks. |
| `MEDIAGIT_CHUNK_GET_SEND_RETRIES` | `8` | 0.3.0-rc.5 | pull/clone | stable | Max retries for a chunk GET whose **request never completed** — connection refused, reset, or reused after the peer closed it. Deliberately separate from, and larger than, the 3 retries a 5xx gets: a 503 means the server already exhausted its own storage retries, so asking again mostly delays a real error, whereas a transport failure means nothing was ever established and is usually brief. Measured (ga47): an aws clone died to 65 `send()` failures over loopback against an *idle* server, 12 of them exhausting the old shared budget of 3 — under 7s of patience against a condition lasting minutes, while recovery was demonstrably available. The exponential floor is capped at 8s, so 8 retries is ~40s per chunk, well inside `MEDIAGIT_PULL_DEADLINE_SECS`. Lower it only to make a clone fail faster against a genuinely dead peer. |
| `MEDIAGIT_RATE_LIMIT_RETRIES` | `10` | 0.3.0-rc.4 | push/control-plane | stable | Max HTTP 429 retries for a single control-plane request. Honours the server's `Retry-After` header when present, otherwise backs off exponentially. Relevant when a large push (one control-plane request per chunk) outruns a server-side rate limiter. |
| `MEDIAGIT_RATE_LIMIT_MAX_WAIT_SECS` | `60` | 0.3.0-rc.4 | push/clone/control-plane | stable | Hard ceiling on a single 429 backoff, however large the server's `Retry-After` is. Without it the honoured value was unbounded: a `Retry-After: 900` measured 965,668ms for one attempt and 13.08 hours across the retry budget, during which the client sits idle and silent while the server stays healthy — indistinguishable from a hang. Raise it only if a legitimate backend genuinely needs longer than a minute between retries. |
| `MEDIAGIT_SHORT_REQUEST_ATTEMPTS` | `6` | 0.3.0-rc.5 | push/clone/control-plane | stable | How many bounded, fresh-connection attempts a short control-plane request gets before falling through to the final unbounded one. **Was 2; raised after ga49 measured 2 as too few.** A push failed after 720.1s with `Failed to send GET /info/refs: operation timed out` — the stall outlived both bounded attempts, and the unbounded fallback then inherited a third stalled connection and rode it to the 300s `MEDIAGIT_CONTROL_READ_TIMEOUT_SECS` (30+30+300 = 360s per request, twice = 720s). Server-side counters showed `accepted` climbing while `routed` stayed frozen: connections accepted, no request ever reaching the router. Six attempts give a stall ~3 minutes of fresh-connection chances instead of ~1. Worst case for a request that fails anyway grows 360s → 480s. Mitigation, not a cure — the underlying stall is still unexplained. `0` is clamped to `1`. |
| `MEDIAGIT_CONTROL_READ_TIMEOUT_SECS` | `300` | 0.3.0-rc.4 | push/clone/control-plane | stable | Max seconds with NO bytes received on a control-plane request before it fails. This is a READ (inter-byte) timeout, not a total request timeout - a slow but progressing transfer resets it on every byte, so it does not reintroduce the total-`.timeout()` regression that broke healthy slow Azure/S3 uploads. Catches what `tcp_keepalive` cannot: a peer that is ALIVE but has stopped ANSWERING. Set `0` to disable and restore the old unbounded behaviour. |
| `MEDIAGIT_SHORT_REQUEST_DEADLINE_SECS` | `30` | 0.3.0-rc.4 | push/clone/control-plane | stable | Seconds a SHORT control-plane request (no body, tiny response — e.g. `GET /info/refs`) gets to produce response headers before that attempt is abandoned and retried. `MEDIAGIT_SHORT_REQUEST_ATTEMPTS` bounded attempts, then a final UNBOUNDED one, so a genuinely slow server is never newly broken by this — it is still covered by `MEDIAGIT_CONTROL_READ_TIMEOUT_SECS`. Bounds the stall in which a connection is accepted and then answered by nobody; it does not explain that stall. `0` disables the bound entirely. |
| `MEDIAGIT_PACK_PUT_RETRY_BUDGET_SECS` | `120` | 0.3.0-rc.4 | push/cloud-packs | stable | Wall-clock budget for retrying ONE presigned pack PUT through transport and transient-status failures. Replaces a 5-attempt bound whose equal-jitter backoff summed to 1.9-3.75s -- i.e. a pack upload could survive under four seconds of link trouble. 20260901-ga38 exhausted it five times, with the gcs arm dropping to 97 per-chunk proxy PUTs. An outage is measured in seconds-to-minutes, so the bound that matters is wall-clock, not a count; backoff now grows to a 30s ceiling instead of ~2s. Safe to retry at length: the PUT targets a content-addressed key with the same bytes, so re-sending is idempotent, and a healthy link never reaches the retry path. `0` restores the old 5-attempt behaviour. |
| `MEDIAGIT_CONNECT_TIMEOUT_SECS` | `10` | 0.3.0-rc.4 | push/pull/clone/control-plane | stable | TCP+TLS connect timeout for BOTH protocol clients (control plane and the presigned data plane). Neither set one before, so a connect to a black-holed peer fell back to the OS default -- ~21s of SYN retries on Windows. `minio.rs` already carries the measured version of this bug for the AWS SDK path ("a stalled TCP can cost ~30 s/attempt x default-3 retries = ~90 s/op ... compounded to the 10-15 min outages we saw"); the fix was never applied to the reqwest clients. This is what makes a retry BUDGET effective: an unbounded connect turns ~20 fast attempts into ~5 slow ones. 10s rather than MinIO's 5s because this path includes cross-region TLS handshakes. Bounds the CONNECT only -- a slow but progressing transfer is governed by the read timeouts. `0` restores the OS default. |
| `MEDIAGIT_LOG` | unset | 0.3.0-rc.4 | cli/diagnostics | stable | Tracing filter for the CLI (e.g. `debug`, or `mediagit_protocol=debug`). Unset means SILENT - default output is unchanged, which matters because the QA harness parses this CLI's stdout. Logs go to stderr. Takes precedence over `RUST_LOG` so MediaGit diagnostics can be enabled without inheriting another tool's setting. Use this first when a client hangs: without it the CLI cannot say what it is waiting on. |
| `MEDIAGIT_LOG_FORMAT` | `full` (server), `pretty` (CLI) | 0.4.0-rc.1 | cli+server/diagnostics | stable | Log OUTPUT FORMAT, distinct from the filter above: `full`, `pretty`, `compact`, `json`. Overrides `log_format` in `mediagit-server.toml`; the CLI's `--log-format` flag wins over it. An unrecognised value is a hard error, not a silent fallback to the default, because a script that asked for `json` and quietly got `pretty` produces a log nothing can parse. The server default is `full` -- tracing's own single-line format, which is what it emitted before `mediagit-observability` was wired in -- and NOT `pretty`, which is the multi-line renderer; that distinction is load-bearing because the QA harness parses the server log. |
| `MEDIAGIT_PUSH_CHUNK_CONCURRENCY` | computed (`64 / PUSH_OBJECT_CONCURRENCY`, min 4) | 0.3.0-rc.4 | push | stable | Per-object chunk upload concurrency when `PUSH_PIPELINE=1`; targets ~64 total in-flight PUTs. |
| `MEDIAGIT_FETCH_DOWNLOAD_CONCURRENCY` | computed (`DOWNLOAD_CONCURRENCY` split across branches, floor per branch) | 0.3.0-rc.4 | fetch | stable | Per-branch chunk download concurrency when fetching multiple branches in parallel (`fetch --all`). Overrides the computed default. |
| `MEDIAGIT_STRONG_VERIFY` | `0` (OFF) | 0.3.0-rc.4 | push | stable | `1` runs a full BLAKE3 re-hash verification of pushed chunks after transfer. `push --repair` always runs it regardless of this knob. |
| `MEDIAGIT_DATA_READ_TIMEOUT_SECS` | `300` | 0.3.0-rc.4 | push/pull/clone | stable | Data-plane INTER-BYTE timeout, not a total-request cap: a slow but progressing transfer resets it on every byte. Catches a peer that holds the connection open and goes silent, which `tcp_keepalive` cannot. `0` disables it. |
| `MEDIAGIT_UPLOAD_VERIFY_CONCURRENCY` | `32` | 0.3.0-rc.4 | server | stable | Max concurrent chunk read-back verifications when `verify_content_on_complete` is on. |

## Storage Backends

| Knob | Default | Introduced | Scope | Status | Description |
|------|---------|-----------|-------|--------|-------------|
| `MEDIAGIT_AZURE_PUT_BLOCK_CONCURRENCY` | `8` | 0.3.0-rc.4 | storage/azure | stable | Concurrent block uploads per blob for Azure `put_block`. |
| `MEDIAGIT_GCS_UPLOAD_CONCURRENCY` | `4` | 0.3.0-rc.4 | storage/gcs | stable | Upload semaphore capacity for the GCS proxy-path uploader (bounds TCP fan-out; see [[project_gcs_concurrent_upload_fix]]). |
| `MEDIAGIT_GCS_DISABLE_PRESIGN` | unset (presigned URLs required) | 0.3.0-rc.4 | storage/gcs | stable | Any value forces all GCS transfers through the server proxy, skipping presigned URLs. Since 0.4.0 it is also the **only** way to run the proxy path: ADC that cannot sign V4 URLs (workload-identity federation, or a default service account without `iam.serviceAccounts.signBlob`) now fails backend construction with an actionable error instead of downgrading to the proxy behind one `warn` line. The proxy costs ~26x the requests — 4,073 against 155 packs on a 16 GB repo — so it has to be chosen, not inherited. |
| `MEDIAGIT_MPU_PART_SIZE` | auto-computed (~`total_size / 96`, floor 16 MiB, clamped to [5 MiB, 5 GiB]) | 0.3.0-rc.4 | storage/s3 | stable | Override the multipart-upload part size for S3/MinIO. Values outside [5 MiB, 5 GiB] are ignored (auto-compute wins). |
| `MEDIAGIT_MINIO_OP_CONCURRENCY` | `64` | 0.3.0-rc.4 | storage/minio | stable | Bounds concurrent in-flight MinIO `with_retry` operations (put/get/exists/delete/head) to prevent socket exhaustion during backend outages. |
| `MEDIAGIT_AZURE_IO_TIMEOUT_SECS` | `120` | 0.3.0-rc.4 | azure | stable | Per-IO deadline for Azure blob operations. Guards against a hang no retry policy can rescue. |
| `MEDIAGIT_GCS_IO_TIMEOUT_SECS` | `120` | 0.3.0-rc.4 | gcs | stable | Per-IO deadline for GCS operations. Added after gcs.rs was found to be the only backend with no timeout at all. |

## Server / Auth / Locks

| Knob | Default | Introduced | Scope | Status | Description |
|------|---------|-----------|-------|--------|-------------|
| `MEDIAGIT_STARTUP_PROBE` | `1` (ON) | 0.3.0-rc.4 | server | stable | Server scans `repos_dir` for repo health at startup. `0` skips the probe. |
| `MEDIAGIT_ALLOW_INSECURE_BIND` | `0` (OFF) | 0.3.0-rc.4 | server | stable | Bypasses the startup refusal to bind a non-loopback host when auth is disabled. Security-relevant — see the server startup guard in `main.rs`. |
| `MEDIAGIT_JWT_SECRET` | none (falls back to config `jwt_secret`) | 0.3.0-rc.4 | server/auth | stable | JWT signing secret for server auth. If both the env var and the config file set it, the env var wins (and a warning is logged). |
| `MEDIAGIT_METRICS_ADDR` | unset (metrics endpoint not started) | 0.3.0-rc.4 | server | stable | `host:port` to bind the Prometheus metrics endpoint. |
| `MEDIAGIT_AUTH_PERSIST` | `1` (ON) | 0.3.0-rc.4 | server/auth | stable | Persist auth state (users/grants) to disk. `0` keeps auth in-memory only (used by tests to avoid cross-test races). |
| `MEDIAGIT_GRANTS_ENFORCE` | `1` (ON, only when at least one grant is configured) | 0.3.0-rc.4 | server/auth | stable | Enforce per-repo permission grants. `0` falls back to the flat auth check. No-op if no grants exist. |
| `MEDIAGIT_LOCKS_ENFORCE` | `1` (ON) | 0.3.0-rc.4 | server/locks | stable | Server rejects pushes that touch paths locked by another user. `0` disables lock enforcement. |
| `MEDIAGIT_LOCKS_MAX_COMMITS` | `1000` | 0.3.0-rc.4 | server/locks | stable | Max commits walked when computing touched paths for lock enforcement on a push. |
| `MEDIAGIT_PACK_VERIFY_QUIET_SECS` | `5` | 0.4.0-rc.1 | server/verify | stable | Seconds the data plane must be idle before a registered pack is verified. Verification reads the pack back over the same link the push is using, so it waits for quiet rather than competing. |
| `MEDIAGIT_PACK_VERIFY_MAX_DEFER_SECS` | `900` (15 min) | 0.4.0-rc.1 | server/verify | stable | Upper bound on that deferral, so continuous traffic cannot postpone verification forever. `0` disables waiting entirely and verifies immediately (pre-2026-09-15 behaviour). |
| `MEDIAGIT_PACK_VERIFY_ASSUMED_MBPS` | `2` | 0.4.0-rc.1 | server/verify | stable | Assumed floor throughput, MiB/s, used to estimate how long registered packs will keep the link busy. Only sizes the deferral lease; it does not throttle anything. `0` and unparseable values fall back to the default. |
| `MEDIAGIT_ALLOW_MULTI_INSTANCE` | unset (refuse to start) | 0.3.0-rc.3 | server | stable | Downgrades the AU-10 single-instance-per-directory startup refusal to a warning and continues without the lock. `=1` only — for an operator who has genuinely separated every piece of shared state and only trips over a lock file on a shared mount. See `crates/mediagit-server/src/instance_lock.rs`. |
| `MEDIAGIT_NO_KEYRING` | unset (OS keychain used) | 0.3.0-rc.4 | auth | stable | Any value disables OS keychain credential storage/lookup on the client. |
| `MEDIAGIT_ENCRYPTION_KEYFILE` | unset (master key read from the OS keychain) | 0.3.0-rc.4 | encryption | stable | Path to a file holding the at-rest master key, for hosts with no usable keychain (CI, headless servers). The master key is machine-wide, not per-repo: each repository has its own random repo key, so one master unlocking several leaks nothing between them. |
| `MEDIAGIT_ADMIN_PASSWORD` | none | 0.3.0-rc.4 | server setup | stable | Password for the bootstrap admin created by `mediagit-server init`. Supplied via env so it never lands in shell history or a config file; requires `--admin-username` and `--admin-email` too. Empty is treated as unset. |
| `MEDIAGIT_BCRYPT_COST` | bcrypt `DEFAULT_COST` | 0.3.0-rc.4 | server | stable | bcrypt work factor for password hashing. Clamped to `10`-`31`: lower would be insecure, higher is refused by bcrypt. Lower it ONLY in tests, where the default cost dominates runtime. |
| `MEDIAGIT_MAX_LOGIN_FAILURES` | `5` | 0.3.0-rc.4 | server | stable | Consecutive failed logins before an account is locked out. |
| `MEDIAGIT_LOGIN_LOCKOUT_SECS` | `900` | 0.3.0-rc.4 | server | stable | How long a lockout lasts, in seconds (15 minutes). |
| `MEDIAGIT_APIKEY_LAST_USED_RESOLUTION` | `300` | 0.3.0-rc.4 | server | stable | Coarseness, in seconds, of an API key's `last_used` timestamp. Coarse on purpose: a per-request write would make every authenticated read a write. `0` records exactly; negatives are ignored. |
| `MEDIAGIT_STARTUP_PROBE_TIMEOUT_SECS` | `90` | 0.3.0-rc.4 | server | stable | Budget for the startup storage-backend probe. Junk or `0` falls back to the default rather than to zero - a 0s budget would time out instantly and refuse to start every server. Disabling the probe is `MEDIAGIT_STARTUP_PROBE=0`, a different knob. |
| `MEDIAGIT_AUTH_TIMEOUT_SECS` | `60` | 0.3.0-rc.4 | cli auth | stable | Connect and read timeout for `mediagit auth` HTTP calls (connect is capped at 15s). `0` restores the previous unbounded behaviour. Added after `auth key revoke` was seen blocking ~60s with the request never reaching the server. |

## GC / Housekeeping

| Knob | Default | Introduced | Scope | Status | Description |
|------|---------|-----------|-------|--------|-------------|
| `MEDIAGIT_GC_REFLOG_HORIZON_DAYS` | `90` | 0.3.0-rc.4 | gc | stable | Reflog entries older than this are no longer GC roots (mirrors git's `gc.reflogExpire`). `0` disables reflog roots entirely. |
| `MEDIAGIT_GC_GRACE_SECS` | `3600` | 0.4.0-rc.1 | gc | experimental | VC-2 prune grace period: gc will not collect an unreachable object written within this many seconds. Rooting is inherently racy against a concurrent writer — a chunk uploaded but not yet referenced is collectible, and the victim cannot recover because push dedups without re-verifying. `0` restores the pre-v0.4.0 behaviour and is a deliberate data-loss risk with concurrent writers. Requires `StorageBackend::modified_at`; only `LocalBackend` implements it today (`NamespacedBackend` forwards), so cloud-backed repos report unknown age and gc warns rather than silently skipping. |
| `MEDIAGIT_NO_AUTO_GC` | unset (auto-gc ON) | 0.3.0-rc.4 | gc | stable | Any value disables auto-gc for the current invocation (mirrors git's `GC_AUTO`). |
| `MEDIAGIT_AUTO_GC_INTERVAL` | `100` | 0.3.0-rc.3 | gc | stable | How many auto-gc triggers must elapse between full repo scans (`should_scan`, `crates/mediagit-cli/src/auto_gc.rs:158`). Any positive integer; unset/unparseable/`0` falls back to `100`. `1` (or less) restores the old scan-every-time behavior. Fails open: any I/O problem reading/writing the counter file forces a scan rather than silently skipping GC forever. |

## Server App Config Overrides — REMOVED

Fourteen `MEDIAGIT_*` names were listed here as knobs until 2026-09-10. They are
named in full below so that anyone searching this file for one of them lands
here rather than finding nothing. Grouped in fours, because each group needs its
own retraction close enough for the `14_docs_surface` gate to see it.

Removed, no such variables: `MEDIAGIT_APP_NAME`, `MEDIAGIT_APP_PORT`,
`MEDIAGIT_APP_HOST`, `MEDIAGIT_APP_ENVIRONMENT`.

Also removed, none exist: `MEDIAGIT_APP_DEBUG`, `MEDIAGIT_LOG_LEVEL`,
`MEDIAGIT_METRICS_ENABLED`, `MEDIAGIT_METRICS_PORT`.

Also removed, none exist: `MEDIAGIT_COMPRESSION_ENABLED`,
`MEDIAGIT_COMPRESSION_LEVEL`, `MEDIAGIT_MAX_CONCURRENCY`.

Also removed, none exist: `MEDIAGIT_BUFFER_SIZE`, `MEDIAGIT_HTTPS_ENABLED`,
`MEDIAGIT_AUTH_ENABLED`.

**None of them do anything, so they are no longer documented as knobs.** They
were read only by an env-var overlay that had no caller in the workspace
outside the crate's own tests; that overlay has been deleted in v0.4.0
(FUTURE_TODOS item 22, resolved by removal rather than wiring it up). Setting
any of them was silently ignored while the overlay existed. This note exists
so that anyone who set one before today can find out why nothing happened.

Working alternatives: `MEDIAGIT_METRICS_ADDR` for the metrics endpoint,
`MEDIAGIT_LOG` / `RUST_LOG` for log level. Two variables read by that same dead
function are genuinely live via other read sites and remain documented in their
own sections: `MEDIAGIT_API_KEY` and `MEDIAGIT_CHUNK_WRITE_CONCURRENCY`.

## Legacy Repo-Config Overrides

Pre-CDC-era per-repo toggles read by `mediagit-versioning::RepoConfig`; largely superseded by always-on chunking/delta candidacy logic, kept for compatibility.

| Knob | Default | Introduced | Scope | Status | Description |
|------|---------|-----------|-------|--------|-------------|
| `MEDIAGIT_SMART_COMPRESSION` | `1` (ON, overrides repo config `smart_compression`) | 0.3.0-rc.4 | config | stable | Legacy per-repo compression toggle predating the SmartCompressor default. |
| `MEDIAGIT_CHUNKING_ENABLED` | `0` (OFF, overrides repo config `chunking_enabled`) | 0.3.0-rc.4 | config | experimental | Legacy toggle predating always-on CDC chunking for large files. |
| `MEDIAGIT_DELTA_ENABLED` | `0` (OFF, overrides repo config `delta_enabled`) | 0.3.0-rc.4 | config | experimental | Legacy toggle; delta candidacy is now decided per-file by `should_use_delta()`. |
| `MEDIAGIT_PACK_ENABLED` | `0` (OFF, overrides repo config `pack_enabled`) | 0.3.0-rc.4 | config | experimental | Legacy toggle, distinct from `MEDIAGIT_CLOUD_PACKS` (transfer-layer pack bundling). |

## Internal / Dev-only

| Knob | Default | Introduced | Scope | Status | Description |
|------|---------|-----------|-------|--------|-------------|
| `MEDIAGIT_REPO` | unset | 0.3.0-rc.4 | cli | stable | Internal: repo root override set by `-C <path>` handling; not intended for direct use. |
| `MEDIAGIT_AUTHOR_NAME` | none (falls back to config `[author].name` then `$USER`) | 0.3.0-rc.4 | commit/tag/lock | stable | Commit/tag/lock-owner author name. Priority: `--author`/`--tagger` CLI flag > this env var > `config.toml [author]` > `$USER`. See also [Author Identity](book/src/reference/environment.md#author-identity). |
| `MEDIAGIT_AUTHOR_EMAIL` | none (falls back to config `[author].email` then `$USER@localhost`) | 0.3.0-rc.4 | commit/tag | stable | Commit/tag author email. Same precedence as `MEDIAGIT_AUTHOR_NAME`. |
| `MEDIAGIT_BENCH_CORPUS` | repo's `test-files/` dir | 0.3.0-rc.4 | dev/bench | experimental | Corpus root for the `dedup_report` bench example (`cargo run --example dedup_report`). Not read by the CLI or server. |
| `MEDIAGIT_REQUIRE_TEST_FILES` | `0` (OFF) | 0.3.0-rc.4 | dev/test | stable | Turn a missing fixture root into a test FAILURE instead of a skip. `test-files/` and `dev-tests/dedup-pairs/` are gitignored, so they exist on a developer machine and never in CI, and the media tests must skip there. Set `=1` on a machine that HAS the fixtures to prove those tests actually ran — without it, "skipped" and "passed" are indistinguishable from the outside. Read only by `TestPaths::announce_fixture_root`; never by product code. |

Not documented here (test-harness only, gated behind `#[ignore]` integration tests, never read by product code): `MEDIAGIT_TEST_REAL_KEYRING`, `MEDIAGIT_GCS_BUCKET`, `MEDIAGIT_GCS_PROJECT` (GCS integration tests use `GOOGLE_CLOUD_PROJECT` as a fallback for the latter).

## Notes

- **B2 graduation**: flip `MEDIAGIT_PUSH_PIPELINE=1` after two clean runs of `dev-tests/deep-tests/parity_matrix.ps1 -Backends all` with no R2 threshold violations. **Progress**: MinIO 2/2, AWS 2/2, Azure 2/2 (all 2026-05-22). GCS still pending (no config available).
- **B4 graduation**: **COMPLETE (2026-05-22).** Windows stress PASS (20×50MB, 0 handle errors), MinIO 148/148, AWS 148/148, Azure 147/147 all with knob ON. Ready to flip default ON in next release. GCS pending (no config).
- **B7 graduation**: **COMPLETE (2026-05-22).** AWS clone 159.8s→134.5s (15.8% improvement, threshold ≥10%). 148/148 PASS. Ready to flip default ON for S3/MinIO. Other backends use default-impl wrapper (parity preserved).
- Setting `MEDIAGIT_HTTP_POOL_MAX` too high on Windows risks handle exhaustion — keep ≤ 128.
- The now-deleted env-var overlay (see "Server App Config Overrides" above) used to read 16 `MEDIAGIT_*` vars total, but only 14 of them were dead. `MEDIAGIT_API_KEY` and `MEDIAGIT_CHUNK_WRITE_CONCURRENCY` were also read there, yet both stay live through separate, real read sites elsewhere (`MEDIAGIT_API_KEY` in the client auth path documented near the top of this file; `MEDIAGIT_CHUNK_WRITE_CONCURRENCY` directly at `odb/chunks.rs:646`) — its removal does not affect them.

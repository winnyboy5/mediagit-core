# Configuration Reference

Complete reference for `.mediagit/config.toml`. All sections are optional — MediaGit uses sensible defaults for any missing values.

## File Location

```
<repo-root>/.mediagit/config.toml
```

## Minimal Configuration

```toml
[author]
name = "Alice Smith"
email = "alice@example.com"
```

## Full Example

```toml
[author]
name = "Alice Smith"
email = "alice@example.com"

[storage]
backend = "s3"
bucket = "my-media-bucket"
region = "us-east-1"
prefix = "repos/my-project"
access_key_id = "..."
secret_access_key = "..."

[performance]
buffer_size = 65536
upload_concurrency = 32
download_concurrency = 24
pack_workers = 8

[performance.cache]
enabled = true
cache_type = "memory"
max_size = 536870912  # 512 MB
ttl = 3600

[observability]
log_level = "info"
log_format = "json"

[remotes.origin]
url = "http://media-server.example.com/my-project"

[protected_branches.main]
prevent_force_push = true
prevent_deletion = true
require_reviews = true
min_approvals = 1
```

---

## Top-Level — Repository Identity & Layout

Written once at `mediagit init`/`clone`; not meant to be edited by hand.

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `cdc_seed` | u64 | `0` | Per-repo content-defined chunking seed. Generated at `init`, propagated to clones via protocol capabilities. `0` reproduces legacy unseeded chunk boundaries (pre-existing repos). |
| `repo_namespace` | string | *(derived)* | Per-repo storage namespace (layout v2): every object key is prefixed `<repo_namespace>/` so one bucket/root can host multiple repos. Absent on pre-v2 repos (falls back to sanitized repo-dir basename). Never change it after init — existing keys would be orphaned. |
| `layout_version` | u32 | `2` | Physical storage layout version (`1` = flat pre-namespace, `2` = namespaced + hash fanout). Mirrored in the storage root's `LAYOUT` marker; a mismatched client fails fast. |
| `repo_id` | string | *(generated)* | Unique repo identifier recorded in the `LAYOUT` marker so a namespace collision between independent repos is a hard error instead of a silent key-space merge. |
| `config_version` | u32 | `0` | config.toml schema version, migrated automatically (distinct from `layout_version`). |

---

## `[security]` — Schema-Only (not read at runtime)

This section exists in the config schema but is **not read by the CLI or by
`mediagit-server`** — actual server security settings live in
`mediagit-server.toml` (see `CONFIGURATION.md` Part 2 at the repo root, and
[Authentication](./authentication.md) for the auth model). The keys below are
documented for schema completeness only.

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `auth_enabled` | bool | `false` | Enable JWT/API-key authentication. The JWT secret comes from `MEDIAGIT_JWT_SECRET` (required when enabled). |
| `https_enabled` | bool | `false` | Serve TLS (enables HTTP/2) |
| `tls_cert_path` / `tls_key_path` | string | — | Certificate/key files for TLS |
| `api_key` | string | — | Static API key (legacy single-key mode) |
| `cors_origins` | array | `[]` | Allowed CORS origins |
| `rate_limiting` | table | — | Rate-limit configuration for auth and data routes |

> `encryption_at_rest` / `encryption_key_path` used to be documented here.
> They were removed from `SecurityConfig` entirely — nothing ever read them,
> even the "key path must exist" check never ran — and the real at-rest
> encryption switch is `[encryption]` in `mediagit-server`'s own config. If
> your `config.toml` still has them, they parse without complaint and do
> nothing; remove them.

---

## `[author]` — Author Identity

Used when creating commits. Override with `MEDIAGIT_AUTHOR_NAME` / `MEDIAGIT_AUTHOR_EMAIL` env vars or `--author` CLI flag.

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `name` | string | `$USER` | Display name on commits |
| `email` | string | `""` | Email address on commits |

---

## `[storage]` — Storage Backend

The storage backend is selected with the `backend` key. All backend-specific fields are placed **directly in `[storage]`** alongside `backend` — there are no nested `[storage.s3]` or `[storage.filesystem]` subsections.

### Local Filesystem (default)

```toml
[storage]
backend = "filesystem"
base_path = "./data"
create_dirs = true
sync = false
file_permissions = "0644"
```

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `backend` | string | `"filesystem"` | Must be `"filesystem"` |
| `base_path` | string | `"./data"` | Storage root directory |
| `create_dirs` | bool | `true` | Auto-create directories |
| `sync` | bool | `false` | Sync writes to disk (slower, safer) |
| `file_permissions` | string | `"0644"` | Octal file permission string |

### Amazon S3

```toml
[storage]
backend = "s3"
bucket = "my-bucket"
region = "us-east-1"
prefix = ""
access_key_id = "..."
secret_access_key = "..."
```

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `backend` | string | — | Must be `"s3"` |
| `bucket` | string | — | **Required.** S3 bucket name |
| `region` | string | — | **Required.** AWS region |
| `access_key_id` | string | — | **Required for real AWS/MinIO/S3-compatible.** No env var or IAM-role fallback; defaults to empty if unset. |
| `secret_access_key` | string | — | **Required for real AWS/MinIO/S3-compatible.** Same caveat as `access_key_id`. |
| `endpoint` | string | — | Custom endpoint for S3-compatible services (e.g. MinIO) |
| `prefix` | string | `""` | Object key prefix |

> **Warning**: `encryption` and `encryption_algorithm` are not real keys —
> `S3Storage` has no such fields. A config carrying them parses without
> complaint and the values are silently ignored. If you copied them from an
> older doc, remove them; they do not enable server-side encryption.

### Azure Blob Storage

```toml
[storage]
backend = "azure"
container = "media-container"
prefix = ""
auth = { type = "account_key", account_name = "mystorageaccount", account_key = "..." }
```

Credentials are a tagged `auth` block (`config_version` 3+), exactly one of:
`account_key` (`account_name` + `account_key`), `connection_string` (`value`),
`sas` (`account_name` + `token`), or `emulator` (no fields). A pre-v3 flat
config is migrated on first open; if migration cannot decide (both credentials
present, or neither) it fails with the exact `auth` block to write.

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `backend` | string | — | Must be `"azure"` |
| `container` | string | — | **Required.** Blob container name |
| `auth` | tagged table | — | **Required.** The credential — see the variants above. |
| `prefix` | string | `""` | Blob path prefix |

> **Note:** `account_name`, `account_key` and `connection_string` are **not**
> top-level keys. They live *inside* the `auth` table. Writing them directly
> under `[storage]` is the pre-v3 layout and is rejected with a migration error.
> There is no environment-variable fallback for any of them.

### Google Cloud Storage

```toml
[storage]
backend = "gcs"
bucket = "my-gcs-bucket"
project_id = "my-gcp-project"
prefix = ""
# credentials_path is optional; omit it to use Application Default Credentials,
# which honour GOOGLE_APPLICATION_CREDENTIALS or a gcloud login session.
```

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `backend` | string | — | Must be `"gcs"` |
| `bucket` | string | — | **Required.** GCS bucket name |
| `project_id` | string | — | **Required.** GCP project ID |
| `credentials_path` | string | unset | Path to a service-account JSON key. **Optional** — when unset, Application Default Credentials are used, which honour `GOOGLE_APPLICATION_CREDENTIALS` or a `gcloud auth application-default login` session. GCS is the only backend with a credential path outside this file. |
| `prefix` | string | `""` | Object prefix |

---

## Compression — automatic, not configurable

There is **no `[compression]` section**. One existed until v0.4.0, but none of its
keys were ever read at runtime, so they were removed from `schema.rs` rather than
left to imply a control that did not exist. A `[compression]` table in an existing
`config.toml` is now simply an unknown key and is ignored (with a warning).

Compression is decided entirely by `SmartCompressor`, per file type:
- Already-compressed formats (JPEG, MP4, ZIP, docx, AI/InDesign): stored as-is (`none`) — PDF is *not* in this group, see below
- Raw/uncompressed image formats (TIFF, RAW, EXR) and 3D interchange formats (OBJ/FBX/GLB/STL/PLY): `zstd` at `Best` level (level 19 — levels 20-22 are deliberately never used, they OOM under parallel adds for <0.5% extra ratio)
- PSD and other creative project files (After Effects, Premiere, Blender, Maya, ...), plus PDF/SVG: `zstd` at `Default` level (level 3)
- Text, JSON, TOML: `brotli` at `Default` level (falls back to `zstd` above 500 MB)
- ML checkpoints: `zstd` at `Fast` level (level 1)

---

## `[performance]` — Performance Tuning

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `upload_concurrency` | integer | unset | Override for client-side parallel chunk uploads. Falls back to `MEDIAGIT_UPLOAD_CONCURRENCY` / internal default (`32`) when unset. |
| `download_concurrency` | integer | unset | Override for client-side parallel chunk downloads. Falls back to `MEDIAGIT_DOWNLOAD_CONCURRENCY` / internal default (`24`) when unset. |
| `pack_workers` | integer | unset | Override for server-side concurrent pack-write workers. Falls back to `MEDIAGIT_PACK_WORKERS` / internal default (`8`) when unset. |
| `buffer_size` | integer | `65536` | I/O buffer size in bytes (64 KB) |

### `[performance.cache]`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | bool | `true` | Enable in-memory object cache |
| `cache_type` | string | `"memory"` | Cache type (`"memory"`) |
| `max_size` | integer | `536870912` | Max cache size in bytes (512 MB) |
| `ttl` | integer | `3600` | Cache entry TTL in seconds |
| `compression` | bool | `false` | Compress cached objects |

> `[performance.connection_pool]` and `[performance.timeouts]` were removed in
> v0.4.0. Both were parsed and round-tripped but never read by any consumer. The
> HTTP pool and timeout settings that are genuinely live are environment knobs —
> see [Environment Variables](environment.md).

---

## `[observability]` — Logging and Tracing

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `log_level` | string | `"info"` | Log level: `"error"`, `"warn"`, `"info"`, `"debug"`, `"trace"` |
| `log_format` | string | `"json"` | Log format: `"json"` or `"text"` |
| `tracing_enabled` | bool | `true` | Enable distributed tracing |
| `sample_rate` | float | `0.1` | Trace sampling rate (0.0–1.0) |

Override `log_level` with the `RUST_LOG` environment variable.

### `[observability.metrics]`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | bool | `true` | Enable Prometheus metrics |
| `port` | integer | `9090` | Metrics HTTP server port |
| `endpoint` | string | `"/metrics"` | Metrics endpoint path |
| `interval` | integer | `60` | Collection interval in seconds |

---

## `[remotes.<name>]` — Remote Repositories

```toml
[remotes.origin]
url = "http://media-server.example.com/my-project"

[remotes.backup]
url = "http://backup-server.example.com/my-project"
```

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `url` | string | — | **Required.** Remote server URL |
| `fetch` | string | `url` | Fetch URL if different from `url` |
| `push` | string | `url` | Push URL if different from `url` |

---

## `[branches.<name>]` — Branch Tracking

```toml
[branches.main]
remote = "origin"
merge = "refs/heads/main"
```

Set automatically by `mediagit push -u origin main`. Rarely edited manually.

---

## `[protected_branches.<name>]` — Branch Protection

```toml
[protected_branches.main]
prevent_force_push = true
prevent_deletion = true
require_reviews = false
min_approvals = 1
```

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `prevent_force_push` | bool | `true` | Block force pushes |
| `prevent_deletion` | bool | `true` | Block branch deletion |
| `require_reviews` | bool | `false` | Require PR review before merge |
| `min_approvals` | integer | `1` | Minimum approvals required |

---

## See Also

- [Environment Variables](./environment.md) — env var overrides
- [Storage Backend Configuration](../guides/storage-config.md) — detailed backend setup
- [Security](../architecture/security.md) — encryption and authentication details

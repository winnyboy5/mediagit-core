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
encryption = true

[compression]
enabled = true
algorithm = "zstd"
level = 3
min_size = 1024

[performance]
max_concurrency = 8
buffer_size = 65536

[performance.cache]
enabled = true
cache_type = "memory"
max_size = 536870912  # 512 MB
ttl = 3600

[performance.timeouts]
request = 60
read = 30
write = 30
connection = 30

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

## `[author]` — Author Identity

Used when creating commits. Override with `MEDIAGIT_AUTHOR_NAME` / `MEDIAGIT_AUTHOR_EMAIL` env vars or `--author` CLI flag.

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `name` | string | `$USER` | Display name on commits |
| `email` | string | `""` | Email address on commits |

---

## `[storage]` — Storage Backend

The storage backend is selected with the `backend` key. Each backend has its own sub-keys.

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
encryption = false
encryption_algorithm = "AES256"
# access_key_id and secret_access_key from env vars or IAM role
```

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `backend` | string | — | Must be `"s3"` |
| `bucket` | string | — | **Required.** S3 bucket name |
| `region` | string | — | **Required.** AWS region |
| `access_key_id` | string | env | AWS access key (prefer env var) |
| `secret_access_key` | string | env | AWS secret key (prefer env var) |
| `endpoint` | string | — | Custom endpoint for S3-compatible services |
| `prefix` | string | `""` | Object key prefix |
| `encryption` | bool | `false` | Enable server-side encryption |
| `encryption_algorithm` | string | `"AES256"` | SSE algorithm: `AES256` or `aws:kms` |

### Azure Blob Storage

```toml
[storage]
backend = "azure"
account_name = "mystorageaccount"
container = "media-container"
prefix = ""
# account_key from env AZURE_STORAGE_KEY or use connection_string
```

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `backend` | string | — | Must be `"azure"` |
| `account_name` | string | — | **Required.** Storage account name |
| `container` | string | — | **Required.** Blob container name |
| `account_key` | string | env | Storage account key (prefer env var) |
| `connection_string` | string | env | Full connection string (alternative to account_name/key) |
| `prefix` | string | `""` | Blob path prefix |

### Google Cloud Storage

```toml
[storage]
backend = "gcs"
bucket = "my-gcs-bucket"
project_id = "my-gcp-project"
prefix = ""
# credentials_path from GOOGLE_APPLICATION_CREDENTIALS env var
```

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `backend` | string | — | Must be `"gcs"` |
| `bucket` | string | — | **Required.** GCS bucket name |
| `project_id` | string | — | **Required.** GCP project ID |
| `credentials_path` | string | env | Path to service account JSON key |
| `prefix` | string | `""` | Object prefix |

---

## `[compression]` — Compression Settings

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | bool | `true` | Enable compression |
| `algorithm` | string | `"zstd"` | Algorithm: `"zstd"`, `"brotli"`, `"none"` |
| `level` | integer | `3` | Compression level (zstd: 1–22, brotli: 1–11) |
| `min_size` | integer | `1024` | Min file size in bytes to compress |

**Algorithm selection by file type** (automatic, overrides `algorithm` setting):
- Already-compressed formats (JPEG, MP4, ZIP, docx, AI, PDF): stored as-is (`none`)
- PSD, raw formats, 3D models: `zstd` at `Best` level
- Text, JSON, TOML: `zstd` at `Default` level

---

## `[performance]` — Performance Tuning

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `max_concurrency` | integer | CPU count (min 4) | Max parallel operations |
| `buffer_size` | integer | `65536` | I/O buffer size in bytes (64 KB) |

### `[performance.cache]`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | bool | `true` | Enable in-memory object cache |
| `cache_type` | string | `"memory"` | Cache type (`"memory"`) |
| `max_size` | integer | `536870912` | Max cache size in bytes (512 MB) |
| `ttl` | integer | `3600` | Cache entry TTL in seconds |
| `compression` | bool | `false` | Compress cached objects |

### `[performance.connection_pool]`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `min_connections` | integer | `1` | Minimum pool connections |
| `max_connections` | integer | `10` | Maximum pool connections |
| `timeout` | integer | `30` | Connection timeout in seconds |
| `idle_timeout` | integer | `600` | Idle connection timeout in seconds |

### `[performance.timeouts]`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `request` | integer | `60` | Total request timeout in seconds |
| `read` | integer | `30` | Read timeout in seconds |
| `write` | integer | `30` | Write timeout in seconds |
| `connection` | integer | `30` | Connection timeout in seconds |

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

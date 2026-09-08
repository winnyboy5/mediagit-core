# Storage Backend Configuration

MediaGit supports multiple storage backends. The backend is configured in `.mediagit/config.toml` under the `[storage]` section.

For a complete reference of every option, see [Configuration Reference — Storage](../reference/config.md#storage--storage-backend).

---

## Local Filesystem (Default)

No configuration required. MediaGit uses `./data` relative to the repo root:

```toml
[storage]
backend = "filesystem"
base_path = "./data"
create_dirs = true
sync = false
```

Set `sync = true` to flush writes to disk before confirming (slower but safer on crash-prone systems).

---

## Amazon S3

```toml
[storage]
backend = "s3"
bucket = "my-media-bucket"
region = "us-east-1"
prefix = "repos/my-project"
access_key_id = "AKIAIOSFODNN7EXAMPLE"
secret_access_key = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
```

> **Credentials must live in this file.** MediaGit does not read
> `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` or `AWS_SESSION_TOKEN`, and has
> no IAM-role or instance-profile path — the S3 family is built through
> `MinIOBackend`, whose key and secret come straight from `[storage]`
> (`mediagit-server/src/handlers/mod.rs:610-638`). Exporting those variables
> leaves the shell looking configured while the push fails with "access key
> cannot be empty". Protect the file instead: keep it out of version control
> and restrict its permissions.
>
> GCS is the one exception — see [GCS](#google-cloud-storage) below.

---

## MinIO (S3-Compatible)

MinIO uses the same S3 backend with a custom `endpoint` — that's the only
thing that distinguishes it from real AWS:

```toml
[storage]
backend = "s3"
bucket = "my-media-bucket"
region = "us-east-1"
prefix = ""
access_key_id = "minioadmin"
secret_access_key = "minioadmin"
endpoint = "http://localhost:9000"
```

Create the bucket first:

```bash
mc alias set local http://localhost:9000 minioadmin minioadmin
mc mb local/my-media-bucket
```

---

## Azure Blob Storage

```toml
[storage]
backend = "azure"
container = "media-container"
prefix = ""
auth = { type = "account_key", account_name = "mystorageaccount", account_key = "..." }
```

Credentials live in a tagged `auth` block -- exactly one credential, chosen by
`type`: `account_key`, `connection_string { value }`, `sas { account_name,
token }`, or `emulator` (local Azurite). The pre-`config_version` 3 flat form
(`account_name`/`account_key` directly under `[storage]`) is migrated
automatically the first time a repo is opened. There is no environment-variable
path for any of these — `AZURE_STORAGE_CONNECTION_STRING`,
`AZURE_STORAGE_ACCOUNT` and `AZURE_STORAGE_KEY` are not read. Use
`connection_string` instead of `account_key` if that's what you have:

```toml
auth = { type = "connection_string", value = "DefaultEndpointsProtocol=https;AccountName=mystorageaccount;AccountKey=base64key==;EndpointSuffix=core.windows.net" }
```

---

## Google Cloud Storage

```toml
[storage]
backend = "gcs"
bucket = "my-gcs-bucket"
project_id = "my-gcp-project"
prefix = ""
```

GCS is the **only** backend with a genuine environment-variable path: leave
`credentials_path` unset in the config and MediaGit falls back to Application
Default Credentials, which honours `GOOGLE_APPLICATION_CREDENTIALS`:

```bash
export GOOGLE_APPLICATION_CREDENTIALS=/path/to/service-account.json
```

For local testing with the GCS emulator:

```bash
export GCS_EMULATOR_HOST=http://localhost:4443
```

---

## Performance Tuning

All backends benefit from increased connection pool and concurrency for large parallel uploads:

```toml
[performance]
max_concurrency = 32

[performance.connection_pool]
max_connections = 32

[performance.timeouts]
request = 300   # 5 min — for very large chunks
write = 120
```

For the local filesystem backend, `max_concurrency` controls how many concurrent chunk writes are issued.

---

## See Also

- [Configuration Reference](../reference/config.md)
- [Environment Variables](../reference/environment.md)
- [Architecture — Storage Backends](../architecture/storage-backends.md)
- [Large File Optimization](../advanced/large-files.md)

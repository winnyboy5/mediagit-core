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
encryption = false
```

Credentials via environment variables (recommended over config file):

```bash
export AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE
export AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY
export AWS_REGION=us-east-1
```

For temporary credentials (IAM role or STS):

```bash
export AWS_SESSION_TOKEN=...
```

For named profiles from `~/.aws/credentials`:

```bash
export AWS_PROFILE=my-profile
```

---

## MinIO (S3-Compatible)

MinIO uses the same S3 configuration with a custom endpoint:

```toml
[storage]
backend = "s3"
bucket = "my-media-bucket"
region = "us-east-1"
prefix = ""
```

```bash
export AWS_ACCESS_KEY_ID=minioadmin
export AWS_SECRET_ACCESS_KEY=minioadmin
export AWS_ENDPOINT_URL=http://localhost:9000
export AWS_REGION=us-east-1
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
account_name = "mystorageaccount"
container = "media-container"
prefix = ""
```

Authentication via environment variable (full connection string):

```bash
export AZURE_STORAGE_CONNECTION_STRING="DefaultEndpointsProtocol=https;AccountName=mystorageaccount;AccountKey=base64key==;EndpointSuffix=core.windows.net"
```

Or account name + key:

```bash
export AZURE_STORAGE_ACCOUNT=mystorageaccount
export AZURE_STORAGE_KEY=base64key==
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

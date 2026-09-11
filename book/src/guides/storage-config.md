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

`create_dirs` and `sync` are accepted by the schema but not consulted by the
filesystem backend — `LocalBackend::new` takes only the resolved path, no
config (`crates/mediagit-storage/src/local.rs:153`). In practice, parent
directories are always created (`ensure_parent_dir`, local.rs:337) and every
write is always `fsync`'d before the atomic rename (`file.sync_all()`,
local.rs:596) regardless of what `sync` is set to — there is no faster,
unsynced write path today.

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

There is no supported environment variable for pointing MediaGit at a local GCS
emulator. `GCS_EMULATOR_HOST` was documented here previously and is read by
nothing — the name does not appear anywhere in the codebase. The emulator tests
in `crates/mediagit-storage/tests/gcs_emulator_tests.rs` set
`STORAGE_EMULATOR_HOST` themselves, for the Google SDK's benefit, and it is not
a user-facing knob.

---

## Performance Tuning

`max_concurrency`, `[performance.connection_pool]` and `[performance.timeouts]`
were **removed in v0.4.0**. They had parsed into the config struct and
round-tripped faithfully, but nothing outside `mediagit-config` ever read them
back out, so setting them changed no behavior. They are now unknown keys and are
ignored with a warning. The knobs that do gate concurrency are the other
`[performance]` fields:

```toml
[performance]
upload_concurrency = 32     # client-side parallel chunk uploads (push/pull/fetch)
download_concurrency = 24   # client-side parallel chunk downloads
pack_workers = 8            # server-side concurrent pack-write workers
```

`upload_concurrency` and `download_concurrency` can also be set with
`mediagit config set performance.upload_concurrency <n>` (and the
`download_concurrency` equivalent); `pack_workers` has no `config` key and
must be edited directly in `.mediagit/config.toml`. Each falls back to an
env var when unset — `MEDIAGIT_UPLOAD_CONCURRENCY`, `MEDIAGIT_DOWNLOAD_CONCURRENCY`,
`MEDIAGIT_PACK_WORKERS` — and the env var wins if both are set; only then
does the internal default (32 / 24 / 8) apply.

---

## See Also

- [Configuration Reference](../reference/config.md)
- [Environment Variables](../reference/environment.md)
- [Architecture — Storage Backends](../architecture/storage-backends.md)
- [Large File Optimization](../advanced/large-files.md)

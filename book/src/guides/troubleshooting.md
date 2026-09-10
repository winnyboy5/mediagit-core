# Troubleshooting

Common issues and solutions for MediaGit.

## Push / Upload Issues

### "Direct upload to storage backend unreachable; switching to server-proxy upload"

This warning appears during `mediagit push` when the **presigned URL fast path** is unavailable from your machine. The push still completes via the server-proxy route — it is slower but correct.

**Affected backends:** AWS S3, MinIO, Backblaze B2, DigitalOcean Spaces.  
**Not affected:** GCS (proxy-only), Azure Blob (SAS errors are caught server-side), local filesystem.

The fast path lets the client upload chunk bytes directly to the storage bucket, bypassing the MediaGit server. It requires the client machine to have direct HTTPS access to the bucket endpoint.

**Common causes and fixes:**

| Cause | Fix |
|---|---|
| Bucket has a VPC-only policy (production default) | Expected on prod — proxy path is correct. No action needed. |
| Corporate / ISP firewall blocks outbound S3 HTTPS | Use the proxy path (already the fallback). Or open egress to `*.s3.<region>.amazonaws.com:443`. |
| S3 bucket policy restricts by IP | Add your machine's public IP: `aws s3api put-bucket-policy` with `aws:SourceIp` condition. |
| MinIO behind private network | Ensure the repo's `[storage] endpoint` in `.mediagit/config.toml` points to a host the client can reach, or use the proxy path. (There is no `MEDIAGIT_STORAGE_ENDPOINT` env var — storage settings are config-file only.) |

**To verify connectivity from your machine:**
```bash
# Should return 403 (auth required) — means network is reachable
curl -I https://<your-bucket>.s3.<region>.amazonaws.com/
# "Could not connect" or timeout means network is blocked
```

A 403 response confirms network connectivity is fine; the bucket policy is restricting direct uploads. A connection error means egress is blocked.

**To check your public IP (for IP-allowlist policies):**
```bash
curl ifconfig.me
```

If direct uploads are not needed from your environment, the proxy fallback is fully supported and transparent — no configuration change is required.

---

## Repository Corruption

### `mediagit fsck` reports errors

Run a full verification to identify the issue:

```bash
mediagit fsck
```

If corruption is found, attempt repair:

```bash
mediagit fsck --repair
```

If objects are missing, check if they exist in the remote and fetch them:

```bash
mediagit fetch
mediagit fsck
```

### Repository is inaccessible after system crash

```bash
# Verify integrity first
mediagit fsck

# If OK, run GC to clean up any partial writes
mediagit gc

# If corrupt, attempt repair
mediagit fsck --repair
```

### "Not a mediagit repository" in a valid repo

This usually means the current directory is not inside a MediaGit repository, or the `.mediagit/` directory was deleted.

```bash
# Check where MediaGit expects the repo root
ls .mediagit/

# Use -C to point to the repo explicitly
mediagit -C /path/to/repo status
```

The `MEDIAGIT_REPO` environment variable can also override the repo path:

```bash
export MEDIAGIT_REPO=/path/to/repo
mediagit status
```

---

## Network Errors

### Push / pull fails with connection error

```bash
# Verify the remote URL is correct
mediagit remote -v

# Test connectivity
curl -v http://media-server.example.com/healthz
```

Check the remote URL in `.mediagit/config.toml`:

```toml
[remotes.origin]
url = "http://media-server.example.com/my-project"
```

### S3 upload fails with "Access Denied"

Credentials come from `.mediagit/config.toml` — **not** from the environment.
Exporting `AWS_ACCESS_KEY_ID` and friends does nothing, so if that is what you
tried, the shell will look correctly configured while the push keeps failing.
Check the file:

```toml
[storage]
backend = "s3"
bucket = "my-media-bucket"
region = "us-east-1"
access_key_id = "AKIA..."
secret_access_key = "..."
```

Common causes, in the order worth checking:

1. `access_key_id` / `secret_access_key` absent from `[storage]` — the backend
   refuses to start with "access key cannot be empty".
2. Keys present but belonging to an identity without `s3:PutObject` on the
   bucket or prefix.
3. `region` not matching the bucket's actual region (SigV4 signs over region,
   so a mismatch reads as an auth failure rather than a routing one).
4. A misspelled key name. Unknown keys are **silently discarded**, so
   `acces_key_id` is indistinguishable from having written nothing at all.

For MinIO or other S3-compatible services, the `endpoint` key is what
distinguishes them from real AWS — it belongs in the same `[storage]` block,
not in the environment:

```toml
endpoint = "http://localhost:9000"
```

### Azure Blob upload fails

`AZURE_STORAGE_CONNECTION_STRING`, `AZURE_STORAGE_ACCOUNT` and
`AZURE_STORAGE_KEY` are not read. The credential is a tagged `auth` table in
`.mediagit/config.toml`:

```toml
[storage]
backend = "azure"
container = "my-container"
auth = { type = "connection_string", value = "DefaultEndpointsProtocol=https;AccountName=...;AccountKey=...;EndpointSuffix=core.windows.net" }
```

If the error names the "removed flat format", the config still has
`account_name` / `account_key` directly under `[storage]`; move them inside an
`auth` block as above.

### Push times out on large files

Large files can take time to upload. Increase timeouts in `.mediagit/config.toml`:

```toml
[performance.timeouts]
request = 300   # 5 minutes
write = 120
```

---

## Performance Issues

### `mediagit add` is slow

By default, `mediagit add` uses all CPU cores. If your system is under memory pressure:

```bash
# Limit parallelism
mediagit add --jobs 4 assets/

# Disable parallelism (for debugging)
mediagit add --no-parallel assets/
```

Check if files are being re-compressed unnecessarily. Pre-compressed formats (JPEG, MP4, ZIP, PDF, AI) should be stored without re-compression — verify with:

```bash
RUST_LOG=mediagit_compression=debug mediagit add file.jpg
```

### `mediagit log` is slow in large repos

Run garbage collection to build the commit graph:

```bash
mediagit gc
```

### Clone or pull is slow

If the remote is on S3, increase upload concurrency:

```toml
[performance]
max_concurrency = 32

[performance.connection_pool]
max_connections = 32
```

---

## Common Command Issues

### `mediagit status` crashes on empty repo

This was fixed in the current release. If you see a crash, ensure you are running the latest binary:

```bash
mediagit --version
```

An empty repo correctly shows:

```
On branch: main (no commits yet)
```

### `mediagit add --all` says "no files"

`--all` collects all files recursively from the repo root. Ensure you are inside a MediaGit repository:

```bash
mediagit -C /path/to/repo add --all
```

### `mediagit log -3` not recognized

Use the `-n` flag directly, or upgrade to the latest binary which preprocesses `-N` shorthand:

```bash
mediagit log -n 3    # always works
mediagit log -3      # works in current release
```

### `mediagit commit` uses wrong author name

The priority chain for author identity is:

1. `--author "Name <email>"` CLI flag
2. `MEDIAGIT_AUTHOR_NAME` / `MEDIAGIT_AUTHOR_EMAIL` env vars
3. `[author]` section in `.mediagit/config.toml`
4. `$USER` environment variable

Set your identity in config:

```toml
[author]
name = "Alice Smith"
email = "alice@example.com"
```

Or via environment:

```bash
export MEDIAGIT_AUTHOR_NAME="Alice Smith"
export MEDIAGIT_AUTHOR_EMAIL="alice@example.com"
```

---

## File Format Issues

### WAV file produces too many chunks

This was a known bug (WAV was routed to the AVI/RIFF chunker) and is fixed in the current release. Verify:

```bash
mediagit --version
```

### STL / USDZ / PLY detected as "Unknown"

These 3D formats are now detected correctly in the current release. If you see "Unknown" for a supported format, check your binary version.

Supported 3D extensions: `stl`, `obj`, `fbx`, `glb`, `gltf`, `ply`, `dae`, `abc`, `3ds`, `usd`, `usda`, `usdc`, `usdz`.

### AI / InDesign files inflate in size after add

AI and InDesign (`.indd`) files are PDF containers with embedded compressed streams. MediaGit stores them as-is (no re-compression) because re-compressing compressed data increases size. This is correct behavior.

---

## Storage Backend Issues

### MinIO: "bucket does not exist"

Create the bucket first:

```bash
mc mb myminio/media-bucket
```

Or set `create_dirs = true` in config for the filesystem backend.

### GCS: authentication fails

Set the credentials path:

```bash
export GOOGLE_APPLICATION_CREDENTIALS=/path/to/service-account.json
```

For local testing with the fake-gcs-server emulator, MediaGit's own GCS
integration tests set `STORAGE_EMULATOR_HOST` (for the Google SDK's benefit)
rather than reading it as a documented user-facing knob:

```bash
export STORAGE_EMULATOR_HOST=http://localhost:4443
```

`GCS_EMULATOR_HOST` is not read anywhere in the codebase — setting it has no effect.

---

## Debug Logging

Enable detailed logs to investigate any issue:

```bash
# All MediaGit logs
RUST_LOG=mediagit=debug mediagit add file.psd

# Specific crate
RUST_LOG=mediagit_versioning=trace mediagit add file.psd

# Human-readable format (instead of JSON)
RUST_LOG_FORMAT=text RUST_LOG=mediagit=debug mediagit status
```

---

## See Also

- [mediagit fsck](../cli/fsck.md) — full repository verification
- [mediagit gc](../cli/gc.md) — garbage collection and repair
- [mediagit verify](../cli/verify.md) — verify specific objects
- [Configuration Reference](../reference/config.md)
- [Environment Variables](../reference/environment.md)

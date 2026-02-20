# Environment Variables

All environment variables recognized by MediaGit. Environment variables take precedence over `config.toml` values where both are supported.

## Core Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `MEDIAGIT_REPO` | Override repository root path. Used internally by `-C <path>`. | — |

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

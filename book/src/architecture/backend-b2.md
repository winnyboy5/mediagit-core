# b2 Storage Backend

Cloud storage backend for B2.

## Configuration

See [Storage Backend Configuration](../guides/storage-config.md) for detailed setup.

## Authentication

Config-file only — `access_key_id`/`secret_access_key` under `[storage]` in `.mediagit/config.toml`. No client-side environment-variable fallback, and no IAM-role/instance-profile path. The one exception is the server: `mediagit-server` reads `MEDIAGIT_S3_ACCESS_KEY_ID` and `MEDIAGIT_S3_SECRET_ACCESS_KEY` for S3-compatible storage (they win over the file). The client still reads storage keys from `config.toml` only.

## Transfer

Backblaze B2 (via its S3-compatible API) supports presigned PUT/GET URLs. The server mints them so chunks and [cloud packs](./cloud-packs.md) move **direct-to-backend**, with a server proxy fallback when signing is unavailable.

## Performance

Cloud-based storage with network latency. Use for distributed teams and backup.

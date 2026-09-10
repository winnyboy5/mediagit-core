# do Storage Backend

Cloud storage backend for DO.

## Configuration

See [Storage Backend Configuration](../guides/storage-config.md) for detailed setup.

## Authentication

Config-file only — `access_key_id`/`secret_access_key` under `[storage]` in `.mediagit/config.toml`. No environment-variable fallback, and no IAM-role/instance-profile path.

## Transfer

DigitalOcean Spaces is S3-compatible and supports presigned transfer. The server mints presigned PUT URLs for upload and presigned GET URLs for download/clone, moving chunks and [cloud packs](./cloud-packs.md) **direct-to-backend**, with a server proxy fallback when signing is unavailable. Presigned multipart upload is not available on this backend (only S3 and MinIO implement it); large packs upload as single presigned PUTs.

## Performance

Cloud-based storage with network latency. Use for distributed teams and backup.

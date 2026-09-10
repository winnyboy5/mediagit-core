# minio Storage Backend

Cloud storage backend for MINIO.

## Configuration

See [Storage Backend Configuration](../guides/storage-config.md) for detailed setup.

## Authentication

Config-file only — `access_key_id`/`secret_access_key` under `[storage]` in `.mediagit/config.toml`. No environment-variable fallback, and no IAM-role/instance-profile path.

## Transfer

MinIO is S3-compatible and supports presigned transfer end to end. The server mints presigned PUT URLs for upload and presigned GET URLs for download/clone, moving chunks and [cloud packs](./cloud-packs.md) **direct-to-backend**. Large packs use presigned **multipart upload (MPU)**, with a server proxy fallback.

## Performance

Cloud-based storage with network latency. Use for distributed teams and backup.

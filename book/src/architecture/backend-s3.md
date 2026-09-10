# s3 Storage Backend

Cloud storage backend for S3.

## Configuration

See [Storage Backend Configuration](../guides/storage-config.md) for detailed setup.

## Authentication

Config-file only — `access_key_id`/`secret_access_key` under `[storage]` in `.mediagit/config.toml`. No environment-variable fallback, and no IAM-role/instance-profile path.

## Transfer

S3 fully supports presigned transfer. The server mints presigned PUT URLs for upload and presigned GET URLs for download/clone, so chunks and [cloud packs](./cloud-packs.md) move **direct-to-backend** without proxying through the server. Large packs use presigned **multipart upload (MPU)**. A server proxy path is the fallback if signing is unavailable.

## Performance

Cloud-based storage with network latency. Use for distributed teams and backup.

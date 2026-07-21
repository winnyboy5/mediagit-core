# do Storage Backend

Cloud storage backend for DO.

## Configuration

See [Storage Backend Configuration](../guides/storage-config.md) for detailed setup.

## Authentication

Requires appropriate credentials configured via environment variables or config file.

## Transfer

DigitalOcean Spaces is S3-compatible and supports presigned transfer. The server mints presigned PUT URLs for upload and presigned GET URLs for download/clone, moving chunks and [cloud packs](./cloud-packs.md) **direct-to-backend**, with a server proxy fallback when signing is unavailable. Presigned multipart upload is not available on this backend (only S3 and MinIO implement it); large packs upload as single presigned PUTs.

## Performance

Cloud-based storage with network latency. Use for distributed teams and backup.

# b2 Storage Backend

Cloud storage backend for B2.

## Configuration

See [Storage Backend Configuration](../guides/storage-config.md) for detailed setup.

## Authentication

Requires appropriate credentials configured via environment variables or config file.

## Transfer

Backblaze B2 (via its S3-compatible API) supports presigned PUT/GET URLs. The server mints them so chunks and [cloud packs](./cloud-packs.md) move **direct-to-backend**, with a server proxy fallback when signing is unavailable.

## Performance

Cloud-based storage with network latency. Use for distributed teams and backup.

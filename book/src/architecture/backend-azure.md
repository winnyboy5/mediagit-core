# azure Storage Backend

Cloud storage backend for AZURE.

## Configuration

See [Storage Backend Configuration](../guides/storage-config.md) for detailed setup.

## Authentication

Requires appropriate credentials configured via environment variables or config file.

## Transfer

Azure Blob supports presigned transfer via **SAS (Shared Access Signature)** URLs. The server mints SAS PUT URLs for upload and SAS GET URLs for download/clone, moving chunks and [cloud packs](./cloud-packs.md) **direct-to-backend**, with a server proxy fallback when signing is unavailable.

## Performance

Cloud-based storage with network latency. Use for distributed teams and backup.

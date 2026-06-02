# gcs Storage Backend

Cloud storage backend for GCS.

## Configuration

See [Storage Backend Configuration](../guides/storage-config.md) for detailed setup.

## Authentication

Requires appropriate credentials configured via environment variables or config file.

## Transfer

GCS supports V4 presigned PUT/GET URLs, but **signing requires a service-account key**. When MediaGit authenticates via Application Default Credentials (ADC), no private key is available, signing returns null, and transfer falls back to the **server proxy** path. With a service-account JSON key, chunks and [cloud packs](./cloud-packs.md) move direct-to-backend.

## Performance

Cloud-based storage with network latency. Use for distributed teams and backup.

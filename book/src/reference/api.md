# API Documentation

## Server REST API

`mediagit-server` exposes a REST API over HTTP/1.1 (or HTTP/2 when TLS is enabled).

### Health

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| GET | `/health` | No | Health check — returns `{"status":"ok","version":"…"}` |
| GET | `/healthz` | No | Alias for `/health` (Kubernetes liveness probe) |

### Repository — Refs

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| GET | `/:repo/info/refs` | `repo:read` | List all refs (branches, tags, HEAD) |
| POST | `/:repo/refs/update` | `repo:write` | Create, update, or delete refs |

### Repository — Object Transfer

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| POST | `/:repo/objects/want` | `repo:read` | Submit want/have lists for pack negotiation |
| GET | `/:repo/objects/pack` | `repo:read` | Download negotiated pack (requires `X-Request-ID` header) |
| POST | `/:repo/objects/pack` | `repo:write` | Upload a streaming pack file |

### Repository — Chunk Transfer

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| POST | `/:repo/chunks/check` | `repo:read` | Check which chunk IDs already exist on the server |
| POST | `/:repo/chunks/upload-urls` | `repo:write` | Get presigned upload URLs for direct-to-storage uploads |
| POST | `/:repo/chunks/complete` | `repo:write` | Confirm chunk upload completion |
| GET | `/:repo/chunks/:chunk_id` | `repo:read` | Download a single chunk |
| PUT | `/:repo/chunks/:chunk_id` | `repo:write` | Upload a single chunk |

### Repository — Chunk Delta Sidecars

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| POST | `/:repo/chunk-deltas/check` | `repo:read` | Check which chunk deltas are available |
| GET | `/:repo/chunk-deltas/:chunk_id` | `repo:read` | Download a chunk delta sidecar |
| PUT | `/:repo/chunk-deltas/:chunk_id` | `repo:write` | Upload a chunk delta sidecar |

### Repository — Manifests

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| GET | `/:repo/manifests/:oid` | `repo:read` | Download a chunk manifest |
| PUT | `/:repo/manifests/:oid` | `repo:write` | Upload a chunk manifest |

### Repository — File Serving (Read-Only)

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| GET | `/:repo/files/*path?ref=HEAD` | `repo:read` | Stream a file from committed state |
| GET | `/:repo/tree/*path?ref=HEAD` | `repo:read` | List directory entries as JSON |
| GET | `/:repo/tree?ref=HEAD` | `repo:read` | List root tree entries as JSON |

### Authentication (when enabled)

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| POST | `/auth/login` | No | Obtain a JWT token |
| POST | `/auth/register` | No | Create a new user account |
| POST | `/auth/refresh` | Bearer | Refresh an expiring JWT token |
| POST | `/auth/api-keys` | Bearer | Create an API key |

## Rust API

See [docs.rs/mediagit](https://docs.rs/mediagit) for complete Rust API reference.
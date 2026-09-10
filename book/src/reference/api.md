# API Documentation

## Server REST API

`mediagit-server` exposes a REST API over HTTP/1.1 (or HTTP/2 when TLS is enabled).

### Contract version — `api-v1`

`GET /:repo/info/refs` advertises `api-v1` in its `capabilities` array. That
token names the frozen shape of every request and response body in
`mediagit_protocol::types`; a client reads it to distinguish "a protocol I
understand" from "a server newer than me".

The freeze is enforced, not promised: `crates/mediagit-protocol/tests/api_contract.rs`
serializes a canonical instance of each type and compares it to a literal
expected document, so a field renamed or retyped in a refactor fails the build.

**What may change inside `api-v1`:** adding a field, and only with
`#[serde(default)]` so senders that predate it still deserialize. Unknown fields
are ignored by design, so a newer server may send fields an older client has
never heard of.

**What may not:** removing, renaming or retyping a field, or making a defaulted
field required. Those need a new token (`api-v2`) advertised alongside `api-v1`
during the overlap — never a redefinition of `api-v1` under deployed clients.

Endpoint *additions* do not change the token: a client that has never heard of a
route simply does not call it.

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
| POST | `/:repo/chunks/download-urls` | `repo:read` | Get presigned download URLs for direct-from-storage downloads |
| POST | `/:repo/chunks/complete` | `repo:write` | Confirm chunk upload completion |
| GET | `/:repo/chunks/:chunk_id` | `repo:read` | Download a single chunk |
| PUT | `/:repo/chunks/:chunk_id` | `repo:write` | Upload a single chunk |
| POST | `/:repo/chunks/mpu/start` | `repo:write` | Start a multipart upload for a large chunk |
| POST | `/:repo/chunks/mpu/complete` | `repo:write` | Complete a multipart upload |
| POST | `/:repo/chunks/mpu/abort` | `repo:write` | Abort a multipart upload |
| POST | `/:repo/chunks/locate` | `repo:read` | Resolve chunk IDs to their pack (or loose) locations |
| POST | `/:repo/chunks/verify-integrity` | `repo:read` | Server-side BLAKE3 re-hash of stored chunks |
| POST | `/:repo/objects/verify-integrity` | `repo:read` | Server-side BLAKE3 re-hash of stored objects |

### Repository — Cloud Packs

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| POST | `/:repo/packs/upload-urls` | `repo:write` | Get presigned upload URLs for pack files |
| PUT | `/:repo/packs/:pack_id` | `repo:write` | Upload a pack via server proxy (no-presign backends) |
| POST | `/:repo/packs/complete` | `repo:write` | Register a completed pack upload |
| POST | `/:repo/packs/mpu/start` | `repo:write` | Start a multipart upload for a pack (same handlers as chunk MPU, `packs/` prefix) |
| POST | `/:repo/packs/mpu/complete` | `repo:write` | Complete a pack multipart upload |
| POST | `/:repo/packs/mpu/abort` | `repo:write` | Abort a pack multipart upload |
| POST | `/:repo/packs/presign-download-urls` | `repo:read` | Get presigned download URLs for pack ranges |
| POST | `/:repo/packs/batch-get` | `repo:read` | Batch-fetch multiple chunk slices from one pack (no-presign backends) |
| POST | `/:repo/packs/rebuild-index` | `repo:write` | Rebuild a pack's embedded index |

### Repository — Encryption Key Escrow (DC-7)

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| PUT | `/:repo/encryption-key` | `repo:write` | Store the repo's wrapped encryption key |
| GET | `/:repo/encryption-key` | `repo:read` | Retrieve the repo's wrapped encryption key |

Both 404 when `[encryption]` is off server-side, so a server without the feature is indistinguishable from one that predates it.

### Repository — File Locks

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| GET | `/:repo/locks` | `repo:read` | List active locks |
| POST | `/:repo/locks` | `repo:write` | Create a lock on a path |
| DELETE | `/:repo/locks/:lock_id` | `repo:write` | Release a lock (`--force` requires `repo:admin`) |

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
| POST | `/auth/logout` | Bearer | Invalidate the current session |
| GET | `/auth/me` | Bearer | Current user info |
| GET | `/auth/whoami` | Bearer | Self-scoped: identity, role, and per-repo grants |
| POST | `/auth/password` | Bearer | Self-scoped: change the caller's own password |
| POST | `/auth/keys` | Bearer | Self-scoped: mint an API key (admin may target another `user_id`) |
| GET | `/auth/keys/mine` | Bearer | Self-scoped: list the caller's own API keys |

### Administration (requires `user:manage`, i.e. Admin role)

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| GET | `/auth/users` | Admin | List users |
| POST | `/auth/users` | Admin | Create a user with an explicit role |
| DELETE | `/auth/users/:id` | Admin | Delete a user (cascades their grants) |
| PATCH | `/auth/users/:id/role` | Admin | Change a user's role |
| PATCH | `/auth/users/:id/disabled` | Admin | Suspend or restore a user without deleting it |
| PATCH | `/auth/users/:id/password` | Admin | Reset a user's password (no current password needed) |
| POST | `/auth/users/:id/grants` | Admin | Create or update a per-repo grant (Read/Write/Admin) |
| DELETE | `/auth/users/:id/grants` | Admin | Remove a per-repo grant |
| GET | `/auth/keys` | Admin | List all API keys (metadata only) |
| DELETE | `/auth/keys/:id` | Admin | Revoke an API key (caller's own, or any as admin) |

See [Authentication](./authentication.md) for the auth model, persistence, and grant semantics.

## Rust API

See [docs.rs/mediagit](https://docs.rs/mediagit) for complete Rust API reference.
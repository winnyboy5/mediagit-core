# Azure Blob Storage Backend

Cloud storage backend for Microsoft Azure Blob Storage, built on
[Apache OpenDAL](https://opendal.apache.org/).

> **Why OpenDAL.** The community `azure_storage_blobs` 0.21 SDK is EOL (moved
> to `/tree/legacy`) and carried several RUSTSEC advisories. Microsoft's GA
> line (`azure_storage_blob` 1.x) authenticates with Microsoft Entra ID only
> and cannot use a shared account key
> ([azure-sdk-for-rust#2975](https://github.com/Azure/azure-sdk-for-rust/issues/2975)),
> so it is not a drop-in replacement. OpenDAL supports shared key, SAS,
> connection strings, and the Azurite emulator.

## Configuration

Credentials go in a tagged `auth` block under `[storage]`
(`config_version` 3+) — see
[Storage Backend Configuration](../guides/storage-config.md#azure-blob-storage).
Exactly one of:

- `account_key` — `account_name` + `account_key`
- `connection_string` — `value`
- `sas` — `account_name` + `token`
- `emulator` — no fields; local Azurite with its published dev credentials

A pre-v3 flat config (`account_name`/`account_key` directly under `[storage]`)
is migrated automatically on first open.

## Container creation

OpenDAL is data-plane only, so an absent container is created with a single
Shared-Key-signed REST call (via `reqsign-azure-storage`, the signer OpenDAL
itself uses). This needs the account key, so SAS-authenticated backends
require the container to already exist and report a clear error otherwise.

## Transfer

Presigned transfer uses **Service SAS** URLs minted from the account key: the
server issues SAS PUT URLs for upload and SAS GET URLs for download/clone,
moving chunks and [cloud packs](./cloud-packs.md) **direct-to-backend**, with a
server-proxy fallback when signing is unavailable.

> **Testing note.** Presigned URLs cannot be exercised against the **Azurite**
> emulator: OpenDAL emits a Service SAS at `sv=2020-12-06`, which Azurite
> rejects even though real Azure accepts it (verified against a live account).
> Presign coverage therefore lives in the live-Azure leg of the QA campaign's
> remote phase, not in the Azurite suite. Non-presign operations (CRUD,
> chunked upload, listing, auto-create) are fully covered against Azurite.

## Performance

Cloud-based storage with network latency. Use for distributed teams and
backup. The 4 MiB staged-block upload concurrency is tunable via
`MEDIAGIT_AZURE_PUT_BLOCK_CONCURRENCY` (default 8).

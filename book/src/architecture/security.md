# Security

MediaGit security model and best practices.

## Authentication
- **Local**: File system permissions
- **Server mode**: JWT tokens + API key authentication, both off by
  default (`enable_auth = false`). A non-loopback server refuses to start
  with auth disabled unless `MEDIAGIT_ALLOW_INSECURE_BIND=1` is set.
- **Cloud**: IAM roles, service principals, API keys

See the [Authentication reference](../reference/authentication.md) for the
full model: endpoints, JWT/API key mechanics, and the client credential
resolution order.

### Persistence

When auth is enabled, user accounts, API keys, and per-repo grants are
persisted as JSONL files (`users.jsonl`, `api_keys.jsonl`, `grants.jsonl`)
under the server's auth store directory, written via tmp+rename on every
mutation. A corrupt or unparsable store file is a **hard error at
startup** rather than silently starting empty — "no users registered" and
"storage is broken" must never look the same. Set
`MEDIAGIT_AUTH_PERSIST=0` to force in-memory-only behavior.

### Per-Repo Grants

Beyond the flat `Read`/`Write`/`Admin` role a user account carries, the
server supports per-repo grants (`{user_id, repo, level}`, levels ordered
`Read < Write < Admin`) that scope a user's access to specific
repositories. A zero-grants deployment (or `MEDIAGIT_GRANTS_ENFORCE=0`)
behaves exactly like the flat role check; once any grant is recorded,
per-repo enforcement activates for every repo-scoped permission check.
Admin-role users (`user:manage`) always bypass grant checks.

### File Locking

[`mediagit lock`](../cli/lock.md) is a security-adjacent feature that sits
on top of the same auth identity: a lock's owner is the authenticated user
when auth is enabled, and push-time enforcement
(`MEDIAGIT_LOCKS_ENFORCE`, on by default) rejects pushes that touch a path
locked by someone else. With auth disabled, a push can never prove it owns
a lock, so any touched, locked path rejects the push unconditionally.

## Data Integrity
- BLAKE3 hashing for all objects (content-addressed OIDs)
- Cryptographic verification on read
- `mediagit verify` for repository health

## Path-Traversal Hardening

Every storage backend and the server's chunk/pack handlers reject
caller-supplied keys and object ids that could escape the intended
storage root: `validate_object_key` (the single choke point every
`StorageBackend` implementation calls before turning a key into a
filesystem path or remote object key) rejects `..` components, absolute
paths, and Windows drive/UNC prefixes on both `/`- and `\`-separated
input; server-side handlers additionally enforce that chunk/object ids
are exactly 64 hex characters before using them to build a path. Together
these close a class of bug where a malformed id could otherwise read or
write outside a repo's namespace — a cross-tenant escape on a
multi-repo server.

## Encryption
- **At-rest (MediaGit's own)**: **implemented; local-only.**
  The object envelope exists and is wired: `mediagit-security` implements AES-256-GCM
  with Argon2id key derivation, and `SmartCompressor` seals every object it writes
  and opens every sealed object it reads (`MGEN`, `FORMATS.md` §10b). Key management
  is implemented too (`mediagit key init/status/recover`) — a master key from a
  passphrase (Argon2id), the OS keychain, or a keyfile/env var. What's still missing
  is delivering that key to a client for the presigned-upload path, so `push`
  **hard-refuses** on an encrypted repository by design — encrypted repos can only be
  used entirely locally today. With no key configured the bytes written are
  byte-for-byte identical to a build without the feature, which is asserted by test.
  Tracked as DC-7 (D4 open).
- **At-rest (cloud)**: **also not wired.** `[storage] encryption` and
  `[storage] encryption_algorithm` are parsed and *validated* (`validation.rs:130`
  rejects anything but `AES256` / `aws:kms*`), which makes them look live — but no
  storage backend reads either field. Grep for `ServerSideEncryption` / `sse_algorithm`
  returns nothing: no `PutObject` call sets an SSE header on any backend. Setting these
  keys has no effect.
  These docs previously described this as "a *different, real* thing" from the above.
  It is different; it is not real. The 2026-07-29 correction fixed the client-side claim
  and introduced this one in its place. Verified 2026-08-05.
  Bucket-level encryption configured **outside** MediaGit (an S3 bucket default, an
  Azure storage-account policy) does of course still apply — it simply has nothing to do
  with these config keys.
- **In-transit**: TLS 1.3 on the server's TLS listener by default; set `tls_min_version = "1.2"` in `mediagit-server.toml` as an escape hatch for TLS 1.2-only clients/proxies (any other value fails config load). Client certificates (mTLS) are **not** wired — `TlsConfig` carries the fields but the server has no knob to set them and builds with `with_no_client_auth`.

## Best Practices
1. Use IAM roles (avoid hardcoded keys)
2. Enable bucket versioning
3. Regular `mediagit verify` checks
4. Restrict branch protection rules
5. Audit logs for sensitive repositories
6. Prefer per-repo grants over the flat role system on multi-tenant servers

## See Also

- [Authentication](../reference/authentication.md) - full auth model, endpoints, persistence, and client credential resolution
- [mediagit lock](../cli/lock.md) - server-enforced file locking
- [Configuration Reference](../reference/config.md) - security settings

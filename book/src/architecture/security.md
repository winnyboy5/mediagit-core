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
repositories. Enforcement is decided **per repository**: a repo with no
grants recorded behaves exactly like the flat role check, regardless of how
many grants exist on other repos, so onboarding one tenant cannot change how
any other repo is authorized. `MEDIAGIT_GRANTS_ENFORCE=0` disables grants
everywhere; `=strict` enforces on every repo including ungranted ones, which
then deny rather than falling back. Admin-role users (`user:manage`) always
bypass grant checks.

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
- **At-rest (MediaGit's own)**: **implemented, including push and clone.**
  `mediagit-security` implements XAES-256-GCM with Argon2id key derivation, and
  `SmartCompressor` seals every object it writes and opens every sealed object it
  reads (`MGEN` v2, specified in `docs/FORMATS.md` §10b — a local-only working
  doc, gitignored and absent from a fresh clone). Key management is implemented
  (`mediagit key init/status/recover/rotate-master`) — a master key from a
  passphrase (Argon2id), the OS keychain, or a keyfile/env var.

  Push and clone work through **key escrow** (DC-7 D4). On the first push the
  client hands its repository key to the server over `PUT /{repo}/encryption-key`;
  the server wraps it under its own master key — read from the file named by
  `[encryption] master_key_path` in `mediagit-server.toml`, not from an
  environment variable — and keeps it in `<repo>/.mediagit/key.json`. A server
  started with `[encryption] enabled = true` and no readable master key
  refuses to bind at all, rather than accepting escrow requests it could not
  honour and letting the operator hear about it from a user. It needs the key because
  presigned uploads go client→bucket directly, leaving the server holding objects
  it must still verify, register and walk. A clone fetches the key back with
  `repo:read` — key access *is* read access — and re-wraps it under a local
  master. Escrow never overwrites: a different key is `409 Conflict`, because
  replacing it would orphan everything already sealed under the first.

  The threat model is a compromised **object store**, not a compromised server.

  Encryption is enabled at repository creation or not at all: `key init` refuses
  on a repository that already holds objects. Encrypting an existing repository
  needs a full re-seal pass, which is not built.

  With no key configured the bytes written are byte-for-byte identical to a build
  without the feature, which is asserted by test.
- **At-rest (cloud)**: **also not wired.** `[storage] encryption` and
  `[storage] encryption_algorithm` are not fields at all — `S3Storage`
  (`schema.rs:396-418`) has only `bucket`, `region`, `access_key_id`,
  `secret_access_key`, `endpoint` and `prefix`. Nothing validates them, because
  nothing recognises them. The config layer does not set `deny_unknown_fields`,
  so they are silently discarded exactly like any typo — a config carrying
  `encryption = true` loads without a murmur, and so does one carrying
  `totally_made_up_key = 42` (verified against the shipping binary). Grep for
  `ServerSideEncryption` / `sse_algorithm` returns nothing: no `PutObject` call
  sets an SSE header on any backend.

  > Because unknown keys are dropped in silence, a misspelled key anywhere in
  > `config.toml` is indistinguishable from one you never wrote. Check spelling
  > against [Configuration](../reference/config.md) rather than trusting a
  > clean startup.
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

# Changelog

All notable changes to MediaGit will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [v0.3.0-rc.3] - 2026-08-12

At-rest encryption, plus a correctness and transfer-reliability cycle.

**Compat.** rc.3 adds two things that are new rather than changed: the `MGEN`
object envelope (persisted, additive, and written only by a repository that
opted in) and the key-escrow endpoints (a new wire API). The
`docs/FORMATS.md` §11 promise, in effect since v0.3.0-rc.1, still holds —
nothing changes how an existing format reads, and with no key configured the
bytes written are byte-for-byte what they were before, which is asserted by
test and by the frozen-fixture gate.

### Fixed — rate limiting made usable, and actually tested

**If you generated a config with `mediagit-server init`, ordinary pushes were
being rejected with HTTP 429.** That path enabled rate limiting and left the
budget at the serde defaults of 10 requests/second, burst 20 — while a push
costs roughly one request per chunk. Anyone who started from
`mediagit-server-production.example.toml` was unaffected.

The budget is now **1000 rps / 2000 burst**, which is the number
`RateLimitConfig::default()` has documented since the `psds` incident and which
had simply never been reachable: `config.rs` and `security.rs` each carried
their own defaults, and the serde pair won.

- **The limiter is keyed per credential again.** `IdentityOrIpKeyExtractor`
  existed, was documented as the key, and had no callers — the router hand-
  inlined a second builder keyed by client IP, so every user behind one NAT, VPN
  or CI runner pool drew on a single shared budget. The duplicate builder is
  gone; both listeners now go through one.
- **HTTPS plus rate limiting returned 500 on every request.** The TLS listener
  was served without `ConnectInfo`, which the IP extractor requires, on the port
  most likely to face the internet. Invisible until now because rate limiting is
  off by default and no test had ever switched it on.
- **Client retry now covers the whole API.** 429 retry was wired into three
  call sites, all on one upload path; presign, pack, complete, refs, locks,
  escrow and the pull path had none. All server-bound calls now route through
  the same chokepoint. Presigned storage requests deliberately do not — those
  are handled by the storage-error classifier.
- **Backoff is jittered.** It was `250ms * 2^n` with no spread, so clients that
  were limited together retried together. Now Full Jitter, and `Retry-After` is
  honoured on the pull path too, which previously ignored it.

Rate limiting is still **off by default**; this changes what you get when you
turn it on.

### Fixed — encryption state is checked on every transfer

Two gaps, neither of them the deferred re-seal work:

- **An unencrypted repository could push plaintext into an encrypted one.**
  `push` checked the remote's key only when the *local* repository had one, so a
  clone without a key uploaded unsealed objects into a repository the server
  considered encrypted — silently, because reads pass unsealed bytes through.
- **`fetch` and `pull` had no encryption handling at all**, so sealed objects
  failed deep in the compressor with a message about MGEN envelopes that named
  nothing actionable.

All four transfer commands now share one check, so they cannot drift apart
again. `mediagit init` and `mediagit key status` also now state that encryption
is an empty-repository decision, at the point where it can still be acted on.

### Fixed — the QA suite was measuring a rate limiter that was not running

No harness path ever set `enable_rate_limiting`, so roughly 200 gates per
campaign ran against a disabled limiter. `07_abuse`'s A13 — whose entire
assertion is that a per-chunk push must *not* trip 429 — could not fail. Rate
limiting is now on for every QA server, and a new `07_ratelimit` phase proves
enforcement, the `Retry-After` and `x-ratelimit-*` headers, that the shipped
defaults carry real pushes and clones, and that the tighter public profile is
survivable. New `06_encrypted` drills cover both encryption fixes above.

Also fixed: the `Stream was not readable` harness fault that voided the
`S2-churn` and `S3-conflicts` scale drills across three campaigns. It was
`Add-Content` losing a race on a log file and raising `ArgumentException`, which
the retry helper written for exactly that race did not catch — it caught only
`IOException`.

### Added — at-rest encryption (DC-7)

Opt-in per repository, at creation time: `mediagit key init` on a fresh repo,
then commit as usual. Objects, chunk manifests and reachability bitmaps are
sealed under a per-repository key; that key is wrapped under a master taken
from a keyfile/env var, the OS keychain, or a passphrase (Argon2id, m=64 MiB
t=3 p=4). A one-time recovery code is a second, independent way in.

- **`MGEN` v2 envelope, XAES-256-GCM.** Per-message subkey via the SP 800-108r1
  KDF and a 192-bit nonce, which lifts NIST SP 800-38D's 2³² messages-per-key
  cap to roughly 2⁸⁰ — the repository key is permanent and never rotates, so
  the original cap was reachable. Magic and version are bound as AAD, nonces
  come from `getrandom`, and a BLAKE3 fingerprint in the key file gives the
  key commitment AES-GCM does not. Implemented inline against the `aes` crate
  already present and checked against the published C2SP test vectors: no new
  dependency, and none of it is a pre-release AEAD.
- **Push and clone work, via key escrow.** On the first push the client hands
  its repository key to `PUT /{repo}/encryption-key` (`repo:write`); the
  server wraps it under `MEDIAGIT_SERVER_ENCRYPTION_KEYFILE` and keeps it in
  `<repo>/.mediagit/key.json`. It needs the key because presigned uploads go
  client→bucket directly, leaving the server holding objects it must still
  verify, register and walk. A clone fetches it back with `repo:read` — key
  access *is* read access — and re-wraps it under a local master without
  prompting. Escrow never overwrites: a different key is `409 Conflict`,
  because replacing it would orphan everything already sealed under the first.
- **`mediagit key rotate-master`** re-wraps the same repository key under a new
  master, for the stolen-laptop case. The repository key itself does not
  change, so nothing needs re-sealing and escrow is untouched.
- **The threat model is a compromised object store**, not a compromised server.

Known limits, deliberate for this release: encryption can only be enabled on an
**empty** repository (`key init` refuses otherwise — encrypting an existing one
needs a full re-seal pass, which is not built); there is no full repository-key
rotation; and `chunk-deltas/*.meta` sidecars stay plaintext, leaking which
chunk deltas against which base — shape, not content.

### Fixed — the rest of the cycle

The headline is a cloud-upload defect that had been costing roughly 200
permanently-failed chunk uploads per large push while remaining invisible: the
client fell back to a slower path and the affected gate had a floor low enough
to pass anyway. Fixing it took MinIO's 10 GB push from a failure to comfortably
inside its SLO, and made an 11 GB clone that had been attributed to MinIO's own
limits complete with byte parity. All four backends — MinIO, AWS S3, Azure and
GCS — now push *and* clone with verified byte parity.

### Fixed
- **Presigned PUT sent two `Content-Length` headers, so the signature could not
  verify** (transfer reliability, cloud): the per-chunk upload path set
  `CONTENT_LENGTH` explicitly *and* replayed the server's signed
  `required_headers`, which already carry one whenever the server presigns a
  concrete length. `reqwest::RequestBuilder::header` **appends** rather than
  inserts, so both reached the wire, and SigV4 signs `content-length` — a
  duplicated signed header does not canonicalize back to what was signed, and
  S3/MinIO answer `SignatureDoesNotMatch`. Two QA campaigns logged 976 and 748
  of them, with a permanent-failure counter reaching 198. It stayed hidden
  because the failure classifies as permanent, so each chunk silently fell back
  to the authenticated server-proxy relay and the push still completed. **No
  data was ever at risk** — `verify_chunk_uploads` re-checks every chunk — but
  the cost was a wasted direct PUT plus a slower relayed round-trip, roughly 200
  times per large push. Measured effect on a 10,990 MB MinIO push: **9.87 MB/s
  and a failed push → 55.14 MB/s and a clean one**, with the 11 GB clone
  completing at 67 MB/s with byte parity for the first time. The pack upload
  path never had the defect, because it only ever replays `required_headers`.
- **Azure transfers timed out on slow links** (transfer reliability): the
  OpenDAL `TimeoutLayer` was constructed with its default **10 s `io_timeout`**,
  which is a *per-IO* deadline rather than a whole-operation one. That is
  reasonable on a LAN and wrong over a WAN — a 2 GB push at ~1.8 MB/s with
  concurrent block writes exceeded it, the server returned 500, the client's
  retry hit the same wall, and the push failed outright. Raised to 120 s and
  made configurable via `MEDIAGIT_AZURE_IO_TIMEOUT_SECS`; `0` falls back to the
  default rather than being honoured, since OpenDAL reads it as an
  already-expired deadline. The non-IO timeout keeps OpenDAL's default, because
  stat/delete/list are small round-trips where a long hang is a real fault worth
  surfacing quickly.
- **A single transient chunk download aborted an entire clone** (transfer
  reliability): the parallel download path had no retry at all, so one `503`
  discarded a multi-GB clone along with every byte already transferred. Adds a
  bounded retry (3 attempts, 500 ms doubling) for 5xx, 429 and transport errors.
  Deliberately does not retry verdicts: `404`/`403` are answers, and `409` is
  load-bearing — the server returns it when a chunk is delta-only and the client
  re-routes accordingly. The existing pull deadline still bounds the whole
  download.
- **Storage retry exhaustion reported no cause** (diagnosability): the storage
  layer attaches the underlying error via `.context()` and its message ends
  "last error follows" — but the chunk handlers rendered it with plain
  `Display`, which prints only the outermost context. Every exhausted retry
  therefore ended at "follows" with nothing following it: 4,092 undiagnosable
  errors across two campaigns, including the one failure that sank a throughput
  gate. All three sites now render the full chain.
- **Actionable `503` guidance was unreachable** (diagnosability): the message
  naming the operator action ("server storage backend unreachable — verify the
  storage service is running") existed only on the sequential chunk-download
  path, while clone uses the parallel one. Both now share a single error
  constructor, so they cannot drift apart again.

### Fixed (QA harness — these gate the release, so their defects hide product bugs)
- **A drill that failed to run was indistinguishable from one that measured a
  failure.** Three scale drills died mid-campaign and reported `FAIL` exactly as
  a real product defect would, so churn-cost, conflict-data-loss and peak-RSS
  silently had no result for that build while appearing to be three bugs. Adds a
  distinct `ERROR` verdict carried through the phase runner, gate table and
  report, plus a `campaign-no-harness-errors` gate — because separating errors
  out of the failure count would otherwise have let a campaign that voided three
  drills report "no gate failures".
- **The performance gate counted what it *could* have compared, not what it
  did.** Its coverage number counted size-eligible records rather than records
  actually held against a baseline; while the baseline lacked `commit` rows,
  every campaign reported healthy-looking coverage while comparing half as much,
  and the "measured nothing" guard could not fire. Now counts actual
  comparisons, fails when none occur, and names any measurement that had no
  baseline row.
- **Unquoted phase numbers ran the wrong phases.** PowerShell strips a leading
  zero from an unquoted numeric argument and phase tokens glob, so `00` became
  `0` and matched nineteen scripts including the report aggregator, while `1`
  matched the memory profiler — which must run alone. Single digits are now
  normalized, and the correction is logged rather than applied silently.

### Changed
- The S3 range-read resume path added in the previous cycle is now documented as
  never having executed: the failures it was introduced for are dispatch
  failures, which by definition occur before any response exists and therefore
  cannot reach a loop that runs only after one succeeds. The code is retained —
  it remains correct for genuine mid-body interruptions — but its original
  justification was a misattribution and is corrected in place, and a successful
  resume is now logged at a level the default filter does not discard.

### Fixed (GA correctness program, earlier in this cycle)
- **Clone could silently omit objects and still report success** (data integrity,
  P0): the want-side walk (`collect_objects_bfs`) read each object with
  `odb.read(..).ok()` and, on `None`, logged a warning and continued — dropping
  that object from the pack *and* abandoning its entire subtree, then answering
  `200`. The client streams to the object count declared in the pack header, so a
  short pack is indistinguishable from a complete one; the damage surfaced much
  later as `Object <oid> not found: no loose object and no pack files` in a
  repository whose clone had reported success. The server now refuses to serve an
  incomplete closure and names the offending object, and the client surfaces that
  message instead of a bare `500`. Leniency remains correct on the *have* side
  (`walk_reachable`), where a client may legitimately name objects that do not
  exist; that asymmetry is now documented at both sites. This was the root cause
  of the intermittent `tag_object_push_clone_round_trips_*` failure.
- **Checkout silently overwrote files whose paths differ only by case** (data
  loss): `Tree` keys entries case-sensitively, so a commit made on Linux can hold
  both `Logo.psd` and `logo.psd`; checking that out on Windows or default macOS
  wrote one over the other and reported both written. Checkout now refuses, with
  case-insensitivity probed from the filesystem rather than inferred from the
  platform.
- **`fsck` reported healthy delta chains as too deep**: depth was compared and
  printed as visited *nodes* rather than hops, so a chain at exactly
  `MAX_DELTA_DEPTH` — the deepest the writer can build — was flagged
  "11 hops deep (max 10)". Warnings no repair could ever clear.
- **`fsck --repair` could not repair a corrupt chunk, and claimed success anyway**:
  corruption inside a chunked blob was reported under the *manifest's* oid, so
  repair probed the loose path for an object that lives at `manifests/<oid>`,
  found nothing and blamed "likely packed". Chunks are now identified from the
  manifest and repaired directly. Separately, `Successfully repaired 0 issue(s)`
  no longer prints a green check for work that did not happen.
- **Transfer byte counts were wrong or absent**: `bytes_uploaded` was assigned the
  metadata pack's size and the chunk upload's own count was discarded, so a
  15.53 GiB push summarised as `up 2.71 KiB`; `bytes_downloaded` was never assigned
  anywhere, so pull/clone/fetch reported no transfer at all.
- **Progress rate and ETA were fabricated when unknown**: `eta 0s` at 0% and
  `33 B/s` mid-transfer were an absent measurement rendered as fact, and resetting
  the ETA on every pack seal produced readings like `747 MiB/s`. Unknown values now
  render `--`.
- **HTTPS listener had no rate limiting**: enabling TLS silently disabled it on the
  port most likely to face the internet, while startup still logged
  "Rate limiting ENABLED". Both listeners now share one limiter — sharing rather
  than rebuilding, so splitting traffic across ports cannot double the budget.
- **`logout` did nothing server-side.** It returned 204 while the token kept
  authenticating for the rest of its life, so a token captured beforehand still
  worked. Tokens now carry an id and are revoked on logout — the presented
  token only, so logging out of one machine does not sign the user out
  everywhere.
- **Deleting a user left their file locks held forever** by an account that no
  longer existed: nobody else could take the lock and the only party entitled
  to release it was gone. Deletion now releases them.
- **Grants were accepted for users and repositories that do not exist.** A
  mistyped id read as "access granted" in every listing while the real user
  still had none.
- **The bootstrap admin password is no longer a command-line flag.**
  `--admin-password` put the credential in `ps` output and shell history; it is
  now read from `MEDIAGIT_ADMIN_PASSWORD`.
- **Access and refresh tokens were interchangeable**: an access token could refresh
  itself indefinitely (so a session never needed re-authentication) and a 30-day
  refresh token could authenticate requests directly. They now carry a type.
- **Pack objects at or above 4 GiB silently truncated the pack**: the 4-byte size
  field was written with an unchecked cast, so the header understated the length
  and every following object in the pack was read from the wrong offset.
- **Operator-supplied pack caps were unclamped**: `MEDIAGIT_PACK_BYTES=0` sealed one
  pack per chunk — back to one cloud object per chunk, the problem cloud packs
  exist to solve — and an over-large value is an OOM, since peak push memory is
  `PACK_BYTES x PACK_UPLOAD_CONCURRENCY`. Both knobs, and the concurrency
  multiplier, are now bounded and correct loudly.

### Added
- **`mediagit config`** — `get`/`set`/`unset`/`list` for repository settings.
  Previously `init` and `commit` help referenced a `mediagit-config(1)` that did
  not exist, and with `commit` now requiring a configured author the only
  remedy on offer was hand-editing TOML.
- **Accounts can be suspended without being deleted** (`PATCH
  /auth/users/{id}/disabled`). Deletion was the only way to stop someone signing
  in, forcing a choice between leaving access open and destroying the record of
  what they did. Takes effect on the account's next request.
- **API keys record `last_used`**, so a leaked key is distinguishable from an
  unused one and stale keys can be found. Recorded at coarse granularity
  (`MEDIAGIT_APIKEY_LAST_USED_RESOLUTION`, default 300s) to keep a disk write
  off the authentication hot path.
- **`log --since` / `--until` now filter.** They were declared, demonstrated in
  `log --help`, and never read, so a date-bounded log returned the entire
  history. Accepts `YYYY-MM-DD` and RFC 3339; `--until <date>` includes that
  whole day.
- **`push --force-with-lease` now does something.** Previously declared and
  never read, so asking for the safe option performed an ordinary push.

### Changed
- **Media-aware merge strategies no longer claim auto-merges they cannot
  perform** (data integrity). `MergeResult::AutoMerged(Vec<u8>)` means "these
  bytes are the merged file", and a caller writes them straight to the working
  tree. The PSD, video, audio and image strategies returned serialized
  *metadata* there — a successful PSD "auto-merge" produced a JSON document —
  while logging "auto-merge successful". They were never wired into `merge`, so
  no user was affected, but wiring them (as was planned) would have replaced a
  designer's `.psd` with JSON. They now report an informative conflict instead.
  MediaGit can analyse whether edits overlap; it cannot yet write a merged file
  back in these formats.
- **Dead flags now refuse instead of silently doing nothing**: `diff
  --word-diff`, `show --stat` (use `diff --stat`), `pull --abort` (use `merge
  --abort`), `pull --no-commit`, `gc --aggressive`. Each is hidden from help and
  errors when used, following the existing `rebase --rebase-merges` pattern.
- **`commit` refuses an unconfigured author** rather than recording
  `Unknown <unknown@localhost>`. Authorship cannot be changed afterwards, and
  the previous `$USER` fallback was unset on Windows, so most unconfigured
  commits on that platform were attributed to nobody.
- **Documentation now matches the code.** The README claimed AES-256-GCM
  encryption at rest as shipped; the module exists and is tested but has no CLI or
  server call sites (the real `[storage] encryption` setting is S3 server-side
  encryption, a different thing — the two are no longer conflated). The book
  documented PSD layer merge and video timeline merge with worked "Auto-merge"
  examples; `mediagit_media::MergeStrategy` has no callers, and the section now
  describes what merge actually does with a binary conflict. Windows ARM64 was
  listed "Supported" while no such binary is built.

### Fixed (earlier in this cycle)
- **Stored objects whose content began with a codec magic were unreadable** (data
  integrity, P0): `SmartCompressor` writes incompressible data as `0x00 + raw`, but
  `decompress_typed` stripped that prefix only when the remaining bytes did not look
  compressed. An object starting `78 F9`/`78 DA` (valid zlib headers), `28 B5 2F FD`
  (zstd) or `BRT\x01` (brotli) was returned one byte too long, failed its oid check, and
  could never be read again — `add` and `commit` succeeded, then the repo refused to
  push. Roughly 1 object in 8000. The prefix is now stripped unconditionally, since no
  codec we emit can begin with `0x00`. **Existing repositories self-heal on upgrade**:
  the bytes on disk were always correct, only the read framing was wrong, so no repair,
  migration, or format change is involved.

### Removed
- **`MEDIAGIT_PACK_MIN_CHUNKS`**: removed along with the dead `PackBuilder::flush_at_boundary` it only backed (zero callers, and unguarded `=0` could panic in `seal()`). `finish()` remains the only flush path.
- **`mediagit branch merge`**: the subcommand was never implemented — it only ever
  errored, telling the user to run `mediagit merge`, which already performs the merge.
  Removed rather than shipped as a documented command that cannot succeed. Deferred as
  future work should branch-scoped merge semantics ever diverge from `mediagit merge`.

## [v0.3.0-rc.2] - 2026-07-22

Toolchain and edition modernization — no wire/persisted-format changes, so the
`docs/FORMATS.md` §11 compat promise (in effect since v0.3.0-rc.1) is preserved
(verified byte-for-byte by the frozen-fixture fsck).

### Added
- **Azure backend migrated to OpenDAL**: `mediagit-storage`'s Azure Blob
  backend now runs on `opendal`'s `services-azblob`, replacing the EOL
  `azure_storage`/`azure_storage_blobs` crates.
- **Pack-mode `[bench]` instrumentation**: `throughput_mbs`, presign, and
  coalesced-range counters — previously always zero on the default pack
  path — are now populated.
- **Range-GET hardening**: reject `200`-status responses to a nonzero-offset
  range request that would otherwise mis-slice the body; short/truncated
  bodies fall back to per-chunk GET instead of silently serving bad bytes.
- **Chunk-delta directory resolution helpers** for locating a chunk's delta
  directory consistently across callers.
- **Phase 10 (SCALE) QA tier**: concurrency/churn/conflict/RSS/throughput
  drills added to `dev-tests/qa-suite`.

### Changed
- **Rust toolchain → 1.97.1** (from 1.92.0). Pinned via a new
  `rust-toolchain.toml`; CI `RUST_VERSION` and the MSRV gate track it. MSRV
  (`rust-version`) raised `1.92.0` → `1.97`.
- **Edition 2021 → 2024** across all 14 crates (`cargo fix --edition`), plus
  `rustfmt` `style_edition = "2024"`. Migration is semantics-preserving:
  `env::set_var`/`remove_var` (now `unsafe` under edition 2024) are almost all
  test-only; `expr` macro fragments pinned to `expr_2021`; and `if let … else`
  scrutinees rewritten to `match` to preserve 2021 temporary-drop order.
- **GCS backend hardening:** `GcsBackend::new`/`with_config` no longer mutate the
  process-global `GOOGLE_APPLICATION_CREDENTIALS` env var to load a service
  account — credentials are now passed explicitly to the storage/control clients
  and signer. Removes a latent `set_var` data race in the multi-threaded server.
- **Dependencies:** `Cargo.lock` refreshed within existing semver ranges
  (`cargo update`; no direct-dependency major bumps); `cargo audit` clean.
- **`unsafe_code` lint `forbid` → `deny`** (workspace lint table, inherited by
  `mediagit-security`/`-config`/`-compression`) so audited, test-only
  `env::set_var` sites can carry a scoped `#[allow(unsafe_code)]`. One production
  site remains — `mediagit-cli` startup sets `MEDIAGIT_REPO` on its dedicated
  single-threaded runtime thread (no concurrent env access; audited safe).

## [v0.3.0-rc.1] - 2026-07-18

Collaboration primitives, auth persistence, and a GA format freeze. Version
bumped from `0.2.8-beta.1` — a compat promise is now in effect (see
`docs/FORMATS.md` §11): breaking a frozen wire/persisted format requires a
version bump and a hard-error reader, never a silent misparse. Verified by
the 2026-07-16 release-build QA campaign (`reports/20260716-172951`):
STANDARD suite green on all 4 backends (MinIO/AWS/Azure/GCS), zero findings.

### Added
- **Server-enforced file locking**: new `mediagit lock create|unlock|list`
  command. Locks are stored server-side (`.mediagit/locks.jsonl`) with three
  HTTP endpoints; `push` enforces locks by tree-diffing the pushed commit
  range against active locks (`MEDIAGIT_LOCKS_ENFORCE`, default on;
  `MEDIAGIT_LOCKS_MAX_COMMITS`, default 1000, fails open on oversized ranges).
  `lock unlock --force` releases someone else's lock (requires `repo:admin`).
- **Auth persistence**: users, API keys, and per-repo grants now persist to
  `users.jsonl` / `api_keys.jsonl` / `grants.jsonl` (versioned `{"v":1}`
  envelopes, atomic tmp+rename writes) instead of living only in memory.
  `MEDIAGIT_AUTH_PERSIST` (default on).
- **Per-repo authorization grants**: `GrantsStore` with a `Read ⊂ Write ⊂
  Admin` hierarchy, checked per `{repo}` instead of globally
  (`MEDIAGIT_GRANTS_ENFORCE`). Previously a `Write`-role user could push to
  any repo name on the server.
- **Admin endpoints**: `GET`/`DELETE /auth/users` (+ `/{id}/grants`),
  `GET`/`DELETE /auth/keys` — gated on the `user:manage` permission,
  metadata-only responses.
- **OS-keychain credential storage** for CLI remote credentials (`keyring`
  crate; Windows Credential Manager). Lookup order: env → keychain (service
  `mediagit`, account = remote URL) → `config.toml`. Written through only
  after a verified server response; `MEDIAGIT_NO_KEYRING` opts out; keychain
  failures degrade silently to the existing config-file path.
- **`gc --repack` chunk consolidation**: loose chunks are now folded into
  Track-F cloud packs during repack (64 MiB / 1024-chunk caps, per-pack JSONL
  index), not just loose objects. Abort-safe write order (pack → index →
  memory → delete-loose). `MEDIAGIT_REPACK_CHUNKS=0` restores the previous
  (loose-objects-only) behavior.
- **Object-level and pack-aware repair**: `push --repair` and the server's
  chunk verify-integrity endpoint (`/{repo}/chunks/verify-integrity`) now
  detect and evict corrupted entries from Track-F cloud packs, not just loose
  `chunks/` objects; `ObjectDatabase::delete_object` supports targeted
  object-level repair.
- **Startup backend connectivity probe** (`MEDIAGIT_STARTUP_PROBE`, default
  on): storage backends are probed at boot instead of surfacing bad
  credentials as a 500 on the first client request.
- **`/metrics` endpoint** wired into the server binary behind
  `MEDIAGIT_METRICS_ADDR` (off by default) — the `mediagit-metrics` crate was
  previously built but never linked into `mediagit-server`.
- **Graceful shutdown** on all serve paths (HTTP, HTTPS, HTTP+HTTPS
  concurrent) — `ctrl_c`/SIGTERM now drains in-flight requests instead of
  hard-stopping mid-upload.
- **Client-side push deadline** (`MEDIAGIT_PUSH_DEADLINE_SECS`, default
  3600s) — bounds `upload_pack`/`upload_chunked_objects`/`update_refs` so a
  mid-push backend outage fails fast with a clear error instead of hanging.
- **Format freeze + compat promise** (`docs/FORMATS.md`): all 10
  persisted/wire formats inventoried and frozen — pack v3 header, chunk
  manifest (`MGCM` envelope), chunk-delta `.meta`, `LAYOUT` v2 marker,
  auth/locks JSONL, JWT claims, HTTP DTOs, BLAKE3 OID. Every versioned format
  now hard-errors on an unknown or higher version instead of silently
  misparsing.
- New docs: `docs/OPERATIONS.md` (backup/restore), `docs/DEPLOYMENT.md` (TLS
  direct + reverse proxy), `BENCHMARKS.md`, `docs/PRODUCTION_ROADMAP.md`.
- **Server setup wizard** (`mediagit-server init`): interactive/flag-driven
  bootstrap that creates the first admin account
  (`--admin-username`/`--admin-email`/`--admin-password`), generates a random
  JWT secret, closes open registration, and enables rate limiting in one flow.
  Refuses to write a config that binds a non-loopback host with auth off.
  Offline `mediagit-server admin create-user` provisions users without a
  running server (for closed-registration deployments).
- **CLI auth commands** (`mediagit auth …`): `login`, `logout`, `register`,
  `whoami`, `status`, and `key create|list|revoke`. `login` accepts
  `--token`/`--api-key`/`--username`+password, stores the credential (env →
  keychain → config order), and prints the resolved identity, role, and grants.
- **`auth login` records the commit author**: a successful login writes the
  authenticated identity into the repo's `[author]` config, so commits are
  attributed to the logged-in user without a separate `git config`-style step.

### Changed
- `enable_auth`/insecure-bind guard: the server now refuses to bind to a
  non-loopback host with auth disabled (`MEDIAGIT_ALLOW_INSECURE_BIND=1`
  overrides), instead of silently serving an open port.
- JWT secret can now be supplied via `MEDIAGIT_JWT_SECRET` (wins over
  `config.toml`), not TOML-only.
- Per-route body limits: `/refs/update` and lock routes now cap at 1 MiB
  (data-plane chunk/pack routes keep the 2 GiB cap).
- Optional CORS support via `[server] cors_allowed_origins`; absent behaves
  as before (no layer).
- TLS: building with `enable_tls=true` on a non-`tls` cargo feature build is
  now a hard startup error instead of a silent fallback to plain HTTP.

### Fixed
- **Path traversal (cross-tenant storage escape)**: layout-v2's
  `LocalBackend::object_path` dropped the v1 `/`→`::` key encoding, and
  user-supplied chunk/pack/manifest/OID ids reached storage joins
  unvalidated — an authenticated write on one repo could read/write into
  another repo's storage, bypassing `GrantsStore`. Fixed with key validation
  (rejects `..`, absolute paths, drive prefixes) at both `NamespacedBackend`
  and `LocalBackend`, plus hex-format guards on the affected handlers.
- **Self-registration privilege escalation**: `POST /auth/register` accepted
  a client-supplied `role` field with no restriction, letting an
  unauthenticated caller mint an Admin account. `role` removed from
  `RegisterRequest`; self-registration now always creates `Role::Write`.
- **Clone manifest deserialization**: the parallel per-manifest fetch path in
  `clone` used raw format-deserialize instead of `ChunkManifest::from_bytes`,
  so the new `MGCM` envelope broke every clone of a chunked repo. Fixed; all
  other manifest read sites were already correct.
- **`create_router_with_rate_limit` never mounted `/auth/*`** — admin and
  auth endpoints were unreachable whenever rate limiting was enabled. Fixed.
- **Unbounded retry chains under backend outage**: a mid-push S3/MinIO
  outage caused ~1.9k independent per-chunk retry chains to exhaust sockets
  and stop the server from accepting new connections. Bounded by a semaphore
  (`MEDIAGIT_MINIO_OP_CONCURRENCY`, default 64) held across each retry
  lifetime.
- fsck chunk-delta cycle-detection test coverage confirmed (the guard itself
  was already correct; this closes a stale backlog entry).

### Security
- J6 security review: 1 HIGH finding (the path-traversal issue above), fixed
  and verified. All other new surface (grants ordering, admin gating, JWT
  default, keychain, API-key hashing) reviewed clean. Zero open P0/P1 at GA
  go/no-go.

## [v0.2.8-beta.1] - 2026-06-02

Cloud-pack hardening, the god-file refactor, and a full documentation accuracy
pass. Verified by the 2026-06-02 deep-test run: **614/614 PASS** across
MinIO/AWS/Azure/GCS (151/150/154/159), fsck + F8 compressed-hash integrity clean,
26.3–26.5% cloud storage savings, cross-backend delta parity (35 delta objects).

### Added
- **Cloud packs (Phase-3 Track F, F1–F11)**: client-side chunk bundling into pack
  objects (≤64 MiB / ≤1024 chunks) with an embedded index; clone via pack-locate +
  Range-GET. Cuts cloud object count from thousands to hundreds. Per-slice
  compressed-hash integrity (F8).
- **Presigned-URL transfer**: server mints presigned PUT/GET URLs; client transfers
  directly to/from S3/Azure/GCS/MinIO with automatic server-proxy fallback when a
  backend cannot sign (e.g. GCS ADC) or a URL 404s. Presigned MPU for large chunks
  on S3/MinIO.
- New mdBook chapters: **BLAKE3 Hashing** and **Cloud Packs**.

### Changed
- **God-file refactor**: `odb.rs`, `chunking.rs`, `smart_compressor.rs`, the protocol
  client, and the server handlers split into submodule directories. No behavior change.
- Documentation refreshed for accuracy across README, ARCHITECTURE, CLOUD_ARCHITECTURE,
  CLI_REFERENCE, SUPPORTED_FORMATS, DEVELOPMENT_GUIDE, FUTURE_TODOS, comparison, and the
  mdBook — including new/updated diagrams, the corrected server endpoint inventory, and
  SHA-256 → BLAKE3 corrections throughout.

### Fixed
- Push/pull ETA reset on object transitions and upload jumps; live progress in pack-mode
  pull/clone.

## [v0.2.7-beta.1] - 2026-05-25

This release covers all Phase-2 work completed between 2026-04-03 and 2026-05-25,
including the BLAKE3 migration, presigned-URL resilience, push/pull pipelining,
throughput improvements, pack negotiation fixes, and several cloud-backend bug fixes.

### Breaking Changes (beta — no backward-compat obligation)
- **Chunk IDs now use BLAKE3** instead of SHA-256. Existing `.mediagit` repos created with
  v0.2.6-beta.1 or earlier will need to be re-initialized or migrated. Pointer files now
  carry the `blake3:` prefix. The `sha2` crate is retained in `mediagit-security` (KDF +
  API key derivation) but removed from all storage/versioning/protocol crates.

### Added

#### BLAKE3 Migration (Track A — Phase-2)
- **`crates/mediagit-versioning/src/hash.rs`** (NEW) — `hash::Hasher` shim over
  `blake3::Hasher`; single call-site for all chunk-ID computation.
- **BLAKE3 throughout versioning layer** — `oid.rs`, `pack.rs`, `streaming_pack.rs`,
  `pointer.rs` (`blake3:` prefix), `filter.rs` (LFS clean-filter), `migration/verify.rs`.
- **Tree-parallel hashing** — BLAKE3 hashes 1 KB leaf nodes in parallel across CPU cores;
  10–20× faster than SHA-256 on non-SHA-NI hardware (ARM, older x86); 2–4× faster on
  SHA-NI hosts. Enable with `MEDIAGIT_HASH_PARALLEL=1`.
- **`sha2` removed** from `mediagit-versioning`, `mediagit-protocol`, `mediagit-git`,
  `mediagit-migration`; kept in `mediagit-security` for KDF and API-key derivation.
- **Bench schema** bumped to `BENCH_SCHEMA_VERSION=2`; `manifest_to_first_byte_ns` metric
  added to bench output (`MEDIAGIT_BENCH=1`).

#### Presigned URL Resilience
- **`crates/mediagit-protocol/src/error_class.rs`** (NEW) — cross-cloud `UploadOutcome`
  enum with `classify_auto()` / `classify_s3()` / `classify_azure()` / `classify_gcs()`.
  Parses `<Code>` XML (S3/Azure) and `"reason"` JSON (GCS). 19 unit tests.
- **Per-chunk 5-attempt retry with exponential backoff** — 1 s → 2 s → 4 s → 8 s, cap
  30 s, with jitter. Replaces the former global `AtomicBool` that poisoned all remaining
  chunks on one transient error.
- **Dedicated `direct_client`** built with `pool_idle_timeout(15 s)` + `tcp_keepalive(45 s)`
  + explicit `Content-Length` header on all direct PUTs. Defeats stale keep-alive 400s on
  long WAN pushes.
- **Configurable presigned URL TTL** — `presigned_url_ttl_seconds` in server config
  (default 43 200 = 12 h). Propagated through `AppState` via `with_presigned_ttl()`.
- **Multipart Upload (MPU) for S3/MinIO** (`MEDIAGIT_STAGED_UPLOAD=1`) — `upload_chunk_mpu()`
  uploads parts with the same per-part 5-attempt retry loop. Adaptive part size
  (`mpu_part_size_s3()` / `MEDIAGIT_MPU_PART_SIZE`): default floor 16 MiB, target ~96
  parts, scales up to 64 MiB parts for large chunks. Falls through to single-PUT on failure.
  Gated on `MEDIAGIT_MPU_THRESHOLD_BYTES` (default 16 MiB).

#### Throughput Improvements (W1–W5 — 2026-05-15)
- **W1 — Bench module** (`mediagit-protocol/src/bench.rs`) — `MEDIAGIT_BENCH=1` emits
  `[bench]` summary with `throughput_mbs` and `util_pct`; decision gate: ≥80% util =
  WAN-limited, ≥80% cpu = CPU-limited.
- **W4 — HTTP client split** — control-plane keeps HTTP/2; upload/download `direct_client`
  forced `http1_only()` + `tcp_nodelay(true)` + `pool_idle_timeout(60 s)`.
  `MEDIAGIT_HTTP_POOL_MAX` (default 64) now has a single source of truth.
- **W2 — Adaptive MPU part size** — `mpu_part_size_s3()` in `s3.rs` / `minio.rs`.
  Override via `MEDIAGIT_MPU_PART_SIZE`.
- **W5 — Range-parallel GET** — `download_chunk_ranged()`: chunks ≥ 64 MiB
  (`MEDIAGIT_RANGE_PARALLEL_THRESHOLD`) download in `MEDIAGIT_RANGE_PARALLEL=4` parallel
  byte-range GETs; falls back to single-stream on any failure.

#### Push/Pull Pipeline — Track B (default ON as of 2026-05-22)
- **B1 — Pull pipeline** `MEDIAGIT_PULL_PIPELINE=1` (ON) +
  `MEDIAGIT_PULL_MANIFEST_CONCURRENCY=8` — `buffer_unordered` manifest processing.
- **B2 — Push pipeline** `MEDIAGIT_PUSH_PIPELINE=1` (ON) — parallel object upload with
  per-object semaphore; gated knob flipped ON after MinIO/AWS/Azure sign-off.
- **B3 — Fetch branch concurrency** `MEDIAGIT_FETCH_BRANCH_CONCURRENCY=4` (ON).
- **B4 — Stream-to-disk** `MEDIAGIT_STREAM_CHUNK_TO_DISK=1` (ON) — chunks streamed to a
  temp file on download instead of buffering in heap; prevents RSS spike during large clones.
- **B5 — `bytes::Bytes` refcount** in push hot loop — O(1) clone instead of memcpy
  for chunk data shared across concurrent upload tasks.
- **B6 — Decompress-blocking** `MEDIAGIT_DECOMPRESS_BLOCKING=1` (ON) +
  `MEDIAGIT_DECOMPRESS_BLOCKING_THRESHOLD=262144`.
- **B7 — Storage streaming** `MEDIAGIT_STORAGE_STREAMING=1` (ON) — `get_streaming` trait
  with native impl in `minio.rs` and `s3.rs`; AWS clone 15.8% faster (159.8 s → 134.5 s).
- **B8 — `MEDIAGIT_HTTP_POOL_MAX`** single source of truth at `client.rs`.
- **B9 — `manifest_to_first_byte_ns`** metric added; `BENCH_SCHEMA_VERSION=2`.

#### New Environment Knobs
- **`MEDIAGIT_PUSH_CHUNK_CONCURRENCY`** — per-object chunk upload concurrency override
  (default: `(64 / push_object_concurrency).max(4).min(concurrent_uploads)`; targets 64
  total in-flight PUTs).
- **`MEDIAGIT_FETCH_DOWNLOAD_CONCURRENCY`** — per-branch download concurrency cap during
  `fetch --all` (default: `max(MEDIAGIT_DOWNLOAD_CONCURRENCY / branch_concurrency, 8)`).
- **`MEDIAGIT_GCS_UPLOAD_CONCURRENCY`** — concurrent `write_object` slots for GCS backend
  (default 4); prevents TCP transport timeouts under B2 pipeline load.

### Fixed

#### Pack Negotiation
- **StreamingPack header offset** — `StreamingPackWriter` initialized `current_offset: 12`
  but `PackHeader::to_bytes()` produces 13 bytes (signature 4 + version 4 + count 4 +
  kind 1). Fixed to 13, resolving "Index data too short for entry count" on all
  server-served packs after BLAKE3 migration. (`streaming_pack.rs`)

#### Clone / Fetch
- **MinIO `get()` retries "service error" indefinitely** — MinIO returns "service error"
  (not "nosuchkey") for missing keys. Mapped to `NoSuchKey` so `with_retry` treats it as
  a permanent failure immediately, not after 5 timeouts. (`minio.rs`)
- **Server `download_chunk` returns 503 for missing chunks** — changed to 404 for missing
  objects, allowing proper client error reporting. (`handlers.rs`)
- **GCS B4 hash mismatch** — `MEDIAGIT_STREAM_CHUNK_TO_DISK` path incorrectly verified
  `BLAKE3(compressed_bytes)` against `chunk_id = BLAKE3(uncompressed)`; removed the
  broken check. GCS has no presigned URLs so all chunks hit this path. (`client.rs`)
- **GCS concurrent upload 500s** — B2 pipeline's 8-concurrent uploads exhausted TCP
  connections on the GCS proxy path; fixed with `upload_semaphore` (default 4,
  `MEDIAGIT_GCS_UPLOAD_CONCURRENCY`) in `GcsBackend::put()`. (`gcs.rs`)

#### Push Progress Display
- **Progress bar overshoot** — `bytes_total_progress: Arc<AtomicU64>` added as a separate
  denominator atomic published immediately after the chunk-existence check (not after
  object completion). Retry pass no longer double-counts bytes. All increments unified to
  manifest chunk sizes. Bar stays ≤ 100% at all times.
- **Push throughput regression** — replaced per-object concurrency formula with
  `TOTAL_IN_FLIGHT_TARGET = 64`, restoring ~2.1 MB/s to AWS ap-south-1 (was 32 in-flight
  after the original B2 semaphore capped it).
- **Fetch over-concurrency on `--all`** — per-branch cap: `max(download_concurrency /
  branch_concurrency, 8)`; peak in-flight ≤ 128 (was 512). (`fetch.rs`)

#### Tests & Scripts
- **MinIO test: bucket not purged between runs** — added `aws s3 rm` purge at test
  startup to prevent false dedup from prior runs. (`deep_test_minio.ps1`)
- **Presigned tests read stdout instead of stderr** — test log reads changed to
  `server_err.log` (tracing writes to stderr). (`deep_test_minio.ps1`)
- **Scripts cleanup** — deleted `scripts/init-aws.sh` (LocalStack not used); updated
  `scripts/start-test-services.sh`; overhauled `scripts/run_comprehensive_tests.sh`
  (fixed ANSI color codes, corrected stale test names, added 8 new test suites).

### Test Coverage
- 459/459 deep tests pass across AWS S3 (ap-south-1), Azure Blob Storage (South India),
  and GCS (us-central1 proxy). All 23 supported file types validated on all 3 backends.
  +6 new concurrency-knob correctness tests.

---

## [v0.2.6-beta.1] - 2026-04-03


### Changed
- **`STREAMING_THRESHOLD` lowered 100MB → 5MB** (S4, phase 4 of item #4) — `add` and
  `status` now route all files ≥ 5MB through `write_chunked_from_file()`, which uses
  mmap + `chunk_media_aware()` for structure-aware deduplication. Previously only files
  ≥ 100MB got format-aware chunking; the 5-100MB range received generic CDC with no
  structural parsing. Both constants updated together to prevent OID mismatches between
  the two commands. (`crates/mediagit-cli/src/commands/add.rs:507`,
  `crates/mediagit-cli/src/commands/status.rs:199`)
- **GLB BIN large-payload CDC sub-chunking** (S5, phase 5 of item #4) — `chunk_glb()`
  now emits each GLB section header (8 bytes) as a stable `Metadata` chunk, then
  CDC-subdivides BIN payloads \> 4MB using FastCDC (1MB avg / 512KB min / 4MB max),
  matching the MKV large-Cluster pattern. Small BIN chunks (≤ 4MB) and all JSON chunks
  remain as single chunks. Common for scanned meshes, photogrammetry, and terrain models
  where the binary buffer is 20-200MB. (`crates/mediagit-versioning/src/chunking.rs`)
- **mmap-based format-aware chunking for all file sizes** — `collect_file_chunks_blocking()`
  now memory-maps files of any size and routes them through `chunk_media_aware()`, eliminating
  the previous StreamCDC fallback that made files ≥100 MB get generic CDC chunking with no
  format awareness. mmap fails gracefully to StreamCDC on network/FUSE filesystems and 32-bit
  targets. (`crates/mediagit-versioning/src/chunking.rs`)
- **MP4 mdat CDC sub-chunking** — `chunk_mp4()` emits the `mdat` atom header as a stable
  `Metadata` chunk, then CDC-subdivides the payload using FastCDC (2MB avg / 1MB min / 8MB max)
  to produce byte-exact `VideoStream` chunks. Fragmented MP4 (DASH/fMP4, detected by `moof`
  atoms) follows the same path. (`crates/mediagit-versioning/src/chunking.rs`)
- **MKV Cluster CDC sub-chunking** — `chunk_matroska()` emits each Cluster header as a stable
  `Metadata` chunk, then CDC-subdivides the cluster payload using FastCDC (2MB avg / 1MB min /
  8MB max) for byte-exact `VideoStream` chunks. Removes the earlier per-stream block-walking
  approach which violated the reconstruction invariant for interleaved A/V data.
  (`crates/mediagit-versioning/src/chunking.rs`)
- **Adaptive CDC params for video content** — FastCDC subdivision within `mdat` and Cluster
  now uses video-optimized parameters (2 MB avg / 1 MB min / 8 MB max) instead of the generic
  1 MB / 512 KB / 4 MB, improving delta dictionary matching for large video sub-elements.
  (`crates/mediagit-versioning/src/chunking.rs`)
- **Lower parallel processing threshold** — `write_chunked_parallel()` activates parallel
  chunk I/O at 2 chunks instead of 4; even 2-chunk files now benefit from concurrent storage
  writes. (`crates/mediagit-versioning/src/odb.rs`)
- **Delta encoding skip for pre-compressed VideoStream chunks** — streaming workers skip the
  delta-encode attempt for `ChunkType::VideoStream` chunks when the file type uses
  `CompressionStrategy::Store` (MP4, MOV, AVI, MKV, WebM, FLV, WMV, MPEG). Saves CPU on
  futile delta attempts against already-compressed H.264/H.265 frames; audio and metadata
  chunks are unaffected. (`crates/mediagit-versioning/src/odb.rs`)
- **Matroska EBML chunking: per-element metadata splitting** — each top-level metadata
  element (Info, Tracks, SeekHead, Cues, Chapters, Tags) now gets its own chunk instead
  of being grouped into one monolithic metadata blob. Re-tagging a video only invalidates
  the Tags chunk, not the entire metadata block. Consistent with MP4's per-atom approach.
  (`crates/mediagit-versioning/src/chunking.rs`)
- **Matroska large Cluster CDC subdivision** — Clusters > 4MB are now sub-chunked using
  FastCDC (1MB avg / 512KB min / 4MB max), matching the MP4 `mdat` subdivision strategy.
  Cluster header emitted separately for stable dedup across re-muxes.
  (`crates/mediagit-versioning/src/chunking.rs`)
- **AVI/RIFF chunking rewrite: movi CDC descent + OpenDML AVIX support** — rewrote
  `chunk_avi()` to walk at the RIFF-block level, handling both AVI 1.0 (`RIFF/AVI `) and
  AVI 2.0 OpenDML (`RIFF/AVIX`) extension blocks. Descends into `LIST/movi` and CDC-subdivides
  its payload using FastCDC (2MB avg / 1MB min / 8MB max) for byte-exact `VideoStream` chunks.
  Structural chunks (`LIST/hdrl`, `LIST/INFO`, `idx1`, `JUNK`) emitted as `Metadata`.
  (`crates/mediagit-versioning/src/chunking.rs`)

### Added
- **GLB unit tests** (6 tests) — `test_glb_small_bin_single_chunk`,
  `test_glb_large_bin_is_subdivided`, `test_glb_json_chunk_always_single_metadata`,
  `test_glb_large_bin_different_data_different_chunk_ids`, `test_glb_no_bin_sections_ok`,
  `test_glb_invalid_data_falls_back`. (`crates/mediagit-versioning/src/chunking.rs`)
- **Matroska chunking tests** — `test_chunk_matroska_metadata_splitting` verifies each
  metadata element (Info, Tracks, Tags) is emitted as its own chunk with distinct hashes.
  `test_chunk_matroska_large_cluster_subdivision` verifies 5MB Clusters get CDC-subdivided
  into multiple chunks. (`crates/mediagit-versioning/src/chunking.rs`)
- **AVI chunking tests** — `test_chunk_avi_movi_descends_into_subchunks` verifies movi
  payload is CDC-subdivided into `VideoStream` chunks and reconstruction is byte-exact.
  (`crates/mediagit-versioning/src/chunking.rs`)

### Fixed
- **Branch switch reconstruction size mismatch** — `branch switch` produced
  `Reconstructed size mismatch` errors (extra or missing bytes) for AVI, MP4, and MKV files.
  Root cause: per-stream batching helpers (`chunk_mdat_by_tracks`, `chunk_cluster_by_tracks`,
  `parse_avi_movi_subchunks`) accumulated non-contiguous interleaved bytes but stored
  `(offset, size)` as if contiguous; `fill_coverage_gaps` then re-read those byte ranges
  from the file, duplicating them (+22MB for a 228MB AVI). Fixed by replacing all
  per-stream batching with CDC (`chunk_fastcdc`, 2MB/1MB/8MB) which guarantees
  `chunk.data == file[offset..offset+size]`. Added `fill_coverage_gaps` as a free function
  called at the end of all three container parsers to patch EBML Void/CRC-32 and other
  structural gaps that format parsers intentionally skip. Covers all video/audio containers:
  AVI, MP4/MOV/M4V/M4A/3GP, MKV/WebM/MKA/MK3D. (`crates/mediagit-versioning/src/chunking.rs`)
- **`mka`/`mk3d` missing from ObjectType** — Matroska Audio (`.mka`) and Matroska 3D
  (`.mk3d`) extensions now map to `ObjectType::Mkv` in the smart compressor, ensuring
  they receive `CompressionStrategy::Store` instead of wastefully compressing
  pre-compressed media data. (`crates/mediagit-compression/src/smart_compressor.rs`)
- **AVI LIST chunk type detection was dead code** — the old `chunk_avi()` matched on
  `b"movi"` and `b"hdrl"` as FourCC values, but RIFF LIST chunks always have FourCC
  `b"LIST"` with the list type at offset +8. The match arms never fired, causing the
  entire `movi` LIST (all interleaved A/V data) to be stored as one opaque chunk with
  zero dedup potential. Now correctly reads the list type field.
  (`crates/mediagit-versioning/src/chunking.rs`)
- **AVI chunk size overflow on 32-bit targets** — `block_end` / `data_end` calculations
  used plain addition that could wrap on 32-bit `usize` with crafted RIFF headers.
  Switched to `saturating_add()` in all three AVI parsing functions.
  (`crates/mediagit-versioning/src/chunking.rs`)
- **`revert` did not update working directory after creating revert commit** — `revert_single_commit`
  performed a 3-way merge and updated refs but never called `CheckoutManager::checkout_commit()`,
  leaving the working tree out-of-sync with HEAD. Every other tree-modifying command (merge,
  cherry-pick, branch switch, reset --hard, stash save) correctly updates the working directory.
  Fixed by adding `checkout_commit()` in both the commit and no-commit paths, matching the
  pattern used by merge and cherry-pick. This also resolves the cascading clone verification
  failure (BUG-CLONE-01) where clone file counts appeared incorrect due to revert desync.
  (`crates/mediagit-cli/src/commands/revert.rs`)
- **`log -N` shorthand not active in release binary** — the `preprocess_args` fix that converts
  `log -5` → `log -n 5` was present in source but the release binary had not been rebuilt.
  Binary is now compiled with the fix active. (`crates/mediagit-cli/src/main.rs`)


---

## [v0.2.5-beta.1] - 2026-03-26

### Added
- **`.mediagitignore` support** in `add` and `status` commands — `.gitignore`-compatible
  pattern matching using the `ignore` crate (`v0.4.25` / `globset v0.4.18`).
  - `add`: files and directories matching `.mediagitignore` are silently skipped during
    file discovery. Entire ignored directories are pruned (no recursion), preventing
    unnecessary I/O. Explicit named paths that are ignored print a warning; `--force`
    bypasses all ignore rules entirely. `--verbose` logs each skipped path.
  - `status`: ignored files are hidden from the "Untracked files:" section by default.
    `--ignored` flag activates a new "Ignored files:" section listing all excluded files.
    `--porcelain --ignored` uses the `!! path` prefix, matching Git convention.
  - Graceful fallback: missing `.mediagitignore` is a no-op; malformed file logs a warning
    and continues without rules.
  - Pattern syntax: full `.gitignore` semantics — globs (`*.tmp`), directory markers
    (`build/`), negation (`!important.log`), comments (`#`), anchored paths (`/src`).
  - New module: `crates/mediagit-cli/src/ignore_rules.rs` — `IgnoreMatcher` struct wrapping
    the `ignore` crate for consistent use across commands.
  - `crates/mediagit-cli/Cargo.toml` — added `ignore = "0.4"` dependency.
  - (`crates/mediagit-cli/src/commands/add.rs`, `crates/mediagit-cli/src/commands/status.rs`)

- **Integration test suite for `.mediagitignore`** (`crates/mediagit-cli/tests/ignore_integration_test.rs`):
  8 tests covering basic glob ignore, `--force` override, directory pruning, negation (`!`
  pattern), `--ignored` flag display, porcelain `!!` prefix, and no-file fallback.
  All 8 tests pass.

### Changed
- `book/src/cli/add.md` — Options section corrected (removed non-existent `--chunk-size`;
  added `--no-chunking`, `--no-delta`, `--no-parallel`, `-j`). New `.mediagitignore` section
  with full syntax reference, ignore example, and `--force` override example.
- `book/src/cli/status.md` — `--ignored` option corrected from mode-based description to
  simple boolean flag matching the implementation. Updated example shows real output format
  with "Ignored files:" section. Added `--porcelain --ignored` example with `!! path` prefix.
  Notes section updated to reference `.mediagitignore` properly.

## [v0.2.4-beta.1] - 2026-03-26


### Changed
- Delta encoder replaced: suffix-array (divsufsort/sacabase) sliding-window approach replaced
  with **zstd dictionary compression**. Base chunk is used as a raw zstd dictionary at level 19
  to compress target chunks. Wire format v2: `[0x5A, 0x44]` magic + varint sizes + zstd bytes.
  Results: +1.3-2.1pp better savings on AI files, 1.4-2.4× faster throughput, 73% less code.
  (`crates/mediagit-versioning/src/delta.rs`)

### Added
- `/health` route alias added alongside `/healthz` in both `create_router` and
  `create_router_with_rate_limit`. Kubernetes liveness probes, load balancers, and uptime
  monitors that probe `/health` (without the `z`) now get a 200 response.
  (`crates/mediagit-server/src/lib.rs`)
- `bisect replay` now executes scripted bisect sessions: parses the log file format
  (`YYYY-MM-DD HH:MM:SS: command: args`), strips the timestamp prefix, and dispatches
  `good`/`bad`/`skip`/`start` entries to the existing async bisect handlers. Previously
  the command printed log lines without acting on them.
  (`crates/mediagit-cli/src/commands/bisect.rs`)
- `log <REVISION>` now resolves branch names, tags, and abbreviated OIDs via `resolve_revision`,
  so `mediagit log main` or `mediagit log feat/my-branch` shows that branch's history.
  (`crates/mediagit-cli/src/commands/log.rs`)
- Standalone test suite passes 173/173 tests (release build, Windows/WSL2). Covers all
  active CLI commands, MinIO S3 backend, and push/pull/clone over local HTTP server.
- HTTP/2 adaptive window tuning (`http2_adaptive_window`, 2 MB stream window, 8 MB connection
  window) in the protocol client for 2-4× throughput improvement on WAN connections.
  (`crates/mediagit-protocol/src/client.rs`)
- Server TLS config now advertises HTTP/2 via ALPN (`h2`, `http/1.1`), enabling HTTP/2
  negotiation over TLS. Plaintext HTTP/1.1 connections (local dev, CI) are unaffected.
  (`crates/mediagit-server/src/main.rs`)
- Raw file serving endpoints on the HTTP server: `GET /{repo}/files/{*path}` streams a file
  at a given path from any commit ref, and `GET /{repo}/tree[/{*path}]` lists tree entries
  as JSON. (`crates/mediagit-server/src/handlers.rs`, `crates/mediagit-server/src/lib.rs`)
- Abbreviated OID resolution: `show`, `revert`, `verify`, and all other revision-accepting
  commands now accept shortened commit hashes (≥4 hex chars), matching `git log --oneline`
  output. Prefix-scans the object store; errors on ambiguous matches.
  (`crates/mediagit-versioning/src/odb.rs`, `crates/mediagit-versioning/src/revision.rs`)
- `stash push` subcommand as a git-compatible alias for `stash save`. Accepts `-m/--message`
  flag and positional paths, identical to `stash save`. (`crates/mediagit-cli/src/commands/stash.rs`)
- `verify [COMMIT]` optional positional argument: pass a commit OID, abbreviated hash,
  branch name, or `HEAD` to verify a specific commit and its reachable objects rather than
  the full repository. (`crates/mediagit-cli/src/commands/verify.rs`)

### Fixed
- `show <short-hash>` now resolves abbreviated OIDs instead of failing with "OID hex string
  must be 64 characters". (shared fix: abbreviated OID resolution in `revision.rs`)
- `revert <short-hash>` now resolves abbreviated OIDs instead of failing with the same error.
- `verify HEAD` and `verify <short-hash>` no longer fail with "unexpected argument". The
  `verify` command now accepts an optional `[COMMIT]` positional argument.
- `stash push -m "msg"` now works — previously rejected as an unrecognised subcommand.
- `verify` `resolve_commit` now uses `refdb.resolve()` (which follows symbolic refs like HEAD)
  instead of `refdb.read()`, so `verify HEAD` correctly resolves to the HEAD commit.

### Removed
- Removed `filter`, `install`, `track`, and `untrack` commands — git migration tooling is a
  future milestone. The `mediagit-git` crate remains in the workspace and compiles
  independently for when the migration milestone arrives.
- Removed `mediagit-git` dependency from the CLI binary.

## [0.2.3-beta.1] - 2026-03-13

### Fixed
- `add` command: ETA showed wildly incorrect values (e.g. "eta 2d") when most files were
  unchanged. Skipped (stat-cache / HEAD-match) files now advance the byte progress counter
  so `indicatif`'s ETA calculation is based on total work, not just newly staged bytes.
  (`crates/mediagit-cli/src/commands/add.rs`)
- `add` command: Speed dropped to "0 B/s" and ETA reached astronomical values (e.g.
  "eta 11710991569y") while staging large files (≥100 MB). Added a per-chunk `on_progress`
  callback to `ObjectDatabase::write_chunked_from_file` that fires after every chunk
  (deduped, delta, or full), giving continuous byte-level progress updates during multi-GB
  file ingestion. (`crates/mediagit-versioning/src/odb.rs`,
  `crates/mediagit-cli/src/commands/add.rs`)

### Changed
- `ObjectDatabase::write_chunked_from_file` now accepts an optional
  `on_progress: Option<Arc<dyn Fn(u64) + Send + Sync>>` callback for incremental byte
  reporting. Pass `None` to retain previous behaviour.

### Security
- Upgraded `quinn-proto` from 0.11.13 → 0.11.14 (RUSTSEC-2026-0037, CVSS 8.7 — DoS in
  Quinn QUIC endpoints). Transitive dependency via `reqwest → quinn → quinn-proto`.
  Only `Cargo.lock` updated; no `Cargo.toml` changes required.

### Code Quality
- `crates/mediagit-cli/src/commands/log.rs`: Changed `walk_tree` parameter from
  `&'a PathBuf` to `&'a Path` (clippy `ptr_arg` warning).
- `crates/mediagit-cli/src/commands/show.rs`: Same `&PathBuf` → `&Path` fix.
- `crates/mediagit-security/src/auth/jwt.rs`: Marked `JwtAuth::new` doctest as `no_run`
  to prevent Avast false-positive (`rust_out.exe` blocked on Windows) from failing CI.
- `crates/mediagit-versioning/src/odb.rs`: Updated `write_chunked_from_file` doctest to
  pass the new `None` argument.

## [0.2.1-beta.2]

### Fixed
- PowerShell install warning: added `-UseBasicParsing` to `iwr` in `install.ps1` usage comment,
  `install.sh` (Windows fallback message), `RELEASING.md`, and `.github/workflows/release.yml`
  release notes body — prevents IE-engine security prompt on Windows PowerShell
- Install scripts (`install.ps1`, `install.sh`) now fall back to the `/releases` list API when
  `/releases/latest` returns 404 — this occurs when only pre-release versions exist (e.g. before
  the first stable release); scripts pick the most recent release including pre-releases

### Changed
- `README.md`: Added complete 32-command CLI reference section, grouped by workflow with flag docs
- `README.md`: Replaced compression efficiency table with accurate per-type data (conservative
  numbers — ~30% average across mixed media projects; pre-compressed formats explicitly shown
  as Store / 0% additional reduction)
- `README.md`: Added scenario-based deduplication table (replaces single "66% identical files" row)
- `README.md`: Updated roadmap to match actual CHANGELOG history (v0.1.0 → v0.2.0 → v0.2.1
  → v0.3.0 planned → v1.0.0 stable); removed fictional v0.1.1 entry
- `README.md`: Fixed Statistics section — staging throughput corrected to 80–240 MB/s (release
  build); removed misleading 3-35 MB/s figure
- `README.md`: Added "Could not fetch latest version" troubleshooting entry with install workaround

## [0.2.1-beta.1] - 2026-03-06

### Changed
- Automated version extraction from Cargo.toml in release workflow dry-run mode
- Updated all documentation to reflect correct version, URLs, and archive names
- Added `scripts/bump-version.sh` for automated version bumping across the project

## [0.2.0] - 2026-03-05

### Added
- Dual-layer delta encoding (bsdiff + sliding-window)
- AES-256-GCM client-side encryption with Argon2id key derivation
- TLS 1.3 for all network operations
- JWT + API key authentication for server mode
- Video and audio track-based merging (fully implemented)
- Multi-platform distribution (Linux, macOS, Windows, Docker, crates.io)
- Automated release pipeline with cross-compilation

### Changed
- Delta max chain depth reduced from 50 to 10 for faster reads
- Chunk sizes now adaptive (1-8 MB) instead of fixed 64 MB
- Similarity thresholds tuned per file type for better delta compression
- macOS Intel CI runner updated to macos-15-intel

### Fixed
- macOS Intel (x86_64-apple-darwin) build failure due to retired macos-13 runner
- Docker push to GHCR (added packages:write permission)
- Comprehensive documentation sync with codebase (book, architecture docs, CLI reference)

## [0.1.0] - 2026-02-27

### Added
- Core MediaGit CLI implementation
- Object database with SHA-256 content addressing
- Intelligent compression (Zstd, Brotli)
- Branch management system
- 3-way merge algorithm
- Media-aware merge intelligence (PSD layer-aware)
- Git integration layer
- Multi-cloud storage backends:
  - Local filesystem
  - AWS S3
  - Azure Blob Storage
  - Google Cloud Storage
  - MinIO (S3-compatible)
  - Backblaze B2
  - DigitalOcean Spaces
- Security: AES-256-GCM encryption at rest
- Observability: Structured logging with Tracing
- Metrics: Prometheus metrics endpoint
- Operations: Garbage collection, FSCK, storage migration
- Comprehensive test suite (960 tests, 80%+ coverage)
- Documentation and user guide
- Multi-platform binaries (Linux, macOS, Windows on x86_64 and ARM64)

### Security
- AGPL-3.0 license enforcement
- Dependency security audits in CI
- Encryption at rest with Argon2 key derivation

[Unreleased]: https://github.com/winnyboy5/mediagit-core/compare/v0.3.0-rc.3...HEAD
[v0.3.0-rc.3]: https://github.com/winnyboy5/mediagit-core/compare/v0.3.0-rc.2...v0.3.0-rc.3
[v0.2.6-beta.3]: https://github.com/winnyboy5/mediagit-core/compare/v0.2.6-beta.2...v0.2.6-beta.3
[v0.2.6-beta.2]: https://github.com/winnyboy5/mediagit-core/compare/v0.2.6-beta.1...v0.2.6-beta.2
[v0.2.6-beta.1]: https://github.com/winnyboy5/mediagit-core/compare/v0.2.5-beta.1...v0.2.6-beta.1
[v0.2.5-beta.1]: https://github.com/winnyboy5/mediagit-core/compare/v0.2.4-beta.1...v0.2.5-beta.1
[v0.2.4-beta.1]: https://github.com/winnyboy5/mediagit-core/compare/v0.2.3-beta.1...v0.2.4-beta.1
[v0.2.3-beta.1]:https://github.com/winnyboy5/mediagit-core/compare/v0.2.1-beta.2...v0.2.3-beta.1
[v0.2.1-beta.2]: https://github.com/winnyboy5/mediagit-core/compare/v0.2.1-beta.1...v0.2.1-beta.2
[0.2.1-beta.1]: https://github.com/winnyboy5/mediagit-core/compare/v0.2.0...v0.2.1-beta.1
[0.2.0]: https://github.com/winnyboy5/mediagit-core/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/winnyboy5/mediagit-core/releases/tag/v0.1.0

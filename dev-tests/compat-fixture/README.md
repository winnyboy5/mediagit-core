# Compat Fixture (format-freeze gate)

**DO NOT REGENERATE.** These bytes were produced once, at the wire/persisted-format
freeze (`0.3.0-rc.1`, see `docs/FORMATS.md`), by that commit's release `mediagit`. Their whole
purpose is to be *old bytes*: every future build must still read them. Regenerating
them with a newer build defeats the test — it would only ever prove the current
build can read its own output.

## What's frozen here

`repo/.mediagit/` is a real local MediaGit repository (3 commits of a 6 MB chunked
binary edited across versions), deliberately built to contain one instance of each
persisted format under freeze:

| Format | Where | Frozen contract (FORMATS.md) |
|---|---|---|
| Pack v3 (13-byte header, `PACK` sig, `PackKind` byte) | `objects/repo/packs/*.pack` | §2 |
| Chunk manifest (`MGCM` magic + version-byte envelope) | `objects/repo/manifests/**` | §3 |
| Chunk-delta `.meta` (`base:<hex>`) | `objects/repo/chunk-deltas/**/*.meta` | §4 |
| `LAYOUT` v2 marker | `objects/repo/LAYOUT` | §5 |
| Reachability bitmap | `objects/repo/bitmaps/**` | internal |
| Commit/tree objects, refs, reflog | `objects/repo/objects`, `refs`, `logs` | §1 |

Not covered here (covered elsewhere, no frozen bytes needed):

- **Auth/locks JSONL (`{"v":1}`)** — version-rejection is unit-tested
  (`persist::load_jsonl` / `locks.rs` `*_rejects_future_version_header`).
- **Wire/transfer path (pack streaming, presign, batch-get)** — exercised every
  run by `06_remote` against live repos on all four backends.

## The gate

`dev-tests/qa-suite/scripts/02_compat.ps1` copies this repo, asserts each format
above is present, then runs `mediagit fsck` on the copy. fsck parses every
persisted format, so a reader regression (e.g. a pack v4 that a v3 reader
misparses, or a manifest envelope change) fails the gate. Runs on every campaign.

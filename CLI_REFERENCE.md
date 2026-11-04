# MediaGit CLI Reference

Object IDs (OIDs) throughout MediaGit — commits, blobs, chunks — are **BLAKE3** hashes displayed as 64 lowercase hex characters.

This is an index. **[book/src/cli/](book/src/cli/README.md) is the canonical source** for
per-command flags, examples, exit codes, and environment variables — see the linked page for
each command's full detail. For architecture (transfer protocol, cloud packs, storage backends),
see [book/src/architecture/](book/src/architecture/README.md).

## Core Commands

| Command | Description | Reference |
|---------|-------------|-----------|
| `init` | Initialize a new repository | [book/src/cli/init.md](book/src/cli/init.md) |
| `add` | Stage file contents for commit with smart compression, chunking, and delta encoding | [book/src/cli/add.md](book/src/cli/add.md) |
| `commit` | Record staged changes as a commit | [book/src/cli/commit.md](book/src/cli/commit.md) |
| `status` | Show working tree status | [book/src/cli/status.md](book/src/cli/status.md) |
| `log` | Show commit history | [book/src/cli/log.md](book/src/cli/log.md) |
| `diff` | Show changes between commits, the index, and the working tree | [book/src/cli/diff.md](book/src/cli/diff.md) |
| `show` | Show object details | [book/src/cli/show.md](book/src/cli/show.md) |

## Branch Management

| Command | Description | Reference |
|---------|-------------|-----------|
| `branch` | Create, list, switch, protect, and delete branches | [book/src/cli/branch.md](book/src/cli/branch.md) |
| `merge` | Join development histories | [book/src/cli/merge.md](book/src/cli/merge.md) |
| `rebase` | Rebase commits onto another branch | [book/src/cli/rebase.md](book/src/cli/rebase.md) |
| `cherry-pick` | Apply changes from specific commits | [book/src/cli/cherry-pick.md](book/src/cli/cherry-pick.md) |
| `bisect` | Find a bug-introducing commit by binary search | [book/src/cli/bisect.md](book/src/cli/bisect.md) |
| `stash` | Temporarily save uncommitted changes | [book/src/cli/stash.md](book/src/cli/stash.md) |
| `reset` | Reset current HEAD to a specified state | [book/src/cli/reset.md](book/src/cli/reset.md) |
| `revert` | Create new commits that undo changes from existing commits | [book/src/cli/revert.md](book/src/cli/revert.md) |
| `reflog` | Show reference logs — when branch tips and other refs were updated | [book/src/cli/reflog.md](book/src/cli/reflog.md) |
| `tag` | Create and manage tags, including signed annotated tags | [book/src/cli/tag.md](book/src/cli/tag.md) |
| `lock` | Manage server-enforced file locks | [book/src/cli/lock.md](book/src/cli/lock.md) |

## Remote Operations

| Command | Description | Reference |
|---------|-------------|-----------|
| `clone` | Clone a remote repository | [book/src/cli/clone.md](book/src/cli/clone.md) |
| `remote` | Manage remote repositories | [book/src/cli/remote.md](book/src/cli/remote.md) |
| `fetch` | Fetch remote changes without merging | [book/src/cli/fetch.md](book/src/cli/fetch.md) |
| `push` | Push local commits to a remote | [book/src/cli/push.md](book/src/cli/push.md) |
| `pull` | Fetch and integrate remote changes | [book/src/cli/pull.md](book/src/cli/pull.md) |
| `download` | Download a single file from a remote repository by path | [book/src/cli/download.md](book/src/cli/download.md) |
| `auth` | Manage authentication with a MediaGit server | [book/src/cli/auth.md](book/src/cli/auth.md) |

## Media & Sparse Checkout

| Command | Description | Reference |
|---------|-------------|-----------|
| `media` | Inspect media file metadata (image/video/audio/PSD/3D) | [book/src/cli/media.md](book/src/cli/media.md) |
| `sparse-checkout` | Materialize only part of the working tree | [book/src/cli/sparse-checkout.md](book/src/cli/sparse-checkout.md) |

## Maintenance

| Command | Description | Reference |
|---------|-------------|-----------|
| `gc` | Garbage collection and optimization | [book/src/cli/gc.md](book/src/cli/gc.md) |
| `fsck` | Check repository integrity | [book/src/cli/fsck.md](book/src/cli/fsck.md) |
| `verify` | Quick integrity verification (BLAKE3 checksums, ref validity) | [book/src/cli/verify.md](book/src/cli/verify.md) |
| `stats` | Show repository statistics | [book/src/cli/stats.md](book/src/cli/stats.md) |
| `config` | Get and set repository configuration | [book/src/cli/config.md](book/src/cli/config.md) |
| `key` | Manage this repository's at-rest encryption key | [book/src/cli/key.md](book/src/cli/key.md) |

## Machine-readable output (`--machine`)

The global `--machine` flag is for programs (the desktop app, scripts): it turns
off colour and human output and replaces them with one JSON envelope on stdout
and NDJSON progress on stderr.

```
mediagit -C <repo> --machine status
```

**Envelope** (one line, contract version `v` = 1; bumped only on incompatible change):

```json
{"v":1,"ok":true,"data":{ }}
{"v":1,"ok":false,"error":{"code":"error","message":"..."}}
```

**Error codes and exit codes**

| `error.code` | Meaning | Exit |
|--------------|---------|------|
| `json_unsupported` | The subcommand has no machine output yet | 2 |
| `needs_input` | A prompt (passphrase, confirmation) was reached; collect the input and re-run with it supplied | 1 |
| `invalid_args` | Flags combined wrongly (currently `show`: `--output` without `<rev>:<path>`, `--tree` with `<rev>:<path>`) | 1 |
| `empty_commit` | `cherry-pick` (including `--continue`) or `revert` would create a commit whose tree equals HEAD's; nothing is committed (`revert --no-commit` skips the check) | 1 |
| `error` | Any other failure; `message` is the full error chain | 1 |

Success exits 0. A panic exits 2 with no envelope.

**Progress** (stderr, one line per bar roughly every 200 ms, final line has `done:true`):

```json
{"type":"progress","id":3,"msg":"Pushing","pos":10,"len":100,"unit":"bytes","done":false}
```

`id` is stable for one run, `len` is `null` while the total is unknown, and
`unit` is `bytes` or `items`. Other stderr text is ordinary log output; ignore
lines that are not `type:"progress"`.

**Supported commands:** `status`, `version`, `log`, `show`, `branch`, `tag`,
`add`, `commit`, `init`, `media`, `remote`, `lock`, `fetch`, `pull`, `push`,
`clone`, `merge`, `diff`, `stash`, `reset`, `revert`, `download`. A `merge` or
`revert` that stops on conflicts prints an `ok:true` envelope with
`status:"conflict"` and a `conflicts` list, and exits 1. `show <rev> --tree` lists a commit's files; `show <rev>:<path> --output <file>`
writes a blob to a file. `completions` prints a `json_unsupported`
envelope and exits 2 without running.

W4 completes the set: every subcommand now has machine output (`auth register|passwd|key|admin`,
`branch protect|show`, `commit --dry-run`, `reflog show --all|delete|expire`, `remote rename|show|set-url`,
`tag show|verify`, `verify <commit>|--start|--end`, `bisect replay`). Destructive ones (`reflog delete|expire`,
`auth key revoke`, `auth admin revoke-grant`) need `--yes` in machine mode, else `needs_input`.

W3 adds `auth` (`login|status|whoami|logout`; `whoami`/`status` carry the
account `identity` to compare with lock owners), `key`, `stats`, `gc`, `fsck`,
`verify` (whole repository only), `config`, `sparse-checkout`, `rebase`,
`cherry-pick`, `bisect` (not `replay`) and `reflog` (one ref's entries). `rebase`,
`cherry-pick` and `revert` conflicts use the same `status:"conflict"` shape as
`merge`; `fsck`/`verify`/`gc` that find errors print their report with
`healthy:false` / non-empty `errors` and exit 1. `config get` of an unset key
returns `value:null`. `rebase` drops a replayed commit that adds nothing unless
`--keep-empty` is given. `key init` returns the one-time `recovery_code` in its
envelope: show it once, it is not stored.

**Secrets (`--secret-stdin`).** Passphrases, passwords and recovery codes are
never accepted in argv or the environment under `--machine`. With
`--machine --secret-stdin` (the flag requires `--machine`) every secret prompt
(`key init|recover`, the encryption passphrase any object-touching command asks
for, `auth login` password) reads exactly one line from stdin, in the order the
prompts occur, into a zeroizing buffer; no confirmation re-type is asked. Without
the flag the prompt fails `needs_input` and stdin is left unread. Empty stdin is
`invalid_args`. `--secret-stdin` on `key init`/`rotate-master`/`clone` selects a
passphrase master even where an OS keychain exists (a key file named by
`MEDIAGIT_ENCRYPTION_KEYFILE` still wins and leaves stdin unread); the envelope's
`source` reports which master was used. A passphrase repository is unwrapped once
per process, so `push`/`fetch`/`pull` read exactly one line. `key recover <CODE>` and `auth login --token/--api-key` are
`invalid_args` in machine mode, because they put the secret in argv.

`tag` supports `list`, `create` and `delete`. `remote ls-tree <remote> [path] [--ref <ref>]` lists one remote directory
(`{remote, ref, path, entries:[{name, path, kind: file|dir, size|null, oid}]}`);
human output is `<type>\t<size>\t<path>` per entry. See `book/src/cli/remote.md`.

Report types are generated for TypeScript into `ui/src/api/generated/` by
`cargo test -p mediagit-cli`.

## See Also

- [Architecture](ARCHITECTURE.md)
- [Supported Formats](SUPPORTED_FORMATS.md)
- [Development Guide](DEVELOPMENT_GUIDE.md)
- [Cloud Architecture](CLOUD_ARCHITECTURE.md)
- [Future TODOs](FUTURE_TODOS.md)

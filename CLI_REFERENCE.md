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

## See Also

- [Architecture](ARCHITECTURE.md)
- [Supported Formats](SUPPORTED_FORMATS.md)
- [Development Guide](DEVELOPMENT_GUIDE.md)
- [Cloud Architecture](CLOUD_ARCHITECTURE.md)
- [Future TODOs](FUTURE_TODOS.md)

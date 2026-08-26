# MediaGit CLI Reference

Complete command reference for MediaGit v0.3.0-rc.4 — Git for Media Files.

Object IDs (OIDs) throughout MediaGit — commits, blobs, chunks — are **BLAKE3** hashes displayed as 64 lowercase hex characters.

---

## Command Taxonomy

```mermaid
graph LR
    subgraph Setup
        init
        clone
        remote
    end
    subgraph "File Ops"
        add
        commit
        status
        diff
        show
    end
    subgraph "Branch & History"
        branch
        merge
        rebase
        cherry-pick
        log
        reset
        revert
        reflog
        stash
        bisect
        tag
    end
    subgraph Remote
        push
        pull
        fetch
        auth
    end
    subgraph "Media & Sparse"
        media
        sparse-checkout
    end
    subgraph Utility
        gc
        fsck
        verify
        stats
        config
        lock
        version
        completions
    end
```

## Quick Reference

| Category | Commands |
|----------|----------|
| **Setup** | `init`, `clone`, `remote` |
| **File Ops** | `add`, `commit`, `status`, `diff`, `show` |
| **Branch & History** | `branch`, `merge`, `rebase`, `cherry-pick`, `log`, `reset`, `revert`, `reflog`, `stash`, `bisect`, `tag` |
| **Remote** | `push`, `pull`, `fetch`, `download`, `auth` |
| **Media & Sparse** | `media`, `sparse-checkout` |
| **Utility** | `gc`, `fsck`, `verify`, `stats`, `config`, `lock`, `key`, `version`, `completions` |

### Git-Compatibility Shims

MediaGit preprocesses arguments before parsing to provide familiar git muscle-memory:

| You type | Becomes |
|----------|---------|
| `mediagit checkout <ref>` | `mediagit branch switch <ref>` |
| `mediagit checkout -b <ref>` | `mediagit branch switch -c <ref>` |
| `mediagit co <ref>` | `mediagit branch switch <ref>` |
| `mediagit log -5` | `mediagit log -n 5` |
| `mediagit reflog -5` | `mediagit reflog -n 5` |
| `mediagit branch` (no args) | `mediagit branch list` |
| `mediagit branch <name>` | `mediagit branch create <name>` |
| `mediagit tag` (no args) | `mediagit tag list` |
| `mediagit tag <name>` | `mediagit tag create <name>` |
| `mediagit remote` (no args) | `mediagit remote list` |

### Global Flags

These flags are available on **every** command:

| Flag | Description |
|------|-------------|
| `-v, --verbose` | Enable verbose / debug output |
| `-q, --quiet` | Suppress output |
| `--color <WHEN>` | Colored output (`always`, `auto`, `never`) |
| `-C, --repository <PATH>` | Run as if started in `<PATH>` |
| `-h, --help` | Show help |

---

## Repository Setup

### `mediagit init`

Initialize a new MediaGit repository.

```bash
mediagit init [PATH]
```

| Flag | Description |
|------|-------------|
| `--bare` | Create bare repository (no working tree) |
| `--initial-branch <NAME>` | Set initial branch name (default: `main`) |
| `--template <PATH>` | Use template directory |
| `-q, --quiet` | Suppress output |

**Examples:**
```bash
mediagit init                    # Initialize in current directory
mediagit init my-project         # Initialize in new directory
mediagit init --bare repo.git    # Create bare repository
```

---

### `mediagit clone`

Clone a remote repository.

```bash
mediagit clone <URL> [DIRECTORY]
```

| Flag | Description |
|------|-------------|
| `-b, --branch <BRANCH>` | Clone specific branch |
| `-q, --quiet` | Suppress output |
| `-v, --verbose` | Detailed output |

**Examples:**
```bash
mediagit clone http://server:3000/project
mediagit clone http://server:3000/project my-copy
mediagit clone -b develop http://server:3000/project
```

---

### `mediagit remote`

Manage remote repositories.

```bash
mediagit remote <SUBCOMMAND>
```

**Subcommands:**

| Subcommand | Usage | Description |
|------------|-------|-------------|
| `add` | `remote add [-f] <NAME> <URL>` | Add remote (`-f` fetches immediately) |
| `remove` | `remote remove <NAME>` | Remove remote |
| `list` | `remote list [-v]` | List remotes (`-v` shows URLs) |
| `rename` | `remote rename <OLD> <NEW>` | Rename remote |
| `show` | `remote show <NAME>` | Show remote info |
| `set-url` | `remote set-url [--push] <NAME> <URL>` | Change URL (`--push` sets push URL) |

**Examples:**
```bash
mediagit remote add origin http://server:3000/project
mediagit remote list -v
mediagit remote set-url origin http://new-server:3000/project
```

---

## Basic Workflow

### `mediagit add`

Stage file contents for commit with smart compression, chunking, and delta encoding.

```bash
mediagit add <PATHS>...
```

| Flag | Description |
|------|-------------|
| `-A, --all` | Add all changes |
| `-p, --patch` | Interactive staging |
| `--dry-run` | Preview what would be added |
| `-f, --force` | Add ignored files |
| `-u, --update` | Update tracked files only |
| `--ignore-removal` | Ignore removal of files in the index |
| `--no-chunking` | Disable chunking for large files |
| `--no-delta` | Disable delta compression |
| `--no-parallel` | Disable parallel file processing |
| `-j, --jobs <N>` | Number of parallel worker threads (default: CPU cores, max 8) |
| `-q, --quiet` | Suppress output |
| `-v, --verbose` | Detailed output |

**Examples:**
```bash
mediagit add .                   # Stage all changes
mediagit add file.psd            # Stage specific file
mediagit add -A --verbose        # Stage all with details
mediagit add --dry-run *.psd     # Preview what would be staged
mediagit add -j 4 *.psd         # Limit to 4 threads
```

---

### `mediagit commit`

Record changes to repository.

```bash
mediagit commit
```

| Flag | Description |
|------|-------------|
| `-m, --message <MSG>` | Commit message |
| `-e, --edit` | Edit message in editor |
| `-F <FILE>` | Read message from file |
| `-a, --all` | Stage and commit all changes |
| `--author <NAME>` | Override author |
| `--date <DATE>` | Override date |
| `--allow-empty` | Allow empty commit |
| `-s, --signoff` | Add signed-off-by |
| `--dry-run` | Preview commit |
| `-q, --quiet` | Suppress output |
| `-v, --verbose` | Show diff in editor |

**Examples:**
```bash
mediagit commit -m "Add new assets"
mediagit commit -a -m "Update all files"
mediagit commit --author "Artist <artist@example.com>"
```

---

### `mediagit status`

Show working tree status.

```bash
mediagit status
```

| Flag | Description |
|------|-------------|
| `--tracked` | Show only tracked files |
| `--untracked` | Show only untracked files |
| `--ignored` | Show ignored files |
| `-s, --short` | Short format output |
| `--porcelain` | Machine-readable output |
| `-b, --branch` | Show branch info, including upstream ahead/behind when configured |
| `-q, --quiet` | Suppress output |
| `-v, --verbose` | Detailed output (adds total size to the summary line) |
| `--json` | Single JSON document on stdout (implies no colors/progress) |

**Examples:**
```bash
mediagit status
mediagit status -s              # Short format
mediagit status --porcelain     # For scripting
mediagit status -b              # Branch + ahead/behind vs. upstream
mediagit status --json          # Machine-readable structured output
```

---

### `mediagit log`

Show commit history.

```bash
mediagit log [REVISION] [-- PATHS]...
```

`REVISION` accepts a branch name, tag, full OID (BLAKE3, 64 hex chars), or abbreviated OID (≥4 hex chars).

| Flag | Description |
|------|-------------|
| `-n, --max-count <N>` | Limit commits shown |
| `--skip <N>` | Skip first N commits |
| `--oneline` | One line per commit |
| `--graph` | ASCII graph |
| `--stat` | Show file stats |
| `-p, --patch` | Show diffs |
| `--author <PATTERN>` | Filter by author |
| `--grep <PATTERN>` | Filter by message |
| `--since <DATE>` | After date |
| `--until <DATE>` | Before date |

**Examples:**
```bash
mediagit log --oneline -10
mediagit log --graph --all
mediagit log --author="John" --since="2024-01-01"
mediagit log -p -- assets/
```

---

### `mediagit diff`

Show changes between commits.

```bash
mediagit diff [REVISION1] [REVISION2] [PATHS]...
```

| Flag | Description |
|------|-------------|
| `--cached` | Show staged changes |
| `--word-diff` | Word-level diff |
| `--stat` | Show statistics |
| `--summary` | Show summary |
| `-U, --unified <N>` | Context lines |
| `-q, --quiet` | Suppress output |

**Examples:**
```bash
mediagit diff                    # Working vs staged
mediagit diff --cached           # Staged vs HEAD
mediagit diff HEAD~3             # Compare to 3 commits ago
mediagit diff main develop       # Between branches
```

---

### `mediagit show`

Show object information.

```bash
mediagit show [OBJECT]
```

| Flag | Description |
|------|-------------|
| `--stat` | Show file change statistics |
| `-q, --quiet` | Suppress output |
| `-v, --verbose` | Detailed output |

**Examples:**
```bash
mediagit show                    # Show HEAD commit
mediagit show HEAD~2             # Show specific commit
mediagit show v1.0.0             # Show tag
```

---

## Branching & Merging

### `mediagit branch`

Manage branches.

```bash
mediagit branch <SUBCOMMAND>
```

**Subcommands:**

| Subcommand | Usage | Description |
|------------|-------|-------------|
| `list` | `branch list` | List branches |
| `create` | `branch create <NAME> [START]` | Create branch |
| `switch` | `branch switch <BRANCH>` | Switch branch |
| `delete` | `branch delete <BRANCHES>...` | Delete branches |
| `rename` | `branch rename <OLD> <NEW>` | Rename specific branch |
| `rename` | `branch rename <NEW>` | Rename current branch to NEW |
| `show` | `branch show [BRANCH]` | Show info |
| `protect` | `branch protect <BRANCH>` | Protect branch |

**Flags for `branch list`:**

| Flag | Description |
|------|-------------|
| `-r, --remote` | List remote-tracking branches |
| `-a, --all` | List all branches (local + remote) |
| `--sort <KEY>` | Sort branches by key |
| `-v, --verbose` | Detailed output |

**Flags for `branch create`:**

| Flag | Description |
|------|-------------|
| `-u, --set-upstream <UPSTREAM>` | Set upstream branch |
| `--track` | Track a remote branch |
| `--no-track` | Don't set tracking |

**Flags for `branch switch`:**

| Flag | Description |
|------|-------------|
| `-c, --create` | Create and switch to new branch |
| `-f, --force` | Force switch even with local changes |

**Flags for `branch delete`:**

| Flag | Description |
|------|-------------|
| `-D, --force` | Force delete (ignore merge status) |
| `-d, --delete-merged` | Delete only if merged |
| `-r, --remote` | Delete remote-tracking ref |

**Flags for `branch rename`:**

| Flag | Description |
|------|-------------|
| `-f, --force` | Force rename |

**Examples:**
```bash
mediagit branch list -a              # List all local and remote branches
mediagit branch list -r              # List remote-tracking branches only
mediagit branch create feature/new-asset
mediagit branch switch develop
mediagit branch delete -D old-branch
mediagit branch delete -r origin/stale-branch   # Delete local remote-tracking ref
mediagit branch rename feature/old-name feature/new-name   # Rename specific branch
mediagit branch rename new-name                            # Rename current branch
```

---

### `mediagit merge`

Join development histories.

```bash
mediagit merge <BRANCH>
```

| Flag | Description |
|------|-------------|
| `-m, --message <MSG>` | Merge commit message |
| `--no-ff` | Create merge commit |
| `--ff-only` | Fast-forward only |
| `--squash` | Squash commits |
| `-s, --strategy <STRATEGY>` | Merge strategy |
| `-X, --strategy-option <OPT>` | Strategy option |
| `--no-commit` | Don't commit |
| `--abort` | Abort merge |
| `--continue` | (alias `--continue-merge`) Continue merge after resolving conflicts |
| `-q, --quiet` | Suppress output |
| `-v, --verbose` | Detailed output |

**Examples:**
```bash
mediagit merge feature/complete
mediagit merge develop --no-ff -m "Merge develop into main"
mediagit merge --squash hotfix
mediagit merge --continue
```

---

### `mediagit rebase`

Rebase commits onto another branch.

```bash
mediagit rebase <UPSTREAM> [BRANCH]
```

| Flag | Description |
|------|-------------|
| `--keep-empty` | Keep empty commits |
| `--abort` | Abort rebase |
| `--continue` | (alias `--continue-rebase`) Continue rebase after resolving conflicts |
| `--skip` | Skip current commit |
| `-q, --quiet` | Suppress output |
| `-v, --verbose` | Detailed output |

> `-i/--interactive`, `-m/--rebase-merges`, and `--autosquash` are accepted but not yet implemented.

**Examples:**
```bash
mediagit rebase main
mediagit rebase --continue
mediagit rebase --abort
```

---

### `mediagit cherry-pick`

Apply changes from specific commits.

```bash
mediagit cherry-pick <COMMITS>...
```

| Flag | Description |
|------|-------------|
| `--continue` | (alias `--continue-pick`) Continue operation after resolving conflicts |
| `--abort` | Abort operation |
| `--skip` | Skip current commit |
| `-n, --no-commit` | Don't commit |
| `-e, --edit` | Edit message |
| `-x, --append-message` | Append original commit reference to message |
| `-q, --quiet` | Suppress output |

**Examples:**
```bash
mediagit cherry-pick abc123
mediagit cherry-pick abc123 def456 ghi789
mediagit cherry-pick --continue
```

---

## Remote Operations

### Transfer Architecture

Push, pull, clone, and fetch use **presigned-URL direct transfer**: the server mints short-lived presigned PUT/GET URLs and the client transfers data straight to the cloud backend (S3, MinIO, Azure Blob, GCS) without proxying bytes through the server. A proxy fallback is used when the backend cannot sign (e.g. GCS without a service-account key) or when a presigned URL returns 404. Large chunks on S3/MinIO use multipart upload (MPU) via presigned part URLs.

Pull and clone also benefit from **cloud packs**: the server bundles related chunks into pack objects stored directly in the backend. The client requests a pack index, locates needed chunks via pack-locate, and fetches them with Range-GET — reducing object count by up to 10× and dramatically cutting clone latency on small-chunk repos.

```mermaid
sequenceDiagram
    participant C as Client
    participant S as mediagit-server
    participant B as Cloud Backend<br/>(S3/MinIO/Azure/GCS)

    Note over C,B: Push
    C->>S: POST /push (ref updates + OID list)
    S-->>C: presigned PUT URLs (per chunk)
    C->>B: PUT chunks directly (parallel)
    C->>S: POST /push/complete

    Note over C,B: Pull / Clone
    C->>S: POST /pull (want OIDs)
    S-->>C: pack index + presigned GET URLs
    C->>B: GET pack / Range-GET chunks (parallel)
    C->>C: reconstruct + write ODB
```

### `mediagit push`

Push local commits to remote.

```bash
mediagit push [REMOTE] [REFSPEC]...
```

| Flag | Description |
|------|-------------|
| `-a, --all` | Push all branches |
| `--tags` | Push all tags |
| `--follow-tags` | Push annotated tags |
| `--dry-run` | Preview push |
| `-f, --force` | Force push |
| `--force-with-lease` | Safe force push |
| `-d, --delete` | Delete remote ref |
| `-u, --set-upstream` | Set upstream |
| `--no-track` | Push without setting upstream tracking |
| `--repair` | Verify remote chunk integrity and force re-upload of any chunk the server reports as corrupted |
| `-q, --quiet` | Suppress output |
| `-v, --verbose` | Detailed output |

> **Auto-upstream**: `main`/`master` branches automatically set upstream tracking on first push.
> Other branches display a hint to use `-u`.

**Examples:**
```bash
mediagit push                     # Push current branch
mediagit push origin main         # Push specific branch
mediagit push --all               # Push all branches
mediagit push -u origin feature   # Set upstream
mediagit push --force-with-lease  # Safe force push

# Delete a remote branch (also removes local remote-tracking ref)
mediagit push origin --delete feature/old-branch
```

> **HEAD Protection**: You cannot delete the branch that is currently checked out on the remote
> (typically `main` or `master`). The server will reject the deletion with a clear error message.

---

### `mediagit pull`

Fetch and integrate remote changes.

```bash
mediagit pull [REMOTE] [BRANCH]
```

| Flag | Description |
|------|-------------|
| `-r, --rebase` | Rebase instead of merge |
| `--dry-run` | Preview pull |
| `--continue` | Continue after resolving conflicts (hidden; implemented) |
| `-q, --quiet` | Suppress output |
| `-v, --verbose` | Detailed output |

> `-s/--strategy` and `-X/--strategy-option` are accepted for git-compatibility but hidden; MediaGit uses binary-aware merge for media files.
>
> `--no-commit` and `--abort` are declared but **not implemented** — both exit with an error telling you so. Use `mediagit merge --abort` to abort a conflicted pull.

**Examples:**
```bash
mediagit pull
mediagit pull origin develop
mediagit pull --rebase
# NOTE: `pull --continue`, `-s` and `-X` are not implemented and now REFUSE
# rather than being silently ignored. To resume after conflicts, resolve them
# and use `mediagit merge --continue`.
```

---

### `mediagit fetch`

Fetch remote changes without merging.

```bash
mediagit fetch [REMOTE] [BRANCH]
```

| Flag | Description |
|------|-------------|
| `--all` | Fetch all remotes |
| `-p, --prune` | Remove stale refs |
| `-q, --quiet` | Suppress output |
| `-v, --verbose` | Detailed output |

**Examples:**
```bash
mediagit fetch
mediagit fetch origin
mediagit fetch --all --prune
```

---

### `mediagit download`

Download a single file from a remote repository by path — a plain streaming
GET against the server's file-browse endpoint, not a full clone. Works
**without a local repository** when given a full URL (the point: CI/scripting
can pull one asset without cloning). When run inside a repository with a
non-URL path, resolves against the `origin` remote.

```bash
mediagit download <REMOTE_PATH> [--ref <REF>] [-o <PATH>]
```

| Flag | Description |
|------|-------------|
| `--ref <REF>` | Branch, tag, or commit OID to download from (default: server's `main`, else `master`, else first branch) |
| `-o, --output <PATH>` | Output file path (default: the file's base name in the current directory) |
| `-q, --quiet` | Suppress output |

**Examples:**
```bash
# No local repository needed — first path segment after the host is the repo name
mediagit download http://server:3000/my-project/assets/logo.png

mediagit download http://server:3000/my-project/assets/logo.png --ref v1.0
mediagit download http://server:3000/my-project/assets/logo.png -o logo.png

# Inside a repo, resolves against the 'origin' remote
mediagit download assets/logo.png
```

Rejects `..` path-traversal components client-side (in addition to server-side
validation). Attaches client credentials (`MEDIAGIT_TOKEN`/`MEDIAGIT_API_KEY`
or per-remote config) exactly like `push`/`pull`/`fetch`/`clone` when run
inside a repository with a non-URL path. In full-URL mode, credentials are
attached only if the typed URL's host matches one of the current
repository's configured remotes (scheme + host + effective port); with no
local repository, or no matching remote, no credentials are sent — this
keeps `MEDIAGIT_TOKEN`/`MEDIAGIT_API_KEY` from being sent to an arbitrary
host.

---

## Tags

### `mediagit tag`

Manage tags. Annotated tags (`-a`/`-m`) are real objects in the object
database (`ObjectType::Tag`) — a tag name, tagger, message, and a target
OID, just like a commit. Lightweight tags are still just a ref pointing
directly at a commit.

```bash
mediagit tag <SUBCOMMAND>
```

**Subcommands:**

| Subcommand | Usage | Description |
|------------|-------|-------------|
| `create` | `tag create <NAME> [COMMIT]` | Create tag |
| `list` | `tag list [PATTERN]` | List tags |
| `delete` | `tag delete <NAME>...` | Delete tags |
| `show` | `tag show <NAME>` | Show tag info |
| `verify` | `tag verify <NAME>` | Verify tag ref, and signature if annotated |

#### Signing (`MEDIAGIT_SIGN`)

Annotated tags can be signed with your **existing OpenSSH ed25519 key**
(`~/.ssh/id_ed25519` by default — override with `MEDIAGIT_SIGN_KEY`), so
signing reuses key management you already have. The signature format is
**MediaGit-native** (an OpenSSH-armored `SshSig` blob stored inside the Tag
object) — this is not git's tag-signing format, and git interop is not a
goal; MediaGit is a standalone VCS.

```bash
MEDIAGIT_SIGN=1 mediagit tag create v1.0.0 -m "Release 1.0.0"
mediagit tag verify v1.0.0    # valid signature, signed by <fingerprint> / INVALID (exits non-zero) / unsigned
                              # verifies against the key embedded in the signature (TOFU) — no local key needed
```

Off by default (opt-in). Passphrase-protected keys are detected and
rejected with a clear error — passphrase prompting isn't implemented yet.

**Flags for `tag create`:**

| Flag | Description |
|------|-------------|
| `-a, --annotated` | Create annotated tag |
| `-m, --message <MSG>` | Tag message (implies annotated) |
| `--tagger <NAME>` | Override tagger name (annotated tags) |
| `--email <EMAIL>` | Override tagger email (annotated tags) |
| `-f, --force` | Replace existing tag |
| `-q, --quiet` | Suppress output |

**Flags for `tag list`:**

| Flag | Description |
|------|-------------|
| `-n, --verbose` | Show verbose output (include commit info) |
| `--sort <KEY>` | Sort by key (default: `refname`) |
| `--reverse` | Reverse sort order |

**Flags for `tag show`:**

| Flag | Description |
|------|-------------|
| `--full` | Show full OID details |

**Examples:**
```bash
mediagit tag create v1.0.0
mediagit tag create v1.0.0 -m "Release version 1.0.0"
mediagit tag list
mediagit tag delete v0.9.0
```

---

## Media & Sparse Checkout

### `mediagit media`

Inspect media file metadata — image, video, audio, PSD, and 3D-model formats.
Reads a working-tree file directly (never touches the ODB) and reports every
top-level field the corresponding `mediagit-media` parser produces.

```bash
mediagit media info <PATH> [--json]
```

| Flag | Description |
|------|-------------|
| `--json` | Output the full parsed metadata struct as JSON instead of `label: value` lines |

Supported extensions: `jpg/jpeg/png/tif/tiff/webp` (image), `mp4/mov/m4v`
(video), `wav/mp3/flac/aac/ogg/m4a` (audio), `psd` (PSD), `obj/fbx/blend/
gltf/glb/stl/usd/usda/usdc/usdz/ply` (3D). Files over 256 MB are skipped
(same cap as the `status`/`show` `media: ...` summary line, gated by
`MEDIAGIT_MEDIA_META`). An unsupported extension prints a one-line message
and exits `0`.

**Examples:**
```bash
mediagit media info assets/hero.png
mediagit media info assets/clip.mp4 --json
```

### `mediagit sparse-checkout`

Materialize only part of the working tree. Two pattern styles: **cone mode**
(default) takes directory prefixes, included recursively; **pattern mode**
(`--patterns`) takes gitignore-style globs, where a match means *include*
(the inverse of `.mediagitignore`).

```bash
mediagit sparse-checkout <SUBCOMMAND>
```

**Subcommands:**

| Subcommand | Usage | Description |
|------------|-------|-------------|
| `set` | `sparse-checkout set <PATTERN>... [--patterns]` | Write patterns and apply: remove newly-excluded tracked files, materialize newly-included ones |
| `list` (alias `ls`) | `sparse-checkout list` | Show the active mode and patterns |
| `disable` | `sparse-checkout disable` | Remove the pattern file and restore the full working tree |

**Semantics:** excluded files are never written by ordinary checkout
operations (branch switch, clone, reset, etc.), and an excluded file that
already exists on disk is never deleted by them either — it's simply outside
the cone. Only `sparse-checkout set`/`disable` materialize or remove files in
response to a pattern change. `status` treats sparse-excluded paths as
absent, not deleted.

**Examples:**
```bash
mediagit sparse-checkout set assets/textures assets/audio
mediagit sparse-checkout set --patterns '*.png' '*.wav'
mediagit sparse-checkout list
mediagit sparse-checkout disable
```

---

## Stashing

### `mediagit stash`

Temporarily save changes.

```bash
mediagit stash <SUBCOMMAND>
```

**Subcommands:**

| Subcommand | Usage | Description |
|------------|-------|-------------|
| `save` | `stash save [-m MSG] [-u] [PATHS...]` | Save changes |
| `push` | `stash push [-m MSG] [-u] [PATHS...]` | Save changes (git-compatible alias for `save`) |
| `apply` | `stash apply [STASH]` | Apply stash |
| `list` | `stash list` | List stashes |
| `show` | `stash show [STASH]` | Show stash |
| `drop` | `stash drop [STASH]` | Remove stash |
| `pop` | `stash pop [STASH]` | Apply and remove |
| `clear` | `stash clear` | Clear all |

**Flags for `stash save` / `stash push`:**

| Flag | Description |
|------|-------------|
| `-m, --message <MSG>` | Stash message |
| `-u, --include-untracked` | Include untracked files |
| `-q, --quiet` | Suppress output |

**Flags for `stash apply` / `stash pop`:**

| Flag | Description |
|------|-------------|
| `--index` | Reinstate index (staged) changes |

**Flags for `stash show`:**

| Flag | Description |
|------|-------------|
| `-p, --patch` | Show patch diff |

**Examples:**
```bash
mediagit stash save "WIP: new feature"
mediagit stash list
mediagit stash pop
mediagit stash apply stash@{2}
```

---

## History Manipulation

### `mediagit reset`

Reset current HEAD to specified state.

```bash
mediagit reset [COMMIT] [PATHS]...
```

**Modes:**

| Flag | Effect |
|------|--------|
| `--soft` | Only move HEAD (keep index and working tree) |
| *(default)* | Move HEAD and reset index (mixed mode) |
| `--hard` | Move HEAD, reset index, **and** reset working tree |

| Flag | Description |
|------|-------------|
| `-q, --quiet` | Suppress output |

> **Path mode**: When `PATHS` are specified, `reset` unstages the given files
> (restores the index entry to match HEAD) without changing HEAD or working tree.
> `--soft` and `--hard` cannot be used with paths.

**Examples:**
```bash
mediagit reset --soft HEAD~1     # Undo last commit, keep changes staged
mediagit reset HEAD~1            # Undo last commit, unstage changes
mediagit reset --hard HEAD~1     # Undo last commit, discard all changes
mediagit reset file.txt          # Unstage a specific file
```

---

### `mediagit revert`

Create new commits that undo changes from existing commits.
The working directory is updated to reflect the reverted state after the commit is created.

```bash
mediagit revert <COMMITS>...
```

| Flag | Description |
|------|-------------|
| `-n, --no-commit` | Apply revert without committing |
| `-m, --message <MSG>` | Custom commit message |
| `--continue` | Continue after resolving conflicts |
| `--abort` | Abort current revert |
| `--skip` | Skip current commit |
| `-q, --quiet` | Suppress output |

**Examples:**
```bash
mediagit revert HEAD             # Revert the last commit
mediagit revert abc1234          # Revert a specific commit
mediagit revert --no-commit HEAD # Revert without auto-committing
mediagit revert --continue       # Continue after conflict resolution
mediagit revert --abort          # Abort the revert operation
```

---

### `mediagit reflog`

Show reference logs — when branch tips and other refs were updated.

```bash
mediagit reflog [SUBCOMMAND] [REF]
```

**Subcommands:**

| Subcommand | Usage | Description |
|------------|-------|-------------|
| `show` | `reflog show [REF]` | Show reflog entries (default) |
| `delete` | `reflog delete <REF>` | Delete reflog for a reference |
| `expire` | `reflog expire [REF]` | Prune old reflog entries |

| Flag | Description |
|------|-------------|
| `-n, --count <N>` | Number of entries to show |
| `--all` | Show reflogs for all refs (with `show`) |
| `--keep <N>` | Entries to keep when expiring (default: 90) |
| `-q, --quiet` | Only show OIDs |

**Examples:**
```bash
mediagit reflog                             # Show reflog for HEAD
mediagit reflog show refs/heads/main        # Show reflog for main
mediagit reflog show -n 5                   # Show last 5 entries
mediagit reflog show --all                  # Show all reflogs
mediagit reflog delete refs/heads/feature   # Delete reflog
mediagit reflog expire --keep 30            # Keep last 30 entries
```

---

## Debugging

### `mediagit bisect`

Find bug-introducing commit using binary search.

```bash
mediagit bisect <SUBCOMMAND>
```

**Subcommands:**

| Subcommand | Usage | Description |
|------------|-------|-------------|
| `start` | `bisect start [BAD] [GOOD]` | Start bisect |
| `good` | `bisect good [COMMIT]` | Mark as good |
| `bad` | `bisect bad [COMMIT]` | Mark as bad |
| `skip` | `bisect skip [COMMIT]` | Skip commit |
| `reset` | `bisect reset [COMMIT]` | Reset session |
| `log` | `bisect log` | Show log |
| `replay` | `bisect replay <LOGFILE>` | Replay log |

**Examples:**
```bash
mediagit bisect start HEAD v1.0.0
mediagit bisect bad
mediagit bisect good
mediagit bisect reset
```

---

## Maintenance

### `mediagit gc`

Garbage collection and optimization.

```bash
mediagit gc
```

| Flag | Description |
|------|-------------|
| `--aggressive` | Aggressive optimization pass |
| `--no-prune` | Skip pruning unreachable objects (gc prunes by default) |
| `--auto` | Run only if thresholds are exceeded |
| `--dry-run` | Preview changes without deleting |
| `-y, --yes` | Skip confirmation prompts |
| `--repack` | Repack loose objects into pack files |
| `--max-pack-size <N>` | Max objects per pack file (0 = unlimited) |
| `-q, --quiet` | Suppress output |
| `-v, --verbose` | Detailed output |

**GC performs three cleanup phases:**
1. **Loose objects** — sweep unreachable objects not referenced by any branch, tag, or reflog
2. **Chunk manifests** — remove manifests whose blob OID is no longer reachable
3. **Chunks** — remove chunks not referenced by any surviving manifest (content-addressed, so shared chunks are preserved)

**Examples:**
```bash
mediagit gc                       # Standard garbage collection
mediagit gc --repack --yes       # Repack loose objects into packs, skip confirmation
mediagit gc --dry-run             # Preview what would be deleted
mediagit gc --verbose             # Show each deleted object/chunk/manifest
```

> **Branch cleanup workflow**: After deleting a remote branch with `push --delete`,
> run `mediagit gc` to reclaim storage from orphaned chunks and manifests.

---

### `mediagit fsck`

Check repository integrity (verifies BLAKE3 checksums, reference validity, and commit graph connectivity).

```bash
mediagit fsck
```

| Flag | Description |
|------|-------------|
| `--full` | Full check |
| `--quick` | Quick check |
| `--all` | Check all objects |
| `--lost-found` | Write dangling objects |
| `--no-dangling` | Don't report dangling |
| `--repair` | Attempt repairs |
| `--dry-run` | Preview repairs |
| `--max-objects <N>` | Limit objects checked |
| `--path <PATH>` | Check specific path |
| `-q, --quiet` | Suppress output |
| `-v, --verbose` | Detailed output |

**Examples:**
```bash
mediagit fsck
mediagit fsck --full --verbose
mediagit fsck --repair --dry-run
```

---

### `mediagit verify`

Quick integrity verification — checks BLAKE3 object checksums and reference validity. For full graph analysis use `fsck`.

```bash
mediagit verify [COMMIT]
```

| Flag | Description |
|------|-------------|
| `--file-integrity` | Check file checksums |
| `--checksums` | Verify object checksums |
| `--start <COMMIT>` | Start commit |
| `--end <COMMIT>` | End commit |
| `--quick` | Quick check |
| `--detailed` | Detailed report |
| `--path <PATH>` | Check specific path |
| `-q, --quiet` | Suppress output |
| `-v, --verbose` | Detailed output |

---

### `mediagit stats`

Show repository statistics.

```bash
mediagit stats
```

| Flag | Description |
|------|-------------|
| `--storage` | Storage statistics |
| `--files` | File statistics |
| `--commits` | Commit statistics |
| `--branches` | Branch statistics |
| `--authors` | Author statistics |
| `--compression` | Compression stats |
| `--all` | All statistics |
| `--json` | JSON output |
| `--prometheus` | Prometheus format |
| `-q, --quiet` | Suppress output |
| `-v, --verbose` | Detailed output |

**Examples:**
```bash
mediagit stats --all
mediagit stats --storage --compression
mediagit stats --json > stats.json
```

---

## Encryption

### `mediagit key`

Manage this repository's at-rest encryption key. Encryption is per repository
and is **enabled at creation or not at all** — `key init` refuses on a
repository that already holds objects, because sealing them would mean
rewriting every one.

```bash
mediagit key init            # enable encryption (empty repository only)
mediagit key status          # is this repository encrypted, and how does it unlock
mediagit key recover         # unlock with the one-time recovery code
mediagit key rotate-master   # re-lock the key under a new master key
```

`rotate-master` accepts `--new-keyfile <PATH>`, needed when rotating from one
key file to another. It changes only what protects the repository key, not the
key itself, so no object is rewritten and the recovery code keeps working.

There is no way to encrypt an existing repository, to remove encryption, or to
replace the repository key — all three would have to rewrite every object.

`key init` prints a one-time recovery code. If both it and the master key are
lost, the objects cannot be recovered.

| Variable | Effect |
| --- | --- |
| `MEDIAGIT_ENCRYPTION_KEYFILE` | File holding the master key, used instead of the OS keychain |
| `MEDIAGIT_NO_KEYRING` | Skip the OS keychain entirely |

See [book/src/cli/key.md](book/src/cli/key.md) for the long form.

---

## Meta

### `mediagit version`

Show version information including Rust toolchain version and license.

```bash
mediagit version
```

---

### `mediagit completions`

Generate shell completions for your shell.

```bash
mediagit completions <SHELL>
```

Supported shells: `bash`, `elvish`, `fish`, `powershell`, `zsh`

**Examples:**
```bash
# Bash
mediagit completions bash > ~/.local/share/bash-completion/completions/mediagit

# Zsh
mediagit completions zsh > ~/.zfunc/_mediagit

# Fish
mediagit completions fish > ~/.config/fish/completions/mediagit.fish

# PowerShell
mediagit completions powershell >> $PROFILE
```

---

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | General error |
| 2 | Fatal error (panic) |

---

## Environment Variables

| Variable | Description |
|----------|-------------|
| `MEDIAGIT_REPO` | Repository path (set by `-C` flag) |
| `MEDIAGIT_AUTHOR_NAME` | Default author name |
| `MEDIAGIT_AUTHOR_EMAIL` | Default author email |
| `MEDIAGIT_TOKEN` | Bearer token for remote authentication (client auth). **Highest** precedence: beats per-remote `token` in `config.toml` and the OS keychain |
| `MEDIAGIT_API_KEY` | API key for remote authentication (client auth). Same tier as `MEDIAGIT_TOKEN`, and likewise beats config and keychain; `MEDIAGIT_TOKEN` wins if both are set |
| `MEDIAGIT_SIGN` | Sign annotated tags with your SSH key (`1`/`true`/`on`). Off by default |
| `MEDIAGIT_SIGN_KEY` | Path to the ed25519 key used to sign/verify tags. Default: `~/.ssh/id_ed25519` |
| `MEDIAGIT_CHECKOUT_PARALLELISM` | Number of parallel file I/O operations during checkout (branch switch, clone, etc.). Default: number of CPUs capped at 8. Set to `1` for sequential behavior |
| `MEDIAGIT_BITMAP` | Enable reachability bitmap generation on push/gc and consumption on fetch/pull (`0`/`1`, `off`/`on`, `false`/`true`). Default: on. Set to `0` to disable (falls back to BFS walk) |
| `MEDIAGIT_REPO_NAMESPACE` | Override the default repository namespace prefix used by multi-repo storage backends. Default: sanitized repository directory basename |
| `MEDIAGIT_REFLOG_MAX` | Maximum number of reflog entries to keep per ref. Default: `1000`. Older entries are pruned during `git` operations |

### Performance & Concurrency

| Variable | Default | Description |
|----------|---------|-------------|
| `MEDIAGIT_UPLOAD_CONCURRENCY` | `32` | Total upload semaphore slots for push operations |
| `MEDIAGIT_PUSH_OBJECT_CONCURRENCY` | `8` | Number of objects uploaded concurrently during push |
| `MEDIAGIT_PUSH_CHUNK_CONCURRENCY` | `(64 / push_object_concurrency).max(4)` | Per-object chunk upload concurrency. Targets 64 total in-flight PUTs across all concurrent objects. Override when tuning for specific cloud regions or connection profiles. |
| `MEDIAGIT_DOWNLOAD_CONCURRENCY` | `32` | Total concurrent chunk downloads during pull/clone |
| `MEDIAGIT_FETCH_BRANCH_CONCURRENCY` | `4` | Number of branches fetched concurrently during `fetch --all` |
| `MEDIAGIT_FETCH_DOWNLOAD_CONCURRENCY` | `max(32 / branch_concurrency, 8)` | Per-branch download concurrency cap during `fetch --all`. Prevents TCP pool exhaustion (peak in-flight ≤ 128 at defaults). |
| `MEDIAGIT_GCS_UPLOAD_CONCURRENCY` | `4` | Concurrent PUT slots for GCS backend (proxy path). Lower than S3 default to avoid 500s on shared TCP connections. |
| `MEDIAGIT_RANGE_PARALLEL` | `4` | Parallel range-GET requests per chunk during download |

### Throughput Pipeline (Phase 2)

These knobs control the pipelined transfer engine shipped in v0.2.7-beta.1. All defaults are tuned for production use — override only when profiling specific backends or network conditions.

| Variable | Default | Description |
|----------|---------|-------------|
| `MEDIAGIT_PULL_PIPELINE` | `1` (ON) | Enable pipelined pull — overlaps manifest fetch with chunk download |
| `MEDIAGIT_PULL_MANIFEST_CONCURRENCY` | `8` | Number of manifests fetched concurrently via `buffer_unordered` during pull |
| `MEDIAGIT_PUSH_PIPELINE` | `1` (ON) | Enable pipelined push — overlaps chunk upload with ODB reads |
| `MEDIAGIT_STREAM_CHUNK_TO_DISK` | `1` (ON) | Stream downloaded chunks to disk during clone/pull instead of buffering in heap. Prevents OOM on large repos |
| `MEDIAGIT_STORAGE_STREAMING` | `1` (ON) | Use streaming GET from S3/MinIO backends instead of buffered GET. 15.8% faster AWS clone measured |
| `MEDIAGIT_DECOMPRESS_BLOCKING` | `1` (ON) | Offload decompression to `spawn_blocking` threadpool to avoid starving the async executor |
| `MEDIAGIT_DECOMPRESS_BLOCKING_THRESHOLD` | `262144` | Minimum compressed size (bytes) before offloading to blocking threadpool. Below this, decompress inline |
| `MEDIAGIT_HTTP_POOL_MAX` | `64` | Max idle TCP connections per host in the HTTP connection pool |

---

## Server Configuration (`mediagit-server`)

The `mediagit-server` daemon provides remote repository access over HTTP/S. Repositories hosted by the server can store their actual data in alternative cloud storage backends like S3 or MinIO instead of the local filesystem.

To configure a repository to use a cloud storage backend, you must edit its `.mediagit/config.toml` file.

### S3 / MinIO Storage Backend

The `s3` backend type covers two distinct modes selected by whether `endpoint` is set:

| `endpoint` field | Mode | Used for |
|---|---|---|
| **Set** | MinIO-compatible | Self-hosted MinIO, DigitalOcean Spaces, Cloudflare R2, any S3-compatible service |
| **Absent** | Native AWS S3 | Real AWS S3 — uses correct SigV4 region signing and virtual-hosted addressing |

**Example: MinIO / S3-compatible service**
```toml
[storage]
backend = "s3"
endpoint = "http://localhost:9000"   # required for MinIO-compatible mode
bucket = "mediagit-production"
access_key_id = "your_access_key"
secret_access_key = "your_secret_key"
region = "us-east-1"
```

**Example: Real AWS S3**
```toml
[storage]
backend = "s3"
bucket = "my-mediagit-bucket"
region = "ap-south-1"               # required: determines SigV4 signing region
access_key_id = "AKIAIOSFODNN7EXAMPLE"
secret_access_key = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
prefix = "media/"
encryption = true
encryption_algorithm = "AES256"
# Do NOT set endpoint — omitting it activates native AWS S3 mode
```

> **Note**: The `backend = "minio"` variant is deprecated. Use `backend = "s3"` for all cases. Use `access_key_id`/`secret_access_key` (not `access_key`/`secret_key`).

---

## See Also

- [Architecture](ARCHITECTURE.md)
- [Supported Formats](SUPPORTED_FORMATS.md)
- [Development Guide](DEVELOPMENT_GUIDE.md)
- [Cloud Architecture](CLOUD_ARCHITECTURE.md)
- [Future TODOs](FUTURE_TODOS.md)

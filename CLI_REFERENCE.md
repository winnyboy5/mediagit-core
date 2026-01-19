# MediaGit CLI Reference

Complete command reference for MediaGit - Git for Media Files.

---

## Quick Reference

| Category | Commands |
|----------|----------|
| **Setup** | `init`, `clone`, `install`, `remote` |
| **Basic** | `add`, `commit`, `status`, `log`, `diff`, `show` |
| **Branching** | `branch`, `merge`, `rebase`, `cherry-pick` |
| **Remote** | `push`, `pull`, `fetch` |
| **Tags** | `tag` |
| **Stashing** | `stash` |
| **Maintenance** | `gc`, `fsck`, `verify`, `stats` |
| **Advanced** | `bisect`, `filter`, `track` |

---

## Repository Setup

### `mediagit init`

Initialize a new MediaGit repository.

```bash
mediagit init [PATH]
```

| Flag | Description |
|------|-------------|
| `--bare` | Create bare repository |
| `--initial-branch <NAME>` | Set initial branch name |
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

### `mediagit install`

Install MediaGit filter driver.

```bash
mediagit install
```

| Flag | Description |
|------|-------------|
| `-f, --force` | Overwrite existing config |
| `-r, --repo <PATH>` | Install for specific repository |
| `-g, --global` | Install globally |

---

### `mediagit remote`

Manage remote repositories.

```bash
mediagit remote <SUBCOMMAND>
```

**Subcommands:**

| Subcommand | Usage | Description |
|------------|-------|-------------|
| `add` | `remote add <NAME> <URL>` | Add remote |
| `remove` | `remote remove <NAME>` | Remove remote |
| `list` | `remote list` | List remotes |
| `rename` | `remote rename <OLD> <NEW>` | Rename remote |
| `show` | `remote show <NAME>` | Show remote info |
| `set-url` | `remote set-url <NAME> <URL>` | Change URL |

**Examples:**
```bash
mediagit remote add origin http://server:3000/project
mediagit remote list -v
mediagit remote set-url origin http://new-server:3000/project
```

---

## Basic Workflow

### `mediagit add`

Stage file contents for commit.

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
| `--no-chunking` | Disable chunking for large files |
| `--no-delta` | Disable delta compression |
| `-q, --quiet` | Suppress output |
| `-v, --verbose` | Detailed output |

**Examples:**
```bash
mediagit add .                   # Stage all changes
mediagit add file.psd            # Stage specific file
mediagit add -p                  # Interactive staging
mediagit add --all --verbose     # Stage all with details
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
| `-b, --branch` | Show branch info |
| `--ahead-behind` | Show ahead/behind counts |
| `-q, --quiet` | Suppress output |
| `-v, --verbose` | Detailed output |

**Examples:**
```bash
mediagit status
mediagit status -s              # Short format
mediagit status --porcelain     # For scripting
```

---

### `mediagit log`

Show commit history.

```bash
mediagit log [REVISION] [PATHS]...
```

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
| `-p, --patch` | Show patch |
| `--stat` | Show statistics |
| `--pretty <FORMAT>` | Output format |
| `-U, --unified <N>` | Context lines |
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
| `rename` | `branch rename <NEW> [OLD]` | Rename branch |
| `show` | `branch show [BRANCH]` | Show info |
| `merge` | `branch merge <BRANCH>` | Merge branch |
| `protect` | `branch protect <BRANCH>` | Protect branch |

| Flag | Description |
|------|-------------|
| `-r, --remote` | List/operate on remote branches |
| `-a, --all` | List all branches |
| `-c, --create` | Create and switch |
| `-f, --force` | Force operation |
| `-D` | Force delete |
| `-u, --set-upstream` | Set upstream |
| `--no-ff` | No fast-forward merge |
| `--ff-only` | Fast-forward only |
| `-v, --verbose` | Detailed output |
| `-q, --quiet` | Suppress output |

**Examples:**
```bash
mediagit branch list -a
mediagit branch create feature/new-asset
mediagit branch switch develop
mediagit branch delete -D old-branch
mediagit branch merge feature/complete --no-ff
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
| `--continue` | Continue merge |
| `-q, --quiet` | Suppress output |
| `-v, --verbose` | Detailed output |

**Examples:**
```bash
mediagit merge feature/complete
mediagit merge develop --no-ff -m "Merge develop into main"
mediagit merge --squash hotfix
```

---

### `mediagit rebase`

Rebase commits onto another branch.

```bash
mediagit rebase <UPSTREAM> [BRANCH]
```

| Flag | Description |
|------|-------------|
| `-i, --interactive` | Interactive rebase |
| `-m, --rebase-merges` | Preserve merges |
| `--keep-empty` | Keep empty commits |
| `--autosquash` | Auto-squash fixup commits |
| `--abort` | Abort rebase |
| `--continue` | Continue rebase |
| `--skip` | Skip current commit |
| `-q, --quiet` | Suppress output |
| `-v, --verbose` | Detailed output |

**Examples:**
```bash
mediagit rebase main
mediagit rebase -i HEAD~5
mediagit rebase --continue
```

---

### `mediagit cherry-pick`

Apply changes from specific commits.

```bash
mediagit cherry-pick <COMMITS>...
```

| Flag | Description |
|------|-------------|
| `--continue` | Continue operation |
| `--abort` | Abort operation |
| `--skip` | Skip current commit |
| `-n, --no-commit` | Don't commit |
| `-e, --edit` | Edit message |
| `-x` | Append commit reference |
| `-q, --quiet` | Suppress output |

**Examples:**
```bash
mediagit cherry-pick abc123
mediagit cherry-pick abc123 def456 ghi789
mediagit cherry-pick --continue
```

---

## Remote Operations

### `mediagit push`

Push local commits to remote.

```bash
mediagit push [REMOTE] [REFSPEC]...
```

| Flag | Description |
|------|-------------|
| `--all` | Push all branches |
| `--tags` | Push all tags |
| `--follow-tags` | Push annotated tags |
| `--dry-run` | Preview push |
| `-f, --force` | Force push |
| `--force-with-lease` | Safe force push |
| `-d, --delete` | Delete remote ref |
| `-u, --set-upstream` | Set upstream |
| `-q, --quiet` | Suppress output |
| `-v, --verbose` | Detailed output |

**Examples:**
```bash
mediagit push                     # Push current branch
mediagit push origin main         # Push specific branch
mediagit push --all               # Push all branches
mediagit push -u origin feature   # Set upstream
mediagit push --force-with-lease  # Safe force push
```

---

### `mediagit pull`

Fetch and integrate remote changes.

```bash
mediagit pull [REMOTE] [BRANCH]
```

| Flag | Description |
|------|-------------|
| `-r, --rebase` | Rebase instead of merge |
| `-s, --strategy <STRATEGY>` | Merge strategy |
| `-X, --strategy-option <OPT>` | Strategy option |
| `--dry-run` | Preview pull |
| `--no-commit` | Don't commit merge |
| `--abort` | Abort pull |
| `--continue` | Continue pull |
| `-q, --quiet` | Suppress output |
| `-v, --verbose` | Detailed output |

**Examples:**
```bash
mediagit pull
mediagit pull origin develop
mediagit pull --rebase
mediagit pull --continue
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

## Tags

### `mediagit tag`

Manage tags.

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
| `verify` | `tag verify <NAME>` | Verify tag |

| Flag | Description |
|------|-------------|
| `-m, --message <MSG>` | Tag message (annotated) |
| `--tagger <NAME>` | Override tagger name |
| `--email <EMAIL>` | Override email |
| `-f, --force` | Replace existing tag |
| `--sort <KEY>` | Sort by key |
| `--reverse` | Reverse sort order |
| `--full` | Show full OIDs |
| `-q, --quiet` | Suppress output |
| `-v, --verbose` | Detailed output |

**Examples:**
```bash
mediagit tag create v1.0.0
mediagit tag create v1.0.0 -m "Release version 1.0.0"
mediagit tag list
mediagit tag delete v0.9.0
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
| `save` | `stash save [MESSAGE]` | Save changes |
| `apply` | `stash apply [STASH]` | Apply stash |
| `list` | `stash list` | List stashes |
| `show` | `stash show [STASH]` | Show stash |
| `drop` | `stash drop [STASH]` | Remove stash |
| `pop` | `stash pop [STASH]` | Apply and remove |
| `clear` | `stash clear` | Clear all |

| Flag | Description |
|------|-------------|
| `-u, --include-untracked` | Include untracked files |
| `--index` | Restore index state |
| `-p, --patch` | Interactive stash |
| `-f, --force` | Force apply |
| `-q, --quiet` | Suppress output |
| `-v, --verbose` | Detailed output |

**Examples:**
```bash
mediagit stash save "WIP: new feature"
mediagit stash list
mediagit stash pop
mediagit stash apply stash@{2}
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
| `--aggressive` | Aggressive optimization |
| `--prune <DAYS>` | Prune objects older than N days |
| `--auto` | Run only if needed |
| `--dry-run` | Preview changes |
| `-y, --yes` | Skip confirmation |
| `--repack` | Repack objects |
| `--max-pack-size <N>` | Max pack size |
| `-q, --quiet` | Suppress output |
| `-v, --verbose` | Detailed output |

**Examples:**
```bash
mediagit gc
mediagit gc --aggressive
mediagit gc --prune=30 --dry-run
```

---

### `mediagit fsck`

Check repository integrity.

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

Quick integrity verification.

```bash
mediagit verify
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

## Advanced Features

### `mediagit filter`

Git filter driver (clean/smudge).

```bash
mediagit filter <SUBCOMMAND>
```

**Subcommands:**

| Subcommand | Usage | Description |
|------------|-------|-------------|
| `clean` | `filter clean [FILE]` | Convert to pointer |
| `smudge` | `filter smudge [FILE]` | Restore from pointer |

---

### `mediagit track`

Manage file pattern tracking.

```bash
mediagit track [PATTERN]
```

| Flag | Description |
|------|-------------|
| `-l, --list` | List tracked patterns |

**Examples:**
```bash
mediagit track "*.psd"
mediagit track "*.mp4"
mediagit track --list
```

---

## Common Flags

These flags are available on most commands:

| Flag | Description |
|------|-------------|
| `-q, --quiet` | Suppress output |
| `-v, --verbose` | Detailed output |
| `--dry-run` | Preview without executing |
| `-h, --help` | Show help |

---

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | General error |
| 128 | Fatal error |

---

## Environment Variables

| Variable | Description |
|----------|-------------|
| `MEDIAGIT_DIR` | Repository path |
| `MEDIAGIT_WORK_TREE` | Working tree path |
| `MEDIAGIT_AUTHOR_NAME` | Default author name |
| `MEDIAGIT_AUTHOR_EMAIL` | Default author email |

---

## See Also

- [Workflow Documentation](WORKFLOW.md)
- [Supported Formats](SUPPORTED_FORMATS.md)
- [Server Configuration](SERVER.md)

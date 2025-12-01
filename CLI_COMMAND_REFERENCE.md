# MediaGit CLI Command Reference

## Global Options

All commands support these global options:

```
-v, --verbose              Enable verbose output
-q, --quiet                Suppress output
--color <WHEN>             Colored output (always|auto|never) [default: auto]
-C, --repository <PATH>    Repository path
```

## Core Commands

### Repository Initialization

#### `mediagit init [PATH]`
Initialize a new MediaGit repository.

**Options:**
- `--bare` - Create a bare repository (no working directory)
- `--initial-branch <BRANCH>` - Set initial branch name (default: main)
- `--template <PATH>` - Use template directory for initialization
- `-q, --quiet` - Minimal output

**Example:**
```bash
mediagit init                           # Initialize in current directory
mediagit init /path/to/repo             # Initialize in specific path
mediagit init --bare --initial-branch develop  # Create bare repo with develop branch
```

### File Staging

#### `mediagit add [PATHS]...`
Stage file contents for the next commit.

**Options:**
- `-A, --all` - Stage all changes
- `-p, --patch` - Interactively choose hunks to stage
- `--dry-run` - Show what would be staged without staging
- `-f, --force` - Force add even if ignored
- `-u, --update` - Update tracked files only
- `-v, --verbose` - Show what is being staged

**Example:**
```bash
mediagit add file.txt                   # Stage specific file
mediagit add *.jpg                      # Stage all JPEG files
mediagit add --all                      # Stage all changes
mediagit add --patch                    # Interactive staging
```

### Committing

#### `mediagit commit [OPTIONS]`
Record changes to the repository.

**Options:**
- `-m, --message <MSG>` - Commit message (required if not interactive)
- `-e, --edit` - Open editor for commit message
- `-F, --file <FILE>` - Use file as commit message
- `-a, --all` - Stage modified/deleted files before committing
- `--author <NAME <EMAIL>>` - Override commit author
- `--date <DATE>` - Override commit date
- `--allow-empty` - Allow empty commits
- `-s, --signoff` - Sign off the commit
- `--dry-run` - Show what would be committed

**Example:**
```bash
mediagit commit -m "Initial commit"
mediagit commit -am "Update files"     # Stage and commit modified files
mediagit commit -e                     # Edit message in editor
mediagit commit --dry-run               # Preview commit
```

### Branch Management

#### `mediagit branch [COMMAND]`
Manage branches. Available subcommands:

##### `mediagit branch list [OPTIONS]`
List branches.

**Options:**
- `-r, --remote` - List remote branches
- `-a, --all` - List all branches (local and remote)
- `-v, --verbose` - Show verbose information
- `--sort <KEY>` - Sort by specified key

**Example:**
```bash
mediagit branch list                    # List local branches
mediagit branch list --all              # List all branches
mediagit branch list --remote           # List remote branches
mediagit branch list --verbose          # Verbose output
mediagit branch ls -v                   # Alias: ls for list
```

##### `mediagit branch create <NAME> [START_POINT]`
Create a new branch.

**Options:**
- `-u, --set-upstream <UPSTREAM>` - Set upstream branch
- `--track` - Track a remote branch
- `--no-track` - Don't track

**Example:**
```bash
mediagit branch create feature/new-ui
mediagit branch create hotfix/bug-123 main
mediagit branch create feature/api --track origin/feature/api
```

##### `mediagit branch switch <BRANCH>`
Switch to a branch.

**Options:**
- `-c, --create` - Create branch if it doesn't exist
- `-f, --force` - Force switch even with uncommitted changes

**Aliases:** `checkout`, `co`

**Example:**
```bash
mediagit branch switch main
mediagit branch switch feature/new-ui
mediagit branch switch -c feature/new          # Create and switch
mediagit branch checkout main                  # Using alias
mediagit branch co main                        # Short alias
```

##### `mediagit branch delete <BRANCHES>...`
Delete branches.

**Options:**
- `-D, --force` - Force delete
- `-d, --delete-merged` - Only delete if merged

**Aliases:** `rm`

**Example:**
```bash
mediagit branch delete feature/old
mediagit branch delete feature/a feature/b
mediagit branch delete -D feature/force-delete
mediagit branch rm feature/done                # Using alias
```

##### `mediagit branch protect <BRANCH>`
Protect a branch from deletion.

**Options:**
- `--require-reviews` - Require reviews before merge
- `--unprotect` - Remove protection

**Example:**
```bash
mediagit branch protect main
mediagit branch protect main --require-reviews
mediagit branch protect main --unprotect
```

##### `mediagit branch rename [OLD_NAME] <NEW_NAME>`
Rename a branch.

**Options:**
- `-f, --force` - Force rename

**Aliases:** `move`, `mv`

**Example:**
```bash
mediagit branch rename old-name new-name
mediagit branch rename new-name             # Rename current branch
mediagit branch mv old-name new-name        # Using alias
```

##### `mediagit branch show [BRANCH]`
Show branch information.

**Example:**
```bash
mediagit branch show                    # Show current branch info
mediagit branch show main               # Show specific branch info
```

##### `mediagit branch merge <BRANCH>`
Merge a branch into current branch.

**Options:**
- `--message <MSG>` - Merge message
- `--no-ff` - Create merge commit
- `--ff-only` - Fast-forward only

**Example:**
```bash
mediagit branch merge feature/new-ui
mediagit branch merge --no-ff feature/new-ui
```

### Merge Operations

#### `mediagit merge <BRANCH>`
Merge branches.

**Options:**
- `-m, --message <MSG>` - Merge message
- `--no-ff` - Create merge commit
- `--ff-only` - Fast-forward only
- `--squash` - Squash commits before merging
- `-s, --strategy <STRATEGY>` - Merge strategy
- `--no-commit` - Don't commit the merge
- `--abort` - Abort ongoing merge
- `--continue` - Continue after resolving conflicts

**Example:**
```bash
mediagit merge feature/new-ui
mediagit merge --no-ff feature/release     # Create merge commit
mediagit merge --squash feature/temp       # Squash commits
```

### Rebase Operations

#### `mediagit rebase <UPSTREAM> [BRANCH]`
Rebase commits onto new base.

**Options:**
- `-i, --interactive` - Interactive rebase
- `-m, --rebase-merges` - Rebase merge commits
- `--keep-empty` - Keep empty commits
- `--autosquash` - Automatically squash/fixup commits
- `--abort` - Abort rebase
- `--continue` - Continue after resolving conflicts
- `--skip` - Skip current commit

**Example:**
```bash
mediagit rebase main
mediagit rebase -i main                 # Interactive rebase
mediagit rebase --autosquash origin/main
```

### History and Inspection

#### `mediagit log [REVISION]`
Show commit history.

**Options:**
- `-n, --max-count <NUM>` - Maximum commits to show
- `--skip <NUM>` - Skip N commits
- `--oneline` - Abbreviated format
- `--graph` - Graph visualization
- `--stat` - Show statistics
- `-p, --patch` - Show patches
- `--author <PATTERN>` - Filter by author
- `--grep <PATTERN>` - Filter by message
- `--since <DATE>` - Show commits since date
- `--until <DATE>` - Show commits until date

**Example:**
```bash
mediagit log                            # Show all commits
mediagit log --oneline                  # Abbreviated view
mediagit log -5                         # Show last 5 commits
mediagit log --graph --oneline          # Graph visualization
mediagit log --author "John Doe"        # Filter by author
mediagit log main..feature              # Commits in feature not in main
```

#### `mediagit diff [FROM] [TO]`
Show changes between commits.

**Options:**
- `--cached` - Diff staged changes
- `--word-diff` - Show word-level differences
- `--stat` - Show statistics
- `-U, --unified <NUM>` - Context lines

**Example:**
```bash
mediagit diff                           # Diff working directory
mediagit diff --cached                  # Diff staged changes
mediagit diff main feature              # Diff between branches
mediagit diff --stat                    # Show statistics
```

#### `mediagit show <OBJECT>`
Show object information.

**Options:**
- `-p, --patch` - Show patch
- `--stat` - Show statistics
- `-U, --unified <NUM>` - Context lines

**Example:**
```bash
mediagit show HEAD                      # Show current commit
mediagit show main:file.txt             # Show file from commit
mediagit show abc1234                   # Show specific commit
```

### Repository Status

#### `mediagit status [OPTIONS]`
Show working tree status.

**Options:**
- `--tracked` - Show tracked files
- `--untracked` - Show untracked files
- `--ignored` - Show ignored files
- `-s, --short` - Short format
- `--porcelain` - Porcelain format (for scripts)
- `-b, --branch` - Show branch information
- `--ahead-behind` - Show ahead/behind commits

**Example:**
```bash
mediagit status                         # Full status
mediagit status --short                 # Short format
mediagit status -s                      # Short format (alias)
```

### Remote Operations

#### `mediagit push [REMOTE] [REFSPEC]...`
Push to remote repository.

**Options:**
- `--all` - Push all branches
- `--tags` - Push all tags
- `--follow-tags` - Follow tags
- `--dry-run` - Validate without pushing
- `-f, --force` - Force push (dangerous)
- `--force-with-lease` - Safe force push
- `-d, --delete` - Delete remote ref
- `-u, --set-upstream` - Set upstream branch

**Example:**
```bash
mediagit push                           # Push to origin
mediagit push origin main               # Push specific branch
mediagit push --all                     # Push all branches
mediagit push --tags                    # Push all tags
mediagit push -u origin feature/new     # Push and set upstream
```

#### `mediagit pull [REMOTE] [BRANCH]`
Fetch and integrate remote changes.

**Options:**
- `-r, --rebase` - Rebase instead of merge
- `-s, --strategy <STRATEGY>` - Merge strategy
- `--dry-run` - Validate without pulling
- `--no-commit` - Don't commit merge
- `--abort` - Abort pull
- `--continue` - Continue after conflicts

**Example:**
```bash
mediagit pull                           # Pull from origin
mediagit pull origin main               # Pull from remote branch
mediagit pull --rebase                  # Rebase instead of merge
mediagit pull --dry-run                 # Preview pull
```

### Maintenance

#### `mediagit gc [OPTIONS]`
Clean up and optimize repository.

**Options:**
- `--aggressive` - Aggressive optimization
- `--prune` - Prune unreachable objects
- `--auto` - Auto gc threshold
- `--dry-run` - Show what would be done

**Example:**
```bash
mediagit gc                             # Standard cleanup
mediagit gc --aggressive                # Aggressive optimization
mediagit gc --dry-run                   # Preview cleanup
```

#### `mediagit fsck [OPTIONS]`
Check repository integrity.

**Options:**
- `--full` - Check all objects
- `--strict` - Strict checks
- `--all` - Show all objects
- `--lost-found` - Show lost objects

**Example:**
```bash
mediagit fsck                           # Basic check
mediagit fsck --full                    # Complete check
mediagit fsck --strict                  # Strict validation
```

#### `mediagit verify [OPTIONS]`
Verify commits and signatures.

**Options:**
- `--signed-commits` - Verify signatures
- `--file-integrity` - Verify file integrity
- `--checksums` - Verify checksums
- `--quick` - Quick verification
- `--detailed` - Detailed report

**Example:**
```bash
mediagit verify                         # Basic verification
mediagit verify --detailed              # Detailed report
mediagit verify --signed-commits        # Verify signatures
```

#### `mediagit stats [OPTIONS]`
Show repository statistics.

**Options:**
- `--storage` - Storage statistics
- `--files` - File statistics
- `--commits` - Commit statistics
- `--branches` - Branch statistics
- `--authors` - Author statistics
- `--all` - All statistics
- `--json` - JSON format

**Example:**
```bash
mediagit stats                          # Basic stats
mediagit stats --all                    # All statistics
mediagit stats --json                   # JSON format
```

### Utility Commands

#### `mediagit version`
Show version information.

**Example:**
```bash
mediagit version                        # Show version
mediagit --version                      # Short form
mediagit -V                             # Very short form
```

#### `mediagit completions <SHELL>`
Generate shell completions.

**Supported shells:** bash, zsh, fish, powershell

**Example:**
```bash
mediagit completions bash               # Generate bash completions
mediagit completions bash >> ~/.bashrc  # Install bash completions
mediagit completions zsh > /usr/share/zsh/site-functions/_mediagit
```

## Tips and Tricks

### Using Colors
```bash
mediagit --color always                 # Always use colors
mediagit --color never                  # Disable colors
mediagit --color auto                   # Auto-detect (default)
```

### Verbose Output
```bash
mediagit -v init                        # Verbose initialization
mediagit --verbose push                 # Verbose push
```

### Quiet Mode
```bash
mediagit -q status                      # Suppress output
mediagit --quiet commit -m "msg"        # Quiet commit
```

### Repository Path
```bash
mediagit -C /path/to/repo status        # Run in specific repo
mediagit --repository /path/to/repo log # Using long form
```

### Command Aliases
```bash
mediagit branch ls                      # list alias
mediagit branch co                      # checkout alias
mediagit branch mv                      # move/rename alias
mediagit branch rm                      # delete alias
```

## Exit Codes

- `0` - Success
- `1` - General error
- `2` - Command-line usage error
- `128` - Repository not found

---

**Last Updated**: 2025-01-07
**Version**: 0.1.0

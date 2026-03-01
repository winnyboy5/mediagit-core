# CLI Reference

MediaGit command-line interface reference documentation.

## Command Categories

### Core Commands
Essential commands for daily workflows:
- [init](./init.md) - Initialize repository
- [add](./add.md) - Stage files
- [commit](./commit.md) - Create commits
- [status](./status.md) - Show working tree status
- [log](./log.md) - Show commit history
- [diff](./diff.md) - Show differences
- [show](./show.md) - Show object details
- [track](./track.md) - Configure file tracking patterns

### Branch Management
Working with branches:
- [branch](./branch.md) - Create, list, delete branches
- [merge](./merge.md) - Merge branches
- [rebase](./rebase.md) - Rebase branches
- [cherry-pick](./cherry-pick.md) - Apply commits from another branch
- [bisect](./bisect.md) - Binary search through commit history
- [stash](./stash.md) - Stash uncommitted changes
- [reset](./reset.md) - Reset current branch to a commit
- [revert](./revert.md) - Revert a commit by creating an inverse commit
- [tag](./tag.md) - Create and manage tags

### Remote Operations
Collaborating with remotes:
- [clone](./clone.md) - Clone a remote repository
- [remote](./remote.md) - Manage remote repositories
- [fetch](./fetch.md) - Fetch from remote without merging
- [push](./push.md) - Push changes to remote
- [pull](./pull.md) - Fetch and merge from remote

### History and Diagnostics
Inspecting repository state:
- [reflog](./reflog.md) - Show reference log (history of HEAD and branch movements)

### Maintenance
Repository maintenance:
- [gc](./gc.md) - Garbage collection
- [fsck](./fsck.md) - File system check
- [verify](./verify.md) - Verify object integrity
- [stats](./stats.md) - Repository statistics
- [filter](./filter.md) - Apply filter-driver transformations

### Setup and Installation
- [install](./install.md) - Install MediaGit hooks and filter drivers

## Global Options

Available for all commands:
- `-C <path>` - Run as if started in this directory
- `--help` - Show command help
- `--version` - Show MediaGit version
- `--verbose`, `-v` - Verbose output
- `--quiet`, `-q` - Suppress output
- `--color <when>` - Colorize output (auto/always/never)

## Environment Variables

- `MEDIAGIT_REPO` - Repository root path (set automatically by `-C`)
- `MEDIAGIT_AUTHOR_NAME` - Author name override
- `MEDIAGIT_AUTHOR_EMAIL` - Author email override

See [Environment Variables](../reference/environment.md) for complete list.

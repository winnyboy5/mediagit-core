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

### Branch Management
Working with branches:
- [branch](./branch.md) - Create, list, delete branches
- [merge](./merge.md) - Merge branches
- [rebase](./rebase.md) - Rebase branches

### Remote Operations
Collaborating with remotes:
- [push](./push.md) - Push changes
- [pull](./pull.md) - Pull changes

### Maintenance
Repository maintenance:
- [gc](./gc.md) - Garbage collection
- [fsck](./fsck.md) - File system check
- [verify](./verify.md) - Verify integrity
- [stats](./stats.md) - Repository statistics

## Global Options

Available for all commands:
- `--help` - Show command help
- `--version` - Show MediaGit version
- `--verbose` - Verbose output
- `--quiet` - Suppress output
- `--color <when>` - Colorize output (auto/always/never)

## Environment Variables

- `MEDIAGIT_DIR` - Repository directory (default: `.mediagit`)
- `MEDIAGIT_AUTHOR_NAME` - Author name override
- `MEDIAGIT_AUTHOR_EMAIL` - Author email override
- `MEDIAGIT_COMPRESSION` - Compression algorithm (zstd/brotli/none)

See [Environment Variables](../reference/environment.md) for complete list.

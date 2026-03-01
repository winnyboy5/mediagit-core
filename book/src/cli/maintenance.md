# Maintenance Commands

Repository maintenance and health check commands.

## Commands

- [gc](./gc.md) - Garbage collection and optimization
- [fsck](./fsck.md) - File system consistency check
- [verify](./verify.md) - Verify object integrity
- [stats](./stats.md) - Repository statistics
- [reflog](./reflog.md) - History of HEAD and branch movements
- [filter](./filter.md) - Git filter driver (clean/smudge) for migration use
- [install](./install.md) - Install MediaGit filter driver into a Git repository

## Recommended Schedule

```bash
# Weekly: Garbage collection
mediagit gc

# Monthly: Full verification
mediagit verify

# As needed: Check stats
mediagit stats
```

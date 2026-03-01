# Branch Management

Commands for working with branches.

## Commands

- [branch](./branch.md) - Create, list, delete, rename branches
- [merge](./merge.md) - Merge branches together
- [rebase](./rebase.md) - Rebase branch onto another
- [cherry-pick](./cherry-pick.md) - Apply specific commits from another branch
- [bisect](./bisect.md) - Binary search to find a regression-introducing commit
- [stash](./stash.md) - Save and restore uncommitted changes
- [reset](./reset.md) - Reset branch pointer to a commit
- [revert](./revert.md) - Undo a commit by creating an inverse commit
- [tag](./tag.md) - Create and manage tags

## Typical Workflow

```bash
# Create feature branch
mediagit branch feature-new-asset

# Switch to branch
mediagit branch feature-new-asset

# Work on feature...
mediagit add new-asset.psd
mediagit commit -m "Add new asset"

# Merge back to main
mediagit branch main
mediagit merge feature-new-asset
```

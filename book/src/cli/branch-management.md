# Branch Management

Commands for working with branches.

## Commands

- [branch](./branch.md) - Create, list, delete, rename branches
- [merge](./merge.md) - Merge branches together
- [rebase](./rebase.md) - Rebase branch onto another

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

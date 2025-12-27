# Core Commands

Essential commands for daily MediaGit workflows.

## Commands

- [init](./init.md) - Initialize a new repository
- [add](./add.md) - Stage files for commit
- [commit](./commit.md) - Create a commit from staged changes
- [status](./status.md) - Show working tree status
- [log](./log.md) - Show commit history
- [diff](./diff.md) - Show differences between versions
- [show](./show.md) - Show object details (commits, blobs, trees)

## Typical Workflow

```bash
# Initialize repository
mediagit init

# Stage files
mediagit add large-file.psd

# Create commit
mediagit commit -m "Add initial PSD file"

# Check status
mediagit status

# View history
mediagit log
```

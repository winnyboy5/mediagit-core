# Branch Model

MediaGit uses lightweight branches similar to Git.

## Branch Storage
Branches are files in `refs/heads/` containing commit hashes.

## Operations
- Create: `mediagit branch create <name>`
- Switch: `mediagit branch switch <name>` (or the `mediagit checkout <name>` shim)
- List: `mediagit branch list` (bare `mediagit branch` defaults to this)
- Delete: `mediagit branch delete <name>`
- Also: `rename`, `show`, `protect` (branch-scoped merge is not a thing — use `mediagit merge`)

## Branch Protection
A protected branch cannot be deleted with `mediagit branch delete` (`--force` does not override it; unprotect the branch first). Protection does not currently block force-push or merges.

See [CLI Reference - branch](../cli/branch.md) for details.

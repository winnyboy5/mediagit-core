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
Protected branches prevent force-push and deletion.

See [CLI Reference - branch](../cli/branch.md) for details.

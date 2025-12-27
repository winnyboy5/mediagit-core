# Branch Model

MediaGit uses lightweight branches similar to Git.

## Branch Storage
Branches are files in `refs/heads/` containing commit hashes.

## Operations
- Create: `mediagit branch <name>`
- Switch: `mediagit branch <name>`
- List: `mediagit branch --list`
- Delete: `mediagit branch --delete <name>`

## Branch Protection
Protected branches prevent force-push and deletion.

See [CLI Reference - branch](../cli/branch.md) for details.

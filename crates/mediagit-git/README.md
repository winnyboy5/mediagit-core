# mediagit-git

> **STATUS: FUTURE MILESTONE — NOT CURRENTLY ACTIVE**
>
> This crate is **not linked to the CLI** and is **not part of active development**.
> It is a planned tool for migrating content FROM existing git/git-lfs repositories INTO MediaGit.
> MediaGit is a standalone VCS — it does NOT wrap git, delegate to git, or require git at runtime.

---

## Purpose

`mediagit-git` will provide a one-way migration path: **git/git-lfs → MediaGit**.

When complete, it will allow teams with existing git-lfs repositories to migrate their media file
history into a native MediaGit repository, after which git is no longer required.

This crate is **not** a git filter driver for ongoing use. It is **not** a git compatibility layer.
MediaGit speaks its own protocol and object format — it does not sit on top of git.

---

## Planned Features (not yet implemented)

- Import git-lfs pointer files and resolve objects into MediaGit object storage
- Replay commit history into MediaGit commits/trees
- Validate migrated content with fsck
- CLI subcommand: `mediagit migrate from-git <path>`

---

## Current State

The crate contains early scaffolding (pointer file format, filter driver skeleton) written during
initial exploration of a git-filter-driver approach. That approach was **superseded** — MediaGit is
now a fully standalone VCS that does not integrate with git at runtime.

The code is retained as a starting point for the future migration tool milestone.

---

## Dependencies

- `git2` (0.20+): Will be used to read source git repository history during migration
- `sha2` (0.10+): SHA-256 hashing
- `serde` (1.0+): Serialization
- `tokio` (1.48+): Async runtime
- `thiserror` (2.0+): Error handling

---

## License

AGPL-3.0 — See LICENSE file for details.

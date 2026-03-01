# mediagit tag

Create and manage tags.

## Synopsis

```bash
mediagit tag create <NAME> [<COMMIT>] [-m <MESSAGE>]
mediagit tag list [--pattern <PATTERN>]
mediagit tag delete <NAME>
mediagit tag show <NAME>
```

## Description

Tags mark specific commits with meaningful names (e.g., release versions,
approved milestone snapshots). MediaGit stores tags as refs under
`.mediagit/refs/tags/`.

## Subcommands

### `create`

Create a new tag pointing to a commit.

```bash
mediagit tag create <NAME> [<COMMIT>] [-m <MESSAGE>]
```

Arguments:
- `NAME` — Tag name (e.g., `v1.0`, `release/2025-q1`)
- `COMMIT` — Commit to tag (default: `HEAD`)

Options:
- `-m`, `--message <MESSAGE>` — Annotated tag message

### `list`

List all tags.

```bash
mediagit tag list [--pattern <PATTERN>]
```

Aliases: `ls`

Options:
- `-p`, `--pattern <PATTERN>` — Filter by glob pattern (e.g., `v1.*`)
- `-v`, `--verbose` — Show tag messages and commit info

### `delete`

Delete a tag.

```bash
mediagit tag delete <NAME>
```

Aliases: `rm`

### `show`

Show tag details.

```bash
mediagit tag show <NAME>
```

## Examples

### Create a lightweight tag at HEAD

```bash
$ mediagit tag create v1.0
Tag 'v1.0' created at HEAD (abc1234d)
```

### Create an annotated tag with a message

```bash
$ mediagit tag create v2.0 -m "Q2 2025 approved asset set"
Tag 'v2.0' created: Q2 2025 approved asset set
```

### Tag a specific commit

```bash
$ mediagit tag create approved-2025-06 def5678
```

### List all tags

```bash
$ mediagit tag list
v1.0
v1.1
v2.0
approved-2025-06
```

### List tags matching a pattern

```bash
$ mediagit tag list --pattern "v*"
v1.0
v1.1
v2.0
```

### Show tag details

```bash
$ mediagit tag show v2.0
tag v2.0
Tagger: Alice Smith <alice@example.com>
Date:   Mon Jun 09 2025 14:30:00

Q2 2025 approved asset set

commit def5678...
```

### Delete a tag

```bash
$ mediagit tag delete v1.0
Deleted tag 'v1.0'
```

## Tag Naming Conventions

Recommended patterns:
- Version releases: `v1.0`, `v1.2.3`
- Quarterly approvals: `approved/2025-q2`
- Milestones: `milestone/alpha`, `milestone/beta`

Tag names may not contain spaces. Use `/` for namespacing.

## Exit Status

- **0**: Success
- **1**: Tag already exists (create) or tag not found (delete/show)

## See Also

- [mediagit log](./log.md) - View commit history with tag decorations
- [mediagit show](./show.md) - Show tag or commit details
- [mediagit branch](./branch.md) - Manage branches

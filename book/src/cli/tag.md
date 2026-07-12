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

There are two kinds of tag:

- **Lightweight** (`tag create <NAME>`) — the ref points directly at a commit.
- **Annotated** (`tag create -a -m <MSG>` or any of `-m`/`--tagger`/`--email`)
  — the ref points at a real `Tag` object in the object database: a target
  OID, tagger, message, and (optionally) a signature. This replaces the old
  `{tag_ref}.meta` sidecar file — annotated tags are now first-class objects
  that push/clone/fsck/gc all understand, not a ref plus a side file.

## Signing (`MEDIAGIT_SIGN`)

Annotated tags can be signed with your **existing OpenSSH ed25519 key**
(`~/.ssh/id_ed25519` — override with `MEDIAGIT_SIGN_KEY`). This is opt-in
(off by default) and reuses the SSH key you already manage for familiar
key-handling UX — but the signature itself is **MediaGit-native**: an
OpenSSH-armored `SshSig` blob stored inside the Tag object. It is not git's
tag-signing format, and git interoperability is not a design goal —
MediaGit is a standalone VCS.

```bash
MEDIAGIT_SIGN=1 mediagit tag create v1.0 -m "Q3 release"
mediagit tag verify v1.0
```

`tag verify` reports one of: `valid signature, signed by <fingerprint>`,
`INVALID signature — contents do not match signature` (exits non-zero), or
`unsigned`. Verification checks the signature against the signer's public
key **embedded in the signature itself**, so it works on any clone with no
local key configured. This is a trust-on-first-use model: "valid" proves
the tag contents are exactly what the reported key signed — whether you
trust the person holding that key is your call (MediaGit keeps no trust
store; compare the fingerprint out of band). Passphrase-protected keys are
detected and rejected with a clear error message; passphrase prompting is
not implemented yet — use an unencrypted key or point `MEDIAGIT_SIGN_KEY`
at one. MediaGit does not warn about lax key-file permissions the way ssh
does — protect your key with filesystem permissions.

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

### `verify`

Verify a tag reference, and — for annotated tags — its signature against
the signer key embedded in the signature (no local key needed; see the
trust-model note above). An INVALID signature exits non-zero.

```bash
mediagit tag verify <NAME>
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
Tag:     v2.0
Commit:  def5678...
Type:    annotated

Message:
Q2 2025 approved asset set

Tagger:  Alice Smith <alice@example.com>
Date:    2025-06-09 14:30:00 UTC
Signature: none (unsigned)
```

### Verify a signed tag

```bash
$ mediagit tag verify v2.0
Tag 'v2.0': valid signature, signed by SHA256:mVXBazcXfPRnLnDLNXkycrLLLQtu6efGZzMg9C6JAZI (contents intact; key ownership not verified)
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

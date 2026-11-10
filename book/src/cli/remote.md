# mediagit remote

Manage remote repository connections.

## Synopsis

```bash
mediagit remote add <NAME> <URL> [--fetch] [--token <TOKEN> | --token-stdin]
mediagit remote remove <NAME>
mediagit remote list [--verbose]
mediagit remote rename <OLD> <NEW>
mediagit remote set-url <NAME> <URL>
mediagit remote show <NAME>
mediagit remote ls-tree <REMOTE> [PATH] [--ref <REF>]
mediagit remote repos list   [--remote <NAME> | --server <URL>]
mediagit remote repos create [--remote <NAME> | --server <URL>] <NAME>
mediagit remote repos delete [--remote <NAME> | --server <URL>] [--force] [--yes] <NAME>
```

## Description

Manages the set of remotes whose branches you track. Remote URLs are stored in
`.mediagit/config.toml`.

## Subcommands

### `add`

Add a new remote.

```bash
mediagit remote add <NAME> <URL>
```

Options:
- `-f`, `--fetch` — Immediately fetch from the new remote after adding

### `remove`

Remove a remote and all its tracking refs.

```bash
mediagit remote remove <NAME>
```

Alias: `rm`

### `list`

List all configured remotes.

```bash
mediagit remote list [--verbose]
```

Options:
- `-v`, `--verbose` — Show URLs alongside remote names

### `rename`

Rename a remote.

```bash
mediagit remote rename <OLD-NAME> <NEW-NAME>
```

### `set-url`

Change the URL of an existing remote.

```bash
mediagit remote set-url <NAME> <NEW-URL>
```

### `show`

Show detailed information about a remote.

```bash
mediagit remote show <NAME>
```

### `ls-tree`

List one directory of a remote repository without cloning it. Uses the same
credentials as `fetch`.

```bash
mediagit remote ls-tree <REMOTE> [PATH] [--ref <REF>]
```

Options:
- `--ref <REF>` — Branch, tag or commit to list (default: the server's default branch)

Prints one line per entry: `<type>\t<size>\t<path>`, where `type` is `blob` or
`tree` and `size` is `-` for directories.

### `repos`

Administer the repositories a MediaGit server hosts. You need an administrator
account on that server. Sign in first with [`mediagit auth login`](./auth.md), or
set `MEDIAGIT_TOKEN` or `MEDIAGIT_API_KEY`.

```bash
mediagit remote repos list
mediagit remote repos create <NAME>
mediagit remote repos delete <NAME>
```

Which server is used:
- `--remote <NAME>` — Use the server of this remote (default: `origin`)
- `--server <URL>` — Use this server address instead (`scheme://host[:port]`); overrides `--remote`

`list` prints one line per repository: name, size in bytes and last-modified time
(Unix seconds). `create` makes an empty repository; if the server has a default
storage configured (the `default_storage` setting) it uses that, otherwise it
keeps the repository in its own folder on the server.

`delete` unregisters a repository and cannot be undone. It asks you to confirm,
or you can pass `--yes`, which is required when there is no terminal and under
`--machine`.

Options for `delete`:
- `--force` — Delete even if the repository is not empty
- `--yes` — Confirm the deletion without asking

If the server keeps repositories in a cloud bucket, deleting removes the
repository from the server but leaves its data in the bucket, and the command
warns you; remove the data in the bucket yourself to reclaim the space.

## Examples

### Add a remote

```bash
$ mediagit remote add origin http://media-server.example.com/my-project
```

### Add a remote and fetch immediately

```bash
$ mediagit remote add upstream http://media-server.example.com/upstream --fetch
```

### List remotes

```bash
$ mediagit remote list
origin
upstream

$ mediagit remote list --verbose
origin    http://media-server.example.com/my-project (fetch)
origin    http://media-server.example.com/my-project (push)
upstream  http://media-server.example.com/upstream   (fetch)
upstream  http://media-server.example.com/upstream   (push)
```

### Rename a remote

```bash
$ mediagit remote rename origin production
```

### Change URL

```bash
$ mediagit remote set-url origin https://new-server.example.com/my-project
```

### Remove a remote

```bash
$ mediagit remote remove old-server
```

## Configuration

Remotes are stored in `.mediagit/config.toml`:

```toml
[remotes.origin]
url = "http://media-server.example.com/my-project"

[remotes.backup]
url = "http://backup-server.example.com/my-project"
```

## Exit Status

- **0**: Success
- **1**: Remote not found or URL invalid


## Subcommand options

`add`:
- `-f`, `--fetch` — Fetch immediately after adding the remote
- `--token <TOKEN>` — Bearer token for the remote. It is stored in the OS
  keychain, keyed by the server address, and `config.toml` keeps no copy. If no
  keychain can be used (none on this machine, or `MEDIAGIT_NO_KEYRING` is set),
  the command is refused with the error `keychain_unavailable` and nothing is
  saved, instead of writing the token in plain text. Set `MEDIAGIT_TOKEN`
  instead on such machines.
- `--token-stdin` — Read the token from standard input instead of the command
  line, so it never appears in a process list. Needs `--machine --secret-stdin`
  (this is how the desktop app adds a token remote). Cannot be combined with
  `--token`. Under `--machine`, `--token` is refused with `invalid_args`.

The environment variables `MEDIAGIT_TOKEN` and `MEDIAGIT_API_KEY` outrank a saved
token, and credentials taken from them are never copied into the keychain.

`set-url`:
- `--push` — Set the push URL instead of the fetch URL


## Common options

Accepted by this command in addition to the options above.

| Flag | Description |
|---|---|
| `--color <WHEN>` | Colored output: `always`, `auto`, or `never` (default `auto`) |
| `-C`, `--repository <PATH>` | Run as if invoked in `PATH` |
| `-q`, `--quiet` | Suppress output |
| `-h`, `--help` | Print help |
| `-V`, `--version` | Print version |
## See Also

- [mediagit fetch](./fetch.md) - Fetch from a remote
- [mediagit push](./push.md) - Push to a remote
- [mediagit pull](./pull.md) - Fetch and merge from remote
- [mediagit clone](./clone.md) - Clone a remote repository

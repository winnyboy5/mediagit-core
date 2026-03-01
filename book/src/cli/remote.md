# mediagit remote

Manage remote repository connections.

## Synopsis

```bash
mediagit remote add <NAME> <URL> [--fetch]
mediagit remote remove <NAME>
mediagit remote list [--verbose]
mediagit remote rename <OLD> <NEW>
mediagit remote set-url <NAME> <URL>
mediagit remote show <NAME>
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

## See Also

- [mediagit fetch](./fetch.md) - Fetch from a remote
- [mediagit push](./push.md) - Push to a remote
- [mediagit pull](./pull.md) - Fetch and merge from remote
- [mediagit clone](./clone.md) - Clone a remote repository

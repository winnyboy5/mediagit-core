# mediagit download

Download a single file from a remote repository by path.

## Synopsis

```bash
mediagit download <REMOTE_PATH> [OPTIONS]
```

## Description

Streams one file straight to disk via a plain GET against the server's
file-browse endpoint (`GET /{repo}/files/{path}`) — not a full clone or pack
transfer. Unlike every other remote command, `download` works **without a
local repository** when given a full URL: this is the point, since it lets
CI pipelines, build scripts, and one-off asset pulls fetch a single file
without cloning the whole repo.

Two ways to invoke it:

- **Full URL** (no local repository required): the first path segment after
  the host is treated as the repository name, and everything after it is the
  file path within that repository.
- **Repo-relative path** (run inside a MediaGit repository): resolved
  against the repository's `origin` remote.

## Arguments

#### `<REMOTE_PATH>`
Either a full URL (`scheme://host[:port]/<repo>/<path...>`) or, when run
inside a repository, a path relative to that repository's root.

## Options

#### `--ref <REF>`
Branch, tag, or commit OID to download the file from. Defaults to the
server's advertised `main`, else `master`, else its first branch — not the
literal string `HEAD`, since a repository populated purely via `push` has no
server-side `HEAD` ref for the browse endpoint to resolve.

#### `-o`, `--output <PATH>`
Output file path (default: the file's base name, written to the current
directory).

#### `-q`, `--quiet`
Suppress output.

## Examples

### Download by full URL — no local repository needed

```bash
$ mediagit download http://media-server.example.com/my-project/textures/rock.png
✔ Downloaded 'textures/rock.png' (2145839 bytes) to rock.png
```

### Download a specific ref

```bash
$ mediagit download http://media-server.example.com/my-project/scene.blend --ref v1.2
```

### Download to a specific path

```bash
$ mediagit download http://media-server.example.com/my-project/scene.blend -o /tmp/scene.blend
```

### From inside a repository, relative to `origin`

```bash
$ cd my-project
$ mediagit download textures/rock.png
```

## Authentication

In repo-relative mode, `download` resolves credentials exactly like
`push`/`pull`/`fetch`/`clone`: the `MEDIAGIT_TOKEN` / `MEDIAGIT_API_KEY`
environment variables first, then the OS keychain (skipped when
`MEDIAGIT_NO_KEYRING` is set), then per-remote `config.toml`
(`remotes.origin.token` / `.api_key`), then none.

In full-URL mode, credentials are attached **only if the typed URL's host
matches one of the current repository's configured remotes** (scheme, host,
and effective port). If there's no local repository, or none of its
remotes' hosts match the typed URL, no credentials are attached at all —
`MEDIAGIT_TOKEN` / `MEDIAGIT_API_KEY` are never sent to a host that isn't
one of your own configured remotes. A notice is printed when this strips a
credential that would otherwise have been sent.

## Security

Remote file paths are rejected client-side if they contain a `..`
path-traversal component or a backslash, in addition to whatever validation
the server performs.

## Exit Status

- **0**: Success
- **1**: Network error, remote/file not found, or invalid path

## See Also

- [mediagit clone](./clone.md) - Clone a full repository
- [mediagit pull](./pull.md) - Fetch and merge into a local repository
- [mediagit fetch](./fetch.md) - Fetch remote changes without merging

# mediagit config

Get and set repository configuration.

## Synopsis

```bash
mediagit config get <KEY>
mediagit config set <KEY> [VALUE]
mediagit config unset <KEY>
mediagit config list
mediagit config schema
```

## Description

`config` reads and writes the settings stored in the current repository's
`.mediagit/config.toml`. It is deliberately **not** a general TOML editor: it
accepts a closed set of keys and rejects anything else.

That refusal is the point. Silently accepting a mistyped key such as
`auther.name` would leave you believing an identity was configured when it was
not, and the next `commit` would refuse for a reason that looked unrelated. An
unrecognised key is treated as a typo, and the error lists every key that is
supported.

Edits are made in place: comments and keys the command does not manage are kept,
the new value is checked against the real configuration before anything is
written, and the file is replaced atomically. All keys are optional; unset keys
fall back to the built-in defaults.

Run `mediagit config schema` to see every key with its type, default,
description, allowed range and whether it is a secret. The desktop app builds
its Settings page from the same list.

## Settable keys

| Key | Meaning |
|---|---|
| `author.name` | Name recorded on commits you create |
| `author.email` | Email recorded on commits you create (must look like an address) |
| `performance.upload_concurrency` | Concurrent uploads during push (1 or more) |
| `performance.download_concurrency` | Concurrent downloads during pull and fetch (1 or more) |
| `performance.pack_workers` | Pack-write workers used when this repository is served |
| `storage.create_dirs` | Filesystem backend: create missing directories (default `true`) |
| `storage.sync` | Filesystem backend: sync writes to disk, slower but safer (default `false`) |
| `storage.file_permissions` | Filesystem backend: octal permissions for stored files (default `0644`) |
| `storage.region` | S3 backend: bucket region |
| `storage.endpoint` | S3 backend: custom endpoint for MinIO and other S3-compatible services |
| `storage.access_key_id` | S3 backend: access key ID (secret) |
| `storage.secret_access_key` | S3 backend: secret access key (secret) |
| `storage.credentials_path` | GCS backend: service-account JSON file; unset uses Application Default Credentials |
| `remotes.<name>.url` | URL of the remote called `<name>` |
| `remotes.<name>.fetch` | Fetch URL, when it differs from the URL |
| `remotes.<name>.push` | Push URL, when it differs from the URL |
| `remotes.<name>.token` | Bearer token for the remote (secret) |
| `remotes.<name>.api_key` | API key for the remote (secret) |

`author.name` and `author.email` are the two `commit` requires; without them (or
the equivalent command-line flag) it refuses rather than recording an empty
identity.

Storage identity (the backend type, bucket, base path and prefix) and the
repository's own identity fields are left out on purpose, because changing them
would orphan stored objects. Branch tracking and branch protection are managed
by `branch` and `pull`.

The `performance.*` upload and download keys are honoured by `push`, `pull` and
`fetch`. `clone` does not read them: a clone has no repository configuration
until it has finished creating one, so use the environment variables described
in [Environment Variables](../reference/environment.md) to tune a clone.

## Secrets

Keys marked secret are never printed by `get` or `list`. To set one without
putting it on the command line, leave the value out. In a terminal you are
prompted for it; from a program, add the global `--machine --secret-stdin` flags
and write the value as one line on standard input:

```bash
echo "the-secret" | mediagit --machine --secret-stdin config set storage.secret_access_key
```

Under `--machine`, passing a secret as an argument is refused with
`invalid_args`. Note that these particular keys are stored in plain text in
`config.toml`. For a remote's token, prefer
adding the remote with a token (see [remote](./remote.md)), which keeps it in the OS keychain.

## Subcommands

| Subcommand | Arguments | Description |
|---|---|---|
| `get` | `<KEY>` | Print a single value; exits non-zero if the key is not set |
| `set` | `<KEY> [VALUE]` | Set a value (omit `VALUE` for a secret) |
| `unset` | `<KEY>` | Remove a value, restoring the default |
| `list` | none | List every settable key and its current value |
| `schema` | none | Describe every settable key: type, default, documentation |

## Options

| Flag | Description |
|---|---|
| `-v`, `--verbose` | Enable verbose output |
| `-q`, `--quiet` | Suppress output |
| `--color <WHEN>` | Colored output: `always`, `auto`, or `never` (default `auto`) |
| `-C`, `--repository <PATH>` | Run as if invoked in `PATH` |
| `--machine` | One JSON result on stdout, for programs and the desktop app |
| `--secret-stdin` | With `--machine`: read a secret value from stdin |
| `-h`, `--help` | Print help |
| `-V`, `--version` | Print version |

## Examples

### Set the identity commits are recorded under

```bash
mediagit config set author.name "Your Name"
mediagit config set author.email you@example.com
```

### Read one value, or list everything settable

```bash
mediagit config get author.email
mediagit config list
```

### See every key, its type and its default

```bash
mediagit config schema
```

### Remove a setting, restoring the default

```bash
mediagit config unset performance.upload_concurrency
```

## See also

- [commit](./commit.md): requires an author identity
- [remote](./remote.md): add remotes and store their tokens
- [Configuration](../reference/config.md): the full `config.toml` schema,
  including the many settings this command does not manage
- [Environment Variables](../reference/environment.md): runtime tuning knobs

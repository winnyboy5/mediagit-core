# mediagit config

Get and set repository configuration.

## Synopsis

```bash
mediagit config get <KEY>
mediagit config set <KEY> <VALUE>
mediagit config unset <KEY>
mediagit config list
```

## Description

`config` reads and writes the settings stored in the current repository's
`.mediagit/config.toml`. It is deliberately **not** a general TOML editor: it
accepts a small, closed set of keys and rejects anything else.

That refusal is the point. Silently accepting a mistyped key such as
`auther.name` would leave you believing an identity was configured when it was
not — and the next `commit` would refuse for a reason that looked unrelated. An
unrecognised key is treated as a typo, and the error lists every key that is
supported.

All four keys are optional. Unset keys fall back to the built-in defaults.

## Settable keys

| Key | Meaning |
|---|---|
| `author.name` | Name recorded on commits you create |
| `author.email` | Email recorded on commits you create |
| `performance.upload_concurrency` | Concurrent uploads during push |
| `performance.download_concurrency` | Concurrent downloads during pull and fetch |

`author.name` and `author.email` are the two `commit` requires; without them (or
the equivalent command-line flag) it refuses rather than recording an empty
identity.

The two `performance.*` keys are honoured by `pull` and `fetch`. Note that
`clone` does not read them — a clone has no repository configuration to read
until it has finished creating one, so use the environment variables described
in [Environment Variables](../reference/environment.md) to tune a clone.

## Subcommands

| Subcommand | Arguments | Description |
|---|---|---|
| `get` | `<KEY>` | Print a single value |
| `set` | `<KEY> <VALUE>` | Set a value |
| `unset` | `<KEY>` | Remove a value, restoring the default |
| `list` | — | List every settable key and its current value |

## Options

| Flag | Description |
|---|---|
| `-v`, `--verbose` | Enable verbose output |
| `-q`, `--quiet` | Suppress output |
| `--color <WHEN>` | Colored output: `always`, `auto`, or `never` (default `auto`) |
| `-C`, `--repository <PATH>` | Run as if invoked in `PATH` |
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

### Remove a setting, restoring the default

```bash
mediagit config unset performance.upload_concurrency
```

## See also

- [commit](./commit.md) — requires an author identity
- [Configuration](../reference/config.md) — the full `config.toml` schema,
  including the many settings this command does not manage
- [Environment Variables](../reference/environment.md) — runtime tuning knobs

# Remote Operations

Commands for working with remote repositories.

## Commands

- [clone](./clone.md) - Clone a remote repository locally
- [remote](./remote.md) - Add, remove, and list remote connections
- [fetch](./fetch.md) - Download objects from remote without merging
- [push](./push.md) - Push commits to remote
- [pull](./pull.md) - Fetch and merge from remote
- [download](./download.md) - Download a single file by path, without cloning

## Authentication

Remote commands (`push`, `pull`, `fetch`, `clone`, `download`) support optional
client authentication. Credentials are resolved in this order (highest to lowest
precedence):

1. **Per-remote config** — `remotes.<name>.token` or `remotes.<name>.api_key` in
   `.mediagit/config.toml`
2. **Environment variables** — `MEDIAGIT_TOKEN` or `MEDIAGIT_API_KEY`
3. **None** — no authentication header sent (compatible with authless servers)

If both `token` and `api_key` are set, `token` (Bearer) wins.

`download` is the one command that can run without a local repository, by
taking a full URL. In that full-URL mode, credentials (per-remote config or
environment variables) are attached only if the typed URL's host matches
one of the current repository's configured remotes — if there's no local
repository, or no remote's host matches, no credentials are sent at all.
This prevents a token from `MEDIAGIT_TOKEN` / `MEDIAGIT_API_KEY` leaking to
an arbitrary host the user happened to type on the command line.

The server verifies credentials via its own auth middleware and may reject
unauthenticated requests if auth is enabled.

## Typical Workflow

```bash
# Push to remote
mediagit push origin main

# Pull from remote
mediagit pull origin main

# With authentication (per-remote config)
# remotes.origin.token = "my-bearer-token"
mediagit pull origin main

# Or via environment variable
MEDIAGIT_TOKEN=my-bearer-token mediagit pull origin main
```

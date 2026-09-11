# mediagit bisect

Find the commit that introduced a problem using binary search.

## Synopsis

```bash
mediagit bisect start [<BAD>] [<GOOD>]
mediagit bisect good [<COMMIT>]
mediagit bisect bad [<COMMIT>]
mediagit bisect skip [<COMMIT>]
mediagit bisect reset [<COMMIT>]
mediagit bisect log
mediagit bisect replay <LOGFILE>
```

## Description

Performs a binary search through commit history to efficiently find the commit
that introduced a regression. Given a "bad" (broken) commit and a "good"
(working) commit, MediaGit checks out the midpoint and asks you to test it.
After O(log N) iterations, it identifies the first bad commit.

Short commit hashes (from `mediagit log --oneline`) are supported.

## Subcommands

### `start`

Begin a bisect session.

```bash
mediagit bisect start [<BAD>] [<GOOD>]
```

- `BAD` — Known-broken commit (default: `HEAD`)
- `GOOD` — Known-working commit (must provide during session)

### `good`

Mark the current (or specified) commit as working.

```bash
mediagit bisect good [<COMMIT>]
```

### `bad`

Mark the current (or specified) commit as broken.

```bash
mediagit bisect bad [<COMMIT>]
```

### `skip`

Skip a commit that cannot be tested (e.g., does not compile).

```bash
mediagit bisect skip [<COMMIT>]
```

### `reset`

End the bisect session and return to the original branch.

```bash
mediagit bisect reset [<COMMIT>]
```

### `log`

Show the bisect session log (good/bad/skip decisions made so far).

```bash
mediagit bisect log
```

### `replay`

Replay a bisect session from a log file.

```bash
mediagit bisect replay <LOGFILE>
```

## Examples

### Full bisect session

```bash
# Start: HEAD is broken, commit from 2 weeks ago was good
$ mediagit bisect start
$ mediagit bisect bad HEAD
$ mediagit bisect good abc1234

Bisecting: 7 revisions left to test (about 3 steps)
[def5678] Add dynamic lighting rig

# Test the checked-out commit, then mark it:
$ mediagit bisect good   # if this revision works
# or:
$ mediagit bisect bad    # if this revision is broken

Bisecting: 3 revisions left (about 2 steps)
...
# After a few more iterations:
abc1234 is the first bad commit
```

### Start with explicit commits

```bash
$ mediagit bisect start HEAD~20 HEAD~5
# HEAD~5 is bad, HEAD~20 is good
```

### Skip untestable commits

```bash
$ mediagit bisect skip
```

### View decisions so far

```bash
$ mediagit bisect log
good: abc1234 (Initial asset set)
bad:  def5678 (Reprocess render outputs)
skip: ghi9012 (Corrupt file, untestable)
bad:  jkl3456 (Add V2 textures)
```

### Save and replay a bisect session

```bash
$ mediagit bisect log > bisect.log
# Later:
$ mediagit bisect replay bisect.log
```

### End the session

```bash
$ mediagit bisect reset
Returned to branch 'main'
```

## Session State

Bisect state is stored in a single file, `.mediagit/BISECT_STATE`, written as
pretty-printed JSON (`commands/bisect.rs:595`). `mediagit bisect reset` deletes
it (`bisect.rs:320`).

This section previously named `.mediagit/BISECT_HEAD`, `.mediagit/BISECT_LOG`
and `.mediagit/BISECT_TERMS` — those are **git's** bisect state files, and none
of them exists anywhere in this codebase. The subcommands above are real; only
the on-disk layout was borrowed from git.

## Exit Status

- **0**: Success / first bad commit found
- **1**: No active bisect session or commit not found


## Subcommand options

`start`:
- `--reset` — Reset an existing bisect session before starting a new one

## Common options

Accepted by this command in addition to the options above.

| Flag | Description |
|---|---|
| `--color <WHEN>` | Colored output: `always`, `auto`, or `never` (default `auto`) |
| `-C`, `--repository <PATH>` | Run as if invoked in `PATH` |
| `-q`, `--quiet` | Suppress output |
| `-v`, `--verbose` | Enable verbose output |
| `-h`, `--help` | Print help |
| `-V`, `--version` | Print version |
## See Also

- [mediagit log](./log.md) - Browse commit history
- [mediagit reflog](./reflog.md) - Track HEAD movements during bisect
- [mediagit revert](./revert.md) - Undo the identified bad commit

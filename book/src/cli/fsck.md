# mediagit fsck

Verify integrity and connectivity of objects in repository.

## Synopsis

```bash
mediagit fsck [OPTIONS]
```

## Description

Verifies the connectivity and validity of objects in the repository database. Checks for:
- Corrupt or missing objects (BLAKE3 checksum mismatches)
- Invalid or unreadable references
- Commit graph connectivity issues
- Dangling (unreferenced) objects
- Broken or excessively deep chunk-delta chains

By default fsck checks objects, refs, connectivity, and chunk-delta chains, but
skips the dangling-object scan (it's the most expensive pass). `--full` adds
the dangling-object scan; `--quick` checks only objects and refs, skipping
connectivity and chunk-delta checks.

## Options

| Flag | Description |
|---|---|
| `--full` | Full check including dangling/unreachable objects (slower) |
| `--quick` | Quick check (objects and refs only, no connectivity) |
| `--all` | Show all objects checked |
| `--lost-found` | Show lost/dangling objects |
| `--no-dangling` | Don't check for dangling objects |
| `--repair` | Repair damaged and missing files (restore, or quarantine; never delete) |
| `--dry-run` | Dry run (show what would be repaired without making changes) |
| `--purge-quarantine` | Delete every copy in `.mediagit/quarantine` (asks first; runs no check and no repair) |
| `--yes` | Do not ask for confirmation (with `--purge-quarantine`; required with `--machine`) |
| `--max-objects <N>` | Limit number of objects to check (0 = unlimited) [default: 0] |
| `-q`, `--quiet` | Quiet mode (only errors) |
| `-v`, `--verbose` | Verbose mode (detailed progress) |
| `--path <PATH>` | Repository path (defaults to current directory) |
| `--color <WHEN>` | Colored output: `always`, `auto`, or `never` (default: `auto`) |
| `-C`, `--repository <PATH>` | Repository path |
| `-V`, `--version` | Print version |

## Examples

### Basic fsck

```bash
$ mediagit fsck
🔍 Checking repository integrity at .mediagit

📊 Statistics:
  • Objects checked: 156
  • References checked: 6

✅ Repository integrity: PERFECT
```

### Full check including dangling objects

```bash
$ mediagit fsck --full
🔍 Checking repository integrity at .mediagit

📊 Statistics:
  • Objects checked: 156
  • References checked: 6

ℹ Information:
  • Dangling blob b4d7e1a9 (not reachable from any ref)

✓ Repository integrity: OK (0 warning(s), 1 info)
```

### Quick check

```bash
$ mediagit fsck --quick
🔍 Checking repository integrity at .mediagit

📊 Statistics:
  • Objects checked: 156
  • References checked: 6

✅ Repository integrity: PERFECT
```

### Verbose output

`-v` prints the check configuration up front, and adds the OID/ref and
`(Repairable with --repair)` marker to each reported issue:

```bash
$ mediagit fsck -v
🔍 Checking repository integrity at .mediagit
⚙ Configuration:
  • Check objects: true
  • Check references: true
  • Check connectivity: true
  • Check dangling: false

📊 Statistics:
  • Objects checked: 156
  • References checked: 6

✅ Repository integrity: PERFECT
```

### Repair mode

```bash
$ mediagit fsck --repair --dry-run
🔍 Checking repository integrity at .mediagit
...
ℹ [DRY RUN] Would repair 2 of 2 issue(s)

$ mediagit fsck --repair
🔍 Checking repository integrity at .mediagit
...
🔧 Attempting to repair 2 issue(s)...
✅ Successfully repaired 2 issue(s)
```

### How repair works

Repair never deletes a damaged object. For each damaged or missing file it
looks for a good copy, in this order:

1. The file in your working folder, if it still matches the saved version.
2. A pack file in the repository that holds an intact copy.
3. The remote (`origin`, or the only remote), if one is configured.

If none has a good copy, the damaged bytes are moved to
`.mediagit/quarantine/<kind>/<object id>`. The object is then missing, so
`fsck` stays red and exits non-zero, and the output lists the files that have
a saved version that can no longer be restored. The file in your working
folder may be fine; it is the saved version that is lost:

```bash
✖ 1 file(s) have saved versions that can no longer be restored:
    photo.bin
```

The quarantine folder is always on the local disk. With a bucket as the
object store (S3, GCS, Azure), a damaged object leaves the bucket and its
bytes are kept locally, next to your working folder.

If the remote cannot help, fsck says why in one line: no remote is
configured, the server could not be reached, the login was refused, or the
server did not have a good copy.

After repair, fsck checks again and reports only what the second check
confirms. The exit code describes the repository as it is afterwards.

Broken branch and tag references are removed, after their value is saved to
`.mediagit/quarantine/refs/<name>`. `HEAD` and remote-tracking references
(`refs/remotes/*`) are never removed: a lazy clone has tracking references for
branches it has not downloaded yet, so run `fetch` for those. Dangling objects are
not repaired: `mediagit gc` owns them.

### The quarantine folder

Every `fsck` run (not `--dry-run`) deletes a quarantined copy when its object
reads back correctly in that same run, and a saved reference when a reference
of the same name now holds the same commit and that commit reads. Nothing else
is deleted automatically. `fsck` prints the number of entries (copies and saved
references) and their size when the folder is not empty. `gc` never touches the
folder.

A reference that repair removed comes back when the remote delivers its
commit.

`mediagit fsck --purge-quarantine` deletes every copy. It asks first. Without
a terminal it refuses unless you pass `--yes`. With `--machine` it always
needs `--yes`. It prints how many copies and bytes it deleted and repeats the
files that can no longer be restored. It runs no repair. With `--dry-run` it
only lists what it would delete. It deletes only the files it listed, never
other files you put in the folder.

### Machine output

`mediagit --machine fsck` adds `quarantine_count` and `quarantine_bytes`.
After `--repair` it also reports `restored`, `quarantined`, `refs_removed`,
`unrestorable_files` and `repair_errors`. `--purge-quarantine` reports `deleted_count`,
`deleted_bytes` and `unrestorable_files`.

### Errors found

```bash
$ mediagit fsck
🔍 Checking repository integrity at .mediagit

❌ Errors found:
  • Missing object referenced by tree d6f0a3c1: blob c5e9f2b4

📊 Statistics:
  • Objects checked: 156
  • References checked: 6

✗ Repository integrity: FAILED (1 error(s), 0 warning(s))

💡 1 issue(s) can be repaired with: mediagit fsck --repair
```

fsck exits non-zero whenever errors are found, even with `--quiet`.

## Built-in Help

`mediagit fsck --help` includes this additional guidance verbatim:

```
EXAMPLES:
    # Basic integrity check
    mediagit fsck

    # Full check including dangling objects
    mediagit fsck --full

    # Quick check (objects and refs only)
    mediagit fsck --quick

    # Repair mode (fix repairable issues)
    mediagit fsck --repair

    # Dry-run repair to see what would be fixed
    mediagit fsck --repair --dry-run

FSCK vs VERIFY:
    fsck    - Comprehensive integrity check (full graph analysis)
            - Checks connectivity, finds dangling/unreachable objects
            - Supports repair mode (--repair)
            - Use for thorough repository audits and recovery

    verify  - Fast integrity check (checksums + refs only)
            - Skips connectivity and dangling object checks
            - Use for quick health checks and CI pipelines

SEE ALSO:
    mediagit-verify(1), mediagit-gc(1)
```

## Exit Status

- **0**: No errors found (warnings/info do not fail the command)
- non-zero: Integrity errors found, files remain that can no longer be restored, a purge was not confirmed, or fsck failed to run (e.g. not a repository)

A new repository with no versions yet is healthy: `HEAD` naming a branch that
has no commit yet is not reported.

## See Also

- [mediagit gc](./gc.md) - Garbage collection and optimization
- [mediagit verify](./verify.md) - Verify commits and signatures
- [mediagit stats](./stats.md) - Repository statistics

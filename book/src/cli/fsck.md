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
| `--repair` | Attempt to repair issues automatically |
| `--dry-run` | Dry run (show what would be repaired without making changes) |
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

If none of the repairable issues could actually be fixed (for example, a
packed object that only a targeted delete can't reach), fsck reports that
explicitly instead of a false "0 issue(s) repaired" success line:

```bash
✖ Repaired 0 of 2 issue(s) — no repair succeeded. See the warnings above for why each was skipped.
```

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
- non-zero: Integrity errors found, or fsck failed to run (e.g. not a repository)

## See Also

- [mediagit gc](./gc.md) - Garbage collection and optimization
- [mediagit verify](./verify.md) - Verify commits and signatures
- [mediagit stats](./stats.md) - Repository statistics

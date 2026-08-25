# mediagit verify

Verify integrity of specific objects or files.

## Synopsis

```bash
mediagit verify [COMMIT]
mediagit verify [OPTIONS] --path <file>...
```

## Description

Verifies the integrity of a commit and its reachable objects, or specific files. Unlike `fsck`
which checks the entire repository, `verify` can target a single commit (OID, abbreviated hash,
branch name, or `HEAD`) or individual files for quick validation.

Useful for:
- Verifying specific media files after download
- Checking object integrity before important operations
- Validating chunks after recovery
- Confirming successful transfers
- Quick integrity spot-checks

## Options

#### `[COMMIT]`
Verify a specific commit and its reachable objects. Accepts a full or abbreviated BLAKE3 OID, branch name, or `HEAD`. Omit to verify the whole repository.

#### `--start <COMMIT>`
Start of a commit range to verify (verify commits from this point forward).

#### `--end <COMMIT>`
End of a commit range to verify (inclusive). Defaults to HEAD when `--start` is given.

#### `--file-integrity`
Verify file checksums (BLAKE3) for all stored objects.

#### `--checksums`
Verify all stored object checksums.

#### `--quick`
Minimal checks only (skip connectivity analysis).

#### `--detailed`
Show a full statistics report and per-object results.

#### `--path <PATH>`
Repository path to verify (defaults to current directory).

#### `-v`, `--verbose`
Show per-commit verification results.

#### `-q`, `--quiet`
Suppress output except errors.

#### `--color <WHEN>`
Colored output: `always`, `auto`, or `never`. Default: `auto`. Global option, shared by every
`mediagit` subcommand.

#### `-C`, `--repository <PATH>`
Run as if `verify` was started in `<PATH>` instead of the current directory. Global option,
shared by every `mediagit` subcommand. Distinct from `--path` above, which is `verify`'s own
(pre-existing) repository-path option.

#### `-h`, `--help`
Print help for `verify` and exit.

#### `-V`, `--version`
Print the `mediagit` version and exit.

## Examples

### Verify specific object

```bash
$ mediagit verify a3c8f9d
✔ Verifying repository integrity...
  Range: (root) → a3c8f9d
  Commits to verify: 1

✅ Verified 1 commits in range - all OK
```

### Verify HEAD (default)

```bash
$ mediagit verify
✔ Verifying repository integrity...
✅ All verifications passed
```

### Verify a commit range

```bash
$ mediagit verify --start abc1234 --end def5678
✔ Verifying repository integrity...
  Range: abc1234 → def5678
  Commits to verify: 5
✅ Verified 5 commits in range - all OK
```

### Verbose output

```bash
$ mediagit verify -v HEAD
✔ Verifying repository integrity...

📊 Verification Statistics:
  • Objects verified: 247
  • References verified: 3

✅ All verifications passed
```

### Detailed report

```bash
$ mediagit verify --detailed
✔ Verifying repository integrity...

📊 Verification Statistics:
  • Objects verified: 1,482
  • References verified: 8

✅ All verifications passed
```

### Quick mode (minimal checks)

```bash
$ mediagit verify --quick
✔ Verifying repository integrity...
✅ All verifications passed
```

### Quiet mode (exit code only)

```bash
$ mediagit verify --quiet
$ echo $?
0  # 0 = all valid, 1 = errors found
```

## Error Detection

### Corrupt Object

```bash
$ mediagit verify --detailed
✔ Verifying repository integrity...

❌ Verification Errors:
  • Object b4d7e1a9... checksum mismatch
    OID: b4d7e1a9f2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9

error: Verification failed with 1 error(s)
```

## Use Cases

### After pull or clone

```bash
$ mediagit pull
$ mediagit verify HEAD
```

### Before push

```bash
$ mediagit verify --detailed
$ mediagit push
```

### CI pipeline (fast)

```bash
$ mediagit verify --quick --quiet || exit 1
```

## Exit Status

- **0**: All verifications passed
- **1**: One or more errors detected
- **2**: Invalid options or could not open repository

## Notes

### Verify vs Fsck

- **verify**: Fast integrity check — checksums and reference validation only. Does not check connectivity or dangling objects. Best for CI pipelines and quick health checks.
- **fsck**: Comprehensive graph analysis — connectivity, dangling objects, and a repair mode
  (`--repair`, not available on `verify`). Use for thorough repository audits.

Use `verify` for spot-checks; `fsck` for complete validation.

## See Also

- [mediagit fsck](./fsck.md) - Full repository verification
- [mediagit gc](./gc.md) - Garbage collection with verification
- [mediagit stats](./stats.md) - Repository statistics
- [mediagit show](./show.md) - Show object contents

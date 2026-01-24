# MediaGit-Core: Unimplemented Features & Incomplete Functionalities

**Last Updated**: 2026-01-24
**Status**: Tracking Document
**PRD Compliance**: 99.6% (excluding items below)

---

## Overview

This document tracks all unimplemented features, incomplete functionalities, and placeholders in the mediagit-core codebase. Use this for planning future development work.

---

## Priority Legend

| Priority | Description | Action Required |
|----------|-------------|-----------------|
| 🔴 P0 | Breaking - Users will hit errors | Immediate fix or remove |
| 🟡 P1 | Major missing feature | Plan for next release |
| 🟠 P2 | Minor missing feature | Backlog |
| 🔵 P3 | Technical debt | Low priority |

---

## 1. Storage Backend

### 🔴 P0: B2/Spaces Backend Not Implemented

**Location**: `crates/mediagit-storage/src/b2_spaces.rs`

| Method | Line | Current Behavior |
|--------|------|------------------|
| `get()` | 476-487 | Returns error: "AWS SDK S3 dependency required" |
| `put()` | 489-513 | Returns error: "AWS SDK S3 dependency required" |
| `exists()` | 515-540 | Returns error: "AWS SDK S3 dependency required" |
| `delete()` | 542-565 | Returns error: "AWS SDK S3 dependency required" |
| `list_objects()` | 567-590 | Returns error: "AWS SDK S3 dependency required" |

**Impact**: Backblaze B2 and DigitalOcean Spaces storage completely unusable.

**Resolution Options**:
1. Implement using `aws-sdk-s3` with custom endpoint
2. Remove from crate and document as unsupported
3. Add compile-time feature flag to exclude

**Effort Estimate**: Medium (2-3 days)

---

### 🟡 P1: GCS Multi-Backend Server Support

**Location**: `crates/mediagit-server/src/handlers.rs:154`

```rust
tracing::error!("GCS and Multi-backend storage are not yet implemented");
```

**Impact**: Server can't use GCS backend directly.

**Effort Estimate**: Medium (1-2 days)

---

## 2. CLI Commands

### 🟡 P1: Rebase Command Incomplete

**Location**: `crates/mediagit-cli/src/commands/rebase.rs`

| Feature | Line | Current Behavior |
|---------|------|------------------|
| Interactive (`-i`) | 72 | `anyhow::bail!("Interactive rebase not yet implemented")` |
| Merge commits | 75 | `anyhow::bail!("Rebase with merge commits not yet implemented")` |
| `--abort` | 263 | `anyhow::bail!("Rebase abort not yet implemented")` |
| `--continue` | 270 | `anyhow::bail!("Rebase continue not yet implemented")` |
| `--skip` | 277 | `anyhow::bail!("Rebase skip not yet implemented")` |

**Impact**: Users cannot use interactive rebase or recover from rebase conflicts.

**Effort Estimate**: Large (1-2 weeks)

---

### 🟡 P1: Pull Rebase Integration

**Location**: `crates/mediagit-cli/src/commands/pull.rs`

| Feature | Line | Current Behavior |
|---------|------|------------------|
| `--rebase` flag | 43 | Documented but falls back to merge |
| Rebase integration | 324-327 | Prints warning, uses merge instead |

**Impact**: `pull --rebase` silently falls back to merge.

**Effort Estimate**: Medium (depends on rebase implementation)

---

### 🟠 P2: Branch Command Features

**Location**: `crates/mediagit-cli/src/commands/branch.rs`

| Feature | Line | Current Behavior |
|---------|------|------------------|
| `--protect` | 579 | `anyhow::bail!("Branch protection not yet implemented")` |
| `--merge` | 695 | `anyhow::bail!("Branch merge not yet implemented")` |

**Impact**: Cannot protect branches or merge via branch command.

**Note**: `--merge` redirects users to use `mediagit merge` command.

**Effort Estimate**: Small-Medium

---

### 🟠 P2: Remote Show Details

**Location**: `crates/mediagit-cli/src/commands/remote.rs`

| Feature | Line | Current Behavior |
|---------|------|------------------|
| HEAD branch | 292 | Prints "(not yet implemented)" |
| Remote refs | 296 | Prints "(not yet implemented)" |

**Impact**: `remote show <name>` provides incomplete information.

**Effort Estimate**: Small (1 day)

---

### 🟠 P2: Verify Commit Range

**Location**: `crates/mediagit-cli/src/commands/verify.rs`

| Feature | Line | Current Behavior |
|---------|------|------------------|
| `--from` | 58 | Struct field exists, not used |
| `--to` | 62 | Struct field exists, not used |

**Impact**: Cannot verify specific commit ranges.

**Effort Estimate**: Small (1 day)

---

## 3. Git Integration

### 🟠 P2: Smudge Filter Object Retrieval

**Location**: `crates/mediagit-git/src/filter.rs:330`

```rust
warn!("Object retrieval not yet implemented, outputting pointer file");
```

**Impact**: Git smudge filter outputs pointer instead of actual file content.

**Effort Estimate**: Medium (2-3 days)

---

## 4. Media Processing

### 🔵 P3: Image Metadata Extraction

**Location**: `crates/mediagit-media/src/image.rs`

| Feature | Line | Current Behavior |
|---------|------|------------------|
| Full metadata | 399 | Returns empty metadata placeholder |
| Extended metadata | 418 | Returns empty metadata placeholder |

**Impact**: Image metadata not fully extracted for merge intelligence.

**Effort Estimate**: Medium (2-3 days)

---

## 5. Versioning

### 🔵 P3: Pack Delta Count

**Location**: `crates/mediagit-versioning/src/pack.rs:696`

```rust
delta_count: 0, // Placeholder
```

**Impact**: Pack statistics don't report accurate delta counts.

**Effort Estimate**: Small (few hours)

---

## 6. Security

### 🔵 P3: TLS Feature Stubs

**Location**: `crates/mediagit-security/src/tls/`

| Feature | File | Line | Status |
|---------|------|------|--------|
| Certificate loading | config.rs | 193 | Stub for non-TLS feature |
| Self-signed generation | cert.rs | 234 | Stub for non-TLS feature |

**Impact**: None when TLS feature is disabled. Stubs exist for API completeness.

**Effort Estimate**: N/A (intentional stubs)

---

### 🔵 P3: Encryption Benchmark

**Location**: `crates/mediagit-security/benches/encryption_benchmark.rs`

Entire file is a placeholder benchmark with no real tests.

**Impact**: No performance baseline for encryption.

**Effort Estimate**: Small (1 day)

---

## Summary Statistics

| Priority | Count | Percentage |
|----------|-------|------------|
| 🔴 P0 (Breaking) | 1 | 7% |
| 🟡 P1 (Major) | 3 | 21% |
| 🟠 P2 (Minor) | 5 | 36% |
| 🔵 P3 (Tech Debt) | 5 | 36% |
| **Total** | **14** | 100% |

---

## Code Markers Reference

Search patterns for finding these in code:

```bash
# All "not implemented" markers
grep -rn "not yet implemented\|not implemented" crates/

# All placeholders
grep -rn "placeholder\|Placeholder" crates/

# All stubs
grep -rn "stub\|Stub" crates/

# B2/Spaces specific
grep -rn "AWS SDK S3 dependency required" crates/
```

---

## Implementation Roadmap Suggestion

### Phase 1: Fix Breaking Issues
- [ ] B2/Spaces: Decide implement vs remove

### Phase 2: Complete Core Git Operations
- [ ] Rebase interactive mode
- [ ] Rebase abort/continue/skip
- [ ] Pull --rebase integration

### Phase 3: Polish CLI
- [ ] Remote show details
- [ ] Verify commit ranges
- [ ] Branch protection

### Phase 4: Technical Debt
- [ ] Smudge filter completion
- [ ] Image metadata extraction
- [ ] Pack delta counting
- [ ] Encryption benchmarks

---

## Change Log

| Date | Change | Author |
|------|--------|--------|
| 2026-01-24 | Initial document created from codebase analysis | Claude |

---

*This document should be updated whenever features are implemented or new incomplete functionality is discovered.*

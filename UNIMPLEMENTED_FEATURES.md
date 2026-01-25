# MediaGit-Core: Unimplemented Features & Implementation Plan

**Last Updated**: 2026-01-25
**Status**: Active Development Tracker
**PRD Compliance**: 99.6%
**Total Items**: 15

---

## Overview

This document tracks unimplemented features, incomplete functionalities, and provides a detailed implementation plan aligned with mediagit-core's roadmap (v0.2.0 and beyond).

---

## Priority Legend

| Priority | Description | SLA | Action |
|----------|-------------|-----|--------|
| 🔴 P0 | Breaking - Users hit errors | This sprint | Immediate fix or feature flag |
| 🟡 P1 | Major missing feature | Next release | Plan for v0.2.0 |
| 🟠 P2 | Minor missing feature | Backlog | Plan for v0.2.x |
| 🔵 P3 | Technical debt | Opportunistic | Low priority |

---

## Summary by Category

| Category | P0 | P1 | P2 | P3 | Total |
|----------|-----|-----|-----|-----|-------|
| Storage Backend | 1 | 1 | 0 | 0 | 2 |
| CLI Commands | 0 | 3 | 4 | 0 | 7 |
| Git Integration | 0 | 0 | 1 | 0 | 1 |
| Media Processing | 0 | 0 | 0 | 1 | 1 |
| Versioning | 0 | 0 | 0 | 1 | 1 |
| Security | 0 | 0 | 0 | 3 | 3 |
| **Total** | **1** | **4** | **5** | **5** | **15** |

---

## 1. Storage Backend

### 🔴 P0-STORAGE-001: B2/Spaces Backend Not Implemented

**Location**: `crates/mediagit-storage/src/b2_spaces.rs:476-590`

**Impact**: Backblaze B2 and DigitalOcean Spaces storage backends advertised but completely unusable.

| Method | Line | Current Behavior |
|--------|------|------------------|
| `get()` | 476-487 | Returns `Err("AWS SDK S3 dependency required")` |
| `put()` | 489-513 | Returns `Err("AWS SDK S3 dependency required")` |
| `exists()` | 515-540 | Returns `Err("AWS SDK S3 dependency required")` |
| `delete()` | 542-565 | Returns `Err("AWS SDK S3 dependency required")` |
| `list_objects()` | 567-590 | Returns `Err("AWS SDK S3 dependency required")` |

**Decision Required**:
| Option | Effort | Recommendation |
|--------|--------|----------------|
| A. Implement using `aws-sdk-s3` with custom endpoint | 2-3 days | ✅ Recommended |
| B. Remove from crate, document as unsupported | 1 day | Acceptable |
| C. Add `#[cfg(feature = "b2_spaces")]` compile-time flag | 0.5 day | Stopgap |

**Dependencies**: aws-sdk-s3 (already in workspace)

**Implementation Notes**:
- Both B2 and Spaces are S3-compatible
- Existing S3Backend can be adapted with custom endpoint configuration
- Consider refactoring to share S3 client code

---

### 🟡 P1-STORAGE-002: GCS Multi-Backend Server Support

**Location**: `crates/mediagit-server/src/handlers.rs:154`

```rust
tracing::error!("GCS and Multi-backend storage are not yet implemented");
```

**Impact**: Server cannot use GCS backend directly. CLI works fine.

**Effort**: 1-2 days

---

## 2. CLI Commands

### 🟡 P1-CLI-001: Branch List Cross-Platform Path Handling (Test Failure)

**Location**: `crates/mediagit-cli/src/commands/branch.rs`
**Test**: `comprehensive_e2e_tests.rs:375` - `e2e_branch_create_and_switch`

**Failure (Windows)**:
```
Expected: * feature
Actual:   refs/heads\feature
```

**Root Causes**:
1. Path separator: Windows `\` vs Unix `/`
2. Full refs path instead of short branch name
3. Missing `*` current branch marker

**Resolution**:
```rust
// Normalize path separators
let normalized = path.replace('\\', "/");
// Strip refs/heads/ prefix
let short_name = normalized.strip_prefix("refs/heads/").unwrap_or(&normalized);
// Add current branch marker
let marker = if is_current { "* " } else { "  " };
```

**Effort**: 0.5 day

---

### 🟡 P1-CLI-002: Rebase Command Incomplete

**Location**: `crates/mediagit-cli/src/commands/rebase.rs`

| Feature | Line | Status |
|---------|------|--------|
| Interactive (`-i`) | 72 | `bail!("not yet implemented")` |
| Merge commits | 75 | `bail!("not yet implemented")` |
| `--abort` | 263 | `bail!("not yet implemented")` |
| `--continue` | 270 | `bail!("not yet implemented")` |
| `--skip` | 277 | `bail!("not yet implemented")` |

**Impact**: Users cannot use interactive rebase or recover from conflicts.

**Effort**: Large (1-2 weeks)

**Dependencies**: Requires conflict resolution framework

---

### 🟡 P1-CLI-003: Pull Rebase Integration

**Location**: `crates/mediagit-cli/src/commands/pull.rs:324-327`

**Current**: `--rebase` flag documented but silently falls back to merge.

**Effort**: Medium (depends on P1-CLI-002)

---

### 🟠 P2-CLI-004: Branch Protection

**Location**: `crates/mediagit-cli/src/commands/branch.rs:579`

```rust
anyhow::bail!("Branch protection not yet implemented")
```

**Impact**: Cannot protect branches from force-push or deletion.

**Effort**: 1-2 days

---

### 🟠 P2-CLI-005: Branch Merge via Branch Command

**Location**: `crates/mediagit-cli/src/commands/branch.rs:695`

**Current**: Redirects users to `mediagit merge` command.

**Resolution**: Either implement or improve error message with exact command.

**Effort**: 0.5 day (message) or 1 day (implement)

---

### 🟠 P2-CLI-006: Remote Show Details

**Location**: `crates/mediagit-cli/src/commands/remote.rs:292-296`

| Feature | Status |
|---------|--------|
| HEAD branch | Prints "(not yet implemented)" |
| Remote refs | Prints "(not yet implemented)" |

**Effort**: 1 day

---

### 🟠 P2-CLI-007: Verify Commit Range

**Location**: `crates/mediagit-cli/src/commands/verify.rs:58-62`

**Current**: `--from` and `--to` flags exist in struct but are unused.

**Effort**: 1 day

---

## 3. Git Integration

### 🟠 P2-GIT-001: Smudge Filter Object Retrieval

**Location**: `crates/mediagit-git/src/filter.rs:330`

```rust
warn!("Object retrieval not yet implemented, outputting pointer file");
```

**Impact**: Git smudge filter outputs pointer instead of actual content.

**Effort**: 2-3 days

---

## 4. Media Processing

### 🔵 P3-MEDIA-001: Image Metadata Extraction

**Location**: `crates/mediagit-media/src/image.rs:399-418`

**Current**: Returns empty metadata placeholder.

**Impact**: Image metadata not available for merge intelligence.

**Effort**: 2-3 days

---

## 5. Versioning

### 🔵 P3-VERSION-001: Pack Delta Count

**Location**: `crates/mediagit-versioning/src/pack.rs:696`

```rust
delta_count: 0, // Placeholder
```

**Impact**: Pack statistics show 0 deltas regardless of actual count.

**Effort**: Few hours

---

## 6. Security

### 🔵 P3-SECURITY-001: TLS Feature Stubs

**Location**: `crates/mediagit-security/src/tls/`

| Feature | File:Line | Status |
|---------|-----------|--------|
| Certificate loading | config.rs:193 | Stub for non-TLS |
| Self-signed generation | cert.rs:234 | Stub for non-TLS |

**Impact**: None when TLS disabled. Intentional API stubs.

---

### 🔵 P3-SECURITY-002: Encryption Benchmark

**Location**: `crates/mediagit-security/benches/encryption_benchmark.rs`

**Current**: Placeholder benchmark with no real tests.

**Effort**: 1 day

---

### 🔵 P3-SECURITY-003: Security Audit Trail

**Status**: No structured audit logging for security events.

**Effort**: 2-3 days

---

## Implementation Plan

### Sprint 1: Fix Breaking Issues (P0)

**Duration**: 1 week
**Goal**: Zero P0 issues

| ID | Task | Owner | Status | Notes |
|----|------|-------|--------|-------|
| P0-STORAGE-001 | B2/Spaces Backend | TBD | 🔲 Pending | Decision: Implement or feature-flag |

**Deliverables**:
- [ ] B2/Spaces decision made and implemented
- [ ] All tests passing on Windows + Linux

---

### Sprint 2: Core CLI Polish (P1)

**Duration**: 2 weeks
**Goal**: Cross-platform stability, basic rebase

| ID | Task | Effort | Dependencies |
|----|------|--------|--------------|
| P1-CLI-001 | Branch list path handling | 0.5 day | None |
| P1-STORAGE-002 | GCS server support | 1-2 days | None |
| P1-CLI-002 | Rebase basics (non-interactive) | 1 week | None |
| P1-CLI-003 | Pull --rebase | 2 days | P1-CLI-002 |

**Deliverables**:
- [ ] `branch list` works identically on Windows/Linux/macOS
- [ ] Basic `rebase` command functional
- [ ] `pull --rebase` uses actual rebase

---

### Sprint 3: CLI Feature Completion (P2)

**Duration**: 1 week
**Goal**: Complete minor CLI features

| ID | Task | Effort |
|----|------|--------|
| P2-CLI-004 | Branch protection | 1-2 days |
| P2-CLI-006 | Remote show details | 1 day |
| P2-CLI-007 | Verify commit range | 1 day |
| P2-GIT-001 | Smudge filter | 2-3 days |

---

### Sprint 4: Technical Debt (P3)

**Duration**: 1 week
**Goal**: Clean up placeholders

| ID | Task | Effort |
|----|------|--------|
| P3-VERSION-001 | Pack delta count | Few hours |
| P3-MEDIA-001 | Image metadata | 2-3 days |
| P3-SECURITY-002 | Encryption benchmark | 1 day |

---

## Code Search Patterns

```bash
# Find all "not implemented" markers
grep -rn "not yet implemented\|not implemented" crates/

# Find all bail! macros (potential incomplete features)
grep -rn 'bail!\|anyhow::bail!' crates/

# Find all placeholders
grep -rn "placeholder\|Placeholder\|TODO\|FIXME" crates/

# Find B2/Spaces specific
grep -rn "AWS SDK S3 dependency required" crates/

# Find test failures
cargo test 2>&1 | grep -E "FAILED|error\[E"
```

---

## Alignment with README Roadmap

### v0.2.0 (Next Release) - Alignment Check

| README Goal | Tracker Item | Status |
|-------------|--------------|--------|
| Branch switching optimization | P1-CLI-001 | ✅ Tracked |
| Real cloud provider testing | P0-STORAGE-001 | ✅ Tracked |
| Enhanced error messages | Multiple P2 items | ✅ Tracked |

### Items NOT in README (Consider Adding)

| Item | Recommendation |
|------|----------------|
| Rebase command | Add to v0.2.0 roadmap |
| Pull --rebase | Add to v0.2.0 roadmap |
| Cross-platform path handling | Add as bug fix |

---

## Change Log

| Date | Change | Author |
|------|--------|--------|
| 2026-01-25 | Restructured document with implementation plan | Claude |
| 2026-01-25 | Added P1-CLI-001: Branch list cross-platform (test failure) | Claude |
| 2026-01-24 | Initial document created from codebase analysis | Claude |

---

## Metrics & Tracking

### Current Status
- **P0 (Breaking)**: 1 item (7%)
- **P1 (Major)**: 4 items (27%)
- **P2 (Minor)**: 5 items (33%)
- **P3 (Tech Debt)**: 5 items (33%)
- **Total**: 15 items

### Target (v0.2.0)
- **P0**: 0 items
- **P1**: 0 items
- **P2**: ≤3 items
- **P3**: Best effort

---

*This document is the source of truth for incomplete features. Update when features are implemented or new gaps discovered.*

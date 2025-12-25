# Performance Optimizations for Faster Push, Pull, and Branch Switching
# Date: 2025-12-19

## Overview

After analyzing the current implementation, I've identified several optimization opportunities to significantly improve performance for large media file operations.

## Current Performance Bottlenecks

| Operation | Bottleneck | Impact |
|-----------|------------|--------|
| **Push** | Collects ALL reachable objects, reads each sequentially | High for large repos |
| **Pull** | No "have" negotiation, downloads everything | High - no incremental pull |
| **Checkout** | Writes every file, even unchanged ones | High for branch switching |
| **Branch Switch** | Full checkout instead of diff-based | High for similar branches |

---

## Proposed Optimizations

### 1. Incremental Push with Object Negotiation
**Expected Improvement**: 50-90% reduction in push time

### 2. Incremental Pull with "Have" List
**Expected Improvement**: 70-95% reduction in pull time after initial clone

### 3. Differential Checkout (Skip Unchanged Files) ✅ IMPLEMENTED
**Expected Improvement**: 80-99% reduction in checkout time for similar branches

### 4. Parallel Object I/O
**Expected Improvement**: 3-5x faster for collecting many objects

### 5. Shallow Clone Support
**Expected Improvement**: Initial clone can be 10-100x faster

### 6. Bitmap Index for Object Reachability
**Expected Improvement**: Near-instant object enumeration for push/pull

### 7. Reference Caching
**Expected Improvement**: 10x faster branch operations

---

## Priority Ranking

| Priority | Optimization | Effort | Impact | Status |
|----------|-------------|--------|--------|--------|
| 1 | Differential Checkout | Medium | Very High | ✅ Done |
| 2 | Incremental Push | Low | High | Pending |
| 3 | Incremental Pull | Low | High | Pending |
| 4 | Parallel Object I/O | Low | Medium | Pending |
| 5 | Reference Caching | Low | Medium | Pending |
| 6 | Bitmap Index | High | High | Pending |
| 7 | Shallow Clone | Medium | High | Pending |

---

# Fast Checkout Implementation Walkthrough

## Summary

Implemented **differential checkout** for mediagit-core to achieve checkout in < 1s for unchanged or similar branches.

## What Was Changed

### checkout.rs
- Added `checkout_diff()` method - Compares two commits and only updates changed files
- Added `CheckoutStats` struct - Reports metrics on files added/modified/deleted/unchanged
- Added helper methods - `get_tree_files_with_oid()`, `checkout_single_file()`

### lib.rs
Exported `CheckoutStats` for public use.

## Key Optimizations

| Check | What Happens |
|-------|--------------|
| Same commit OID | Returns instantly (0 files changed) |
| Same tree OID | Returns instantly (different commit, same content) |
| File has same OID | Skipped (no disk read/write) |
| Different OID | File is updated |

## Performance Impact

| Scenario | Old Behavior | New Behavior |
|----------|--------------|--------------|
| Same branch (no change) | All files rewritten | Instant (0ms) |
| 1 file changed in 1000 | 1000 file writes | 1 file write |
| Video project (10GB) | 10GB disk I/O | Only changed files |

## Usage

```rust
use mediagit_versioning::{CheckoutManager, CheckoutStats};

let checkout_mgr = CheckoutManager::new(&odb, repo_root);

// Differential checkout - fast!
let stats: CheckoutStats = checkout_mgr
    .checkout_diff(&current_commit, &new_commit)
    .await?;

println!("Updated {} files in {}ms", stats.files_changed(), stats.elapsed_ms);
```

## Tests Added

- `test_differential_checkout_same_commit` - Same commit = instant return
- `test_differential_checkout_unchanged_files` - Identical files are skipped
- `test_differential_checkout_add_delete_files` - Handles adds/deletes correctly
- `test_differential_checkout_stats` - CheckoutStats calculations

// MediaGit - Git for Media Files
// Copyright (C) 2025 MediaGit Contributors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

//! Auto-gc trigger.
//!
//! Invoked from `commit`, `pull`, and `clone` so that orphaned objects (e.g.
//! v1 chunks of a re-staged file, or stale objects from a partial fetch) are
//! reclaimed without the user having to remember to run `mediagit gc`.
//!
//! The actual GC work is done by [`crate::commands::gc::run_gc`] in
//! `--auto --quiet` mode, which short-circuits if reclaimable work is below
//! the byte / count thresholds defined in `gc.rs`.
//!
//! **Those thresholds gate the DELETE, not the SCAN.** `run_gc` must walk the
//! whole reachability graph and list every object/chunk/delta before it can
//! conclude there is nothing worth deleting, so an unconditional trigger pays a
//! full-repository scan on every add and every commit. That scan is O(repo),
//! which makes an n-operation session O(n^2).
//!
//! Measured (QA drill S2, 500 commits mutating one 32 MiB asset, 2026-08-03):
//! per-100-commit wall time grew 125s -> 268s -> 970s -> 1620s as history grew,
//! CPU-bound, while chunk-delta chain depth stayed at 1. In that workflow
//! nothing ever becomes unreachable, so all ~1000 scans reclaimed nothing.
//!
//! Hence [`should_scan`]: a cheap persisted counter decides whether to pay for
//! the scan at all, mirroring Git's `gc.auto` loose-object heuristic. Amortized
//! cost per operation becomes O(1); a session becomes O(n) instead of O(n^2).

use crate::commands::gc::{GcOptions, run_gc};
use anyhow::Result;
use std::path::Path;
use tracing::{debug, warn};

/// Why auto-gc is being invoked. Currently informational; reserved for future
/// per-trigger threshold tuning.
#[derive(Debug, Clone, Copy)]
#[allow(clippy::enum_variant_names)]
pub enum TriggerMode {
    /// After `add` — catches orphans from re-staging a file before commit
    /// (stage v1, modify, stage v2 — v1's chunks become orphan immediately).
    PostAdd,
    PostCommit,
    PostPull,
    PostClone,
}

/// Environment variable that disables auto-gc globally for the current
/// invocation. Mirrors Git's `GC_AUTO` convention.
const ENV_DISABLE: &str = "MEDIAGIT_NO_AUTO_GC";

/// Per-repo opt-out marker. Touch `.mediagit/no-autogc` to disable auto-gc
/// for a specific repository (e.g. when iterating fast and orphans are
/// expected to be reclaimed manually).
const REPO_MARKER: &str = ".mediagit/no-autogc";

/// Run auto-gc for `repo_root` if not opted out.
///
/// Failure is logged but never propagated — auto-gc is a best-effort
/// background optimization and must never break the operation that triggered
/// it (a failed gc should not fail the commit/pull/clone the user invoked).
pub async fn maybe_run(repo_root: &Path, mode: TriggerMode) -> Result<()> {
    // Stale upload-journal sweep runs regardless of the auto-gc enable knob
    // below — it isn't garbage collection, it's housekeeping for a resumable-
    // upload mechanism, and MEDIAGIT_NO_AUTO_GC shouldn't accidentally leave
    // `.mediagit/upload/*.journal` growing forever. Age-based and idempotent
    // (see `UploadJournal::sweep_stale`), so running it on every trigger is
    // cheap and safe.
    let swept = mediagit_protocol::journal::UploadJournal::sweep_stale(
        repo_root,
        mediagit_protocol::journal::DEFAULT_JOURNAL_MAX_AGE,
    );
    if swept > 0 {
        debug!(swept, "swept stale upload journals");
    }

    if !is_enabled(repo_root) {
        debug!(?mode, "auto-gc disabled for this repo/invocation");
        return Ok(());
    }

    // Cheap gate BEFORE the expensive scan. See the module docs: run_gc's
    // thresholds cannot help here because it must scan the whole repo to
    // evaluate them.
    if !should_scan(repo_root) {
        debug!(?mode, "auto-gc: below scan interval, skipping full scan");
        return Ok(());
    }

    debug!(?mode, "auto-gc starting");

    let opts = GcOptions {
        auto: true,
        quiet: true,
        // Honor existing safety: never auto-prune if the user previously
        // marked this repo as no-prune. (no-prune is currently a flag,
        // not a persistent setting, so this is a no-op for now — kept
        // for explicitness.)
        no_prune: false,
        // Auto mode never prompts.
        yes: true,
        ..GcOptions::default()
    };

    // run_gc reads `current_dir()` to find the repo. Since trigger callers
    // are already executing inside the repo, this works — but defend against
    // future callers by setting cwd if needed.
    let original_cwd = std::env::current_dir().ok();
    let needs_chdir = original_cwd
        .as_ref()
        .map(|cwd| cwd != repo_root)
        .unwrap_or(true);

    if needs_chdir && let Err(e) = std::env::set_current_dir(repo_root) {
        warn!(
            "auto-gc: failed to set cwd to {}: {}",
            repo_root.display(),
            e
        );
        return Ok(());
    }

    let result = run_gc(&opts).await;

    if needs_chdir && let Some(cwd) = original_cwd {
        let _ = std::env::set_current_dir(&cwd);
    }

    if let Err(e) = result {
        warn!("auto-gc failed (non-fatal): {}", e);
    } else {
        debug!(?mode, "auto-gc completed");
    }

    Ok(())
}

/// How many auto-gc triggers must elapse between full scans. Overridable via
/// [`ENV_INTERVAL`]; `1` restores the old scan-every-time behaviour.
const DEFAULT_SCAN_INTERVAL: u64 = 100;

/// Counter file holding triggers-since-last-scan. Deliberately a plain integer
/// in its own file rather than a field in `config.toml`: it is hot-path state,
/// not configuration, and must never risk corrupting the repo config if a
/// process dies mid-write.
const COUNTER_FILE: &str = ".mediagit/auto-gc-counter";

/// Environment override for the scan interval.
const ENV_INTERVAL: &str = "MEDIAGIT_AUTO_GC_INTERVAL";

/// Decides whether this trigger should pay for a full scan, and advances the
/// persisted counter.
///
/// Fails OPEN: any I/O problem reading or writing the counter returns `true`
/// (scan). Auto-gc is a best-effort optimisation, and the safe direction on
/// uncertainty is to do the work rather than silently stop collecting garbage
/// forever because a counter file went unreadable.
fn should_scan(repo_root: &Path) -> bool {
    let interval = std::env::var(ENV_INTERVAL)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_SCAN_INTERVAL);
    should_scan_with_interval(repo_root, interval)
}

/// The decision itself, with the interval passed in.
///
/// Split from [`should_scan`] so it is testable without mutating process
/// environment — `std::env::set_var` is `unsafe` and this crate forbids
/// `unsafe`, and a global-mutating test would be order-dependent anyway.
fn should_scan_with_interval(repo_root: &Path, interval: u64) -> bool {
    if interval <= 1 {
        return true;
    }

    let path = repo_root.join(COUNTER_FILE);
    let count = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0)
        .saturating_add(1);

    if count >= interval {
        // Reset first: if the write fails we scan again next time, which is
        // wasteful but harmless. The opposite order could skip scans forever.
        let _ = std::fs::write(&path, "0");
        true
    } else {
        match std::fs::write(&path, count.to_string()) {
            Ok(()) => false,
            // Could not persist progress — scan now rather than risk a repo
            // that never collects because its counter never advances.
            Err(_) => true,
        }
    }
}

/// Whether auto-gc is enabled for this invocation.
fn is_enabled(repo_root: &Path) -> bool {
    if std::env::var(ENV_DISABLE)
        .map(|v| !v.is_empty())
        .unwrap_or(false)
    {
        return false;
    }
    if repo_root.join(REPO_MARKER).exists() {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn marker_file_disables_auto_gc() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".mediagit")).unwrap();
        assert!(is_enabled(tmp.path()));
        fs::write(tmp.path().join(REPO_MARKER), b"").unwrap();
        assert!(!is_enabled(tmp.path()));
    }

    /// The scan must be paid for once per interval, not once per operation.
    /// Without this gate an n-operation session is O(n^2) — `run_gc` has to walk
    /// the whole repo before its own thresholds can tell it there is nothing to
    /// delete (see module docs).
    ///
    /// Uses an explicit interval rather than the default so the test states the
    /// invariant (1 scan per N triggers) instead of encoding today's constant.
    #[test]
    fn full_scan_runs_once_per_interval_not_every_trigger() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".mediagit")).unwrap();

        let scans = (0..20)
            .filter(|_| should_scan_with_interval(tmp.path(), 5))
            .count();
        assert_eq!(
            scans, 4,
            "20 triggers at interval 5 must scan 4 times, not 20 — amortizing the \
             scan is the entire point"
        );
    }

    /// `interval = 1` must restore scan-every-time, so the old behaviour stays
    /// reachable for anyone who depends on immediate collection.
    #[test]
    fn interval_of_one_scans_every_trigger() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".mediagit")).unwrap();
        assert!((0..5).all(|_| should_scan_with_interval(tmp.path(), 1)));
    }

    /// Fail OPEN: an unwritable counter must not silently disable garbage
    /// collection forever. A repo that stops collecting is a worse outcome than
    /// one that scans too often.
    #[test]
    fn unwritable_counter_falls_back_to_scanning() {
        let tmp = TempDir::new().unwrap();
        // No .mediagit dir -> the counter write fails.
        assert!(
            should_scan_with_interval(tmp.path(), 100),
            "if progress cannot be persisted, scan rather than risk never collecting"
        );
    }
}

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

use crate::commands::gc::{run_gc, GcOptions};
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
    if !is_enabled(repo_root) {
        debug!(?mode, "auto-gc disabled for this repo/invocation");
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

    if needs_chdir {
        if let Err(e) = std::env::set_current_dir(repo_root) {
            warn!(
                "auto-gc: failed to set cwd to {}: {}",
                repo_root.display(),
                e
            );
            return Ok(());
        }
    }

    let result = run_gc(&opts).await;

    if needs_chdir {
        if let Some(cwd) = original_cwd {
            let _ = std::env::set_current_dir(&cwd);
        }
    }

    if let Err(e) = result {
        warn!("auto-gc failed (non-fatal): {}", e);
    } else {
        debug!(?mode, "auto-gc completed");
    }

    Ok(())
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
}

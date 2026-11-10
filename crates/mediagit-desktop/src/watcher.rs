// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Watches the open repository's working tree and tells the UI when it changed.

use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use notify_debouncer_mini::notify::{self, RecommendedWatcher, RecursiveMode};
use notify_debouncer_mini::{DebounceEventResult, Debouncer, new_debouncer};

/// Quiet period before a burst of file events becomes one notification.
pub const DEBOUNCE: Duration = Duration::from_millis(500);

/// The one active watch; opening another repo replaces (and drops) it.
#[derive(Default)]
pub struct RepoWatcher(Mutex<Option<Debouncer<RecommendedWatcher>>>);

impl RepoWatcher {
    /// Watch `repo` recursively, ignoring `.mediagit/` (the CLI rewrites it on
    /// every command, which would feed back into a refresh loop). `on_change`
    /// runs on the debouncer's thread at most once per [`DEBOUNCE`] window.
    pub fn watch(
        &self,
        repo: &Path,
        debounce: Duration,
        on_change: impl Fn() + Send + 'static,
    ) -> notify::Result<()> {
        let root = repo.to_path_buf();
        let mut debouncer = new_debouncer(debounce, move |res: DebounceEventResult| {
            if let Ok(events) = res
                && events.iter().any(|e| is_working_tree(&root, &e.path))
            {
                on_change();
            }
        })?;
        debouncer.watcher().watch(repo, RecursiveMode::Recursive)?;
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(debouncer);
        Ok(())
    }
}

/// True unless `path` is inside `<repo>/.mediagit`.
fn is_working_tree(repo: &Path, path: &Path) -> bool {
    !path
        .strip_prefix(repo)
        .is_ok_and(|rel| rel.starts_with(".mediagit"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn mediagit_dir_is_not_working_tree() {
        let repo = Path::new("/r");
        assert!(!is_working_tree(repo, Path::new("/r/.mediagit/index")));
        assert!(is_working_tree(repo, Path::new("/r/a/.mediagitx")));
        assert!(is_working_tree(repo, Path::new("/r/a.psd")));
    }

    #[test]
    fn file_changes_notify_once_per_burst_and_store_writes_do_not() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = dunce::canonicalize(tmp.path()).unwrap();
        std::fs::create_dir(repo.join(".mediagit")).unwrap();
        let w = RepoWatcher::default();
        let (tx, rx) = mpsc::channel();
        w.watch(&repo, Duration::from_millis(200), move || {
            let _ = tx.send(());
        })
        .unwrap();

        std::fs::write(repo.join(".mediagit").join("index"), b"x").unwrap();
        assert!(
            rx.recv_timeout(Duration::from_millis(1200)).is_err(),
            ".mediagit writes must be ignored"
        );

        for i in 0..5 {
            std::fs::write(repo.join(format!("f{i}.psd")), b"x").unwrap();
        }
        rx.recv_timeout(Duration::from_secs(5))
            .expect("a working-tree write notifies");
        // A loaded machine may split a burst across two debounce windows, but
        // five writes must never become five notifications.
        let mut extra = 0;
        while rx.recv_timeout(Duration::from_millis(800)).is_ok() {
            extra += 1;
        }
        assert!(extra < 4, "burst was not coalesced: {extra} extra");
    }
}

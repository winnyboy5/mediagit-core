//! Rebase state management for persisting rebase progress across sessions.
//!
//! State is stored in `.mediagit/rebase-apply/state.json` and tracks:
//! - Original HEAD position for abort recovery
//! - Upstream commit being rebased onto
//! - Remaining commits to apply
//! - Current commit being processed
//! - Any files with conflicts

use anyhow::{Context, Result};
use mediagit_versioning::Oid;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Persistent state for an in-progress rebase operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RebaseState {
    /// The original HEAD position before rebase started (for --abort)
    pub original_head: Oid,
    /// The original branch name if HEAD was symbolic
    pub original_branch: Option<String>,
    /// The upstream commit we're rebasing onto
    pub upstream: Oid,
    /// Commits remaining to be applied (in order)
    pub commits_remaining: Vec<Oid>,
    /// The commit currently being applied (if mid-operation)
    pub current_commit: Option<Oid>,
    /// Index of current commit in original sequence
    pub current_index: usize,
    /// Total number of commits to rebase
    pub total_commits: usize,
    /// Files with conflicts that need resolution
    pub conflict_files: Vec<PathBuf>,
    /// The new parent for the next commit to apply
    pub new_parent: Oid,
}

impl RebaseState {
    /// Create a new rebase state at the start of a rebase operation.
    pub fn new(
        original_head: Oid,
        original_branch: Option<String>,
        upstream: Oid,
        commits_to_rebase: Vec<Oid>,
    ) -> Self {
        let total = commits_to_rebase.len();
        Self {
            original_head,
            original_branch,
            upstream,
            commits_remaining: commits_to_rebase,
            current_commit: None,
            current_index: 0,
            total_commits: total,
            conflict_files: Vec::new(),
            new_parent: upstream,
        }
    }

    /// Get the path to the rebase state directory.
    pub fn state_dir(repo_root: &Path) -> PathBuf {
        repo_root.join(".mediagit").join("rebase-apply")
    }

    /// Get the path to the state file.
    pub fn state_file(repo_root: &Path) -> PathBuf {
        Self::state_dir(repo_root).join("state.json")
    }

    /// Check if a rebase is currently in progress.
    pub fn in_progress(repo_root: &Path) -> bool {
        Self::state_file(repo_root).exists()
    }

    /// Load the rebase state from disk.
    pub fn load(repo_root: &Path) -> Result<Self> {
        let state_file = Self::state_file(repo_root);

        if !state_file.exists() {
            anyhow::bail!("No rebase in progress (no state file found)");
        }

        let content = std::fs::read_to_string(&state_file)
            .context("Failed to read rebase state file")?;

        let state: RebaseState = serde_json::from_str(&content)
            .context("Failed to parse rebase state file")?;

        Ok(state)
    }

    /// Save the rebase state to disk.
    pub fn save(&self, repo_root: &Path) -> Result<()> {
        let state_dir = Self::state_dir(repo_root);
        let state_file = Self::state_file(repo_root);

        // Create directory if needed
        if !state_dir.exists() {
            std::fs::create_dir_all(&state_dir)
                .context("Failed to create rebase-apply directory")?;
        }

        let content = serde_json::to_string_pretty(self)
            .context("Failed to serialize rebase state")?;

        std::fs::write(&state_file, content)
            .context("Failed to write rebase state file")?;

        Ok(())
    }

    /// Clear the rebase state (remove state directory).
    pub fn clear(repo_root: &Path) -> Result<()> {
        let state_dir = Self::state_dir(repo_root);

        if state_dir.exists() {
            std::fs::remove_dir_all(&state_dir)
                .context("Failed to remove rebase-apply directory")?;
        }

        Ok(())
    }

    /// Mark the current commit as complete and advance to next.
    pub fn advance(&mut self) -> Option<Oid> {
        if self.commits_remaining.is_empty() {
            self.current_commit = None;
            return None;
        }

        let next = self.commits_remaining.remove(0);
        self.current_commit = Some(next);
        self.current_index += 1;
        self.conflict_files.clear();
        Some(next)
    }

    /// Skip the current commit and advance to next.
    pub fn skip_current(&mut self) -> Option<Oid> {
        // Just advance without applying
        self.advance()
    }

    /// Mark files as conflicted.
    #[allow(dead_code)]
    pub fn set_conflicts(&mut self, files: Vec<PathBuf>) {
        self.conflict_files = files;
    }

    /// Check if there are unresolved conflicts.
    pub fn has_conflicts(&self) -> bool {
        !self.conflict_files.is_empty()
    }

    /// Update the new parent after successfully applying a commit.
    pub fn set_new_parent(&mut self, parent: Oid) {
        self.new_parent = parent;
    }

    /// Check if rebase is complete (no more commits).
    #[allow(dead_code)]
    pub fn is_complete(&self) -> bool {
        self.commits_remaining.is_empty() && self.current_commit.is_none()
    }

    /// Get progress as (current, total) for display.
    pub fn progress(&self) -> (usize, usize) {
        (self.current_index, self.total_commits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make_oid(s: &str) -> Oid {
        // Create a deterministic OID by hashing the string
        Oid::hash(s.as_bytes())
    }

    #[test]
    fn test_rebase_state_lifecycle() {
        let temp = tempdir().unwrap();
        let repo_root = temp.path();

        // Create .mediagit directory
        std::fs::create_dir_all(repo_root.join(".mediagit")).unwrap();

        let original_head = make_oid("original_head_1234");
        let upstream = make_oid("upstream_commit_12");
        let commits = vec![
            make_oid("commit_a_1234567"),
            make_oid("commit_b_1234567"),
            make_oid("commit_c_1234567"),
        ];

        // Create state
        let mut state = RebaseState::new(
            original_head,
            Some("refs/heads/feature".to_string()),
            upstream,
            commits.clone(),
        );

        assert!(!RebaseState::in_progress(repo_root));

        // Save state
        state.save(repo_root).unwrap();
        assert!(RebaseState::in_progress(repo_root));

        // Load state
        let loaded = RebaseState::load(repo_root).unwrap();
        assert_eq!(loaded.original_head, original_head);
        assert_eq!(loaded.upstream, upstream);
        assert_eq!(loaded.commits_remaining.len(), 3);
        assert_eq!(loaded.total_commits, 3);

        // Advance through commits
        let first = state.advance();
        assert_eq!(first, Some(commits[0]));
        assert_eq!(state.current_index, 1);
        assert_eq!(state.progress(), (1, 3));

        // Clear state
        RebaseState::clear(repo_root).unwrap();
        assert!(!RebaseState::in_progress(repo_root));
    }

    #[test]
    fn test_conflict_handling() {
        let original_head = make_oid("original_head_1234");
        let upstream = make_oid("upstream_commit_12");
        let commits = vec![make_oid("commit_a_1234567")];

        let mut state = RebaseState::new(
            original_head,
            None,
            upstream,
            commits,
        );

        assert!(!state.has_conflicts());

        state.set_conflicts(vec![
            PathBuf::from("src/main.rs"),
            PathBuf::from("lib.rs"),
        ]);

        assert!(state.has_conflicts());
        assert_eq!(state.conflict_files.len(), 2);
    }

    #[test]
    fn test_completion_check() {
        let original_head = make_oid("original_head_1234");
        let upstream = make_oid("upstream_commit_12");
        let commits = vec![make_oid("commit_a_1234567")];

        let mut state = RebaseState::new(original_head, None, upstream, commits);

        assert!(!state.is_complete());

        // Advance past the only commit
        state.advance();
        state.current_commit = None; // Simulating commit applied

        assert!(state.is_complete());
    }
}

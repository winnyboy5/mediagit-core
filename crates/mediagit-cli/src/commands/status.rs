// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

use super::super::repo::{create_storage_backend, find_repo_root};
use anyhow::Result;
use clap::Parser;
use console::style;
use indicatif::HumanBytes;
use mediagit_versioning::{Index, ObjectDatabase, Oid, Ref, RefDatabase, SparseFilter};
use rayon::prelude::*;
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use crate::ignore_rules::IgnoreMatcher;

/// Single source of truth for status output: human rendering and `--json`
/// both derive from this struct (built once per invocation). `--porcelain`
/// is a separate, frozen format and does not go through this struct.
#[derive(Debug, Clone, Serialize)]
pub struct StatusReport {
    /// Bumped only if the JSON shape changes incompatibly.
    pub format_version: u32,
    pub branch: BranchReport,
    pub staged: Vec<StagedFileReport>,
    pub modified: Vec<String>,
    pub deleted: Vec<String>,
    pub untracked: Vec<String>,
    pub ignored: Vec<String>,
    /// Media metadata lines for modified/staged media files (M5 T3),
    /// gated by `MEDIAGIT_MEDIA_META`. Additive — empty unless the knob is
    /// on and at least one changed file is a recognized media type.
    pub media: Vec<MediaLineReport>,
    pub summary: StatusSummaryReport,
}

/// A single `media: ...` summary line for one modified/staged file, keyed
/// by path so human rendering can print it right under that file's line.
#[derive(Debug, Clone, Serialize)]
pub struct MediaLineReport {
    pub path: String,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BranchReport {
    /// Branch name; `None` for detached HEAD or an invalid HEAD ref.
    pub name: Option<String>,
    pub detached: bool,
    /// OID (hex) HEAD is detached at, when `detached` is true.
    pub detached_oid: Option<String>,
    pub has_commits: bool,
    pub upstream: Option<UpstreamReport>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpstreamReport {
    /// e.g. "origin/main"
    pub name: String,
    /// `None` when the remote-tracking ref has never been fetched locally
    /// (status never performs network I/O to find out).
    pub ahead: Option<u64>,
    pub behind: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StagedKind {
    Added,
    Modified,
    Deleted,
}

#[derive(Debug, Clone, Serialize)]
pub struct StagedFileReport {
    pub path: String,
    pub kind: StagedKind,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusSummaryReport {
    pub staged: usize,
    pub modified: usize,
    pub deleted: usize,
    pub untracked: usize,
    /// Only computed with `--verbose` (matches prior behavior/perf cost).
    pub total_size: Option<u64>,
}

/// Current branch identity, resolved once and reused for both the printed
/// branch header (`-b`) and the `--json` report.
enum BranchState {
    OnBranch(String),
    Detached(Oid),
    NoCommitsYet,
    Invalid,
}

/// Format the ` — origin/main: ahead N, behind N` suffix appended to the
/// branch header. Falls back to just the upstream name when counts are
/// unavailable (never-fetched tracking ref) or both sides are equal.
fn format_upstream_suffix(u: &UpstreamReport) -> String {
    match (u.ahead, u.behind) {
        (Some(a), Some(b)) if a > 0 && b > 0 => format!(" — {}: ahead {}, behind {}", u.name, a, b),
        (Some(a), Some(_)) if a > 0 => format!(" — {}: ahead {}", u.name, a),
        (Some(_), Some(b)) if b > 0 => format!(" — {}: behind {}", u.name, b),
        _ => format!(" — {}", u.name),
    }
}

/// Ancestor set of `root`, walking only commit parent links (no trees/blobs).
/// Purely local ODB reads — never touches the network.
async fn collect_commit_ancestors(odb: &ObjectDatabase, root: Oid) -> HashSet<Oid> {
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();
    visited.insert(root);
    queue.push_back(root);
    while let Some(oid) = queue.pop_front() {
        if let Ok(data) = odb.read(&oid).await
            && let Ok(commit) =
                mediagit_versioning::format::deserialize::<mediagit_versioning::Commit>(&data)
        {
            for parent in commit.parents {
                if visited.insert(parent) {
                    queue.push_back(parent);
                }
            }
        }
    }
    visited
}

/// ahead = commits reachable from `local` but not from `remote`; behind = the
/// reverse. Matches git's `rev-list --left-right --count` semantics.
async fn compute_ahead_behind(odb: &ObjectDatabase, local: Oid, remote: Oid) -> (u64, u64) {
    if local == remote {
        return (0, 0);
    }
    let local_set = collect_commit_ancestors(odb, local).await;
    let remote_set = collect_commit_ancestors(odb, remote).await;
    let ahead = local_set.difference(&remote_set).count() as u64;
    let behind = remote_set.difference(&local_set).count() as u64;
    (ahead, behind)
}

/// Resolve upstream tracking + ahead/behind for `branch_name`, if configured.
/// Offline only: if the remote-tracking ref has never been fetched locally,
/// the upstream name is still returned but `ahead`/`behind` are `None`.
async fn resolve_upstream(
    config: &mediagit_config::Config,
    refdb: &RefDatabase,
    odb: &ObjectDatabase,
    branch_name: &str,
    local_head: Option<Oid>,
) -> Option<UpstreamReport> {
    let (remote, merge) = config.get_branch_upstream(branch_name)?;
    let remote_branch = merge.strip_prefix("refs/heads/").unwrap_or(merge);
    let name = format!("{}/{}", remote, remote_branch);
    let tracking_ref = format!("refs/remotes/{}/{}", remote, remote_branch);

    let remote_oid = refdb.read(&tracking_ref).await.ok().and_then(|r| r.oid);

    let (ahead, behind) = match (local_head, remote_oid) {
        (Some(local), Some(remote)) => {
            let (a, b) = compute_ahead_behind(odb, local, remote).await;
            (Some(a), Some(b))
        }
        _ => (None, None),
    };

    Some(UpstreamReport {
        name,
        ahead,
        behind,
    })
}

/// Show the working tree status
///
/// Displays the state of the working directory and staging area, showing which
/// changes have been staged, which haven't, and which files aren't being tracked.
#[derive(Parser, Debug)]
#[command(after_help = "EXAMPLES:
    # Show repository status
    mediagit status

    # Show status with branch information
    mediagit status -b

    # Show status in short format
    mediagit status -s

    # Show status in porcelain format (for scripts)
    mediagit status --porcelain

    # Show tracked files only
    mediagit status --tracked

    # Show untracked files only
    mediagit status --untracked

    # Show status as a single JSON document
    mediagit status --json

SEE ALSO:
    mediagit-add(1), mediagit-commit(1), mediagit-diff(1)")]
pub struct StatusCmd {
    /// Show tracked files
    #[arg(long)]
    pub tracked: bool,

    /// Show untracked files
    #[arg(long)]
    pub untracked: bool,

    /// Show ignored files
    #[arg(long)]
    pub ignored: bool,

    /// Show short format
    #[arg(short, long)]
    pub short: bool,

    /// Show porcelain format (for scripts)
    #[arg(long)]
    pub porcelain: bool,

    /// Show branch information
    #[arg(short = 'b', long)]
    pub branch: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,

    /// Verbose mode: also show total size of changed files in the summary line
    #[arg(short, long)]
    pub verbose: bool,

    /// Show status as a single JSON document (implies no colors/progress)
    #[arg(long)]
    pub json: bool,
}

impl StatusCmd {
    pub async fn execute(&self) -> Result<()> {
        use crate::output;

        // Find repository root and canonicalize for consistent path handling on Windows
        let repo_root = dunce::canonicalize(find_repo_root()?)
            .unwrap_or_else(|_| find_repo_root().expect("repo root"));

        if !self.quiet && !self.json && !self.porcelain {
            output::header("Repository Status");
        }

        // Initialize storage and ref database
        let storage_path = repo_root.join(".mediagit");
        let storage = create_storage_backend(&repo_root).await?;
        let refdb = RefDatabase::new(&storage_path);
        let odb = ObjectDatabase::with_smart_compression(storage.clone(), 1000);

        // Read HEAD (may not exist in empty repos with no commits)
        let head = refdb.read("HEAD").await.ok();

        // Check if we have any commits by trying to resolve HEAD
        let head_oid = refdb.resolve("HEAD").await.ok();
        let has_commits = head_oid.is_some();

        // Resolve current branch identity once — used for both the printed
        // branch header (-b) and the --json report below.
        let branch_state = match &head {
            Some(Ref {
                ref_type: mediagit_versioning::RefType::Symbolic,
                target: Some(branch),
                ..
            }) => BranchState::OnBranch(
                branch
                    .strip_prefix("refs/heads/")
                    .unwrap_or(branch)
                    .to_string(),
            ),
            Some(Ref {
                ref_type: mediagit_versioning::RefType::Direct,
                oid: Some(oid),
                ..
            }) => BranchState::Detached(*oid),
            None => BranchState::NoCommitsYet,
            _ => BranchState::Invalid,
        };

        // Upstream tracking + ahead/behind (M2 plumbing). Offline only — see
        // resolve_upstream. Computed whenever a branch name is known so the
        // --json report always has it available, not just under -b.
        let branch_name_for_upstream = match &branch_state {
            BranchState::OnBranch(name) => Some(name.clone()),
            BranchState::NoCommitsYet => Some("main".to_string()),
            _ => None,
        };
        let config = mediagit_config::Config::load(&repo_root).await?;
        let upstream = match &branch_name_for_upstream {
            Some(name) => resolve_upstream(&config, &refdb, &odb, name, head_oid).await,
            None => None,
        };

        // WT-10: surface an operation left mid-flight.
        //
        // `status` inspected HEAD, the index and the working tree, but never
        // the operation-state files — so a user who hit a conflict, walked
        // away and came back had no way to learn the repository was mid-
        // rebase. The one command whose job is answering "what state am I in?"
        // did not answer it. Printed before the branch line because "you are
        // in the middle of X" outranks "you are on branch Y".
        if !self.json
            && !self.porcelain
            && let Some(op) = in_progress_operation(&storage_path)
        {
            output::warning(&format!("{} in progress", op.label));
            for hint in op.hints {
                println!("  {}", hint);
            }
            let unresolved = Index::load(&repo_root)
                .map(|i| i.unresolved_paths())
                .unwrap_or_default();
            if !unresolved.is_empty() {
                println!("  Unresolved path(s):");
                for p in &unresolved {
                    println!("    {}", p.display());
                }
                println!(
                    "  Review each, then `mediagit add <path>` to accept it \
                     (binary files included — staging is the acknowledgement)."
                );
            }
        }

        // Display current branch (independent of --verbose, which instead
        // enriches the summary line below). Skipped for --json (single JSON
        // document, no interleaved human text).
        if self.branch && !self.json {
            match &branch_state {
                BranchState::OnBranch(name) => {
                    let mut line = format!("On branch: {}", name);
                    if let Some(u) = &upstream {
                        line.push_str(&format_upstream_suffix(u));
                    }
                    output::success(&line);
                }
                BranchState::Detached(oid) => {
                    output::info(&format!("HEAD detached at {}", oid));
                }
                BranchState::NoCommitsYet => {
                    // Empty repo: HEAD doesn't exist yet, infer branch from init config
                    let mut line = "On branch: main (no commits yet)".to_string();
                    if let Some(u) = &upstream {
                        line.push_str(&format_upstream_suffix(u));
                    }
                    output::info(&line);
                }
                BranchState::Invalid => {
                    output::warning("HEAD reference is invalid");
                }
            }
        }

        // Load index for file comparison (ISS-005 fix)
        let index = Index::load(&repo_root)?;

        // Scan working directory, collecting ignored files separately
        let mut ignored_files: HashSet<PathBuf> = HashSet::new();
        let matcher = IgnoreMatcher::new(&repo_root).ok();
        let working_files =
            self.scan_working_directory(&repo_root, &matcher, &mut ignored_files)?;

        // Get HEAD commit tree for comparison (index is cleared after commit)
        let mut head_files: HashMap<PathBuf, Oid> = HashMap::new();
        if let Ok(head_oid) = refdb.resolve("HEAD").await
            && let Ok(commit_data) = odb.read(&head_oid).await
            && let Ok(commit) = mediagit_versioning::format::deserialize::<
                mediagit_versioning::Commit,
            >(&commit_data)
            && let Ok(tree_data) = odb.read(&commit.tree).await
            && let Ok(tree) =
                mediagit_versioning::format::deserialize::<mediagit_versioning::Tree>(&tree_data)
        {
            for entry in tree.iter() {
                head_files.insert(PathBuf::from(&entry.name), entry.oid);
            }
        }

        // Build index file map (path -> oid) for staged changes
        let mut index_files: HashMap<PathBuf, Oid> = HashMap::new();
        for entry in index.entries() {
            index_files.insert(entry.path.clone(), entry.oid);
        }

        // OPTIMIZATION: Parallel modified files detection with Rayon
        // Convert to vector for parallel iteration
        let head_files_vec: Vec<_> = head_files.iter().collect();

        let modified_files: Vec<PathBuf> = head_files_vec
            .par_iter() // Parallel iterator for multi-core processing
            .filter_map(|(path, head_oid)| {
                // Skip files not in working directory
                if !working_files.contains(*path) {
                    return None;
                }

                // Skip files already in index (staged changes)
                if index_files.contains_key(*path) {
                    return None;
                }

                let full_path = repo_root.join(path);

                // OPTIMIZATION 1: Size-based quick check and streaming for large files
                if let Ok(metadata) = std::fs::metadata(&full_path) {
                    let file_size = metadata.len();
                    // Matches add.rs STREAMING_THRESHOLD — both commands must agree on the
                    // hash path for every file to avoid false "modified" reports.
                    use super::utils::STREAMING_THRESHOLD;

                    // Compute hash - use streaming for large files
                    let working_oid = if file_size >= STREAMING_THRESHOLD {
                        // STREAMING: Use constant-memory hash for large files
                        match Oid::from_file(&full_path) {
                            Ok(oid) => oid,
                            Err(_) => return None,
                        }
                    } else {
                        // IN-MEMORY: Faster for small files
                        if let Ok(content) = std::fs::read(&full_path) {
                            Oid::hash(&content)
                        } else {
                            return None;
                        }
                    };

                    if working_oid != **head_oid {
                        // File is modified
                        return Some((*path).clone());
                    }
                }

                None
            })
            .collect();

        // Detect deleted files (in HEAD, not in working dir, not staged for deletion).
        // Sparse-excluded paths are filtered out here: their absence is
        // expected (outside the checkout cone), not a real deletion — see
        // mediagit_versioning::sparse. Applies to human and --json alike;
        // porcelain runs its own copy above and is untouched by this filter.
        let sparse = SparseFilter::load(&repo_root).unwrap_or_else(|_| SparseFilter::disabled());
        // Staged deletions (via `mediagit rm` / `add -A` picking up a removed
        // file, see Index::mark_deleted) belong under "Changes to be
        // committed"; a deletion that hasn't been staged (still in HEAD,
        // absent from working dir, not in index.deleted_entries) stays under
        // "Changes not staged for commit". Both skip sparse-excluded paths —
        // their absence is expected, not a real deletion.
        let mut deleted_files = Vec::new();
        let mut staged_deletions = Vec::new();
        for path in head_files.keys() {
            if !sparse.is_included(path) {
                continue;
            }
            if index.is_deleted(path) {
                staged_deletions.push(path.clone());
            } else if !working_files.contains(path) && !index_files.contains_key(path) {
                deleted_files.push(path.clone());
            }
        }

        // Detect untracked files (in working dir, not in HEAD, not in index, not ignored)
        let mut untracked_files = Vec::new();
        for path in &working_files {
            if !head_files.contains_key(path)
                && !index_files.contains_key(path)
                && !ignored_files.contains(path)
            {
                untracked_files.push(path.clone());
            }
        }

        // --tracked / --untracked filter which sections are shown (default: both).
        // Requesting one alone hides the other; requesting both (or neither) shows both.
        let show_tracked = !self.untracked || self.tracked;
        let show_untracked = !self.tracked || self.untracked;

        // Porcelain output mode: machine-readable, no colors/emojis/headers
        if self.porcelain {
            if show_tracked {
                // Staged files (new files in index)
                for entry in index.entries() {
                    // Check if it's a new file or modified staged file
                    if head_files.contains_key(&entry.path) {
                        println!("M  {}", entry.path.display());
                    } else {
                        println!("A  {}", entry.path.display());
                    }
                }
                // Staged deletions
                for path in &staged_deletions {
                    println!("D  {}", path.display());
                }
                // Modified unstaged files
                for path in &modified_files {
                    println!(" M {}", path.display());
                }
                // Deleted files
                for path in &deleted_files {
                    println!(" D {}", path.display());
                }
            }
            // Untracked files
            if show_untracked {
                for path in &untracked_files {
                    println!("?? {}", path.display());
                }
            }
            // Ignored files (shown with !! prefix when --ignored is set)
            if self.ignored {
                let mut ignored_sorted: Vec<&PathBuf> = ignored_files.iter().collect();
                ignored_sorted.sort();
                for path in ignored_sorted {
                    println!("!! {}", path.display());
                }
            }
            return Ok(());
        }

        // Build the single-source-of-truth report. Human rendering below
        // reads from `report`, not from the raw collections above; --json
        // serializes `report` directly.
        let mut staged: Vec<StagedFileReport> = index
            .entries()
            .map(|entry| {
                let kind = if head_files.contains_key(&entry.path) {
                    StagedKind::Modified
                } else {
                    StagedKind::Added
                };
                StagedFileReport {
                    path: entry.path.display().to_string(),
                    kind,
                }
            })
            .collect();
        for path in &staged_deletions {
            staged.push(StagedFileReport {
                path: path.display().to_string(),
                kind: StagedKind::Deleted,
            });
        }
        let modified: Vec<String> = modified_files
            .iter()
            .map(|p| p.display().to_string())
            .collect();
        let deleted: Vec<String> = deleted_files
            .iter()
            .map(|p| p.display().to_string())
            .collect();
        let untracked: Vec<String> = untracked_files
            .iter()
            .map(|p| p.display().to_string())
            .collect();
        let mut ignored_sorted: Vec<&PathBuf> = ignored_files.iter().collect();
        ignored_sorted.sort();
        let ignored: Vec<String> = ignored_sorted
            .iter()
            .map(|p| p.display().to_string())
            .collect();

        // Media metadata lines (M5 T3): a `media: ...` summary for every
        // modified/staged file that's a recognized media type, gated by the
        // existing `MEDIAGIT_MEDIA_META` knob. Additive struct field — never
        // touches porcelain, never changes the shape of `staged`/`modified`.
        let media: Vec<MediaLineReport> = if show_tracked && crate::media_meta::media_meta_enabled()
        {
            let mut lines = Vec::new();
            let media_paths: Vec<&String> = staged
                .iter()
                .map(|s| &s.path)
                .chain(modified.iter())
                .collect();
            for path in media_paths {
                let full_path = repo_root.join(path);
                let Ok(meta) = std::fs::metadata(&full_path) else {
                    continue;
                };
                if meta.len() > crate::media_meta::MAX_MEDIA_META_BYTES {
                    continue;
                }
                let Ok(data) = tokio::fs::read(&full_path).await else {
                    continue;
                };
                if let Some(summary) = crate::media_meta::media_summary_line(&data, path).await {
                    lines.push(MediaLineReport {
                        path: path.clone(),
                        summary,
                    });
                }
            }
            lines
        } else {
            Vec::new()
        };

        let staged_count = if show_tracked { staged.len() } else { 0 };
        let modified_count = if show_tracked { modified.len() } else { 0 };
        let deleted_count = if show_tracked { deleted.len() } else { 0 };
        let untracked_count = if show_untracked { untracked.len() } else { 0 };

        let total_size = if self.verbose {
            let mut total_size: u64 = 0;
            if show_tracked {
                for entry in index.entries() {
                    total_size += entry.size;
                }
                for path in &modified_files {
                    total_size += std::fs::metadata(repo_root.join(path))
                        .map(|m| m.len())
                        .unwrap_or(0);
                }
            }
            if show_untracked {
                for path in &untracked_files {
                    total_size += std::fs::metadata(repo_root.join(path))
                        .map(|m| m.len())
                        .unwrap_or(0);
                }
            }
            Some(total_size)
        } else {
            None
        };

        let branch_report = BranchReport {
            name: branch_name_for_upstream.clone(),
            detached: matches!(branch_state, BranchState::Detached(_)),
            detached_oid: match &branch_state {
                BranchState::Detached(oid) => Some(oid.to_string()),
                _ => None,
            },
            has_commits,
            upstream,
        };

        let report = StatusReport {
            format_version: 1,
            branch: branch_report,
            staged,
            modified,
            deleted,
            untracked,
            ignored,
            media,
            summary: StatusSummaryReport {
                staged: staged_count,
                modified: modified_count,
                deleted: deleted_count,
                untracked: untracked_count,
                total_size,
            },
        };

        if self.json {
            println!("{}", serde_json::to_string_pretty(&report)?);
            return Ok(());
        }

        // Path -> media summary lookup for the staged/modified loops below
        // (M5 T3). Empty when the knob is off or no changed file is media.
        let media_by_path: HashMap<&str, &str> = report
            .media
            .iter()
            .map(|m| (m.path.as_str(), m.summary.as_str()))
            .collect();

        // Display staged files
        if show_tracked && !report.staged.is_empty() && !self.quiet {
            output::header("Changes to be committed:");
            println!("  (use \"mediagit reset <file>...\" to unstage)");
            println!();

            for entry in &report.staged {
                let (long_prefix, short_prefix) = match entry.kind {
                    StagedKind::Added => ("  new file:   ", "A "),
                    StagedKind::Modified => ("  modified:   ", "M "),
                    StagedKind::Deleted => ("  deleted:    ", "D "),
                };
                let status_prefix = if self.short {
                    short_prefix
                } else {
                    long_prefix
                };
                output::success(&format!("{}{}", status_prefix, entry.path));
                if let Some(summary) = media_by_path.get(entry.path.as_str()) {
                    println!("   {}", style(summary).dim());
                }
            }
            println!();
        }

        // Display modified files (ISS-005 fix)
        if show_tracked && !report.modified.is_empty() && !self.quiet {
            output::header("Changes not staged for commit:");
            println!("  (use \"mediagit add <file>...\" to update what will be committed)");
            println!();

            for path in &report.modified {
                let status_prefix = if self.short { " M " } else { "  modified:   " };
                println!("{}", style(format!("{}{}", status_prefix, path)).yellow());
                if let Some(summary) = media_by_path.get(path.as_str()) {
                    println!("   {}", style(summary).dim());
                }
            }
            println!();
        }

        // Display deleted files (ISS-005 fix)
        if show_tracked && !report.deleted.is_empty() && !self.quiet {
            if report.modified.is_empty() {
                output::header("Changes not staged for commit:");
                println!("  (use \"mediagit add <file>...\" to update what will be committed)");
                println!();
            }

            for path in &report.deleted {
                let status_prefix = if self.short { " D " } else { "  deleted:    " };
                println!("{}", style(format!("{}{}", status_prefix, path)).red());
            }
            println!();
        }

        // Display untracked files (ISS-005 fix)
        if show_untracked && !report.untracked.is_empty() && !self.quiet {
            output::header("Untracked files:");
            println!("  (use \"mediagit add <file>...\" to include in what will be committed)");
            println!();

            for path in &report.untracked {
                println!("{}", style(format!("  {}", path)).cyan());
            }
            println!();
        }

        // Display ignored files (only when --ignored flag is set)
        if self.ignored && !report.ignored.is_empty() && !self.quiet {
            output::header("Ignored files:");
            println!("  (add .mediagitignore negation '!<pattern>' to un-ignore)");
            println!();
            for path in &report.ignored {
                println!("  {}", path);
            }
            println!();
        }

        // Summary counts line (Tier-1 polish)
        if !self.quiet {
            let s = &report.summary;
            if s.staged + s.modified + s.deleted + s.untracked > 0 {
                let mut summary = format!(
                    "{} staged, {} modified, {} deleted, {} untracked",
                    s.staged, s.modified, s.deleted, s.untracked
                );
                if let Some(total_size) = s.total_size {
                    summary.push_str(&format!(" (total size: {})", HumanBytes(total_size)));
                }
                println!("{}", style(summary).dim());
                println!();
            }
        }

        // Display clean status
        if !self.quiet {
            if report.staged.is_empty()
                && report.modified.is_empty()
                && report.deleted.is_empty()
                && report.untracked.is_empty()
            {
                if !report.branch.has_commits {
                    output::info("No commits yet");
                }
                output::info("Nothing to commit, working tree clean");
            } else if !report.staged.is_empty()
                || !report.modified.is_empty()
                || !report.deleted.is_empty()
            {
                // Has changes
            } else if !report.untracked.is_empty() {
                output::info("no changes added to commit (use \"mediagit add\" to track)");
            }
        }

        Ok(())
    }

    // ISS-005 fix: Helper function to scan working directory
    // QA-001: made pub(crate) so `branch switch` can reuse it for the
    // untracked-collision guard instead of duplicating the scan.
    pub(crate) fn scan_working_directory(
        &self,
        repo_root: &Path,
        matcher: &Option<IgnoreMatcher>,
        ignored_files: &mut HashSet<PathBuf>,
    ) -> Result<HashSet<PathBuf>> {
        let mut files = HashSet::new();
        self.scan_directory_recursive(repo_root, repo_root, matcher, ignored_files, &mut files)?;
        Ok(files)
    }

    #[allow(clippy::only_used_in_recursion)]
    fn scan_directory_recursive(
        &self,
        repo_root: &Path,
        current_dir: &Path,
        matcher: &Option<IgnoreMatcher>,
        ignored_files: &mut HashSet<PathBuf>,
        files: &mut HashSet<PathBuf>,
    ) -> Result<()> {
        for entry in std::fs::read_dir(current_dir)? {
            let entry = entry?;
            let path = entry.path();

            // Skip .mediagit directory
            if path.file_name().and_then(|n| n.to_str()) == Some(".mediagit") {
                continue;
            }

            // Check .mediagitignore
            if let Some(m) = matcher
                && let Ok(rel) = path.strip_prefix(repo_root)
            {
                let is_dir = path.is_dir();
                if m.is_ignored(rel, is_dir) {
                    if path.is_file() {
                        // Track ignored files for --ignored output
                        let normalized = PathBuf::from(rel.to_string_lossy().replace('\\', "/"));
                        ignored_files.insert(normalized);
                    }
                    // For dirs: prune entire subtree silently (don't enumerate children)
                    continue;
                }
            }

            if path.is_file() {
                // Store as relative path with normalized separators
                if let Ok(rel_path) = path.strip_prefix(repo_root) {
                    let normalized = PathBuf::from(rel_path.to_string_lossy().replace('\\', "/"));
                    files.insert(normalized);
                }
            } else if path.is_dir() {
                self.scan_directory_recursive(repo_root, &path, matcher, ignored_files, files)?;
            }
        }
        Ok(())
    }
}

/// An operation the repository is part-way through.
struct InProgressOp {
    label: &'static str,
    hints: &'static [&'static str],
}

/// Detect a mid-flight operation from its state file (WT-10).
///
/// Checked in the order a user is most likely to be blocked by. Each state
/// file is owned by a different command and they use four different formats,
/// so presence — not content — is the signal; parsing them here would couple
/// `status` to four schemas it has no other reason to know.
fn in_progress_operation(mediagit_dir: &std::path::Path) -> Option<InProgressOp> {
    let exists = |p: &str| mediagit_dir.join(p).exists();

    if exists("rebase-apply/state.json") {
        return Some(InProgressOp {
            label: "Rebase",
            hints: &[
                "`mediagit rebase --continue` to resume",
                "`mediagit rebase --abort` to restore the original HEAD",
            ],
        });
    }
    if exists("CHERRY_PICK_STATE") {
        return Some(InProgressOp {
            label: "Cherry-pick",
            hints: &[
                "`mediagit cherry-pick --continue` to resume",
                "`mediagit cherry-pick --abort` to restore the original HEAD",
            ],
        });
    }
    if exists("REVERT_STATE") {
        return Some(InProgressOp {
            label: "Revert",
            hints: &[
                "`mediagit revert --continue` to resume",
                "`mediagit revert --abort` to restore the original HEAD",
            ],
        });
    }
    if exists("MERGE_HEAD") {
        return Some(InProgressOp {
            label: "Merge",
            hints: &[
                "`mediagit merge --continue` to conclude the merge",
                "`mediagit merge --abort` to restore the pre-merge state",
            ],
        });
    }
    None
}

// MediaGit - Git for Media Files
// Copyright (C) 2025 MediaGit Contributors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU Affero General Public License for more details.

use super::super::repo::{create_storage_backend, find_repo_root};
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use console::style;
use mediagit_versioning::{CheckoutManager, Commit, LcaFinder, ObjectDatabase, Oid, RefDatabase};
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Find commit that introduced a bug using binary search
#[derive(Parser, Debug)]
pub struct BisectCmd {
    #[command(subcommand)]
    pub command: BisectSubcommand,
}

#[derive(Subcommand, Debug)]
pub enum BisectSubcommand {
    /// Start bisect session
    Start(StartOpts),

    /// Mark current commit as good
    Good(GoodOpts),

    /// Mark current commit as bad
    Bad(BadOpts),

    /// Skip current commit
    Skip(SkipOpts),

    /// Reset bisect session
    Reset(ResetOpts),

    /// Show bisect log
    Log(LogOpts),

    /// Replay bisect log
    Replay(ReplayOpts),
}

#[derive(Parser, Debug)]
pub struct StartOpts {
    /// Bad commit (defaults to HEAD)
    #[arg(value_name = "BAD")]
    pub bad: Option<String>,

    /// Good commit
    #[arg(value_name = "GOOD")]
    pub good: Option<String>,

    /// Reset existing bisect session
    #[arg(long)]
    pub reset: bool,
}

#[derive(Parser, Debug)]
pub struct GoodOpts {
    /// Commit to mark as good (defaults to current)
    #[arg(value_name = "COMMIT")]
    pub commit: Option<String>,
}

#[derive(Parser, Debug)]
pub struct BadOpts {
    /// Commit to mark as bad (defaults to current)
    #[arg(value_name = "COMMIT")]
    pub commit: Option<String>,
}

#[derive(Parser, Debug)]
pub struct SkipOpts {
    /// Commit to skip (defaults to current)
    #[arg(value_name = "COMMIT")]
    pub commit: Option<String>,
}

#[derive(Parser, Debug)]
pub struct ResetOpts {
    /// Commit to reset to (defaults to original HEAD)
    #[arg(value_name = "COMMIT")]
    pub commit: Option<String>,
}

#[derive(Parser, Debug)]
pub struct LogOpts {}

#[derive(Parser, Debug)]
pub struct ReplayOpts {
    /// Log file to replay
    #[arg(value_name = "LOGFILE")]
    pub logfile: PathBuf,
}

impl BisectCmd {
    pub async fn execute(&self) -> Result<()> {
        match &self.command {
            BisectSubcommand::Start(opts) => self.start(opts).await,
            BisectSubcommand::Good(opts) => self.good(opts).await,
            BisectSubcommand::Bad(opts) => self.bad(opts).await,
            BisectSubcommand::Skip(opts) => self.skip(opts).await,
            BisectSubcommand::Reset(opts) => self.reset(opts).await,
            BisectSubcommand::Log(opts) => self.log(opts).await,
            BisectSubcommand::Replay(opts) => self.replay(opts).await,
        }
    }

    async fn start(&self, opts: &StartOpts) -> Result<()> {
        let repo_root = find_repo_root()?;
        let mediagit_dir = repo_root.join(".mediagit");
        let refdb = RefDatabase::new(&mediagit_dir);

        // Check if bisect already in progress
        let state_path = mediagit_dir.join("BISECT_STATE");
        if state_path.exists() && !opts.reset {
            anyhow::bail!(
                "Bisect already in progress. Use --reset to start a new session or 'mediagit bisect reset' to end it."
            );
        }

        // Get current HEAD as original position
        let original_head = refdb.resolve("HEAD").await?;

        // Resolve bad commit (defaults to HEAD)
        let bad_oid = if let Some(ref bad_ref) = opts.bad {
            self.resolve_commit(&refdb, &repo_root, bad_ref).await?
        } else {
            original_head
        };

        let mut state = BisectState {
            original_head: original_head.to_hex(),
            bad: bad_oid.to_hex(),
            good: Vec::new(),
            skip: Vec::new(),
            current: None,
            log: Vec::new(),
        };

        // If good commit provided, mark it and start bisecting
        if let Some(ref good_ref) = opts.good {
            let good_oid = self.resolve_commit(&refdb, &repo_root, good_ref).await?;
            state.good.push(good_oid.to_hex());
            state.log_entry(format!(
                "start: bad={}, good={}",
                bad_oid.to_hex(),
                good_oid.to_hex()
            ));
        } else {
            state.log_entry(format!("start: bad={}", bad_oid.to_hex()));
        }

        println!("{} Bisect session started", style("→").cyan());

        // Narrow the range and check out the next midpoint (or declare complete)
        self.advance(&repo_root, &mut state).await?;

        // Save state
        self.save_bisect_state(&mediagit_dir, &state)?;

        Ok(())
    }

    async fn good(&self, opts: &GoodOpts) -> Result<()> {
        let repo_root = find_repo_root()?;
        let mediagit_dir = repo_root.join(".mediagit");
        let refdb = RefDatabase::new(&mediagit_dir);

        // Load bisect state
        let mut state = self.load_bisect_state(&mediagit_dir)?;

        // Get commit to mark as good: an explicit arg, or else the commit
        // currently checked out for testing (NOT the session-start HEAD).
        let good_oid = self
            .resolve_target(&refdb, &repo_root, &opts.commit, &state)
            .await?;

        state.good.push(good_oid.to_hex());
        state.log_entry(format!("good: {}", good_oid.to_hex()));

        println!(
            "{} Marked {} as good",
            style("✓").green(),
            style(&good_oid.to_hex()[..7]).yellow()
        );

        // Narrow the range and check out the next midpoint (or declare complete)
        self.advance(&repo_root, &mut state).await?;

        // Save state
        self.save_bisect_state(&mediagit_dir, &state)?;

        Ok(())
    }

    async fn bad(&self, opts: &BadOpts) -> Result<()> {
        let repo_root = find_repo_root()?;
        let mediagit_dir = repo_root.join(".mediagit");
        let refdb = RefDatabase::new(&mediagit_dir);

        // Load bisect state
        let mut state = self.load_bisect_state(&mediagit_dir)?;

        // Get commit to mark as bad: an explicit arg, or else the commit
        // currently checked out for testing (NOT the session-start HEAD).
        let bad_oid = self
            .resolve_target(&refdb, &repo_root, &opts.commit, &state)
            .await?;

        state.bad = bad_oid.to_hex();
        state.log_entry(format!("bad: {}", bad_oid.to_hex()));

        println!(
            "{} Marked {} as bad",
            style("✓").green(),
            style(&bad_oid.to_hex()[..7]).yellow()
        );

        // Narrow the range and check out the next midpoint (or declare complete)
        self.advance(&repo_root, &mut state).await?;

        // Save state
        self.save_bisect_state(&mediagit_dir, &state)?;

        Ok(())
    }

    async fn skip(&self, opts: &SkipOpts) -> Result<()> {
        let repo_root = find_repo_root()?;
        let mediagit_dir = repo_root.join(".mediagit");
        let refdb = RefDatabase::new(&mediagit_dir);

        // Load bisect state
        let mut state = self.load_bisect_state(&mediagit_dir)?;

        // Get commit to skip: an explicit arg, or else the commit currently
        // checked out for testing (NOT the session-start HEAD).
        let skip_oid = self
            .resolve_target(&refdb, &repo_root, &opts.commit, &state)
            .await?;

        state.skip.push(skip_oid.to_hex());
        state.log_entry(format!("skip: {}", skip_oid.to_hex()));

        println!(
            "{} Skipped {}",
            style("→").cyan(),
            style(&skip_oid.to_hex()[..7]).yellow()
        );

        // Find next commit to test
        self.advance(&repo_root, &mut state).await?;

        // Save state
        self.save_bisect_state(&mediagit_dir, &state)?;

        Ok(())
    }

    /// Resolve the commit a bare `good`/`bad`/`skip` (no argument) applies to:
    /// the commit currently checked out for testing, falling back to HEAD
    /// only if bisection hasn't checked anything out yet (BUG-ML-3 fix).
    async fn resolve_target(
        &self,
        refdb: &RefDatabase,
        repo_root: &Path,
        commit_ref: &Option<String>,
        state: &BisectState,
    ) -> Result<Oid> {
        if let Some(commit_ref) = commit_ref {
            return self.resolve_commit(refdb, repo_root, commit_ref).await;
        }
        if let Some(ref current) = state.current {
            return Oid::from_hex(current);
        }
        refdb.resolve("HEAD").await
    }

    async fn reset(&self, opts: &ResetOpts) -> Result<()> {
        let repo_root = find_repo_root()?;
        let mediagit_dir = repo_root.join(".mediagit");

        // Load bisect state
        let state = self.load_bisect_state(&mediagit_dir)?;

        // Determine reset target
        let reset_oid = if let Some(ref commit_ref) = opts.commit {
            let refdb = RefDatabase::new(&mediagit_dir);
            self.resolve_commit(&refdb, &repo_root, commit_ref).await?
        } else {
            Oid::from_hex(&state.original_head)?
        };

        // Checkout original HEAD
        let storage = create_storage_backend(&repo_root).await?;
        let odb = ObjectDatabase::with_smart_compression(storage.clone(), 1000);
        let refdb = RefDatabase::new(&mediagit_dir);

        // WT-1: bound deletions to tracked paths. Bisect deliberately does not
        // *refuse* on a dirty tree the way `pull`/`merge` do — it re-checks-out
        // on every step, so refusing would make the feature unusable — but it
        // must never take untracked work with it.
        let head_oid = refdb.resolve("HEAD").await.ok();
        let tracked =
            crate::worktree_guard::tracked_paths(&repo_root, &odb, head_oid.as_ref()).await?;
        let checkout_mgr = CheckoutManager::new(&odb, &repo_root).with_tracked_paths(tracked);
        checkout_mgr.checkout_commit(&reset_oid).await?;

        // Update HEAD reference
        let head = refdb.read("HEAD").await?;
        if let Some(target) = head.target {
            let reset_ref = mediagit_versioning::Ref::new_direct(target, reset_oid);
            refdb.write(&reset_ref).await?;
        }

        // Remove bisect state
        let state_path = mediagit_dir.join("BISECT_STATE");
        std::fs::remove_file(&state_path)?;

        println!("{} Bisect session ended", style("✓").green());
        println!("  Reset to {}", style(&reset_oid.to_hex()[..7]).yellow());

        Ok(())
    }

    async fn log(&self, _opts: &LogOpts) -> Result<()> {
        let repo_root = find_repo_root()?;
        let mediagit_dir = repo_root.join(".mediagit");

        // Load bisect state
        let state = self.load_bisect_state(&mediagit_dir)?;

        println!("{}", style("Bisect Log:").bold());
        for entry in &state.log {
            println!("  {}", entry);
        }

        Ok(())
    }

    async fn replay(&self, opts: &ReplayOpts) -> Result<()> {
        let logfile_content =
            std::fs::read_to_string(&opts.logfile).context("Failed to read log file")?;

        println!("{} Replaying bisect log...", style("→").cyan());

        for line in logfile_content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            // Log format: "YYYY-MM-DD HH:MM:SS: command: args"
            // Strip the timestamp prefix (everything up to and including the first ": ")
            let cmd_part = if let Some(pos) = trimmed.find(": ") {
                trimmed[pos + 2..].trim()
            } else {
                trimmed
            };

            println!("  {} {}", style("→").cyan(), style(cmd_part).dim());

            if let Some(hex) = cmd_part.strip_prefix("good: ") {
                self.good(&GoodOpts {
                    commit: Some(hex.trim().to_string()),
                })
                .await?;
            } else if let Some(hex) = cmd_part.strip_prefix("bad: ") {
                self.bad(&BadOpts {
                    commit: Some(hex.trim().to_string()),
                })
                .await?;
            } else if let Some(hex) = cmd_part.strip_prefix("skip: ") {
                self.skip(&SkipOpts {
                    commit: Some(hex.trim().to_string()),
                })
                .await?;
            } else if let Some(stripped) = cmd_part.strip_prefix("start:") {
                // Parse "start: bad=<hex>, good=<hex>" or "start: bad=<hex>"
                let mut bad = None;
                let mut good = None;
                for part in stripped.split(',') {
                    let part = part.trim();
                    if let Some(h) = part.strip_prefix("bad=") {
                        bad = Some(h.to_string());
                    } else if let Some(h) = part.strip_prefix("good=") {
                        good = Some(h.to_string());
                    }
                }
                self.start(&StartOpts {
                    bad,
                    good,
                    reset: true,
                })
                .await?;
            } else {
                println!("  {} Unknown log entry: {}", style("⚠").yellow(), cmd_part);
            }
        }

        println!("{} Replay complete", style("✓").green());

        Ok(())
    }

    /// Narrow the suspect range and check out the next midpoint to test, or
    /// (once the range has collapsed to nothing left between good and bad)
    /// declare `bad` the first bad commit.
    ///
    /// Does nothing but wait if no good commit has been marked yet, since the
    /// range [good, bad] isn't established (BUG-ML-1 fix: this replaces the
    /// old heuristic that declared completion as soon as any good+bad pair
    /// existed, without ever testing intermediate commits).
    async fn advance(&self, repo_root: &Path, state: &mut BisectState) -> Result<()> {
        if state.good.is_empty() {
            state.current = None;
            println!(
                "  Mark a good commit with: {}",
                style("mediagit bisect good <commit>").yellow()
            );
            return Ok(());
        }

        let storage = create_storage_backend(repo_root).await?;
        let odb = Arc::new(ObjectDatabase::with_smart_compression(
            storage.clone(),
            1000,
        ));
        let lca = LcaFinder::new(odb.clone());

        // WT-1: tracked set for this step, computed once — both checkouts below
        // are bounded by it so untracked work survives every bisect hop.
        let bisect_tracked = {
            let refdb = RefDatabase::new(repo_root.join(".mediagit"));
            let head_oid = refdb.resolve("HEAD").await.ok();
            crate::worktree_guard::tracked_paths(repo_root, &odb, head_oid.as_ref()).await?
        };

        let bad_oid = Oid::from_hex(&state.bad)?;
        let good_oids: Vec<Oid> = state
            .good
            .iter()
            .map(|h| Oid::from_hex(h))
            .collect::<Result<_>>()?;
        let skip_set: HashSet<Oid> = state
            .skip
            .iter()
            .filter_map(|h| Oid::from_hex(h).ok())
            .collect();

        for good_oid in &good_oids {
            if !lca.is_ancestor(good_oid, &bad_oid).await? {
                anyhow::bail!(
                    "Good commit {} is not an ancestor of bad commit {}; bisect range is invalid",
                    good_oid.to_hex(),
                    bad_oid.to_hex()
                );
            }
        }

        // Candidates = ancestors of bad, excluding bad itself, excluding
        // anything already known good (or an ancestor of a good commit),
        // and excluding skipped commits.
        let candidates = Self::compute_candidates(&odb, bad_oid, &good_oids, &skip_set).await?;

        if candidates.is_empty() {
            // Range has collapsed: bad_oid is the first bad commit.
            // WT-1: bounded — see `reset` for why bisect bounds but never refuses.
            let checkout_mgr =
                CheckoutManager::new(&odb, repo_root).with_tracked_paths(bisect_tracked.clone());
            checkout_mgr.checkout_commit(&bad_oid).await?;
            state.current = None;

            println!();
            println!("{}", style("Bisect complete!").green().bold());
            println!();
            println!(
                "{} is the first bad commit",
                style(&bad_oid.to_hex()[..7]).red().bold()
            );
            println!();
            println!("{}", style("Bisect log:").bold());
            for entry in &state.log {
                println!("  {}", entry);
            }
            println!();
            println!(
                "  Run {} to return to your original HEAD",
                style("mediagit bisect reset").yellow()
            );

            return Ok(());
        }

        // Binary search: choose midpoint of the remaining candidates
        let midpoint = candidates.len() / 2;
        let next_oid = candidates[midpoint];

        // Checkout next commit
        // WT-1: bounded — see `reset` for why bisect bounds but never refuses.
        let checkout_mgr = CheckoutManager::new(&odb, repo_root).with_tracked_paths(bisect_tracked);
        checkout_mgr.checkout_commit(&next_oid).await?;

        state.current = Some(next_oid.to_hex());

        println!();
        println!(
            "{} Bisecting: {} revision(s) left to test after this",
            style("→").cyan(),
            style(candidates.len() - 1).yellow()
        );
        println!(
            "  Current commit: {}",
            style(&next_oid.to_hex()[..7]).yellow()
        );
        println!();
        println!("After testing, mark the commit:");
        println!(
            "  {} if commit is good",
            style("mediagit bisect good").green()
        );
        println!("  {} if commit is bad", style("mediagit bisect bad").red());
        println!(
            "  {} if commit cannot be tested",
            style("mediagit bisect skip").yellow()
        );

        Ok(())
    }

    /// Breadth-first walk of `start` and all its ancestors (parents,
    /// grandparents, ...), returned in visitation order (descendant-first).
    async fn bfs_ancestors_ordered(odb: &ObjectDatabase, start: Oid) -> Result<Vec<Oid>> {
        let mut visited = HashSet::new();
        let mut order = Vec::new();
        let mut queue = VecDeque::new();
        queue.push_back(start);
        visited.insert(start);

        while let Some(current) = queue.pop_front() {
            order.push(current);
            if let Ok(commit) = Commit::read(odb, &current).await {
                for parent in &commit.parents {
                    if visited.insert(*parent) {
                        queue.push_back(*parent);
                    }
                }
            }
        }

        Ok(order)
    }

    /// Compute the still-untested commits between `good` and `bad`, ordered
    /// oldest-first (chronological, good -> bad), excluding `bad` itself
    /// (already known bad) and any skipped commits.
    async fn compute_candidates(
        odb: &ObjectDatabase,
        bad: Oid,
        goods: &[Oid],
        skip: &HashSet<Oid>,
    ) -> Result<Vec<Oid>> {
        let bad_order = Self::bfs_ancestors_ordered(odb, bad).await?;

        let mut excluded: HashSet<Oid> = HashSet::new();
        for good in goods {
            excluded.extend(Self::bfs_ancestors_ordered(odb, *good).await?);
        }

        let mut candidates: Vec<Oid> = bad_order
            .into_iter()
            .filter(|oid| *oid != bad && !excluded.contains(oid) && !skip.contains(oid))
            .collect();
        candidates.reverse();

        Ok(candidates)
    }

    fn load_bisect_state(&self, mediagit_dir: &std::path::Path) -> Result<BisectState> {
        let state_path = mediagit_dir.join("BISECT_STATE");

        if !state_path.exists() {
            anyhow::bail!("No bisect session in progress. Use 'mediagit bisect start' to begin.");
        }

        let state_json = std::fs::read_to_string(&state_path)?;
        let state: BisectState = serde_json::from_str(&state_json)?;

        Ok(state)
    }

    fn save_bisect_state(&self, mediagit_dir: &std::path::Path, state: &BisectState) -> Result<()> {
        let state_path = mediagit_dir.join("BISECT_STATE");
        let state_json = serde_json::to_string_pretty(state)?;
        std::fs::write(&state_path, state_json)?;

        Ok(())
    }

    async fn resolve_commit(
        &self,
        refdb: &RefDatabase,
        repo_root: &std::path::Path,
        commit_ref: &str,
    ) -> Result<Oid> {
        // Try full 64-char hex OID first
        if let Ok(oid) = Oid::from_hex(commit_ref) {
            return Ok(oid);
        }

        // Try as branch reference
        let branch_ref = format!("refs/heads/{}", commit_ref);
        if refdb.exists(&branch_ref).await? {
            return refdb.resolve(&branch_ref).await;
        }

        // Try as tag reference
        let tag_ref = format!("refs/tags/{}", commit_ref);
        if refdb.exists(&tag_ref).await? {
            return refdb.resolve(&tag_ref).await;
        }

        // Try short hash prefix matching (e.g., 7-char hashes from `log --oneline`)
        // via the central resolver, which also matches pack-embedded objects
        // (post-`gc --repack`, not just loose ones).
        let looks_like_hex = commit_ref.len() >= 4
            && commit_ref.len() < 64
            && commit_ref.chars().all(|c| c.is_ascii_hexdigit());
        if looks_like_hex && let Ok(storage) = create_storage_backend(repo_root).await {
            let odb = ObjectDatabase::with_smart_compression(storage, 1000);
            if let Ok(oid) = odb.resolve_abbreviated_oid(commit_ref).await {
                return Ok(oid);
            }
        }

        // Try resolving directly via refdb
        refdb.resolve(commit_ref).await.context(format!(
            "Cannot resolve commit reference: {}: Reference not found: {}",
            commit_ref, commit_ref
        ))
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct BisectState {
    original_head: String,
    /// Current bad bound (narrows to a closer ancestor as `bad` marks land).
    bad: String,
    /// Known good commits (bounds); a commit and all its ancestors are good.
    good: Vec<String>,
    skip: Vec<String>,
    /// The commit currently checked out for the user/script to test.
    current: Option<String>,
    log: Vec<String>,
}

impl BisectState {
    fn log_entry(&mut self, entry: String) {
        self.log.push(format!(
            "{}: {}",
            chrono::Utc::now().format("%Y-%m-%d %H:%M:%S"),
            entry
        ));
    }
}

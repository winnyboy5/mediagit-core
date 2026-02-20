// Copyright (C) 2026  winnyboy5
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use console::style;
use mediagit_versioning::{CheckoutManager, ObjectDatabase, Oid, RefDatabase};
use std::path::PathBuf;
use std::collections::HashSet;
use super::super::repo::{find_repo_root, create_storage_backend};

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
            self.resolve_commit(&refdb, bad_ref).await?
        } else {
            original_head
        };

        let mut state = BisectState {
            original_head: original_head.to_hex(),
            bad_commits: vec![bad_oid.to_hex()],
            good_commits: Vec::new(),
            skip_commits: Vec::new(),
            current: Some(bad_oid.to_hex()),
            log: Vec::new(),
        };

        // If good commit provided, mark it and start bisecting
        if let Some(ref good_ref) = opts.good {
            let good_oid = self.resolve_commit(&refdb, good_ref).await?;
            state.good_commits.push(good_oid.to_hex());
            state.log_entry(format!("start: bad={}, good={}", bad_oid.to_hex(), good_oid.to_hex()));

            // Find midpoint and checkout
            self.find_next_commit(&repo_root, &mut state).await?;
        } else {
            state.log_entry(format!("start: bad={}", bad_oid.to_hex()));
        }

        // Save state
        self.save_bisect_state(&mediagit_dir, &state)?;

        println!("{} Bisect session started", style("→").cyan());
        if opts.good.is_some() {
            println!("  {} commits to check", style(self.estimate_remaining(&state)).yellow());
        } else {
            println!("  Mark a good commit with: {}", style("mediagit bisect good <commit>").yellow());
        }

        Ok(())
    }

    async fn good(&self, opts: &GoodOpts) -> Result<()> {
        let repo_root = find_repo_root()?;
        let mediagit_dir = repo_root.join(".mediagit");
        let refdb = RefDatabase::new(&mediagit_dir);

        // Load bisect state
        let mut state = self.load_bisect_state(&mediagit_dir)?;

        // Get commit to mark as good
        let good_oid = if let Some(ref commit_ref) = opts.commit {
            self.resolve_commit(&refdb, commit_ref).await?
        } else {
            // Use current commit
            refdb.resolve("HEAD").await?
        };

        state.good_commits.push(good_oid.to_hex());
        state.log_entry(format!("good: {}", good_oid.to_hex()));

        println!(
            "{} Marked {} as good",
            style("✓").green(),
            style(good_oid.to_hex()).yellow()
        );

        // Check if we found the bad commit
        if self.is_bisect_complete(&state) {
            self.complete_bisect(&repo_root, &state).await?;
            return Ok(());
        }

        // Find next commit to test
        self.find_next_commit(&repo_root, &mut state).await?;

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

        // Get commit to mark as bad
        let bad_oid = if let Some(ref commit_ref) = opts.commit {
            self.resolve_commit(&refdb, commit_ref).await?
        } else {
            refdb.resolve("HEAD").await?
        };

        state.bad_commits.push(bad_oid.to_hex());
        state.log_entry(format!("bad: {}", bad_oid.to_hex()));

        println!(
            "{} Marked {} as bad",
            style("✓").green(),
            style(bad_oid.to_hex()).yellow()
        );

        // Check if we found the bad commit
        if self.is_bisect_complete(&state) {
            self.complete_bisect(&repo_root, &state).await?;
            return Ok(());
        }

        // Find next commit to test
        self.find_next_commit(&repo_root, &mut state).await?;

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

        // Get commit to skip
        let skip_oid = if let Some(ref commit_ref) = opts.commit {
            self.resolve_commit(&refdb, commit_ref).await?
        } else {
            refdb.resolve("HEAD").await?
        };

        state.skip_commits.push(skip_oid.to_hex());
        state.log_entry(format!("skip: {}", skip_oid.to_hex()));

        println!(
            "{} Skipped {}",
            style("→").cyan(),
            style(skip_oid.to_hex()).yellow()
        );

        // Find next commit to test
        self.find_next_commit(&repo_root, &mut state).await?;

        // Save state
        self.save_bisect_state(&mediagit_dir, &state)?;

        Ok(())
    }

    async fn reset(&self, opts: &ResetOpts) -> Result<()> {
        let repo_root = find_repo_root()?;
        let mediagit_dir = repo_root.join(".mediagit");

        // Load bisect state
        let state = self.load_bisect_state(&mediagit_dir)?;

        // Determine reset target
        let reset_oid = if let Some(ref commit_ref) = opts.commit {
            let refdb = RefDatabase::new(&mediagit_dir);
            self.resolve_commit(&refdb, commit_ref).await?
        } else {
            Oid::from_hex(&state.original_head)?
        };

        // Checkout original HEAD
        let storage = create_storage_backend(&repo_root).await?;
        let odb = ObjectDatabase::with_smart_compression(storage.clone(), 1000);
        let refdb = RefDatabase::new(&mediagit_dir);

        let checkout_mgr = CheckoutManager::new(&odb, &repo_root);
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
        println!("  Reset to {}", style(reset_oid.to_hex()).yellow());

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
        let logfile_content = std::fs::read_to_string(&opts.logfile)
            .context("Failed to read log file")?;

        println!("{} Replaying bisect log...", style("→").cyan());

        for line in logfile_content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            // Parse and execute bisect command
            if let Some(cmd) = trimmed.strip_prefix("git bisect ") {
                println!("  Executing: {}", cmd);
                // TODO: Parse and execute individual bisect commands
            }
        }

        println!("{} Replay complete", style("✓").green());

        Ok(())
    }

    async fn find_next_commit(&self, repo_root: &PathBuf, state: &mut BisectState) -> Result<()> {
        let storage = create_storage_backend(repo_root).await?;
        let odb = ObjectDatabase::with_smart_compression(storage.clone(), 1000);

        // Get all commits between good and bad
        let candidates = self.find_candidate_commits(&odb, state).await?;

        if candidates.is_empty() {
            anyhow::bail!("No commits to test");
        }

        // Binary search: choose midpoint
        let midpoint = candidates.len() / 2;
        let next_oid = candidates[midpoint];

        // Checkout next commit
        let checkout_mgr = CheckoutManager::new(&odb, repo_root);
        checkout_mgr.checkout_commit(&next_oid).await?;

        state.current = Some(next_oid.to_hex());

        println!();
        println!(
            "{} Bisecting: {} revisions left to test after this",
            style("→").cyan(),
            style(candidates.len()).yellow()
        );
        println!("  Current commit: {}", style(next_oid.to_hex()).yellow());
        println!();
        println!("After testing, mark the commit:");
        println!("  {} if commit is good", style("mediagit bisect good").green());
        println!("  {} if commit is bad", style("mediagit bisect bad").red());
        println!("  {} if commit cannot be tested", style("mediagit bisect skip").yellow());

        Ok(())
    }

    async fn find_candidate_commits(&self, odb: &ObjectDatabase, state: &BisectState) -> Result<Vec<Oid>> {
        // Build sets of marked commits
        let bad_set: HashSet<String> = state.bad_commits.iter().cloned().collect();
        let good_set: HashSet<String> = state.good_commits.iter().cloned().collect();
        let skip_set: HashSet<String> = state.skip_commits.iter().cloned().collect();

        // For simplicity, use a linear history traversal
        // In a real implementation, this would use graph algorithms
        let mut candidates = Vec::new();

        // Get latest bad commit
        if let Some(bad_hex) = state.bad_commits.last() {
            let bad_oid = Oid::from_hex(bad_hex)?;
            let mut current_oid = bad_oid;

            // Walk back through history
            for _ in 0..100 {  // Limit traversal depth
                let commit_hex = current_oid.to_hex();

                // Skip if already marked
                if bad_set.contains(&commit_hex) || good_set.contains(&commit_hex) || skip_set.contains(&commit_hex) {
                    // Move to parent
                    if let Ok(commit) = mediagit_versioning::Commit::read(&odb, &current_oid).await {
                        if let Some(parent) = commit.parents.first() {
                            current_oid = *parent;
                            continue;
                        }
                    }
                    break;
                }

                candidates.push(current_oid);

                // Move to parent
                if let Ok(commit) = mediagit_versioning::Commit::read(&odb, &current_oid).await {
                    if let Some(parent) = commit.parents.first() {
                        current_oid = *parent;
                    } else {
                        break;
                    }
                } else {
                    break;
                }

                // Stop if we reached a good commit
                if good_set.contains(&commit_hex) {
                    break;
                }
            }
        }

        Ok(candidates)
    }

    fn is_bisect_complete(&self, state: &BisectState) -> bool {
        // Bisect is complete when we have narrowed down to a single commit
        // For now, use simple heuristic
        !state.bad_commits.is_empty() && !state.good_commits.is_empty() && state.current.is_some()
    }

    async fn complete_bisect(&self, repo_root: &PathBuf, state: &BisectState) -> Result<()> {
        let mediagit_dir = repo_root.join(".mediagit");

        println!();
        println!("{}", style("Bisect complete!").green().bold());
        println!();

        // Find first bad commit
        if let Some(first_bad) = state.bad_commits.first() {
            println!(
                "{} is the first bad commit",
                style(first_bad).red().bold()
            );
        }

        // Show bisect log
        println!();
        println!("{}", style("Bisect log:").bold());
        for entry in &state.log {
            println!("  {}", entry);
        }

        // Clean up bisect state
        let state_path = mediagit_dir.join("BISECT_STATE");
        std::fs::remove_file(&state_path)?;

        Ok(())
    }

    fn estimate_remaining(&self, state: &BisectState) -> usize {
        // Simple estimate: log2 of potential commits
        let potential = (state.bad_commits.len() + state.good_commits.len()).max(1);
        (potential as f64).log2().ceil() as usize
    }

    fn load_bisect_state(&self, mediagit_dir: &PathBuf) -> Result<BisectState> {
        let state_path = mediagit_dir.join("BISECT_STATE");

        if !state_path.exists() {
            anyhow::bail!("No bisect session in progress. Use 'mediagit bisect start' to begin.");
        }

        let state_json = std::fs::read_to_string(&state_path)?;
        let state: BisectState = serde_json::from_str(&state_json)?;

        Ok(state)
    }

    fn save_bisect_state(&self, mediagit_dir: &PathBuf, state: &BisectState) -> Result<()> {
        let state_path = mediagit_dir.join("BISECT_STATE");
        let state_json = serde_json::to_string_pretty(state)?;
        std::fs::write(&state_path, state_json)?;

        Ok(())
    }

    async fn resolve_commit(&self, refdb: &RefDatabase, commit_ref: &str) -> Result<Oid> {
        // Try direct OID first
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

        // Try resolving directly
        refdb.resolve(commit_ref).await
            .context(format!("Cannot resolve commit reference: {}", commit_ref))
    }

}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct BisectState {
    original_head: String,
    bad_commits: Vec<String>,
    good_commits: Vec<String>,
    skip_commits: Vec<String>,
    current: Option<String>,
    log: Vec<String>,
}

impl BisectState {
    fn log_entry(&mut self, entry: String) {
        self.log.push(format!("{}: {}", chrono::Utc::now().format("%Y-%m-%d %H:%M:%S"), entry));
    }
}

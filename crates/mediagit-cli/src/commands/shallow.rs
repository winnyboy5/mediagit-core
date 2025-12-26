//! Manage shallow clone state and operations.
//!
//! The `shallow` command provides tools to manage shallow clone repositories,
//! including viewing status, converting to full clones, and deepening history.

use anyhow::{Context, Result};
use clap::Parser;
use mediagit_storage::LocalBackend;
use mediagit_versioning::{CommitWalker, ObjectDatabase, RefDatabase, ShallowDatabase};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;

/// Manage shallow clone state
///
/// Provides commands to work with shallow clone repositories, including
/// checking status, converting to full clones, and deepening history.
#[derive(Parser, Debug)]
#[command(after_help = "EXAMPLES:
    # Check shallow clone status
    mediagit shallow --status

    # Convert shallow clone to full repository
    mediagit shallow --unshallow

    # Extend shallow clone by 10 commits
    mediagit shallow --deepen 10

    # Create shallow clone from existing repo (local only)
    mediagit shallow --create --depth 1

SEE ALSO:
    mediagit-clone(1), mediagit-status(1), mediagit-log(1)")]
pub struct ShallowCmd {
    /// Show shallow clone status
    #[arg(long, conflicts_with_all = ["unshallow", "deepen", "create"])]
    pub status: bool,

    /// Convert shallow clone to full repository
    #[arg(long, conflicts_with_all = ["status", "deepen", "create"])]
    pub unshallow: bool,

    /// Deepen shallow clone by N commits
    #[arg(long, value_name = "N", conflicts_with_all = ["status", "unshallow", "create"])]
    pub deepen: Option<usize>,

    /// Create shallow clone from current repository
    #[arg(long, requires = "depth", conflicts_with_all = ["status", "unshallow", "deepen"])]
    pub create: bool,

    /// Depth for shallow clone creation
    #[arg(long, value_name = "N")]
    pub depth: Option<usize>,

    /// Repository path (defaults to current directory)
    #[arg(short = 'C', long, value_name = "PATH")]
    pub repository: Option<String>,

    /// Quiet mode - minimal output
    #[arg(short, long)]
    pub quiet: bool,
}

impl ShallowCmd {
    pub async fn execute(&self) -> Result<()> {
        use crate::output;

        // Determine repository path
        let repo_path = self.get_repo_path()?;

        // Check if MediaGit repository
        let mediagit_dir = repo_path.join(".mediagit");
        if !mediagit_dir.exists() {
            anyhow::bail!(
                "Not a MediaGit repository: {}",
                repo_path.display()
            );
        }

        // Initialize shallow database
        let shallow_db = ShallowDatabase::new(&mediagit_dir);

        // Execute appropriate subcommand
        if self.status {
            self.show_status(&repo_path, &shallow_db).await?;
        } else if self.unshallow {
            self.unshallow_repository(&repo_path, &shallow_db).await?;
        } else if let Some(depth_increase) = self.deepen {
            self.deepen_repository(&repo_path, &shallow_db, depth_increase).await?;
        } else if self.create {
            let depth = self.depth.expect("depth required for --create");
            self.create_shallow(&repo_path, &shallow_db, depth).await?;
        } else {
            // Default: show status
            self.show_status(&repo_path, &shallow_db).await?;
        }

        Ok(())
    }

    async fn show_status(&self, repo_path: &PathBuf, shallow_db: &ShallowDatabase) -> Result<()> {
        use crate::output;

        if !shallow_db.is_shallow() {
            if !self.quiet {
                output::info("Repository is not a shallow clone");
                println!();
                println!("To create a shallow clone:");
                println!("  mediagit shallow --create --depth N");
                println!();
                println!("Or use 'mediagit clone --depth N' when cloning from remote (future feature)");
            }
            return Ok(());
        }

        let boundaries = shallow_db.read_boundaries()?;

        if !self.quiet {
            output::header("Shallow Clone Status");
            println!();
            println!("Repository: {}", repo_path.display());
            println!("Status: Shallow clone");
            println!("Boundaries: {} commit(s)", boundaries.len());
            println!();

            if !boundaries.is_empty() {
                println!("Boundary commits:");
                for (idx, oid) in boundaries.iter().enumerate() {
                    println!("  {}. {}", idx + 1, oid);
                }
                println!();
            }

            output::success("Repository is in shallow clone mode");
            println!();
            println!("To convert to full clone: mediagit shallow --unshallow");
            println!("To extend history: mediagit shallow --deepen N");
        }

        Ok(())
    }

    async fn unshallow_repository(&self, _repo_path: &PathBuf, shallow_db: &ShallowDatabase) -> Result<()> {
        use crate::output;

        if !shallow_db.is_shallow() {
            anyhow::bail!("Repository is not a shallow clone");
        }

        if !self.quiet {
            output::header("Converting Shallow Clone to Full Repository");
            println!();
        }

        let boundaries = shallow_db.read_boundaries()?;

        info!("Removing shallow boundaries: {} commits", boundaries.len());

        // Clear shallow boundaries (removes .mediagit/shallow file)
        shallow_db.write_boundaries(&std::collections::HashSet::new())?;

        if !self.quiet {
            output::success(&format!(
                "Repository converted to full clone ({} boundaries removed)",
                boundaries.len()
            ));
            println!();
            println!("Note: This operation only removes shallow markers.");
            println!("For remote repositories, use 'mediagit fetch --unshallow' to retrieve full history.");
        }

        Ok(())
    }

    async fn deepen_repository(&self, _repo_path: &PathBuf, shallow_db: &ShallowDatabase, _depth_increase: usize) -> Result<()> {
        use crate::output;

        if !shallow_db.is_shallow() {
            anyhow::bail!("Repository is not a shallow clone");
        }

        if !self.quiet {
            output::info("Deepening shallow clone...");
            println!();
            output::warning("Note: Local deepening not yet implemented");
            println!("This operation requires fetching additional commits from the remote.");
            println!("Use 'mediagit fetch --deepen N' when remote protocol support is added (Week 3-5).");
        }

        Ok(())
    }

    async fn create_shallow(&self, repo_path: &PathBuf, shallow_db: &ShallowDatabase, depth: usize) -> Result<()> {
        use crate::output;

        if depth == 0 {
            anyhow::bail!("Depth must be at least 1");
        }

        if shallow_db.is_shallow() {
            anyhow::bail!("Repository is already a shallow clone. Use --unshallow first.");
        }

        if !self.quiet {
            output::header(&format!("Creating Shallow Clone (depth={})", depth));
            println!();
        }

        // Initialize storage and object database
        let mediagit_dir = repo_path.join(".mediagit");
        let storage: Arc<dyn mediagit_storage::StorageBackend> =
            Arc::new(LocalBackend::new(&mediagit_dir).await?);
        let odb = Arc::new(ObjectDatabase::with_smart_compression(storage, 1000));

        // Get HEAD commit
        let refdb = RefDatabase::new(&mediagit_dir);
        let head_oid = refdb.resolve("HEAD").await
            .context("Failed to resolve HEAD")?;

        info!("Creating shallow clone from HEAD: {}", head_oid);

        // Perform shallow walk
        let mut walker = CommitWalker::new(odb.clone());
        let result = walker.walk_shallow(&head_oid, depth - 1).await
            .context("Failed to perform shallow walk")?;

        info!("Shallow walk complete: {} commits, {} boundaries",
              result.commits.len(), result.shallow_boundaries.len());

        // Write shallow boundaries
        shallow_db.write_boundaries(&result.shallow_boundaries)
            .context("Failed to write shallow boundaries")?;

        if !self.quiet {
            let total_commits = result.commits.len();
            let boundaries = result.shallow_boundaries.len();

            output::success(&format!(
                "Shallow clone created: {} commits at depth={} ({} boundaries)",
                total_commits, depth, boundaries
            ));
            println!();
            println!("Boundary commits:");
            for oid in &result.shallow_boundaries {
                println!("  {}", oid);
            }
            println!();

            if total_commits > 0 {
                let reduction_pct = ((boundaries as f64 / total_commits as f64) * 100.0) as usize;
                println!("Note: Actual storage reduction depends on repository size.");
                println!("Boundary markers: {}% of walked commits", reduction_pct);
            }
        }

        Ok(())
    }

    fn get_repo_path(&self) -> Result<PathBuf> {
        if let Some(ref path) = self.repository {
            Ok(PathBuf::from(path))
        } else if let Ok(path) = std::env::var("MEDIAGIT_REPO") {
            Ok(PathBuf::from(path))
        } else {
            std::env::current_dir().context("Failed to get current directory")
        }
    }
}

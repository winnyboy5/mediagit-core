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

mod commands;
mod output;
mod progress;
mod repo;

use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{generate, Shell};
use commands::*;
use mediagit_observability::{init_tracing, LogFormat};
use std::io;

#[derive(Parser)]
#[command(name = "mediagit")]
#[command(version, about = "Git for Media Files - Optimize Your Media Workflows")]
#[command(
    long_about = "MediaGit is a specialized version control system designed for media files.
It optimizes storage for large media assets while maintaining full Git-like workflow compatibility."
)]
#[command(propagate_version = true)]
#[command(author = "MediaGit Contributors")]
#[command(arg_required_else_help = false)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Enable verbose output
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Suppress output
    #[arg(short, long, global = true)]
    quiet: bool,

    /// Colored output (always|auto|never)
    #[arg(long, global = true, value_name = "WHEN", default_value = "auto")]
    color: String,

    /// Repository path
    #[arg(short = 'C', long, global = true, value_name = "PATH")]
    repository: Option<String>,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new MediaGit repository
    Init(InitCmd),

    /// Clone a repository into a new directory
    Clone(CloneCmd),

    /// Stage file contents for commit
    Add(AddCmd),

    /// Record changes to the repository
    Commit(CommitCmd),

    /// Update remote references
    Push(PushCmd),

    /// Fetch and integrate remote changes
    Pull(PullCmd),

    /// Fetch remote changes without merging
    Fetch(FetchCmd),

    /// Manage remote repositories
    Remote(RemoteCmd),

    /// Manage branches
    Branch(BranchCmd),

    /// Manage tags
    Tag(TagCmd),

    /// Merge branches
    Merge(MergeCmd),

    /// Rebase commits
    Rebase(RebaseCmd),

    /// Apply changes from existing commits
    #[command(name = "cherry-pick")]
    CherryPick(CherryPickCmd),

    /// Stash changes in working directory
    Stash(StashCmd),

    /// Find commit that introduced a bug using binary search
    Bisect(BisectCmd),

    /// Show commit history
    Log(LogCmd),

    /// Show changes between commits
    Diff(DiffCmd),

    /// Show object information
    Show(ShowCmd),

    /// Show working tree status
    Status(StatusCmd),

    /// Clean up repository and optimize storage
    Gc(GcCmd),

    /// Check repository integrity
    Fsck(FsckCmd),

    /// Verify commits and signatures
    Verify(VerifyCmd),

    /// Show repository statistics
    Stats(StatsCmd),

    /// Git filter driver operations (clean/smudge)
    #[command(subcommand)]
    Filter(FilterCmd),

    /// Install MediaGit filter driver
    Install(InstallCmd),

    /// Track patterns with MediaGit
    Track(TrackCmd),

    /// Untrack patterns
    Untrack(UntrackCmd),

    /// Show version information
    Version,

    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize structured logging
    if !cli.quiet {
        let level = if cli.verbose { "debug" } else { "info" };
        let format = LogFormat::Pretty; // Pretty format for CLI output

        // Initialize with appropriate log level
        init_tracing(format, Some(level)).ok(); // Ignore errors if already initialized
    }

    // Handle color output
    match cli.color.as_str() {
        "never" => console::set_colors_enabled(false),
        "always" => console::set_colors_enabled(true),
        "auto" => {
            // Auto-detect based on terminal capabilities
        }
        _ => {
            eprintln!("Invalid color option: {}", cli.color);
            std::process::exit(1);
        }
    }

    // Set repository path if provided
    if let Some(repo_path) = cli.repository {
        std::env::set_var("MEDIAGIT_REPO", repo_path);
    }

    // Execute command
    let result = match cli.command {
        Some(Commands::Init(cmd)) => cmd.execute().await,
        Some(Commands::Clone(cmd)) => cmd.execute().await,
        Some(Commands::Add(cmd)) => cmd.execute().await,
        Some(Commands::Commit(cmd)) => cmd.execute().await,
        Some(Commands::Push(cmd)) => cmd.execute().await,
        Some(Commands::Pull(cmd)) => cmd.execute().await,
        Some(Commands::Fetch(cmd)) => cmd.execute().await,
        Some(Commands::Remote(cmd)) => cmd.execute().await,
        Some(Commands::Branch(cmd)) => cmd.execute().await,
        Some(Commands::Tag(cmd)) => {
            let repo_path = std::env::current_dir()?;
            cmd.execute(repo_path).await
        }
        Some(Commands::Merge(cmd)) => cmd.execute().await,
        Some(Commands::Rebase(cmd)) => cmd.execute().await,
        Some(Commands::CherryPick(cmd)) => cmd.execute().await,
        Some(Commands::Stash(cmd)) => cmd.execute().await,
        Some(Commands::Bisect(cmd)) => cmd.execute().await,
        Some(Commands::Log(cmd)) => cmd.execute().await,
        Some(Commands::Diff(cmd)) => cmd.execute().await,
        Some(Commands::Show(cmd)) => cmd.execute().await,
        Some(Commands::Status(cmd)) => cmd.execute().await,
        Some(Commands::Gc(cmd)) => cmd.execute().await,
        Some(Commands::Fsck(cmd)) => cmd.execute().await,
        Some(Commands::Verify(cmd)) => cmd.execute().await,
        Some(Commands::Stats(cmd)) => cmd.execute().await,
        Some(Commands::Filter(cmd)) => cmd.execute(),
        Some(Commands::Install(cmd)) => cmd.execute(),
        Some(Commands::Track(cmd)) => cmd.execute(),
        Some(Commands::Untrack(cmd)) => cmd.execute(),
        Some(Commands::Version) => {
            print_version();
            Ok(())
        }
        Some(Commands::Completions { shell }) => {
            generate_completions(shell)?;
            Ok(())
        }
        None => {
            output::header("MediaGit - Git for Media Files");
            println!();
            println!("Usage: mediagit [OPTIONS] <COMMAND>");
            println!();
            println!("Available commands:");
            println!("  init         Initialize a new MediaGit repository");
            println!("  clone        Clone a repository into a new directory");
            println!("  add          Stage file contents for commit");
            println!("  commit       Record changes to the repository");
            println!("  push         Update remote references");
            println!("  pull         Fetch and integrate remote changes");
            println!("  fetch        Fetch remote changes without merging");
            println!("  remote       Manage remote repositories");
            println!("  branch       Manage branches");
            println!("  tag          Manage tags");
            println!("  merge        Merge branches");
            println!("  rebase       Rebase commits");
            println!("  cherry-pick  Apply changes from existing commits");
            println!("  stash        Stash changes in working directory");
            println!("  bisect       Find commit that introduced a bug using binary search");
            println!("  log          Show commit history");
            println!("  diff         Show changes between commits");
            println!("  show         Show object information");
            println!("  status       Show working tree status");
            println!("  gc           Clean up repository");
            println!("  fsck         Check repository integrity");
            println!("  verify       Verify commits and signatures");
            println!("  stats        Show repository statistics");
            println!();
            println!("Git Integration:");
            println!("  filter       Git filter driver operations (clean/smudge)");
            println!("  install      Install MediaGit filter driver");
            println!("  track        Track file patterns with MediaGit");
            println!("  untrack      Untrack file patterns");
            println!();
            println!("Run 'mediagit <COMMAND> --help' for command-specific help");
            Ok(())
        }
    };

    // Handle errors
    if let Err(e) = result {
        output::error(&format!("Error: {:#}", e));
        std::process::exit(1);
    }

    Ok(())
}

fn print_version() {
    println!("mediagit {}", env!("CARGO_PKG_VERSION"));
    println!("rust-version: {}", env!("CARGO_PKG_RUST_VERSION"));
    println!("license: {}", env!("CARGO_PKG_LICENSE"));
}

fn generate_completions(shell: Shell) -> Result<()> {
    let mut cmd = Cli::command();
    generate(shell, &mut cmd, "mediagit", &mut io::stdout());
    Ok(())
}

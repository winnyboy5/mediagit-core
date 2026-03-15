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

#![allow(missing_docs)] // binary crate — documentation is in book/ not rustdoc

mod commands;
mod output;
mod progress;
mod repo;

use anyhow::{Context, Result};
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

    /// Show reference logs (reflog)
    Reflog(ReflogCmd),

    /// Reset current HEAD to specified state
    Reset(ResetCmd),

    /// Revert commits by creating inverse commits
    Revert(RevertCmd),

    /// Show version information
    Version,

    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
    },
}

/// Preprocess CLI arguments for git-compatibility shims:
///
/// 1. `mediagit checkout [-b] <ref>` → `mediagit branch switch [-c] <ref>`
/// 2. `mediagit log -5` → `mediagit log -n 5`
fn preprocess_args(args: Vec<String>) -> Vec<String> {
    // Find the first non-flag positional arg (the subcommand), skipping the binary name.
    let subcmd_pos = args
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, arg)| !arg.starts_with('-'))
        .map(|(i, _)| i);

    if let Some(pos) = subcmd_pos {
        let subcmd = args[pos].as_str();

        // 1. `checkout` / `co` → `branch switch`
        if subcmd == "checkout" || subcmd == "co" {
            let mut result = Vec::with_capacity(args.len() + 2);
            result.extend_from_slice(&args[..pos]); // binary name + any leading flags
            result.push("branch".to_string());
            result.push("switch".to_string());
            for arg in &args[pos + 1..] {
                // git uses -b / --branch to create-and-switch; branch switch uses -c / --create
                if arg == "-b" || arg == "--branch" {
                    result.push("-c".to_string());
                } else {
                    result.push(arg.clone());
                }
            }
            return result;
        }

        // 2. `log -N` / `reflog -N` shorthand → `log -n N` / `reflog -n N`
        if subcmd == "log" || subcmd == "reflog" {
            let cmd_idx = pos;
            let mut result = Vec::with_capacity(args.len() + 2);
            for (i, arg) in args.into_iter().enumerate() {
                if i > cmd_idx {
                    if let Some(rest) = arg.strip_prefix('-') {
                        if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
                            result.push("-n".to_string());
                            result.push(rest.to_string());
                            continue;
                        }
                    }
                }
                result.push(arg);
            }
            return result;
        }

        // 3. Bare subcommands that should default to their `list` action.
        //    `branch` → `branch list`, `tag` → `tag list`, `remote` → `remote list`
        let list_default: Option<(&str, &[&str])> = match subcmd {
            "branch" => Some((
                "branch",
                &[
                    "list", "create", "delete", "rename", "show", "switch", "checkout", "co",
                    "merge", "protect", "help",
                ][..],
            )),
            "tag" => Some((
                "tag",
                &["list", "create", "delete", "show", "verify", "help"][..],
            )),
            "remote" => Some((
                "remote",
                &["list", "add", "remove", "rename", "show", "set-url", "help"][..],
            )),
            _ => None,
        };
        if let Some((_cmd, known_subcmds)) = list_default {
            let next_positional = args[pos + 1..]
                .iter()
                .find(|a| !a.starts_with('-'))
                .map(|s| s.as_str());
            let needs_default = match next_positional {
                None => true,
                Some(s) => !known_subcmds.contains(&s),
            };
            if needs_default {
                let mut result = Vec::with_capacity(args.len() + 1);
                result.extend_from_slice(&args[..pos + 1]); // include the subcommand
                result.push("list".to_string());
                result.extend(args[pos + 1..].iter().cloned());
                return result;
            }
        }
    }

    args
}

fn main() {
    // Preprocess args to support git-style -N shorthand (e.g., log -5 → log -n 5)
    let args = preprocess_args(std::env::args().collect());
    // Parse CLI args on the main thread (lightweight, no async needed)
    let cli = Cli::parse_from(args);

    // Run async work on a thread with 8MB stack to handle deeply nested
    // async futures (merge engine → LCA finder → checkout → recursive tree).
    // Windows default main thread stack is 1MB which is insufficient.
    const STACK_SIZE: usize = 8 * 1024 * 1024; // 8MB

    let builder = std::thread::Builder::new()
        .name("mediagit-main".into())
        .stack_size(STACK_SIZE);

    let handler = builder
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Failed to create tokio runtime")
                .block_on(async_main(cli))
        })
        .expect("Failed to spawn main thread");

    match handler.join() {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            output::error(&format!("Error: {:#}", e));
            std::process::exit(1);
        }
        Err(_) => {
            eprintln!("Fatal: mediagit panicked");
            std::process::exit(2);
        }
    }
}

async fn async_main(cli: Cli) -> Result<()> {
    // Suppress INFO logs for machine-readable output modes (--json, --prometheus)
    // to avoid mixing log lines with structured data even when stderr is redirected
    let machine_readable = matches!(
        &cli.command,
        Some(Commands::Stats(cmd)) if cmd.json || cmd.prometheus
    );

    // Initialize structured logging
    if !cli.quiet && !machine_readable {
        let level = if cli.verbose { "info" } else { "warn" };
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

    // Set repository path if provided via -C flag.
    // Change working directory so that:
    //   - relative path arguments (e.g. `mediagit -C /repo add file.mp4`) resolve correctly
    //   - `init` without a positional path creates the repo in the -C directory
    if let Some(repo_path) = &cli.repository {
        std::env::set_current_dir(repo_path)
            .with_context(|| format!("Cannot change to directory '{}'", repo_path))?;
        // Set MEDIAGIT_REPO to the resolved absolute path so find_repo_root() works
        // even when called from code that doesn't inspect current_dir() directly.
        if let Ok(cwd) = std::env::current_dir() {
            std::env::set_var("MEDIAGIT_REPO", cwd);
        }
    }

    // Execute command
    match cli.command {
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
        Some(Commands::Reflog(cmd)) => cmd.execute().await,
        Some(Commands::Reset(cmd)) => cmd.execute().await,
        Some(Commands::Revert(cmd)) => cmd.execute().await,
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
            println!("Run 'mediagit <COMMAND> --help' for command-specific help");
            Ok(())
        }
    }
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

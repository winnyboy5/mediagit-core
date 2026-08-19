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
use clap::Parser;
use console::style;
use mediagit_versioning::{Commit, ObjectDatabase, Oid, RefDatabase, Tag, Tree, resolve_revision};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Show commit history
///
/// Display commits in reverse chronological order. The output can be filtered
/// by author, date range, or commit message pattern.
#[derive(Parser, Debug)]
#[command(after_help = "EXAMPLES:
    # Show commit history
    mediagit log

    # Show last 10 commits
    mediagit log -n 10

    # Show commits in one-line format
    mediagit log --oneline

    # Show commits with statistics
    mediagit log --stat

    # Show commits by specific author
    mediagit log --author \"John Doe\"

    # Show commits matching pattern
    mediagit log --grep \"fix bug\"

    # Show commits for specific files
    mediagit log -- path/to/file.psd

    # Show commits in date range
    mediagit log --since \"2024-01-01\" --until \"2024-12-31\"

SEE ALSO:
    mediagit-show(1), mediagit-diff(1), mediagit-reflog(1)")]
pub struct LogCmd {
    /// Revision range (e.g., main..feature, v1.0..v2.0)
    #[arg(value_name = "REVISION")]
    pub revision: Option<String>,

    /// Maximum number of commits to show
    #[arg(short = 'n', long, value_name = "NUM")]
    pub max_count: Option<usize>,

    /// Skip N commits
    #[arg(long, value_name = "NUM")]
    pub skip: Option<usize>,

    /// Show abbreviated commit hash
    #[arg(long)]
    pub oneline: bool,

    /// Show graph representation
    #[arg(long)]
    pub graph: bool,

    /// Show commit statistics
    #[arg(long)]
    pub stat: bool,

    /// Show patches
    #[arg(short = 'p', long)]
    pub patch: bool,

    /// Show commits by author
    #[arg(long, value_name = "PATTERN")]
    pub author: Option<String>,

    /// Show commits with matching message
    #[arg(long, value_name = "PATTERN")]
    pub grep: Option<String>,

    /// Show commits since date
    #[arg(long, value_name = "DATE")]
    pub since: Option<String>,

    /// Show commits until date
    #[arg(long, value_name = "DATE")]
    pub until: Option<String>,

    /// Show only commits affecting these paths
    #[arg(value_name = "PATHS")]
    pub paths: Vec<String>,

    /// Show verbose output
    #[arg(short, long)]
    pub verbose: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,

    /// Format commits using a template string
    #[arg(long, value_name = "TMPL")]
    pub format: Option<String>,

    /// Show commits from all branches
    #[arg(long)]
    pub all: bool,
}

impl LogCmd {
    pub async fn execute(&self) -> Result<()> {
        if self.quiet {
            return Ok(());
        }

        let repo_root = find_repo_root()?;
        let storage_path = repo_root.join(".mediagit");
        let storage = create_storage_backend(&repo_root).await?;
        let refdb = RefDatabase::new(&storage_path);
        let odb = ObjectDatabase::with_smart_compression(storage, 1000);

        // Get starting commit OID
        let start_oid = if let Some(revision) = &self.revision {
            let oid = resolve_revision(revision, &refdb, &odb)
                .await
                .with_context(|| format!("Invalid revision: {}", revision))?;
            // Peel annotated tags (resolve to a tag object, not a commit).
            Self::peel_to_commit(oid, &odb).await?
        } else {
            // Use HEAD
            match refdb.read("HEAD").await {
                Ok(head) => {
                    match head.oid {
                        Some(oid) => oid,
                        None => {
                            // HEAD might be symbolic, resolve it
                            if let Some(target) = head.target {
                                match refdb.read(&target).await {
                                    Ok(target_ref) => match target_ref.oid {
                                        Some(oid) => oid,
                                        None => {
                                            println!("{}", style("No commits yet").dim());
                                            return Ok(());
                                        }
                                    },
                                    Err(_) => {
                                        // Branch doesn't exist yet (e.g., refs/heads/main on fresh repo)
                                        println!("{}", style("No commits yet").dim());
                                        return Ok(());
                                    }
                                }
                            } else {
                                println!("{}", style("No commits yet").dim());
                                return Ok(());
                            }
                        }
                    }
                }
                Err(_) => {
                    // HEAD doesn't exist yet
                    println!("{}", style("No commits yet").dim());
                    return Ok(());
                }
            }
        };

        // Traverse commit history
        let mut commits_to_show = Vec::new();
        let mut visited = HashSet::new();
        let mut stack = vec![start_oid];

        // When --all is requested, seed the stack with every branch tip
        if self.all {
            let branch_refs = refdb.list_branches().await.unwrap_or_default();
            for branch_ref in &branch_refs {
                if let Ok(r) = refdb.read(branch_ref).await
                    && let Some(oid) = r.oid
                    && !stack.contains(&oid)
                {
                    stack.push(oid);
                }
            }
        }

        // Parsed once, before the walk, so a malformed date fails immediately
        // rather than after streaming part of the history.
        let since_bound = match self.since.as_deref() {
            Some(raw) => Some(parse_date_bound(raw, "--since")?),
            None => None,
        };
        let until_bound = match self.until.as_deref() {
            Some(raw) => Some(parse_date_bound(raw, "--until")?),
            None => None,
        };
        if let (Some(a), Some(b)) = (since_bound, until_bound)
            && a > b
        {
            anyhow::bail!("--since ({a}) is after --until ({b}); no commit can match");
        }

        while let Some(oid) = stack.pop() {
            if visited.contains(&oid) {
                continue;
            }
            visited.insert(oid);

            // Read commit object
            let data = odb.read(&oid).await?;
            let commit = Commit::deserialize(&data)
                .with_context(|| format!("Failed to deserialize commit {}", oid))?;

            // Apply filters
            if let Some(author_pattern) = &self.author
                && !commit.author.name.contains(author_pattern)
                && !commit.author.email.contains(author_pattern)
            {
                // Add parents to stack even if this commit is filtered
                for parent in &commit.parents {
                    if !visited.contains(parent) {
                        stack.push(*parent);
                    }
                }
                continue;
            }

            if let Some(grep_pattern) = &self.grep
                && !commit.message.contains(grep_pattern)
            {
                // Add parents to stack even if this commit is filtered
                for parent in &commit.parents {
                    if !visited.contains(parent) {
                        stack.push(*parent);
                    }
                }
                continue;
            }

            // UX-5: --since/--until were declared, demonstrated in this
            // command's own help text, and never read — so a date-bounded log
            // silently returned the whole history. Filter on the author
            // timestamp, which is the date `log` displays.
            let ts = commit.author.timestamp;
            let out_of_range =
                since_bound.is_some_and(|b| ts < b) || until_bound.is_some_and(|b| ts > b);
            if out_of_range {
                // Parents still get walked: an out-of-range commit does not
                // mean its ancestors are, and history is not sorted by date.
                for parent in &commit.parents {
                    if !visited.contains(parent) {
                        stack.push(*parent);
                    }
                }
                continue;
            }

            commits_to_show.push((oid, commit.clone()));

            // Add parents to stack
            for parent in &commit.parents {
                if !visited.contains(parent) {
                    stack.push(*parent);
                }
            }

            // Check if we've reached the limit
            if let Some(max_count) = self.max_count
                && commits_to_show.len() >= max_count + self.skip.unwrap_or(0)
            {
                break;
            }
        }

        // Apply skip
        if let Some(skip) = self.skip {
            if skip < commits_to_show.len() {
                commits_to_show.drain(0..skip);
            } else {
                commits_to_show.clear();
            }
        }

        // Display commits
        if commits_to_show.is_empty() {
            println!("{}", style("No commits to show").dim());
            return Ok(());
        }

        for (oid, commit) in commits_to_show {
            if let Some(format_tmpl) = &self.format {
                // Custom format template
                let output = Self::format_commit(format_tmpl, &oid, &commit);
                println!("{}", output);
            } else if self.oneline {
                // One-line format
                let short_oid = &oid.to_string()[..7];
                let short_msg = commit.message.lines().next().unwrap_or("");
                if self.graph {
                    println!("* {} {}", style(short_oid).yellow(), short_msg);
                } else {
                    println!("{} {}", style(short_oid).yellow(), short_msg);
                }
            } else if self.graph {
                // Graph format
                println!(
                    "* {} {}",
                    style("commit").yellow().bold(),
                    style(oid).yellow()
                );
                println!("| Author: {} <{}>", commit.author.name, commit.author.email);
                println!("| Date:   {}", commit.author.timestamp);
                println!("|");
                for line in commit.message.lines() {
                    println!("|     {}", line);
                }
                println!("|");
            } else {
                // Full format
                println!(
                    "{} {}",
                    style("commit").yellow().bold(),
                    style(oid).yellow()
                );
                println!("Author: {} <{}>", commit.author.name, commit.author.email);
                println!("Date:   {}", commit.author.timestamp);
                println!();
                for line in commit.message.lines() {
                    println!("    {}", line);
                }
                println!();
            }

            // --stat: show file change statistics
            if self.stat {
                // Get current commit's tree files
                let current_tree_files = Self::get_tree_file_list(&odb, &commit.tree)
                    .await
                    .unwrap_or_default();

                // Get parent's tree files (empty if no parent / root commit)
                let parent_tree_files = if let Some(parent_oid) = commit.parents.first() {
                    match odb.read(parent_oid).await {
                        Ok(parent_data) => match Commit::deserialize(&parent_data) {
                            Ok(parent_commit) => {
                                Self::get_tree_file_list(&odb, &parent_commit.tree)
                                    .await
                                    .unwrap_or_default()
                            }
                            _ => HashMap::new(),
                        },
                        _ => HashMap::new(),
                    }
                } else {
                    HashMap::new()
                };

                let mut added = Vec::new();
                let mut modified = Vec::new();
                let mut deleted = Vec::new();

                // Files in current but not in parent = added
                // Files in both but different OID = modified
                for (path, current_oid) in &current_tree_files {
                    match parent_tree_files.get(path) {
                        Some(parent_oid) if parent_oid != current_oid => {
                            modified.push(path.clone());
                        }
                        None => {
                            added.push(path.clone());
                        }
                        _ => {} // unchanged
                    }
                }

                // Files in parent but not in current = deleted
                for path in parent_tree_files.keys() {
                    if !current_tree_files.contains_key(path) {
                        deleted.push(path.clone());
                    }
                }

                let total_changes = added.len() + modified.len() + deleted.len();
                if total_changes > 0 {
                    for path in &added {
                        println!(" {} | {}", path.display(), style("new file").green());
                    }
                    for path in &modified {
                        println!(" {} | {}", path.display(), style("modified").yellow());
                    }
                    for path in &deleted {
                        println!(" {} | {}", path.display(), style("deleted").red());
                    }
                    println!(
                        " {} file(s) changed, {} added, {} modified, {} deleted",
                        total_changes,
                        added.len(),
                        modified.len(),
                        deleted.len()
                    );
                    println!();
                }
            }
        }

        Ok(())
    }

    /// Follow a Tag object to its target, repeating until a Commit is
    /// reached (branches/OIDs already point at a commit and return
    /// immediately).
    async fn peel_to_commit(mut oid: Oid, odb: &ObjectDatabase) -> Result<Oid> {
        loop {
            let data = odb
                .read(&oid)
                .await
                .context(format!("Failed to read object {}", oid))?;
            if Commit::deserialize(&data).is_ok() {
                return Ok(oid);
            }
            match Tag::deserialize(&data) {
                Ok(tag) => oid = tag.target,
                Err(_) => anyhow::bail!("Object {} is not a commit or tag", oid),
            }
        }
    }

    /// Format a commit using a template string
    /// Supported placeholders:
    /// %H - full commit OID hex
    /// %h - first 7 hex chars of commit OID
    /// %s - subject (first line of message)
    /// %aN - author name
    /// %an - author name (alias of %aN)
    /// %ae - author email
    /// %ad - author date
    /// %n - literal newline
    /// %% - literal %
    fn format_commit(tmpl: &str, oid: &Oid, commit: &Commit) -> String {
        let mut result = String::new();
        let mut chars = tmpl.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch == '%' {
                if let Some(&next_ch) = chars.peek() {
                    chars.next(); // consume the next char
                    match next_ch {
                        'H' => result.push_str(&oid.to_string()),
                        'h' => {
                            let full_oid = oid.to_string();
                            result.push_str(&full_oid[..7.min(full_oid.len())]);
                        }
                        's' => {
                            let subject = commit.message.lines().next().unwrap_or("");
                            result.push_str(subject);
                        }
                        'a' => {
                            if let Some(&second) = chars.peek() {
                                chars.next(); // consume the second char
                                match second {
                                    'N' | 'n' => result.push_str(&commit.author.name),
                                    'e' => result.push_str(&commit.author.email),
                                    'd' => result.push_str(&commit.author.timestamp.to_string()),
                                    _ => {
                                        result.push('%');
                                        result.push('a');
                                        result.push(second);
                                    }
                                }
                            } else {
                                result.push('%');
                                result.push('a');
                            }
                        }
                        'n' => result.push('\n'),
                        '%' => result.push('%'),
                        _ => {
                            result.push('%');
                            result.push(next_ch);
                        }
                    }
                } else {
                    result.push('%');
                }
            } else {
                result.push(ch);
            }
        }

        result
    }

    /// Helper to get a flat map of file paths to OIDs from a tree
    async fn get_tree_file_list(
        odb: &ObjectDatabase,
        tree_oid: &Oid,
    ) -> Result<HashMap<PathBuf, Oid>> {
        let mut files = HashMap::new();
        Self::walk_tree(odb, tree_oid, &PathBuf::new(), &mut files).await?;
        Ok(files)
    }

    /// Recursively walk a tree, collecting file entries
    fn walk_tree<'a>(
        odb: &'a ObjectDatabase,
        tree_oid: &'a Oid,
        prefix: &'a Path,
        files: &'a mut HashMap<PathBuf, Oid>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + 'a>> {
        Box::pin(async move {
            let tree_data = odb.read(tree_oid).await?;
            let tree: Tree = mediagit_versioning::format::deserialize(&tree_data)?;

            for entry in tree.iter() {
                let entry_path = prefix.join(&entry.name);
                match entry.mode {
                    mediagit_versioning::FileMode::Directory => {
                        Self::walk_tree(odb, &entry.oid, &entry_path, files).await?;
                    }
                    _ => {
                        files.insert(entry_path, entry.oid);
                    }
                }
            }
            Ok(())
        })
    }
}

/// Parse a `--since`/`--until` bound into a UTC instant.
///
/// Accepts what this command's help advertises (`YYYY-MM-DD`) plus a full
/// RFC 3339 timestamp for callers that need precision. A bare date means
/// midnight UTC, so `--since 2024-01-01 --until 2024-12-31` includes every
/// commit made on 31 December — an exclusive upper bound there would silently
/// drop a day, which is exactly the kind of quiet wrongness this flag was
/// guilty of before.
fn parse_date_bound(raw: &str, flag: &str) -> Result<chrono::DateTime<chrono::Utc>> {
    use chrono::{NaiveDate, NaiveTime, TimeZone, Utc};

    let raw = raw.trim();

    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(raw) {
        return Ok(dt.with_timezone(&Utc));
    }

    if let Ok(date) = NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
        let time = if flag == "--until" {
            // Inclusive end-of-day.
            NaiveTime::from_hms_milli_opt(23, 59, 59, 999).unwrap_or_default()
        } else {
            NaiveTime::MIN
        };
        return Ok(Utc.from_utc_datetime(&date.and_time(time)));
    }

    anyhow::bail!(
        "{flag}: could not parse {raw:?} as a date. Use YYYY-MM-DD \
            (e.g. 2024-01-31) or an RFC 3339 timestamp \
            (e.g. 2024-01-31T14:30:00Z)."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, Timelike};

    /// UX-5: `--since`/`--until` were declared, demonstrated in `log --help`,
    /// and never read — a date-bounded log silently returned all of history.
    #[test]
    fn date_bounds_parse_the_advertised_format() {
        let since = parse_date_bound("2024-01-31", "--since").unwrap();
        assert_eq!((since.year(), since.month(), since.day()), (2024, 1, 31));
        assert_eq!((since.hour(), since.minute()), (0, 0));

        let rfc = parse_date_bound("2024-01-31T14:30:00Z", "--since").unwrap();
        assert_eq!((rfc.hour(), rfc.minute()), (14, 30));
    }

    /// A bare `--until 2024-12-31` must include commits made *during* that day.
    /// Treating it as midnight would silently drop a day's work — the same
    /// class of quiet wrongness the flag had when it did nothing at all.
    #[test]
    fn until_is_inclusive_of_the_whole_day() {
        let until = parse_date_bound("2024-12-31", "--until").unwrap();
        assert_eq!((until.hour(), until.minute(), until.second()), (23, 59, 59));

        let since = parse_date_bound("2024-12-31", "--since").unwrap();
        assert!(
            since < until,
            "the same date as --since and --until must still describe a range"
        );
    }

    #[test]
    fn unparseable_date_is_rejected_with_guidance() {
        let err = parse_date_bound("last tuesday", "--since").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("--since"), "{msg}");
        assert!(
            msg.contains("YYYY-MM-DD"),
            "must say what it accepts: {msg}"
        );
    }
}

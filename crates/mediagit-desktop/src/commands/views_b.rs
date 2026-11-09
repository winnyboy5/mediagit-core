// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Options for the History, Branches, Merge, Compare and Advanced views.
//!
//! Every option is a typed field the webview sets; the argv is rebuilt here and each value
//! is validated, so no webview string is ever forwarded as a flag (free text always rides
//! inside a `--flag=value` word). Risky options (overwrite a branch, force a switch, delete
//! unmerged work, wipe undo history, move the branch on a bisect reset) also need
//! `confirmed: true`, so a bug in the UI cannot send one without the person agreeing.

use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use super::{RepoQueues, invalid, read, ref_name_ok, repo_path, repo_relative, rev_ok, write};
use crate::engine::{Engine, EngineError};

fn flag(argv: &mut Vec<String>, on: bool, name: &str) {
    if on {
        argv.push(name.into());
    }
}

/// Free text for a `--flag=value` word: blank means "not given"; no NUL or other control
/// characters (a message may span lines).
fn opt_text(what: &str, v: Option<&str>, max: usize) -> Result<Option<String>, EngineError> {
    let Some(v) = v.map(str::trim).filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    let bad = v.len() > max || v.chars().any(|c| c.is_control() && !"\n\r\t".contains(c));
    if bad {
        return Err(invalid(what, v));
    }
    Ok(Some(v.to_owned()))
}

fn need_confirmed(what: &str, confirmed: bool) -> Result<(), EngineError> {
    if confirmed {
        Ok(())
    } else {
        Err(invalid(what, "needs the person's confirmation"))
    }
}

/// A branch, tag or commit id.
pub fn target_ok(what: &str, t: &str) -> Result<(), EngineError> {
    if rev_ok(t).is_ok() {
        Ok(())
    } else {
        ref_name_ok(what, t)
    }
}

/// `YYYY-MM-DD` or an RFC 3339 timestamp.
fn date_ok(d: &str) -> bool {
    (8..=40).contains(&d.len())
        && d.starts_with(|c: char| c.is_ascii_digit())
        && d.chars()
            .all(|c| c.is_ascii_digit() || "-:TZ+.".contains(c))
}

// ---------------------------------------------------------------------------
// log
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LogFilter {
    /// Start from this branch, tag or version instead of HEAD.
    pub revision: Option<String>,
    pub author: Option<String>,
    pub grep: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
    /// Only versions that touched one of these files or folders.
    pub paths: Vec<String>,
}

/// `log --skip N -n M [--all] [--author=..] [--grep=..] [--since=..] [--until=..] [<rev>] [-- <paths>]`.
pub fn log_argv(
    skip: u32,
    max: u32,
    all: bool,
    path: Option<&str>,
    f: &LogFilter,
    repo: &Path,
) -> Result<Vec<String>, EngineError> {
    if !(1..=1000).contains(&max) {
        return Err(invalid("page size", &max.to_string()));
    }
    let mut argv = vec![
        "log".to_owned(),
        "--skip".into(),
        skip.to_string(),
        "-n".into(),
        max.to_string(),
    ];
    flag(&mut argv, all, "--all");
    if let Some(a) = opt_text("author", f.author.as_deref(), 200)? {
        argv.push(format!("--author={a}"));
    }
    if let Some(g) = opt_text("message text", f.grep.as_deref(), 200)? {
        argv.push(format!("--grep={g}"));
    }
    for (name, v) in [("since", &f.since), ("until", &f.until)] {
        if let Some(d) = v.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
            if !date_ok(d) {
                return Err(invalid(&format!("{name} date"), d));
            }
            argv.push(format!("--{name}={d}"));
        }
    }
    let revision = f
        .revision
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty());
    if let Some(r) = revision {
        target_ok("revision", r)?;
    }
    let paths: Vec<&str> = path
        .into_iter()
        .chain(f.paths.iter().map(String::as_str))
        .collect();
    if paths.is_empty() {
        argv.extend(revision.map(str::to_owned));
    } else {
        // The revision must come first: the first positional after `--` would
        // otherwise be read as one.
        argv.push(revision.unwrap_or("HEAD").to_owned());
        argv.push("--".into());
        for p in paths {
            argv.push(repo_relative(repo, p)?);
        }
    }
    Ok(argv)
}

// ---------------------------------------------------------------------------
// branches
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BranchCreateOpts {
    pub start_point: Option<String>,
    /// `<remote>/<branch>` this branch should follow.
    pub set_upstream: Option<String>,
    /// `track` or `none`; anything else leaves the CLI's own choice.
    pub tracking: Option<String>,
}

/// `branch create [--set-upstream=..] [--track|--no-track] -- <name> [<start>]`.
pub fn branch_create_argv(name: &str, o: &BranchCreateOpts) -> Result<Vec<String>, EngineError> {
    ref_name_ok("branch name", name)?;
    let mut argv = vec!["branch".to_owned(), "create".into()];
    if let Some(u) = o.set_upstream.as_deref().filter(|u| !u.is_empty()) {
        ref_name_ok("upstream", u)?;
        argv.push(format!("--set-upstream={u}"));
    }
    match o.tracking.as_deref() {
        None | Some("" | "auto") => {}
        Some("track") => argv.push("--track".into()),
        Some("none") => argv.push("--no-track".into()),
        Some(other) => return Err(invalid("tracking", other)),
    }
    argv.extend(["--".into(), name.into()]);
    if let Some(s) = o.start_point.as_deref().filter(|s| !s.is_empty()) {
        target_ok("start point", s)?;
        argv.push(s.into());
    }
    Ok(argv)
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BranchSwitchOpts {
    /// Make the branch first.
    pub create: bool,
    /// With `create`, name a `<remote>/<branch>` and follow it.
    pub track: bool,
    /// Throw away unsaved changes that stand in the way.
    pub force: bool,
    pub confirmed: bool,
}

/// `branch switch [--create] [--track] [--force] -- <name>`.
pub fn branch_switch_argv(name: &str, o: &BranchSwitchOpts) -> Result<Vec<String>, EngineError> {
    ref_name_ok("branch name", name)?;
    if o.track && !o.create {
        return Err(invalid("switch", "--track needs --create"));
    }
    if o.force {
        need_confirmed("force switch", o.confirmed)?;
    }
    let mut argv = vec!["branch".to_owned(), "switch".into()];
    flag(&mut argv, o.create, "--create");
    flag(&mut argv, o.track, "--track");
    flag(&mut argv, o.force, "--force");
    argv.extend(["--".into(), name.into()]);
    Ok(argv)
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BranchDeleteOpts {
    /// Delete even if the work is not merged anywhere else (`-D`).
    pub force: bool,
    /// Refuse unless the work is merged (`-d`).
    pub merged_only: bool,
    /// `name` is a remote-tracking branch (`origin/feature`); forgets the local copy only.
    pub remote: bool,
    pub confirmed: bool,
}

/// `branch delete [-D] [-d] [-r] -- <name>`.
pub fn branch_delete_argv(name: &str, o: &BranchDeleteOpts) -> Result<Vec<String>, EngineError> {
    ref_name_ok("branch name", name)?;
    if o.force && o.merged_only {
        return Err(invalid("delete", "-D and -d together"));
    }
    if o.remote && !name.contains('/') {
        return Err(invalid("remote branch", name));
    }
    if o.force {
        need_confirmed("delete unmerged work", o.confirmed)?;
    }
    let mut argv = vec!["branch".to_owned(), "delete".into()];
    flag(&mut argv, o.force, "-D");
    flag(&mut argv, o.merged_only, "-d");
    flag(&mut argv, o.remote, "-r");
    argv.extend(["--".into(), name.into()]);
    Ok(argv)
}

/// `branch rename [--force] -- <from> <to>`; `force` overwrites a branch already called `to`.
pub fn branch_rename_argv(
    from: &str,
    to: &str,
    force: bool,
    confirmed: bool,
) -> Result<Vec<String>, EngineError> {
    ref_name_ok("branch name", from)?;
    ref_name_ok("branch name", to)?;
    let mut argv = vec!["branch".to_owned(), "rename".into()];
    if force {
        need_confirmed("overwrite branch", confirmed)?;
        argv.push("--force".into());
    }
    argv.extend(["--".into(), from.into(), to.into()]);
    Ok(argv)
}

/// `branch protect [--require-reviews|--unprotect] -- <name>`.
pub fn branch_protect_argv(
    name: &str,
    require_reviews: bool,
    unprotect: bool,
    confirmed: bool,
) -> Result<Vec<String>, EngineError> {
    ref_name_ok("branch name", name)?;
    if require_reviews && unprotect {
        return Err(invalid("protect", "--require-reviews with --unprotect"));
    }
    let mut argv = vec!["branch".to_owned(), "protect".into()];
    flag(&mut argv, require_reviews, "--require-reviews");
    if unprotect {
        need_confirmed("remove protection", confirmed)?;
        argv.push("--unprotect".into());
    }
    argv.extend(["--".into(), name.into()]);
    Ok(argv)
}

// ---------------------------------------------------------------------------
// merge, rebase, cherry-pick, revert
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MergeOpts {
    pub no_ff: bool,
    pub ff_only: bool,
    pub squash: bool,
    pub no_commit: bool,
    pub message: Option<String>,
}

/// `merge --strategy=<s> [--no-ff|--ff-only] [--squash] [--no-commit] [--message=..] -- <branch>`.
/// Strategy is the only merge-wide conflict knob the CLI honours (`-X` it refuses); per-file
/// choices go through `file_at_argv`.
pub fn merge_argv(branch: &str, strategy: &str, o: &MergeOpts) -> Result<Vec<String>, EngineError> {
    ref_name_ok("branch name", branch)?;
    if !matches!(strategy, "recursive" | "ours" | "theirs") {
        return Err(invalid("merge strategy", strategy));
    }
    if o.ff_only && (o.no_ff || o.squash) {
        return Err(invalid("merge", "--ff-only with --no-ff or --squash"));
    }
    let mut argv = vec!["merge".to_owned(), format!("--strategy={strategy}")];
    flag(&mut argv, o.no_ff, "--no-ff");
    flag(&mut argv, o.ff_only, "--ff-only");
    flag(&mut argv, o.squash, "--squash");
    flag(&mut argv, o.no_commit, "--no-commit");
    if let Some(m) = opt_text("merge message", o.message.as_deref(), 4000)? {
        argv.push(format!("--message={m}"));
    }
    argv.extend(["--".into(), branch.into()]);
    Ok(argv)
}

/// `rebase [--keep-empty] -- <upstream> [<branch>]`.
pub fn rebase_start_argv(
    upstream: &str,
    branch: Option<&str>,
    keep_empty: bool,
) -> Result<Vec<String>, EngineError> {
    target_ok("branch", upstream)?;
    let mut argv = vec!["rebase".to_owned()];
    flag(&mut argv, keep_empty, "--keep-empty");
    argv.extend(["--".into(), upstream.into()]);
    if let Some(b) = branch.filter(|b| !b.is_empty()) {
        ref_name_ok("branch", b)?;
        argv.push(b.into());
    }
    Ok(argv)
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PickOpts {
    pub no_commit: bool,
    /// Add "(cherry picked from commit ..)" to the message.
    pub append_message: bool,
}

/// `cherry-pick [--no-commit] [--append-message] -- <commits>`.
pub fn pick_start_argv(commits: &[String], o: &PickOpts) -> Result<Vec<String>, EngineError> {
    if commits.is_empty() {
        return Err(invalid("commits", "none"));
    }
    let mut argv = vec!["cherry-pick".to_owned()];
    flag(&mut argv, o.no_commit, "--no-commit");
    flag(&mut argv, o.append_message, "--append-message");
    argv.push("--".into());
    for c in commits {
        rev_ok(c)?;
        argv.push(c.clone());
    }
    Ok(argv)
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RevertOpts {
    pub no_commit: bool,
    pub message: Option<String>,
}

/// `revert [--no-commit] [--message=..] -- <commits>`.
pub fn revert_argv(revs: &[String], o: &RevertOpts) -> Result<Vec<String>, EngineError> {
    if revs.is_empty() {
        return Err(invalid("versions", "none"));
    }
    let mut argv = vec!["revert".to_owned()];
    flag(&mut argv, o.no_commit, "--no-commit");
    if let Some(m) = opt_text("message", o.message.as_deref(), 4000)? {
        argv.push(format!("--message={m}"));
    }
    argv.push("--".into());
    for r in revs {
        rev_ok(r)?;
        argv.push(r.clone());
    }
    Ok(argv)
}

/// `diff` (working folder against HEAD), `diff --cached` (saved-for-commit changes against
/// HEAD) or `diff <from> <to>`; never half of each.
pub fn diff_argv(
    from: Option<&str>,
    to: Option<&str>,
    cached: bool,
) -> Result<Vec<String>, EngineError> {
    match (from, to) {
        (None, None) => {
            let mut argv = vec!["diff".to_owned()];
            flag(&mut argv, cached, "--cached");
            Ok(argv)
        }
        (Some(a), Some(b)) if !cached => {
            rev_ok(a)?;
            rev_ok(b)?;
            Ok(vec!["diff".into(), a.into(), b.into()])
        }
        _ => Err(invalid("comparison", "needs two versions or none")),
    }
}

// ---------------------------------------------------------------------------
// bisect, reflog
// ---------------------------------------------------------------------------

/// `bisect start [--reset] -- <bad> [<good>]`; `reset` throws away a bisect already running.
pub fn bisect_start_argv(
    bad: &str,
    good: Option<&str>,
    reset: bool,
    confirmed: bool,
) -> Result<Vec<String>, EngineError> {
    target_ok("commit", bad)?;
    let mut argv = vec!["bisect".to_owned(), "start".into()];
    if reset {
        need_confirmed("restart bisect", confirmed)?;
        argv.push("--reset".into());
    }
    argv.extend(["--".into(), bad.into()]);
    if let Some(g) = good.filter(|g| !g.is_empty()) {
        target_ok("commit", g)?;
        argv.push(g.into());
    }
    Ok(argv)
}

/// `bisect reset [-- <commit>]`; a commit moves the branch there, so it needs `confirmed`.
pub fn bisect_reset_argv(
    commit: Option<&str>,
    confirmed: bool,
) -> Result<Vec<String>, EngineError> {
    let mut argv = vec!["bisect".to_owned(), "reset".into()];
    if let Some(c) = commit.filter(|c| !c.is_empty()) {
        rev_ok(c)?;
        need_confirmed("move branch", confirmed)?;
        argv.extend(["--".into(), c.into()]);
    }
    Ok(argv)
}

/// `bisect replay -- <logfile>`: an absolute path to an existing file.
pub fn bisect_replay_argv(logfile: &str) -> Result<Vec<String>, EngineError> {
    crate::abs_dest(logfile)?;
    if !Path::new(logfile).is_file() {
        return Err(invalid("log file", logfile));
    }
    Ok(vec![
        "bisect".into(),
        "replay".into(),
        "--".into(),
        logfile.into(),
    ])
}

fn count_ok(count: u32) -> Result<String, EngineError> {
    if (1..=1000).contains(&count) {
        Ok(count.to_string())
    } else {
        Err(invalid("count", &count.to_string()))
    }
}

/// `reflog show --all -n <count>`: the newest entries of every ref.
pub fn reflog_all_argv(count: u32) -> Result<Vec<String>, EngineError> {
    Ok(vec![
        "reflog".into(),
        "show".into(),
        "--all".into(),
        "-n".into(),
        count_ok(count)?,
    ])
}

/// `reflog delete --yes -- <ref>`: forgets where this ref has been. Cannot be undone.
pub fn reflog_delete_argv(reference: &str, confirmed: bool) -> Result<Vec<String>, EngineError> {
    if reference != "HEAD" {
        ref_name_ok("ref", reference)?;
    }
    need_confirmed("delete undo history", confirmed)?;
    Ok(vec![
        "reflog".into(),
        "delete".into(),
        "--yes".into(),
        "--".into(),
        reference.into(),
    ])
}

/// `reflog expire --yes --keep=<n> [-- <ref>]` (no ref = every ref). Cannot be undone.
pub fn reflog_expire_argv(
    reference: Option<&str>,
    keep: u32,
    confirmed: bool,
) -> Result<Vec<String>, EngineError> {
    if keep > 100_000 {
        return Err(invalid("entries to keep", &keep.to_string()));
    }
    need_confirmed("expire undo history", confirmed)?;
    let mut argv = vec![
        "reflog".to_owned(),
        "expire".into(),
        "--yes".into(),
        format!("--keep={keep}"),
    ];
    if let Some(r) = reference.filter(|r| !r.is_empty()) {
        if r != "HEAD" {
            ref_name_ok("ref", r)?;
        }
        argv.extend(["--".into(), r.into()]);
    }
    Ok(argv)
}

// ---------------------------------------------------------------------------
// commands
// ---------------------------------------------------------------------------

/// One branch's details (`branch show`); no name = the current branch.
#[tauri::command]
pub async fn branch_show(
    engine: tauri::State<'_, Engine>,
    repo: String,
    name: Option<String>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let mut argv = vec!["branch".to_owned(), "show".into()];
    if let Some(n) = name.filter(|n| !n.is_empty()) {
        ref_name_ok("branch name", &n)?;
        argv.extend(["--".into(), n]);
    }
    read(&engine, &repo, &argv).await
}

/// Protect a branch from force pushes and deletion, or lift that (`unprotect`).
#[tauri::command]
pub async fn branch_protect(
    engine: tauri::State<'_, Engine>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    name: String,
    require_reviews: bool,
    unprotect: bool,
    confirmed: bool,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let argv = branch_protect_argv(&name, require_reviews, unprotect, confirmed)?;
    write(&engine, &queues, &repo, &argv).await
}

/// The newest entries of every ref.
#[tauri::command]
pub async fn reflog_all(
    engine: tauri::State<'_, Engine>,
    repo: String,
    count: u32,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    read(&engine, &repo, &reflog_all_argv(count)?).await
}

/// Forget one ref's undo history.
#[tauri::command]
pub async fn reflog_delete(
    engine: tauri::State<'_, Engine>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    reference: String,
    confirmed: bool,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let argv = reflog_delete_argv(&reference, confirmed)?;
    write(&engine, &queues, &repo, &argv).await
}

/// Keep only the newest `keep` undo-history entries of one ref, or of every ref.
#[tauri::command]
pub async fn reflog_expire(
    engine: tauri::State<'_, Engine>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    reference: Option<String>,
    keep: u32,
    confirmed: bool,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let argv = reflog_expire_argv(reference.as_deref(), keep, confirmed)?;
    write(&engine, &queues, &repo, &argv).await
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::needless_pass_by_value)]
mod tests {
    use super::*;

    fn bad<T: std::fmt::Debug>(r: Result<T, EngineError>) -> bool {
        matches!(r, Err(EngineError::InvalidArgument(_)))
    }
    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_owned()).collect()
    }
    const OID: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn log_filters_ride_inside_flag_words_and_the_revision_leads_the_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        let none = LogFilter::default();
        assert_eq!(
            log_argv(0, 200, true, None, &none, repo).unwrap(),
            s(&["log", "--skip", "0", "-n", "200", "--all"])
        );
        let f = LogFilter {
            revision: Some("release/1".into()),
            author: Some("ana".into()),
            grep: Some("fix: grade".into()),
            since: Some("2024-01-01".into()),
            until: Some("2024-12-31T10:00:00Z".into()),
            paths: vec!["art/hero.psd".into(), "video".into()],
        };
        assert_eq!(
            log_argv(5, 10, false, Some("a/b.png"), &f, repo).unwrap(),
            s(&[
                "log",
                "--skip",
                "5",
                "-n",
                "10",
                "--author=ana",
                "--grep=fix: grade",
                "--since=2024-01-01",
                "--until=2024-12-31T10:00:00Z",
                "release/1",
                "--",
                "a/b.png",
                "art/hero.psd",
                "video"
            ])
        );
        let rev_only = LogFilter {
            revision: Some("v1.0".into()),
            ..Default::default()
        };
        assert_eq!(
            log_argv(0, 5, false, None, &rev_only, repo).unwrap(),
            s(&["log", "--skip", "0", "-n", "5", "v1.0"])
        );
    }

    #[test]
    fn log_filters_refuse_flags_dates_and_escapes() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        let with = |f: LogFilter| log_argv(0, 10, false, None, &f, repo);
        assert!(bad(with(LogFilter {
            revision: Some("--all".into()),
            ..Default::default()
        })));
        assert!(bad(with(LogFilter {
            since: Some("yesterday".into()),
            ..Default::default()
        })));
        assert!(bad(with(LogFilter {
            until: Some("2024-01-01; rm".into()),
            ..Default::default()
        })));
        assert!(bad(with(LogFilter {
            author: Some("a\0b".into()),
            ..Default::default()
        })));
        assert!(bad(with(LogFilter {
            grep: Some("x".repeat(300)),
            ..Default::default()
        })));
        assert!(bad(with(LogFilter {
            paths: vec!["../x".into()],
            ..Default::default()
        })));
        assert!(bad(with(LogFilter {
            paths: vec!["-n".into()],
            ..Default::default()
        })));
        // a blank filter is "not given", not an error
        assert!(
            with(LogFilter {
                author: Some("  ".into()),
                since: Some(String::new()),
                ..Default::default()
            })
            .is_ok()
        );
        assert!(bad(log_argv(
            0,
            0,
            false,
            None,
            &LogFilter::default(),
            repo
        )));
        assert!(bad(log_argv(
            0,
            5000,
            false,
            None,
            &LogFilter::default(),
            repo
        )));
    }

    #[test]
    fn branch_create_options_become_flags_before_the_names() {
        assert_eq!(
            branch_create_argv("fx", &BranchCreateOpts::default()).unwrap(),
            s(&["branch", "create", "--", "fx"])
        );
        let o = BranchCreateOpts {
            start_point: Some("origin/feat".into()),
            set_upstream: Some("origin/feat".into()),
            tracking: Some("track".into()),
        };
        assert_eq!(
            branch_create_argv("fx", &o).unwrap(),
            s(&[
                "branch",
                "create",
                "--set-upstream=origin/feat",
                "--track",
                "--",
                "fx",
                "origin/feat"
            ])
        );
        let none = BranchCreateOpts {
            tracking: Some("none".into()),
            start_point: Some(OID.into()),
            ..Default::default()
        };
        assert_eq!(
            branch_create_argv("fx", &none).unwrap(),
            s(&["branch", "create", "--no-track", "--", "fx", OID])
        );
        for n in [
            "--force", "-x", "a b", "a..b", "a~1", "a^", "a:b", "x/", "/x", "x.lock", "x.",
            "a@{1}", "a\\b", "", "a//b", "a\u{7}b",
        ] {
            assert!(
                bad(branch_create_argv(n, &BranchCreateOpts::default())),
                "{n}"
            );
        }
        assert!(bad(branch_create_argv(
            "fx",
            &BranchCreateOpts {
                start_point: Some("--all".into()),
                ..Default::default()
            }
        )));
        assert!(bad(branch_create_argv(
            "fx",
            &BranchCreateOpts {
                set_upstream: Some("-x".into()),
                ..Default::default()
            }
        )));
        assert!(bad(branch_create_argv(
            "fx",
            &BranchCreateOpts {
                tracking: Some("sometimes".into()),
                ..Default::default()
            }
        )));
    }

    #[test]
    fn switching_forcefully_needs_the_persons_yes() {
        assert_eq!(
            branch_switch_argv("dev", &BranchSwitchOpts::default()).unwrap(),
            s(&["branch", "switch", "--", "dev"])
        );
        let c = BranchSwitchOpts {
            create: true,
            track: true,
            ..Default::default()
        };
        assert_eq!(
            branch_switch_argv("origin/feat", &c).unwrap(),
            s(&[
                "branch",
                "switch",
                "--create",
                "--track",
                "--",
                "origin/feat"
            ])
        );
        let f = BranchSwitchOpts {
            force: true,
            ..Default::default()
        };
        assert!(
            bad(branch_switch_argv("dev", &f)),
            "force without confirmation"
        );
        let f = BranchSwitchOpts {
            force: true,
            confirmed: true,
            ..Default::default()
        };
        assert_eq!(
            branch_switch_argv("dev", &f).unwrap(),
            s(&["branch", "switch", "--force", "--", "dev"])
        );
        assert!(
            bad(branch_switch_argv(
                "dev",
                &BranchSwitchOpts {
                    track: true,
                    ..Default::default()
                }
            )),
            "track needs create"
        );
        assert!(bad(branch_switch_argv(
            "--force",
            &BranchSwitchOpts::default()
        )));
    }

    #[test]
    fn deleting_unmerged_work_needs_the_persons_yes() {
        assert_eq!(
            branch_delete_argv("old", &BranchDeleteOpts::default()).unwrap(),
            s(&["branch", "delete", "--", "old"])
        );
        let m = BranchDeleteOpts {
            merged_only: true,
            ..Default::default()
        };
        assert_eq!(
            branch_delete_argv("old", &m).unwrap(),
            s(&["branch", "delete", "-d", "--", "old"])
        );
        let r = BranchDeleteOpts {
            remote: true,
            ..Default::default()
        };
        assert_eq!(
            branch_delete_argv("origin/old", &r).unwrap(),
            s(&["branch", "delete", "-r", "--", "origin/old"])
        );
        assert!(
            bad(branch_delete_argv("old", &r)),
            "a remote branch is remote/name"
        );
        let f = BranchDeleteOpts {
            force: true,
            ..Default::default()
        };
        assert!(bad(branch_delete_argv("old", &f)));
        let f = BranchDeleteOpts {
            force: true,
            confirmed: true,
            ..Default::default()
        };
        assert_eq!(
            branch_delete_argv("old", &f).unwrap(),
            s(&["branch", "delete", "-D", "--", "old"])
        );
        let both = BranchDeleteOpts {
            force: true,
            merged_only: true,
            confirmed: true,
            ..Default::default()
        };
        assert!(bad(branch_delete_argv("old", &both)));
    }

    #[test]
    fn rename_overwrite_and_protection_changes_are_guarded() {
        assert_eq!(
            branch_rename_argv("a", "b", false, false).unwrap(),
            s(&["branch", "rename", "--", "a", "b"])
        );
        assert!(bad(branch_rename_argv("a", "b", true, false)));
        assert_eq!(
            branch_rename_argv("a", "b", true, true).unwrap(),
            s(&["branch", "rename", "--force", "--", "a", "b"])
        );
        assert!(bad(branch_rename_argv("a", "--b", false, false)));

        assert_eq!(
            branch_protect_argv("main", false, false, false).unwrap(),
            s(&["branch", "protect", "--", "main"])
        );
        assert_eq!(
            branch_protect_argv("main", true, false, false).unwrap(),
            s(&["branch", "protect", "--require-reviews", "--", "main"])
        );
        assert!(
            bad(branch_protect_argv("main", false, true, false)),
            "unprotect without confirmation"
        );
        assert_eq!(
            branch_protect_argv("main", false, true, true).unwrap(),
            s(&["branch", "protect", "--unprotect", "--", "main"])
        );
        assert!(bad(branch_protect_argv("main", true, true, true)));
        assert!(bad(branch_protect_argv("-x", false, false, false)));
    }

    #[test]
    fn merge_options_never_contradict_and_keep_text_in_a_flag_word() {
        let plain = MergeOpts::default();
        assert_eq!(
            merge_argv("fx", "recursive", &plain).unwrap(),
            s(&["merge", "--strategy=recursive", "--", "fx"])
        );
        let all = MergeOpts {
            squash: true,
            no_commit: true,
            message: Some("Bring in fx\n\nwhy".into()),
            ..Default::default()
        };
        assert_eq!(
            merge_argv("fx", "theirs", &all).unwrap(),
            s(&[
                "merge",
                "--strategy=theirs",
                "--squash",
                "--no-commit",
                "--message=Bring in fx\n\nwhy",
                "--",
                "fx"
            ])
        );
        let nf = MergeOpts {
            no_ff: true,
            ..Default::default()
        };
        assert_eq!(
            merge_argv("fx", "ours", &nf).unwrap(),
            s(&["merge", "--strategy=ours", "--no-ff", "--", "fx"])
        );
        let ff = MergeOpts {
            ff_only: true,
            ..Default::default()
        };
        assert_eq!(
            merge_argv("fx", "ours", &ff).unwrap(),
            s(&["merge", "--strategy=ours", "--ff-only", "--", "fx"])
        );
        assert!(bad(merge_argv(
            "fx",
            "ours",
            &MergeOpts {
                ff_only: true,
                no_ff: true,
                ..Default::default()
            }
        )));
        assert!(bad(merge_argv(
            "fx",
            "ours",
            &MergeOpts {
                ff_only: true,
                squash: true,
                ..Default::default()
            }
        )));
        assert!(bad(merge_argv("--abort", "ours", &plain)));
        assert!(bad(merge_argv("fx", "octopus", &plain)));
        assert!(bad(merge_argv(
            "fx",
            "ours",
            &MergeOpts {
                message: Some("a\0b".into()),
                ..Default::default()
            }
        )));
        // a blank message is "not given"
        assert_eq!(
            merge_argv(
                "fx",
                "ours",
                &MergeOpts {
                    message: Some("  ".into()),
                    ..Default::default()
                }
            )
            .unwrap(),
            s(&["merge", "--strategy=ours", "--", "fx"])
        );
    }

    #[test]
    fn rebase_and_cherry_pick_options() {
        assert_eq!(
            rebase_start_argv("main", None, false).unwrap(),
            s(&["rebase", "--", "main"])
        );
        assert_eq!(
            rebase_start_argv("main", Some("feat"), true).unwrap(),
            s(&["rebase", "--keep-empty", "--", "main", "feat"])
        );
        assert!(bad(rebase_start_argv("--abort", None, false)));
        assert!(bad(rebase_start_argv("main", Some("--x"), false)));

        let c = s(&[OID]);
        assert_eq!(
            pick_start_argv(&c, &PickOpts::default()).unwrap(),
            s(&["cherry-pick", "--", OID])
        );
        let o = PickOpts {
            no_commit: true,
            append_message: true,
        };
        assert_eq!(
            pick_start_argv(&c, &o).unwrap(),
            s(&["cherry-pick", "--no-commit", "--append-message", "--", OID])
        );
        assert!(bad(pick_start_argv(&[], &o)));
        assert!(
            bad(pick_start_argv(&s(&["main"]), &o)),
            "a version id, not a name"
        );
    }

    #[test]
    fn revert_takes_many_versions_and_a_message() {
        let two = s(&[OID, "HEAD"]);
        assert_eq!(
            revert_argv(&two, &RevertOpts::default()).unwrap(),
            s(&["revert", "--", OID, "HEAD"])
        );
        let o = RevertOpts {
            no_commit: true,
            message: Some("Undo grade".into()),
        };
        assert_eq!(
            revert_argv(&two[..1], &o).unwrap(),
            s(&["revert", "--no-commit", "--message=Undo grade", "--", OID])
        );
        assert!(bad(revert_argv(&[], &RevertOpts::default())));
        assert!(bad(revert_argv(&s(&["--abort"]), &RevertOpts::default())));
        assert!(bad(revert_argv(
            &two[..1],
            &RevertOpts {
                message: Some("x\u{7}".into()),
                ..Default::default()
            }
        )));
    }

    #[test]
    fn diff_is_two_versions_or_none_and_cached_is_only_none() {
        assert_eq!(diff_argv(None, None, false).unwrap(), s(&["diff"]));
        assert_eq!(
            diff_argv(None, None, true).unwrap(),
            s(&["diff", "--cached"])
        );
        assert_eq!(
            diff_argv(Some(OID), Some("HEAD"), false).unwrap(),
            s(&["diff", OID, "HEAD"])
        );
        assert!(bad(diff_argv(Some(OID), Some("HEAD"), true)));
        assert!(bad(diff_argv(Some(OID), None, false)));
        assert!(bad(diff_argv(Some("--cached"), Some(OID), false)));
    }

    #[test]
    fn bisect_restart_and_move_are_guarded() {
        assert_eq!(
            bisect_start_argv("HEAD", Some(OID), false, false).unwrap(),
            s(&["bisect", "start", "--", "HEAD", OID])
        );
        assert!(bad(bisect_start_argv("HEAD", None, true, false)));
        assert_eq!(
            bisect_start_argv("HEAD", None, true, true).unwrap(),
            s(&["bisect", "start", "--reset", "--", "HEAD"])
        );
        assert!(bad(bisect_start_argv("--reset", None, false, false)));

        assert_eq!(
            bisect_reset_argv(None, false).unwrap(),
            s(&["bisect", "reset"])
        );
        assert!(
            bad(bisect_reset_argv(Some(OID), false)),
            "moving the branch needs a yes"
        );
        assert_eq!(
            bisect_reset_argv(Some(OID), true).unwrap(),
            s(&["bisect", "reset", "--", OID])
        );
        assert!(bad(bisect_reset_argv(Some("main"), true)));
    }

    #[test]
    fn bisect_replay_wants_an_existing_absolute_file() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("bisect.log");
        std::fs::write(&log, "x").unwrap();
        let p = log.to_string_lossy().into_owned();
        assert_eq!(
            bisect_replay_argv(&p).unwrap(),
            s(&["bisect", "replay", "--", &p])
        );
        assert!(bad(bisect_replay_argv("bisect.log")), "relative");
        assert!(
            bad(bisect_replay_argv(&dir.path().to_string_lossy())),
            "a folder"
        );
        assert!(bad(bisect_replay_argv(
            &dir.path().join("none.log").to_string_lossy()
        )));
        assert!(bad(bisect_replay_argv("--help")));
    }

    #[test]
    fn reflog_cleanup_cannot_run_unconfirmed() {
        assert_eq!(
            reflog_all_argv(50).unwrap(),
            s(&["reflog", "show", "--all", "-n", "50"])
        );
        assert!(bad(reflog_all_argv(0)));
        assert!(bad(reflog_all_argv(5000)));

        assert!(bad(reflog_delete_argv("main", false)));
        assert_eq!(
            reflog_delete_argv("main", true).unwrap(),
            s(&["reflog", "delete", "--yes", "--", "main"])
        );
        assert_eq!(
            reflog_delete_argv("HEAD", true).unwrap(),
            s(&["reflog", "delete", "--yes", "--", "HEAD"])
        );
        assert!(bad(reflog_delete_argv("--all", true)));

        assert!(bad(reflog_expire_argv(None, 30, false)));
        assert_eq!(
            reflog_expire_argv(None, 30, true).unwrap(),
            s(&["reflog", "expire", "--yes", "--keep=30"])
        );
        assert_eq!(
            reflog_expire_argv(Some("main"), 0, true).unwrap(),
            s(&["reflog", "expire", "--yes", "--keep=0", "--", "main"])
        );
        assert!(bad(reflog_expire_argv(Some("-x"), 30, true)));
        assert!(bad(reflog_expire_argv(None, 1_000_000, true)));
    }
}

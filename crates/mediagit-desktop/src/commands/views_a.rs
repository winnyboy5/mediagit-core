// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Options for the Sync, Remote, Welcome, Locks and Changes views.
//!
//! Every option is a typed field the webview sets; the argv is rebuilt here and each value
//! is validated, so no webview string is ever forwarded as a flag. Risky options (force
//! push, overwrite, delete a remote branch) also need `confirmed: true`, the same guard the
//! server commands use, so a bug in the UI cannot send one without the person agreeing.
//! The remote token reaches the CLI on stdin only.

use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use super::{
    RepoQueues, as_args, invalid, plain_arg, ref_name_ok, repo_path, repo_relative, write,
};
use crate::engine::{Engine, EngineError, Secret};

fn flag(argv: &mut Vec<String>, on: bool, name: &str) {
    if on {
        argv.push(name.into());
    }
}

fn opt_name(what: &str, v: Option<&str>) -> Result<Option<String>, EngineError> {
    v.filter(|s| !s.is_empty())
        .map(|s| ref_name_ok(what, s).map(|()| s.to_owned()))
        .transpose()
}

/// Put `flags` straight after the subcommand words (`prefix` of them).
fn splice(mut argv: Vec<String>, prefix: usize, flags: Vec<String>) -> Vec<String> {
    argv.splice(prefix..prefix, flags);
    argv
}

/// `--<name>=<branch>` after the first `prefix` words (`init --initial-branch`, `clone --branch`).
pub fn with_branch(
    argv: Vec<String>,
    prefix: usize,
    name: &str,
    branch: Option<&str>,
) -> Result<Vec<String>, EngineError> {
    let flags = opt_name("branch name", branch)?
        .map(|b| format!("--{name}={b}"))
        .into_iter()
        .collect();
    Ok(splice(argv, prefix, flags))
}

// ---------------------------------------------------------------------------
// fetch / pull / push
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FetchOpts {
    pub branch: Option<String>,
    pub all: bool,
    pub prune: bool,
}

/// `fetch [--all] [--prune] [-- <remote> [<branch>]]`.
pub fn fetch_argv(remote: Option<&str>, o: &FetchOpts) -> Result<Vec<String>, EngineError> {
    let branch = opt_name("branch name", o.branch.as_deref())?;
    if o.all && branch.is_some() {
        return Err(invalid("fetch", "--all and a branch name together"));
    }
    let mut argv = vec!["fetch".to_owned()];
    flag(&mut argv, o.all, "--all");
    flag(&mut argv, o.prune, "--prune");
    positionals(&mut argv, remote, branch.into_iter().collect())?;
    Ok(argv)
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PullOpts {
    pub branch: Option<String>,
    pub rebase: bool,
    pub dry_run: bool,
}

/// `pull [--rebase] [--dry-run] [-- <remote> [<branch>]]`.
pub fn pull_argv(remote: Option<&str>, o: &PullOpts) -> Result<Vec<String>, EngineError> {
    let branch = opt_name("branch name", o.branch.as_deref())?;
    let mut argv = vec!["pull".to_owned()];
    flag(&mut argv, o.rebase, "--rebase");
    flag(&mut argv, o.dry_run, "--dry-run");
    positionals(&mut argv, remote, branch.into_iter().collect())?;
    Ok(argv)
}

/// `-- <remote> <rest...>`; the rest needs a remote in front of it.
fn positionals(
    argv: &mut Vec<String>,
    remote: Option<&str>,
    rest: Vec<String>,
) -> Result<(), EngineError> {
    match remote {
        Some(r) => {
            ref_name_ok("remote name", r)?;
            argv.extend(["--".to_owned(), r.to_owned()]);
            argv.extend(rest);
            Ok(())
        }
        None if rest.is_empty() => Ok(()),
        None => Err(invalid("remote", "a branch needs a remote")),
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PushOpts {
    /// `main` or `local:remote`, one per entry.
    pub refspecs: Vec<String>,
    pub all: bool,
    pub tags: bool,
    pub follow_tags: bool,
    pub dry_run: bool,
    /// `lease` (`--force-with-lease`) or `force` (`--force`); none = stop if the remote moved.
    pub overwrite: Option<String>,
    pub delete: bool,
    pub set_upstream: bool,
    pub no_track: bool,
    pub repair: bool,
    /// The person agreed to a risky option (overwrite or delete).
    pub confirmed: bool,
}

fn refspec_ok(s: &str) -> Result<(), EngineError> {
    let mut sides = s.split(':');
    let (a, b) = (sides.next().unwrap_or(""), sides.next());
    if sides.next().is_some() {
        return Err(invalid("branch to send", s));
    }
    ref_name_ok("branch to send", a)?;
    b.map_or(Ok(()), |b| ref_name_ok("branch to send", b))
}

/// `push [flags] [-- <remote> [<refspec>...]]`.
pub fn push_argv(remote: Option<&str>, o: &PushOpts) -> Result<Vec<String>, EngineError> {
    for r in &o.refspecs {
        refspec_ok(r)?;
    }
    let overwrite = match o.overwrite.as_deref() {
        None | Some("") => None,
        Some(m @ ("lease" | "force")) => Some(m),
        Some(other) => return Err(invalid("overwrite mode", other)),
    };
    if (overwrite.is_some() || o.delete) && !o.confirmed {
        return Err(EngineError::InvalidArgument(
            "overwriting or deleting on the remote needs confirmation".into(),
        ));
    }
    if o.delete && (o.refspecs.is_empty() || o.all || o.tags || o.follow_tags) {
        return Err(invalid(
            "delete",
            "name the branches to delete, and nothing else",
        ));
    }
    if o.set_upstream && o.no_track {
        return Err(invalid("upstream", "set and not set together"));
    }
    if o.all && !o.refspecs.is_empty() {
        return Err(invalid("push", "--all and named branches together"));
    }
    let mut argv = vec!["push".to_owned()];
    flag(&mut argv, o.all, "--all");
    flag(&mut argv, o.tags, "--tags");
    flag(&mut argv, o.follow_tags, "--follow-tags");
    flag(&mut argv, o.dry_run, "--dry-run");
    flag(&mut argv, overwrite == Some("lease"), "--force-with-lease");
    flag(&mut argv, overwrite == Some("force"), "--force");
    flag(&mut argv, o.delete, "--delete");
    flag(&mut argv, o.set_upstream, "--set-upstream");
    flag(&mut argv, o.no_track, "--no-track");
    flag(&mut argv, o.repair, "--repair");
    positionals(&mut argv, remote, o.refspecs.clone())?;
    Ok(argv)
}

// ---------------------------------------------------------------------------
// remotes
// ---------------------------------------------------------------------------

/// `remote add [--fetch] [--token-stdin] -- <name> <url>`; the token itself goes on stdin.
pub fn remote_add_argv(
    name: &str,
    url: &str,
    fetch: bool,
    token: bool,
) -> Result<Vec<String>, EngineError> {
    let mut flags = Vec::new();
    flag(&mut flags, fetch, "--fetch");
    flag(&mut flags, token, "--token-stdin");
    Ok(splice(super::remote_add_argv(name, url)?, 2, flags))
}

/// `remote rename -- <old> <new>`.
pub fn remote_rename_argv(old: &str, new: &str) -> Result<Vec<String>, EngineError> {
    ref_name_ok("remote name", old)?;
    ref_name_ok("remote name", new)?;
    Ok(vec![
        "remote".into(),
        "rename".into(),
        "--".into(),
        old.into(),
        new.into(),
    ])
}

/// `remote set-url [--push] -- <name> <url>`.
pub fn remote_set_url_argv(name: &str, url: &str, push: bool) -> Result<Vec<String>, EngineError> {
    ref_name_ok("remote name", name)?;
    if !super::clone_source_ok(url) {
        return Err(invalid("remote address", url));
    }
    let mut argv = vec!["remote".to_owned(), "set-url".into()];
    flag(&mut argv, push, "--push");
    argv.extend(["--".into(), name.into(), url.into()]);
    Ok(argv)
}

/// `remote show -- <name>`.
pub fn remote_show_argv(name: &str) -> Result<Vec<String>, EngineError> {
    ref_name_ok("remote name", name)?;
    Ok(vec![
        "remote".into(),
        "show".into(),
        "--".into(),
        name.into(),
    ])
}

// ---------------------------------------------------------------------------
// locks
// ---------------------------------------------------------------------------

/// `--owner=<name>` and `--remote=<name>` for the lock commands.
pub fn lock_flags(owner: Option<&str>, remote: Option<&str>) -> Result<Vec<String>, EngineError> {
    let mut f = Vec::new();
    if let Some(o) = owner.filter(|o| !o.is_empty()) {
        plain_arg("owner", o)?;
        if o.chars().any(char::is_control) {
            return Err(invalid("owner", o));
        }
        f.push(format!("--owner={o}"));
    }
    if let Some(r) = opt_name("remote name", remote)? {
        f.push(format!("--remote={r}"));
    }
    Ok(f)
}

/// Lock argv with the owner/remote flags after `lock <sub>`.
pub fn with_lock_flags(
    argv: Vec<String>,
    owner: Option<&str>,
    remote: Option<&str>,
) -> Result<Vec<String>, EngineError> {
    Ok(splice(argv, 2, lock_flags(owner, remote)?))
}

// ---------------------------------------------------------------------------
// staging, committing, unstaging
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AddOpts {
    pub all: bool,
    pub dry_run: bool,
    pub force: bool,
    pub update: bool,
    pub ignore_removal: bool,
}

/// Flags that go right after `add`.
pub fn add_flags(o: &AddOpts) -> Vec<String> {
    let mut f = Vec::new();
    flag(&mut f, o.all, "--all");
    flag(&mut f, o.dry_run, "--dry-run");
    flag(&mut f, o.force, "--force");
    flag(&mut f, o.update, "--update");
    flag(&mut f, o.ignore_removal, "--ignore-removal");
    f
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CommitOpts {
    /// Absolute path of a file holding the message.
    pub message_file: Option<String>,
    pub all: bool,
    pub include: Vec<String>,
    /// `Name <email>`.
    pub author: Option<String>,
    /// RFC 3339, e.g. `2026-01-15T10:30:00Z`.
    pub date: Option<String>,
    pub allow_empty: bool,
    pub signoff: bool,
    pub dry_run: bool,
}

fn author_ok(a: &str) -> bool {
    let a = a.trim();
    !a.starts_with('-')
        && !a.chars().any(char::is_control)
        && a.find('<')
            .zip(a.rfind('>'))
            .is_some_and(|(l, r)| l > 0 && r == a.len() - 1 && r > l + 1)
}

fn date_ok(d: &str) -> bool {
    d.len() <= 40
        && d.contains('T')
        && d.chars()
            .all(|c| c.is_ascii_digit() || "-:TZ+.".contains(c))
}

/// `commit (--message=<m> | --file=<f>) [flags]`.
pub fn commit_argv(repo: &Path, message: &str, o: &CommitOpts) -> Result<Vec<String>, EngineError> {
    let mut argv = if let Some(f) = o.message_file.as_deref().filter(|f| !f.is_empty()) {
        crate::abs_dest(f)?;
        if !Path::new(f).is_file() {
            return Err(invalid("message file", f));
        }
        vec!["commit".to_owned(), format!("--file={f}")]
    } else {
        crate::commit_argv(message)?
    };
    flag(&mut argv, o.all, "--all");
    flag(&mut argv, o.allow_empty, "--allow-empty");
    flag(&mut argv, o.signoff, "--signoff");
    flag(&mut argv, o.dry_run, "--dry-run");
    if let Some(a) = o.author.as_deref().filter(|a| !a.is_empty()) {
        if !author_ok(a) {
            return Err(invalid("author (use Name <email>)", a));
        }
        argv.push(format!("--author={a}"));
    }
    if let Some(d) = o.date.as_deref().filter(|d| !d.is_empty()) {
        if !date_ok(d) {
            return Err(invalid("date (use 2026-01-15T10:30:00Z)", d));
        }
        argv.push(format!("--date={d}"));
    }
    for p in &o.include {
        argv.push(format!("--include={}", repo_relative(repo, p)?));
    }
    Ok(argv)
}

/// `reset HEAD -- <paths>`: take paths out of the staging area, files untouched.
pub fn unstage_argv(repo: &Path, paths: &[String]) -> Result<Vec<String>, EngineError> {
    if paths.is_empty() {
        return Err(invalid("paths", "none to unstage"));
    }
    let mut argv = vec!["reset".to_owned(), "HEAD".into(), "--".into()];
    for p in paths {
        argv.push(repo_relative(repo, p)?);
    }
    Ok(argv)
}

// ---------------------------------------------------------------------------
// commands
// ---------------------------------------------------------------------------

/// Add a remote. `token` is an optional bearer token, sent on stdin only.
#[tauri::command]
#[allow(clippy::too_many_arguments, clippy::needless_pass_by_value)]
pub async fn remote_add(
    engine: tauri::State<'_, Engine>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    name: String,
    url: String,
    fetch: Option<bool>,
    token: Option<String>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let token = token.filter(|t| !t.is_empty());
    let argv = remote_add_argv(&name, &url, fetch.unwrap_or(false), token.is_some())?;
    let secrets = token
        .map(Secret::new)
        .transpose()?
        .into_iter()
        .collect::<Vec<_>>();
    queues
        .write(
            &repo,
            engine.run_with(Some(&repo), &as_args(&argv), &secrets, |_| {}),
        )
        .await
}

#[tauri::command]
pub async fn remote_rename(
    engine: tauri::State<'_, Engine>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    old: String,
    new: String,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    write(&engine, &queues, &repo, &remote_rename_argv(&old, &new)?).await
}

#[tauri::command]
pub async fn remote_set_url(
    engine: tauri::State<'_, Engine>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    name: String,
    url: String,
    push: bool,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    write(
        &engine,
        &queues,
        &repo,
        &remote_set_url_argv(&name, &url, push)?,
    )
    .await
}

/// What the server says about a remote (reachable, default branch, branches).
#[tauri::command]
pub async fn remote_show(
    engine: tauri::State<'_, Engine>,
    repo: String,
    name: String,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    super::read(&engine, &repo, &remote_show_argv(&name)?).await
}

/// Take files out of the staging area; the files themselves are not touched.
#[tauri::command]
pub async fn repo_unstage(
    engine: tauri::State<'_, Engine>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    paths: Vec<String>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    write(&engine, &queues, &repo, &unstage_argv(&repo, &paths)?).await
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

    #[test]
    fn fetch_options_become_flags_and_positionals() {
        assert_eq!(fetch_argv(None, &FetchOpts::default()).unwrap(), ["fetch"]);
        let o = FetchOpts {
            all: true,
            prune: true,
            ..Default::default()
        };
        assert_eq!(
            fetch_argv(Some("origin"), &o).unwrap(),
            s(&["fetch", "--all", "--prune", "--", "origin"])
        );
        let o = FetchOpts {
            branch: Some("dev".into()),
            ..Default::default()
        };
        assert_eq!(
            fetch_argv(Some("origin"), &o).unwrap(),
            s(&["fetch", "--", "origin", "dev"])
        );
        // bad input
        assert!(bad(fetch_argv(None, &o)), "branch without remote");
        let both = FetchOpts {
            all: true,
            branch: Some("dev".into()),
            ..Default::default()
        };
        assert!(bad(fetch_argv(Some("origin"), &both)));
        let flaglike = FetchOpts {
            branch: Some("--all".into()),
            ..Default::default()
        };
        assert!(bad(fetch_argv(Some("origin"), &flaglike)));
        assert!(bad(fetch_argv(Some("-x"), &FetchOpts::default())));
    }

    #[test]
    fn pull_options_become_flags_and_positionals() {
        let o = PullOpts {
            branch: Some("dev".into()),
            rebase: true,
            dry_run: true,
        };
        assert_eq!(
            pull_argv(Some("origin"), &o).unwrap(),
            s(&["pull", "--rebase", "--dry-run", "--", "origin", "dev"])
        );
        assert_eq!(
            pull_argv(Some("origin"), &PullOpts::default()).unwrap(),
            s(&["pull", "--", "origin"])
        );
        assert!(bad(pull_argv(None, &o)), "branch without remote");
        let o = PullOpts {
            branch: Some("a b".into()),
            ..Default::default()
        };
        assert!(bad(pull_argv(Some("origin"), &o)));
    }

    #[test]
    fn push_safe_options_need_no_confirmation() {
        let o = PushOpts {
            refspecs: vec!["main".into(), "dev:release".into()],
            tags: true,
            follow_tags: true,
            dry_run: true,
            set_upstream: true,
            repair: true,
            ..Default::default()
        };
        assert_eq!(
            push_argv(Some("origin"), &o).unwrap(),
            s(&[
                "push",
                "--tags",
                "--follow-tags",
                "--dry-run",
                "--set-upstream",
                "--repair",
                "--",
                "origin",
                "main",
                "dev:release"
            ])
        );
        let all = PushOpts {
            all: true,
            no_track: true,
            ..Default::default()
        };
        assert_eq!(
            push_argv(Some("origin"), &all).unwrap(),
            s(&["push", "--all", "--no-track", "--", "origin"])
        );
        assert_eq!(push_argv(None, &PushOpts::default()).unwrap(), ["push"]);
    }

    #[test]
    fn push_overwrite_and_delete_are_refused_unless_confirmed() {
        for mode in ["lease", "force"] {
            let o = PushOpts {
                overwrite: Some(mode.into()),
                ..Default::default()
            };
            assert!(bad(push_argv(Some("origin"), &o)), "{mode} unconfirmed");
        }
        let del = PushOpts {
            delete: true,
            refspecs: vec!["old".into()],
            ..Default::default()
        };
        assert!(bad(push_argv(Some("origin"), &del)), "delete unconfirmed");

        let lease = PushOpts {
            overwrite: Some("lease".into()),
            confirmed: true,
            ..Default::default()
        };
        assert_eq!(
            push_argv(Some("origin"), &lease).unwrap(),
            s(&["push", "--force-with-lease", "--", "origin"])
        );
        let force = PushOpts {
            overwrite: Some("force".into()),
            confirmed: true,
            ..Default::default()
        };
        assert_eq!(
            push_argv(Some("origin"), &force).unwrap(),
            s(&["push", "--force", "--", "origin"])
        );
        let del = PushOpts {
            delete: true,
            refspecs: vec!["old".into()],
            confirmed: true,
            ..Default::default()
        };
        assert_eq!(
            push_argv(Some("origin"), &del).unwrap(),
            s(&["push", "--delete", "--", "origin", "old"])
        );
    }

    #[test]
    fn push_refuses_contradictions_and_flaglike_values() {
        let c = |f: fn(&mut PushOpts)| {
            let mut o = PushOpts {
                confirmed: true,
                ..Default::default()
            };
            f(&mut o);
            push_argv(Some("origin"), &o)
        };
        assert!(bad(c(|o| o.delete = true)), "delete names nothing");
        assert!(bad(c(|o| {
            o.delete = true;
            o.refspecs = vec!["x".into()];
            o.tags = true;
        })));
        assert!(bad(c(|o| {
            o.set_upstream = true;
            o.no_track = true;
        })));
        assert!(bad(c(|o| {
            o.all = true;
            o.refspecs = vec!["x".into()];
        })));
        assert!(bad(c(|o| o.overwrite = Some("--force".into()))));
        assert!(bad(c(|o| o.refspecs = vec!["--force".into()])));
        assert!(bad(c(|o| o.refspecs = vec!["a:b:c".into()])));
        assert!(bad(c(|o| o.refspecs = vec![":x".into()])));
    }

    #[test]
    fn remote_argv_spells_out_each_option_and_keeps_the_token_off_argv() {
        assert_eq!(
            remote_add_argv("origin", "https://h.example/r", true, true).unwrap(),
            s(&[
                "remote",
                "add",
                "--fetch",
                "--token-stdin",
                "--",
                "origin",
                "https://h.example/r"
            ])
        );
        assert_eq!(
            remote_add_argv("origin", "https://h.example/r", false, false).unwrap(),
            s(&["remote", "add", "--", "origin", "https://h.example/r"])
        );
        assert!(bad(remote_add_argv(
            "-x",
            "https://h.example/r",
            false,
            false
        )));
        assert_eq!(
            remote_rename_argv("a", "b").unwrap(),
            s(&["remote", "rename", "--", "a", "b"])
        );
        assert!(bad(remote_rename_argv("a", "--b")));
        assert!(bad(remote_rename_argv("a b", "c")));
        assert_eq!(
            remote_set_url_argv("origin", "https://h.example/r", true).unwrap(),
            s(&[
                "remote",
                "set-url",
                "--push",
                "--",
                "origin",
                "https://h.example/r"
            ])
        );
        assert_eq!(
            remote_set_url_argv("origin", "https://h.example/r", false).unwrap(),
            s(&["remote", "set-url", "--", "origin", "https://h.example/r"])
        );
        assert!(bad(remote_set_url_argv("origin", "ftp://h/r", false)));
        assert!(bad(remote_set_url_argv("origin", "-u", false)));
        assert_eq!(
            remote_show_argv("origin").unwrap(),
            s(&["remote", "show", "--", "origin"])
        );
        assert!(bad(remote_show_argv("--all")));
    }

    #[test]
    fn lock_flags_carry_owner_and_remote_as_single_elements() {
        assert!(lock_flags(None, None).unwrap().is_empty());
        assert!(lock_flags(Some(""), Some("")).unwrap().is_empty());
        assert_eq!(
            lock_flags(Some("Ana B"), Some("upstream")).unwrap(),
            s(&["--owner=Ana B", "--remote=upstream"])
        );
        let base = s(&["lock", "create", "--", "a.psd"]);
        assert_eq!(
            with_lock_flags(base, Some("ana"), Some("o")).unwrap(),
            s(&["lock", "create", "--owner=ana", "--remote=o", "--", "a.psd"])
        );
        assert!(bad(lock_flags(Some("-x"), None)));
        assert!(bad(lock_flags(Some("a\nb"), None)));
        assert!(bad(lock_flags(None, Some("--x"))));
        assert!(bad(lock_flags(None, Some("a b"))));
    }

    #[test]
    fn init_and_clone_branch_flags_are_validated_single_elements() {
        let init = s(&["init", "--", "/p"]);
        assert_eq!(
            with_branch(init.clone(), 1, "initial-branch", Some("trunk")).unwrap(),
            s(&["init", "--initial-branch=trunk", "--", "/p"])
        );
        assert_eq!(
            with_branch(init.clone(), 1, "initial-branch", Some("")).unwrap(),
            init
        );
        assert_eq!(
            with_branch(init.clone(), 1, "initial-branch", None).unwrap(),
            init
        );
        let clone = s(&["clone", "--", "u", "/d"]);
        assert_eq!(
            with_branch(clone, 1, "branch", Some("dev")).unwrap(),
            s(&["clone", "--branch=dev", "--", "u", "/d"])
        );
        assert!(bad(with_branch(
            init.clone(),
            1,
            "initial-branch",
            Some("--bare")
        )));
        assert!(bad(with_branch(init, 1, "initial-branch", Some("a b"))));
    }

    #[test]
    fn add_flags_follow_the_options() {
        assert!(add_flags(&AddOpts::default()).is_empty());
        let o = AddOpts {
            all: true,
            dry_run: true,
            force: true,
            update: true,
            ignore_removal: true,
        };
        assert_eq!(
            add_flags(&o),
            s(&[
                "--all",
                "--dry-run",
                "--force",
                "--update",
                "--ignore-removal"
            ])
        );
    }

    #[test]
    fn commit_options_become_flags_and_refuse_bad_values() {
        let dir = std::env::temp_dir();
        let repo = dir.as_path();
        let o = CommitOpts {
            all: true,
            allow_empty: true,
            signoff: true,
            dry_run: true,
            author: Some("Ana B <ana@x.example>".into()),
            date: Some("2026-01-15T10:30:00Z".into()),
            include: vec!["art/a.psd".into()],
            ..Default::default()
        };
        assert_eq!(
            commit_argv(repo, "fix --machine", &o).unwrap(),
            s(&[
                "commit",
                "--message=fix --machine",
                "--all",
                "--allow-empty",
                "--signoff",
                "--dry-run",
                "--author=Ana B <ana@x.example>",
                "--date=2026-01-15T10:30:00Z",
                "--include=art/a.psd",
            ])
        );
        fn with(repo: &Path, f: impl FnOnce(&mut CommitOpts)) -> Result<Vec<String>, EngineError> {
            let mut o = CommitOpts::default();
            f(&mut o);
            commit_argv(repo, "m", &o)
        }
        assert!(bad(with(repo, |o| o.author = Some("Ana".into()))));
        assert!(bad(with(repo, |o| o.author = Some("<a@b>".into()))));
        assert!(bad(with(repo, |o| o.author = Some("--x <a@b>".into()))));
        assert!(bad(with(repo, |o| o.date = Some("yesterday".into()))));
        assert!(bad(
            with(repo, |o| o.date = Some("2026-01-15T10:30:00Z\nx".into()))
        ));
        assert!(bad(with(repo, |o| o.include = vec!["../x".into()])));
        assert!(bad(with(repo, |o| o.include = vec!["--all".into()])));
        assert!(bad(
            with(repo, |o| o.message_file = Some("relative.txt".into()))
        ));
        assert!(bad(with(repo, |o| o.message_file =
            Some(dir.join("no-such-file-zz.txt").to_string_lossy().into()))));
        assert!(
            bad(commit_argv(repo, "  ", &CommitOpts::default())),
            "message still required"
        );
    }

    #[test]
    fn commit_message_file_replaces_the_message() {
        let file = std::env::temp_dir().join(format!("mg-msg-{}.txt", std::process::id()));
        std::fs::write(&file, "hello").unwrap();
        let o = CommitOpts {
            message_file: Some(file.to_string_lossy().into()),
            ..Default::default()
        };
        let argv = commit_argv(&std::env::temp_dir(), "", &o).unwrap();
        std::fs::remove_file(&file).ok();
        assert_eq!(argv.len(), 2);
        assert!(argv[1].starts_with("--file="));
    }

    #[test]
    fn unstage_resets_head_for_paths_only() {
        let repo = std::env::temp_dir();
        assert_eq!(
            unstage_argv(&repo, &s(&["a/b.psd", "."])).unwrap(),
            s(&["reset", "HEAD", "--", "a/b.psd", "."])
        );
        assert!(bad(unstage_argv(&repo, &[])));
        assert!(bad(unstage_argv(&repo, &s(&["--hard"]))));
        assert!(bad(unstage_argv(&repo, &s(&["../x"]))));
    }
}

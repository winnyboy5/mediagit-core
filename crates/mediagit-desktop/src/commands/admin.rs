// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Commands for Storage, Maintenance, Security, Advanced and Settings.
//!
//! Secrets (passphrases, passwords, recovery codes) travel one way: webview argument ->
//! [`SecretCmd::secrets`] -> the child's stdin. The argv builders below never receive them
//! as part of argv, and the tests assert that none ends up there.

use std::path::{Path, PathBuf};

use serde_json::Value;
use tauri::ipc::Channel;

use super::{
    Ops, RepoQueues, Secret, as_args, clone_source_ok, invalid, plain_arg, ref_name_ok, repo_path,
    rev_ok, stream_with,
};
use crate::engine::{Engine, EngineError, Progress, is_unlock_failure};

/// A command line plus the secrets its prompts will ask for, kept apart on purpose.
pub struct SecretCmd {
    argv: Vec<String>,
    secrets: Vec<Secret>,
}

impl SecretCmd {
    fn plain(argv: Vec<String>) -> Self {
        Self {
            argv,
            secrets: Vec::new(),
        }
    }

    /// `secrets` are the answers to the command's prompts, in order; blanks are skipped.
    fn with(argv: Vec<String>, secrets: Vec<Option<String>>) -> Result<Self, EngineError> {
        let secrets = secrets
            .into_iter()
            .flatten()
            .map(Secret::new)
            .collect::<Result<_, _>>()?;
        Ok(Self { argv, secrets })
    }
}

fn optional_repo(repo: Option<&str>) -> Result<Option<PathBuf>, EngineError> {
    repo.map(repo_path).transpose()
}

async fn run(engine: &Engine, repo: Option<&Path>, cmd: &SecretCmd) -> Result<Value, EngineError> {
    engine
        .run_with(repo, &as_args(&cmd.argv), &cmd.secrets, |_| {})
        .await
}

// ---------------------------------------------------------------------------
// argv builders (pure, unit-tested)
// ---------------------------------------------------------------------------

/// Any command that may ask for the encryption passphrase.
fn with_passphrase(
    argv: Vec<String>,
    passphrase: Option<String>,
) -> Result<SecretCmd, EngineError> {
    SecretCmd::with(argv, vec![passphrase])
}

fn gc_argv(dry_run: bool) -> Vec<String> {
    let mut argv = vec!["gc".to_owned()];
    // Only a confirmed, real run may skip gc's own confirmation prompt.
    argv.push(if dry_run { "--dry-run" } else { "--yes" }.into());
    argv
}

fn fsck_argv(deep: bool) -> Vec<String> {
    vec![
        "fsck".into(),
        if deep { "--full" } else { "--quick" }.into(),
    ]
}

fn verify_argv(quick: bool) -> Vec<String> {
    let mut argv = vec!["verify".to_owned()];
    if quick {
        argv.push("--quick".into());
    }
    argv
}

/// An http(s) server address.
fn server_ok(url: &str) -> bool {
    (url.starts_with("http://") || url.starts_with("https://")) && clone_source_ok(url)
}

fn server_flag(argv: &mut Vec<String>, server: Option<&str>) -> Result<(), EngineError> {
    if let Some(s) = server.filter(|s| !s.is_empty()) {
        if !server_ok(s) {
            return Err(invalid("server address", s));
        }
        argv.push(format!("--server={s}"));
    }
    Ok(())
}

/// `auth login [--server=..] [--username=..]`; the password is a secret, never an argument.
fn auth_login_cmd(
    server: Option<&str>,
    username: Option<&str>,
    password: String,
) -> Result<SecretCmd, EngineError> {
    let mut argv = vec!["auth".to_owned(), "login".into()];
    server_flag(&mut argv, server)?;
    if let Some(u) = username.filter(|u| !u.is_empty()) {
        plain_arg("username", u)?;
        if u.chars().any(char::is_control) {
            return Err(invalid("username", u));
        }
        argv.push(format!("--username={u}"));
    }
    SecretCmd::with(argv, vec![Some(password)])
}

fn auth_argv(sub: &str, server: Option<&str>, all: bool) -> Result<Vec<String>, EngineError> {
    if !matches!(sub, "status" | "whoami" | "logout") {
        return Err(invalid("auth action", sub));
    }
    let mut argv = vec!["auth".to_owned(), sub.to_owned()];
    server_flag(&mut argv, server)?;
    if all && sub == "logout" {
        argv.push("--all".into());
    }
    Ok(argv)
}

/// `key init`; the master passphrase, if the machine needs one, is a secret.
fn key_init_cmd(passphrase: Option<String>) -> Result<SecretCmd, EngineError> {
    with_passphrase(vec!["key".into(), "init".into()], passphrase)
}

/// `key recover` with the code on stdin (it is refused in argv), then the new passphrase.
/// The code on stdin turns `--secret-stdin` on, so the CLI reads its master passphrase from
/// the next line and never falls back to the keychain: a missing one would only fail at EOF.
fn key_recover_cmd(code: String, passphrase: Option<String>) -> Result<SecretCmd, EngineError> {
    if passphrase.as_deref().is_none_or(str::is_empty) {
        return Err(invalid("passphrase", "required to recover"));
    }
    SecretCmd::with(
        vec!["key".into(), "recover".into()],
        vec![Some(code), passphrase],
    )
}

/// The settings the app edits; the CLI refuses any other key too.
const CONFIG_KEYS: [&str; 4] = [
    "author.name",
    "author.email",
    "performance.upload_concurrency",
    "performance.download_concurrency",
];

fn config_key_ok(key: &str) -> Result<(), EngineError> {
    if CONFIG_KEYS.contains(&key) {
        Ok(())
    } else {
        Err(invalid("setting", key))
    }
}

fn config_set_argv(key: &str, value: &str) -> Result<Vec<String>, EngineError> {
    config_key_ok(key)?;
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        return Err(invalid("value", value));
    }
    Ok(vec![
        "config".into(),
        "set".into(),
        "--".into(),
        key.into(),
        value.into(),
    ])
}

fn config_unset_argv(key: &str) -> Result<Vec<String>, EngineError> {
    config_key_ok(key)?;
    Ok(vec![
        "config".into(),
        "unset".into(),
        "--".into(),
        key.into(),
    ])
}

fn sparse_set_argv(patterns: &[String], pattern_mode: bool) -> Result<Vec<String>, EngineError> {
    if patterns.is_empty() {
        return Err(invalid("patterns", "none"));
    }
    let mut argv = vec!["sparse-checkout".to_owned(), "set".into()];
    if pattern_mode {
        argv.push("--patterns".into());
    }
    argv.push("--".into());
    for p in patterns {
        let bad = p.is_empty()
            || p.len() > 512
            || p.starts_with('-')
            || p.contains('\\')
            || p.chars().any(char::is_control)
            || p.split('/').any(|s| s == "..");
        if bad {
            return Err(invalid("pattern", p));
        }
        argv.push(p.clone());
    }
    Ok(argv)
}

/// A branch, tag or commit id.
fn target_ok(what: &str, t: &str) -> Result<(), EngineError> {
    if rev_ok(t).is_ok() {
        Ok(())
    } else {
        ref_name_ok(what, t)
    }
}

fn rebase_start_argv(upstream: &str) -> Result<Vec<String>, EngineError> {
    target_ok("branch", upstream)?;
    Ok(vec!["rebase".into(), "--".into(), upstream.into()])
}

/// `rebase|cherry-pick --continue|--abort|--skip`.
fn control_argv(cmd: &str, action: &str) -> Result<Vec<String>, EngineError> {
    if !matches!(cmd, "rebase" | "cherry-pick") || !matches!(action, "continue" | "abort" | "skip")
    {
        return Err(invalid("action", action));
    }
    Ok(vec![cmd.into(), format!("--{action}")])
}

fn pick_start_argv(commits: &[String]) -> Result<Vec<String>, EngineError> {
    if commits.is_empty() {
        return Err(invalid("commits", "none"));
    }
    let mut argv = vec!["cherry-pick".to_owned(), "--".into()];
    for c in commits {
        rev_ok(c)?;
        argv.push(c.clone());
    }
    Ok(argv)
}

fn bisect_start_argv(bad: &str, good: Option<&str>) -> Result<Vec<String>, EngineError> {
    target_ok("commit", bad)?;
    let mut argv = vec!["bisect".to_owned(), "start".into(), "--".into(), bad.into()];
    if let Some(g) = good {
        target_ok("commit", g)?;
        argv.push(g.into());
    }
    Ok(argv)
}

/// `bisect good|bad|skip [-- <commit>]`; no commit marks the one being tested.
fn bisect_mark_argv(verdict: &str, commit: Option<&str>) -> Result<Vec<String>, EngineError> {
    if !matches!(verdict, "good" | "bad" | "skip") {
        return Err(invalid("verdict", verdict));
    }
    let mut argv = vec!["bisect".to_owned(), verdict.into()];
    if let Some(c) = commit {
        rev_ok(c)?;
        argv.extend(["--".into(), c.into()]);
    }
    Ok(argv)
}

fn reflog_argv(reference: &str, count: u32) -> Result<Vec<String>, EngineError> {
    if !(1..=1000).contains(&count) {
        return Err(invalid("count", &count.to_string()));
    }
    if reference != "HEAD" {
        ref_name_ok("ref", reference)?;
    }
    Ok(vec![
        "reflog".into(),
        "show".into(),
        "-n".into(),
        count.to_string(),
        "--".into(),
        reference.into(),
    ])
}

// ---------------------------------------------------------------------------
// commands
// ---------------------------------------------------------------------------

/// Unlock an encrypted repository for this session: the passphrase is checked with a cheap
/// read, then answers that repository's prompts (on stdin, in this process only) until
/// `lock_repo` or exit. A wrong passphrase leaves it locked.
#[tauri::command]
pub async fn unlock_repo(
    engine: tauri::State<'_, Engine>,
    repo: String,
    passphrase: String,
) -> Result<(), EngineError> {
    let repo = repo_path(&repo)?;
    let secret = Secret::new(passphrase)?;
    engine.lock(&repo);
    let cmd = SecretCmd {
        argv: vec!["log".into(), "-n".into(), "1".into()],
        secrets: vec![secret.duplicate()],
    };
    // Only the CLI's own "that key did not open it" is a wrong passphrase; anything else (a
    // missing repository, a broken store) is reported as it is, not asked about again.
    run(&engine, Some(&repo), &cmd).await.map_err(|e| {
        if is_unlock_failure(&e) {
            EngineError::Cli {
                code: "wrong_passphrase".into(),
                message: "That passphrase did not unlock the repository.".into(),
            }
        } else {
            e
        }
    })?;
    engine.unlock(&repo, secret);
    Ok(())
}

/// Forget the session passphrase for a repository.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // tauri injects State by value
pub fn lock_repo(engine: tauri::State<'_, Engine>, repo: String) -> Result<(), EngineError> {
    engine.lock(&repo_path(&repo)?);
    Ok(())
}

/// Repository size and shape. `passphrase` answers an encryption prompt on retry.
#[tauri::command]
pub async fn repo_stats(
    engine: tauri::State<'_, Engine>,
    repo: String,
    passphrase: Option<String>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    run(
        &engine,
        Some(&repo),
        &with_passphrase(vec!["stats".into()], passphrase)?,
    )
    .await
}

/// Clean up unreachable data. A real run needs `confirmed`; `dry_run` only reports.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn repo_gc(
    engine: tauri::State<'_, Engine>,
    ops: tauri::State<'_, Ops>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    dry_run: bool,
    confirmed: bool,
    passphrase: Option<String>,
    on_progress: Channel<Progress>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    if !dry_run && !confirmed {
        return Err(invalid("clean-up", "not confirmed"));
    }
    let cmd = with_passphrase(gc_argv(dry_run), passphrase)?;
    // A dry run changes nothing, so it does not wait behind (or block) writes.
    let own = RepoQueues::default();
    let lane: &RepoQueues = if dry_run { &own } else { &queues };
    stream_with(
        &engine,
        &ops,
        lane,
        &repo,
        &cmd.argv,
        &cmd.secrets,
        "",
        false,
        &on_progress,
    )
    .await
}

/// Check every object and ref. `deep` re-reads object content.
#[tauri::command]
pub async fn repo_fsck(
    engine: tauri::State<'_, Engine>,
    ops: tauri::State<'_, Ops>,
    repo: String,
    deep: bool,
    passphrase: Option<String>,
    on_progress: Channel<Progress>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let cmd = with_passphrase(fsck_argv(deep), passphrase)?;
    let lane = RepoQueues::default();
    stream_with(
        &engine,
        &ops,
        &lane,
        &repo,
        &cmd.argv,
        &cmd.secrets,
        "",
        false,
        &on_progress,
    )
    .await
}

/// Verify the whole repository against its hashes.
#[tauri::command]
pub async fn repo_verify(
    engine: tauri::State<'_, Engine>,
    ops: tauri::State<'_, Ops>,
    repo: String,
    quick: bool,
    passphrase: Option<String>,
    on_progress: Channel<Progress>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let cmd = with_passphrase(verify_argv(quick), passphrase)?;
    let lane = RepoQueues::default();
    stream_with(
        &engine,
        &ops,
        &lane,
        &repo,
        &cmd.argv,
        &cmd.secrets,
        "",
        false,
        &on_progress,
    )
    .await
}

/// Sign in to a server. The password is passed on stdin only.
#[tauri::command]
pub async fn auth_login(
    engine: tauri::State<'_, Engine>,
    repo: Option<String>,
    server: Option<String>,
    username: Option<String>,
    password: String,
) -> Result<Value, EngineError> {
    let repo = optional_repo(repo.as_deref())?;
    let cmd = auth_login_cmd(server.as_deref(), username.as_deref(), password)?;
    run(&engine, repo.as_deref(), &cmd).await
}

/// Who is signed in (`status`, `whoami`) or sign out (`logout`).
#[tauri::command]
pub async fn auth_run(
    engine: tauri::State<'_, Engine>,
    repo: Option<String>,
    action: String,
    server: Option<String>,
    all: bool,
) -> Result<Value, EngineError> {
    let repo = optional_repo(repo.as_deref())?;
    let cmd = SecretCmd::plain(auth_argv(&action, server.as_deref(), all)?);
    run(&engine, repo.as_deref(), &cmd).await
}

/// Whether the repository is encrypted and how it unlocks.
#[tauri::command]
pub async fn key_status(
    engine: tauri::State<'_, Engine>,
    repo: String,
    passphrase: Option<String>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let cmd = with_passphrase(vec!["key".into(), "status".into()], passphrase)?;
    run(&engine, Some(&repo), &cmd).await
}

/// Turn on encryption. The report carries the one-time recovery code.
#[tauri::command]
pub async fn key_init(
    engine: tauri::State<'_, Engine>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    passphrase: Option<String>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let cmd = key_init_cmd(passphrase)?;
    queues.write(&repo, run(&engine, Some(&repo), &cmd)).await
}

/// Unlock with the recovery code and re-wrap the key for this machine.
#[tauri::command]
pub async fn key_recover(
    engine: tauri::State<'_, Engine>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    code: String,
    passphrase: Option<String>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let cmd = key_recover_cmd(code, passphrase)?;
    queues.write(&repo, run(&engine, Some(&repo), &cmd)).await
}

/// Every setting.
#[tauri::command]
pub async fn config_list(
    engine: tauri::State<'_, Engine>,
    repo: String,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    run(
        &engine,
        Some(&repo),
        &SecretCmd::plain(vec!["config".into(), "list".into()]),
    )
    .await
}

#[tauri::command]
pub async fn config_set(
    engine: tauri::State<'_, Engine>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    key: String,
    value: String,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let cmd = SecretCmd::plain(config_set_argv(&key, &value)?);
    queues.write(&repo, run(&engine, Some(&repo), &cmd)).await
}

#[tauri::command]
pub async fn config_unset(
    engine: tauri::State<'_, Engine>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    key: String,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let cmd = SecretCmd::plain(config_unset_argv(&key)?);
    queues.write(&repo, run(&engine, Some(&repo), &cmd)).await
}

/// The sparse-checkout patterns in force.
#[tauri::command]
pub async fn sparse_list(
    engine: tauri::State<'_, Engine>,
    repo: String,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let cmd = SecretCmd::plain(vec!["sparse-checkout".into(), "list".into()]);
    run(&engine, Some(&repo), &cmd).await
}

/// Replace the patterns; files outside them leave the disk, files inside come back.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn sparse_set(
    engine: tauri::State<'_, Engine>,
    ops: tauri::State<'_, Ops>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    patterns: Vec<String>,
    pattern_mode: bool,
    on_progress: Channel<Progress>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let argv = sparse_set_argv(&patterns, pattern_mode)?;
    stream_with(
        &engine,
        &ops,
        &queues,
        &repo,
        &argv,
        &[],
        "",
        false,
        &on_progress,
    )
    .await
}

/// Materialize every file again.
#[tauri::command]
pub async fn sparse_disable(
    engine: tauri::State<'_, Engine>,
    ops: tauri::State<'_, Ops>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    on_progress: Channel<Progress>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let argv = vec!["sparse-checkout".to_owned(), "disable".into()];
    stream_with(
        &engine,
        &ops,
        &queues,
        &repo,
        &argv,
        &[],
        "",
        false,
        &on_progress,
    )
    .await
}

/// Replay this branch onto `upstream`. A conflict is a normal result, with the paths.
#[tauri::command]
pub async fn rebase_start(
    engine: tauri::State<'_, Engine>,
    ops: tauri::State<'_, Ops>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    upstream: String,
    on_progress: Channel<Progress>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let argv = rebase_start_argv(&upstream)?;
    stream_with(
        &engine,
        &ops,
        &queues,
        &repo,
        &argv,
        &[],
        "",
        false,
        &on_progress,
    )
    .await
}

/// `continue`, `abort` or `skip` a rebase (`rebase`) or cherry-pick (`cherry-pick`) in progress.
#[tauri::command]
pub async fn sequencer_control(
    engine: tauri::State<'_, Engine>,
    ops: tauri::State<'_, Ops>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    command: String,
    action: String,
    on_progress: Channel<Progress>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let argv = control_argv(&command, &action)?;
    stream_with(
        &engine,
        &ops,
        &queues,
        &repo,
        &argv,
        &[],
        "",
        false,
        &on_progress,
    )
    .await
}

/// Copy the changes of one or more versions onto this branch.
#[tauri::command]
pub async fn pick_start(
    engine: tauri::State<'_, Engine>,
    ops: tauri::State<'_, Ops>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    commits: Vec<String>,
    on_progress: Channel<Progress>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let argv = pick_start_argv(&commits)?;
    stream_with(
        &engine,
        &ops,
        &queues,
        &repo,
        &argv,
        &[],
        "",
        false,
        &on_progress,
    )
    .await
}

/// Start a bisect between a bad version and (optionally) a good one.
#[tauri::command]
pub async fn bisect_start(
    engine: tauri::State<'_, Engine>,
    ops: tauri::State<'_, Ops>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    bad: String,
    good: Option<String>,
    on_progress: Channel<Progress>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let argv = bisect_start_argv(&bad, good.as_deref())?;
    stream_with(
        &engine,
        &ops,
        &queues,
        &repo,
        &argv,
        &[],
        "",
        false,
        &on_progress,
    )
    .await
}

/// Mark the version being tested (or `commit`) `good`, `bad` or `skip`.
#[tauri::command]
pub async fn bisect_mark(
    engine: tauri::State<'_, Engine>,
    ops: tauri::State<'_, Ops>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    verdict: String,
    commit: Option<String>,
    on_progress: Channel<Progress>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let argv = bisect_mark_argv(&verdict, commit.as_deref())?;
    stream_with(
        &engine,
        &ops,
        &queues,
        &repo,
        &argv,
        &[],
        "",
        false,
        &on_progress,
    )
    .await
}

/// End the bisect and go back to where it started.
#[tauri::command]
pub async fn bisect_reset(
    engine: tauri::State<'_, Engine>,
    ops: tauri::State<'_, Ops>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    on_progress: Channel<Progress>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let argv = vec!["bisect".to_owned(), "reset".into()];
    stream_with(
        &engine,
        &ops,
        &queues,
        &repo,
        &argv,
        &[],
        "",
        false,
        &on_progress,
    )
    .await
}

/// Where the bisect stands, without changing anything.
#[tauri::command]
pub async fn bisect_log(
    engine: tauri::State<'_, Engine>,
    repo: String,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let cmd = SecretCmd::plain(vec!["bisect".into(), "log".into()]);
    run(&engine, Some(&repo), &cmd).await
}

/// One ref's recent positions, newest first.
#[tauri::command]
pub async fn reflog_list(
    engine: tauri::State<'_, Engine>,
    repo: String,
    reference: String,
    count: u32,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let cmd = SecretCmd::plain(reflog_argv(&reference, count)?);
    run(&engine, Some(&repo), &cmd).await
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[allow(clippy::needless_pass_by_value)] // call sites pass temporaries
    fn bad<T>(r: Result<T, EngineError>) -> bool {
        matches!(r, Err(EngineError::InvalidArgument(_)))
    }

    const PASS: &str = "hunter2-Zebra!9";

    /// No part of any command line, or of the Debug text of what we pass around, is a secret.
    fn clean(cmd: &SecretCmd, secrets: &[&str]) {
        for s in secrets {
            assert!(
                cmd.argv.iter().all(|a| !a.contains(s)),
                "secret in argv: {:?}",
                cmd.argv
            );
            assert!(!format!("{:?}", cmd.secrets).contains(s));
            assert!(
                cmd.secrets.iter().any(|x| x.is(s)),
                "secret not queued for stdin"
            );
        }
    }

    #[test]
    fn secrets_go_to_stdin_never_to_argv() {
        let login = auth_login_cmd(Some("https://s.example"), Some("mira"), PASS.into()).unwrap();
        assert_eq!(
            login.argv,
            [
                "auth",
                "login",
                "--server=https://s.example",
                "--username=mira"
            ]
        );
        clean(&login, &[PASS]);
        clean(&key_init_cmd(Some(PASS.into())).unwrap(), &[PASS]);
        let rec = key_recover_cmd("ABCD-EFGH-IJKL".into(), Some(PASS.into())).unwrap();
        assert_eq!(rec.argv, ["key", "recover"]);
        clean(&rec, &[PASS, "ABCD-EFGH-IJKL"]);
        for blank in [None, Some(String::new())] {
            assert!(key_recover_cmd("ABCD-EFGH-IJKL".into(), blank).is_err());
        }
        // Retrying any object-touching command with the passphrase.
        for argv in [
            vec!["stats".to_owned()],
            gc_argv(true),
            fsck_argv(true),
            verify_argv(false),
        ] {
            clean(&with_passphrase(argv, Some(PASS.into())).unwrap(), &[PASS]);
        }
        // No secret at all: no stdin lines, so the engine will not add --secret-stdin.
        assert!(
            with_passphrase(fsck_argv(false), None)
                .unwrap()
                .secrets
                .is_empty()
        );
    }

    #[test]
    fn secrets_that_would_split_a_prompt_are_refused_without_echo() {
        for s in ["", "a\nb", "a\rb", "a\0b"] {
            let e = key_init_cmd(Some(s.into())).err().unwrap();
            assert!(matches!(e, EngineError::InvalidArgument(_)));
        }
        assert!(bad(auth_login_cmd(None, None, "x".repeat(2000))));
        assert_eq!(
            format!("{:?}", Secret::new(PASS.into()).unwrap()),
            "Secret(***)"
        );
    }

    #[test]
    fn auth_argv_validates_server_username_and_action() {
        assert_eq!(
            auth_argv("status", None, false).unwrap(),
            ["auth", "status"]
        );
        assert_eq!(
            auth_argv("logout", Some("https://s.example"), true).unwrap(),
            ["auth", "logout", "--server=https://s.example", "--all"]
        );
        assert_eq!(auth_argv("whoami", None, true).unwrap(), ["auth", "whoami"]);
        assert!(bad(auth_argv("login", None, false)));
        assert!(bad(auth_argv("status", Some("--token=x"), false)));
        assert!(bad(auth_argv("status", Some("ftp://h"), false)));
        assert!(bad(auth_argv("status", Some("https://h x"), false)));
        assert!(bad(auth_login_cmd(None, Some("--api-key"), "p".into())));
        assert!(bad(auth_login_cmd(Some("file:///x"), None, "p".into())));
    }

    #[test]
    fn gc_only_skips_its_prompt_for_a_real_run_and_never_both() {
        assert_eq!(gc_argv(true), ["gc", "--dry-run"]);
        assert_eq!(gc_argv(false), ["gc", "--yes"]);
        assert_eq!(fsck_argv(false), ["fsck", "--quick"]);
        assert_eq!(fsck_argv(true), ["fsck", "--full"]);
        assert_eq!(verify_argv(true), ["verify", "--quick"]);
        assert_eq!(verify_argv(false), ["verify"]);
    }

    #[test]
    fn config_and_sparse_argv_keep_values_positional() {
        assert_eq!(
            config_set_argv("author.name", "-Mira Okafor").unwrap(),
            ["config", "set", "--", "author.name", "-Mira Okafor"]
        );
        assert_eq!(
            config_unset_argv("author.email").unwrap(),
            ["config", "unset", "--", "author.email"]
        );
        for k in ["--all", "core.editor", "", "author.name --global"] {
            assert!(bad(config_set_argv(k, "v")), "{k}");
            assert!(bad(config_unset_argv(k)), "{k}");
        }
        assert!(bad(config_set_argv("author.name", "")));
        assert!(bad(config_set_argv("author.name", "a\nb")));
        assert_eq!(
            sparse_set_argv(&["art/".into(), "/video/".into()], true).unwrap(),
            [
                "sparse-checkout",
                "set",
                "--patterns",
                "--",
                "art/",
                "/video/"
            ]
        );
        assert_eq!(
            sparse_set_argv(&["art".into()], false).unwrap(),
            ["sparse-checkout", "set", "--", "art"]
        );
        assert!(bad(sparse_set_argv(&[], false)));
        for p in ["", "-x", "../x", "a/../b", "a\\b", "a\nb"] {
            assert!(bad(sparse_set_argv(&[p.into()], false)), "{p:?}");
        }
    }

    #[test]
    fn sequencer_argv_guards_targets_and_actions() {
        let oid = "0123456789abcdef";
        assert_eq!(rebase_start_argv("main").unwrap(), ["rebase", "--", "main"]);
        assert!(bad(rebase_start_argv("--abort")));
        assert!(bad(rebase_start_argv("a b")));
        assert_eq!(
            control_argv("rebase", "skip").unwrap(),
            ["rebase", "--skip"]
        );
        assert_eq!(
            control_argv("cherry-pick", "continue").unwrap(),
            ["cherry-pick", "--continue"]
        );
        assert!(bad(control_argv("merge", "abort")));
        assert!(bad(control_argv("rebase", "--abort")));
        assert_eq!(
            pick_start_argv(&[oid.into(), "HEAD".into()]).unwrap(),
            ["cherry-pick", "--", oid, "HEAD"]
        );
        assert!(bad(pick_start_argv(&[])));
        assert!(bad(pick_start_argv(&["main".into()])));
        assert!(bad(pick_start_argv(&["--no-commit".into()])));
        assert_eq!(
            bisect_start_argv("HEAD", Some(oid)).unwrap(),
            ["bisect", "start", "--", "HEAD", oid]
        );
        assert_eq!(
            bisect_start_argv("main", None).unwrap(),
            ["bisect", "start", "--", "main"]
        );
        assert!(bad(bisect_start_argv("--reset", None)));
        assert_eq!(bisect_mark_argv("good", None).unwrap(), ["bisect", "good"]);
        assert_eq!(
            bisect_mark_argv("skip", Some(oid)).unwrap(),
            ["bisect", "skip", "--", oid]
        );
        assert!(bad(bisect_mark_argv("reset", None)));
        assert!(bad(bisect_mark_argv("good", Some("main"))));
    }

    #[test]
    fn reflog_argv_bounds_the_count_and_guards_the_ref() {
        assert_eq!(
            reflog_argv("HEAD", 50).unwrap(),
            ["reflog", "show", "-n", "50", "--", "HEAD"]
        );
        assert!(reflog_argv("main", 5).is_ok());
        assert!(bad(reflog_argv("HEAD", 0)));
        assert!(bad(reflog_argv("HEAD", 5000)));
        assert!(bad(reflog_argv("--all", 5)));
    }
}

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

use super::views_b::{
    PickOpts, bisect_replay_argv, bisect_reset_argv, bisect_start_argv, pick_start_argv,
    rebase_start_argv, target_ok,
};
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
    pub(super) fn plain(argv: Vec<String>) -> Self {
        Self {
            argv,
            secrets: Vec::new(),
        }
    }

    /// `secrets` are the answers to the command's prompts, in order; blanks are skipped.
    pub(super) fn with(
        argv: Vec<String>,
        secrets: Vec<Option<String>>,
    ) -> Result<Self, EngineError> {
        let secrets = secrets
            .into_iter()
            .flatten()
            .map(Secret::new)
            .collect::<Result<_, _>>()?;
        Ok(Self { argv, secrets })
    }
}

#[cfg(test)]
impl SecretCmd {
    pub(super) fn argv(&self) -> &[String] {
        &self.argv
    }
    pub(super) fn secret_count(&self) -> usize {
        self.secrets.len()
    }
}

fn optional_repo(repo: Option<&str>) -> Result<Option<PathBuf>, EngineError> {
    repo.map(repo_path).transpose()
}

pub(super) async fn run(
    engine: &Engine,
    repo: Option<&Path>,
    cmd: &SecretCmd,
) -> Result<Value, EngineError> {
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

/// Options of `gc` beyond preview-or-run.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GcOptions {
    /// Keep data nothing refers to (`--no-prune`).
    pub no_prune: bool,
    /// Only clean up when it is worth it (`--auto`).
    pub auto: bool,
    /// Pack loose objects together (`--repack`).
    pub repack: bool,
}

/// A real clean-up deletes data; only a dry run may go ahead unconfirmed.
fn gc_guard(dry_run: bool, confirmed: bool) -> Result<(), EngineError> {
    if !dry_run && !confirmed {
        return Err(invalid("clean-up", "not confirmed"));
    }
    Ok(())
}

fn gc_argv(dry_run: bool, o: &GcOptions) -> Vec<String> {
    let mut argv = vec!["gc".to_owned()];
    // Only a confirmed, real run may skip gc's own confirmation prompt.
    argv.push(if dry_run { "--dry-run" } else { "--yes" }.into());
    for (on, flag) in [
        (o.no_prune, "--no-prune"),
        (o.auto, "--auto"),
        (o.repack, "--repack"),
    ] {
        if on {
            argv.push(flag.into());
        }
    }
    argv
}

/// Options of `fsck`. `repair` changes the repository, so a real repair needs `confirmed`;
/// `dry_run` only makes sense with `repair` (it shows what a repair would do).
#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FsckOptions {
    /// Deep check (`--full`) instead of the quick one (`--quick`).
    pub deep: bool,
    /// List every object checked (`--all`).
    pub all: bool,
    /// List lost and dangling objects (`--lost-found`).
    pub lost_found: bool,
    /// Skip the dangling-object check (`--no-dangling`).
    pub no_dangling: bool,
    /// Try to fix what it finds (`--repair`).
    pub repair: bool,
    /// With `repair`: report only (`--dry-run`).
    pub dry_run: bool,
    /// The person has been told what repair does and agreed.
    pub confirmed: bool,
}

fn fsck_argv(o: &FsckOptions) -> Result<Vec<String>, EngineError> {
    if o.dry_run && !o.repair {
        return Err(invalid("preview", "a preview needs repair switched on"));
    }
    if o.repair && !o.dry_run && !o.confirmed {
        return Err(invalid("repair", "not confirmed"));
    }
    let mut argv = vec![
        "fsck".to_owned(),
        if o.deep { "--full" } else { "--quick" }.into(),
    ];
    for (on, flag) in [
        (o.all, "--all"),
        (o.lost_found, "--lost-found"),
        (o.no_dangling, "--no-dangling"),
        (o.repair, "--repair"),
        (o.dry_run, "--dry-run"),
    ] {
        if on {
            argv.push(flag.into());
        }
    }
    Ok(argv)
}

/// Options of `verify`: the whole repository, one version (`commit`), or a stretch of history
/// (`start`..`end`); a version and a range exclude each other.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VerifyOptions {
    pub quick: bool,
    pub commit: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    /// Check every checksum (`--checksums`).
    pub checksums: bool,
    /// Report each problem in detail (`--detailed`).
    pub detailed: bool,
}

fn verify_argv(o: &VerifyOptions) -> Result<Vec<String>, EngineError> {
    fn blank(s: &Option<String>) -> Option<&str> {
        s.as_deref().map(str::trim).filter(|s| !s.is_empty())
    }
    let (commit, start, end) = (blank(&o.commit), blank(&o.start), blank(&o.end));
    if commit.is_some() && (start.is_some() || end.is_some()) {
        return Err(invalid("verify", "choose one version or a range, not both"));
    }
    let mut argv = vec!["verify".to_owned()];
    for (on, flag) in [
        (o.quick, "--quick"),
        (o.checksums, "--checksums"),
        (o.detailed, "--detailed"),
    ] {
        if on {
            argv.push(flag.into());
        }
    }
    for (v, flag) in [(start, "--start"), (end, "--end")] {
        if let Some(v) = v {
            target_ok("version", v)?;
            argv.push(format!("{flag}={v}"));
        }
    }
    if let Some(c) = commit {
        target_ok("version", c)?;
        argv.extend(["--".into(), c.into()]);
    }
    Ok(argv)
}

/// An http(s) server address.
pub(super) fn server_ok(url: &str) -> bool {
    (url.starts_with("http://") || url.starts_with("https://")) && clone_source_ok(url)
}

pub(super) fn server_flag(argv: &mut Vec<String>, server: Option<&str>) -> Result<(), EngineError> {
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

/// `auth register --server=.. --username=.. --email=..`: username and email are flags, the
/// new account's password is a secret on stdin.
fn auth_register_cmd(
    server: Option<&str>,
    username: &str,
    email: &str,
    password: String,
) -> Result<SecretCmd, EngineError> {
    for (what, v) in [("username", username), ("email", email)] {
        plain_arg(what, v)?;
        if v.chars().any(char::is_control) || v.len() > 128 {
            return Err(invalid(what, v));
        }
    }
    if email.split_whitespace().count() != 1 || !email.contains('@') {
        return Err(invalid("email", email));
    }
    let mut argv = vec!["auth".to_owned(), "register".into()];
    server_flag(&mut argv, server)?;
    argv.extend([format!("--username={username}"), format!("--email={email}")]);
    SecretCmd::with(argv, vec![Some(password)])
}

/// `auth passwd [--server=..]`: the current and the new password, in that order, on stdin.
fn auth_passwd_cmd(
    server: Option<&str>,
    current: String,
    new: String,
) -> Result<SecretCmd, EngineError> {
    let mut argv = vec!["auth".to_owned(), "passwd".into()];
    server_flag(&mut argv, server)?;
    SecretCmd::with(argv, vec![Some(current), Some(new)])
}

/// `key rotate-master [--new-keyfile=<path>]`. Passphrases go on stdin in prompt order: the
/// current one (only when this repository's key is passphrase-protected), then the new one
/// (only when the new master is a passphrase).
fn key_rotate_cmd(
    new_keyfile: Option<&str>,
    current: Option<String>,
    new_passphrase: Option<String>,
) -> Result<SecretCmd, EngineError> {
    let mut argv = vec!["key".to_owned(), "rotate-master".into()];
    if let Some(p) = new_keyfile.map(str::trim).filter(|p| !p.is_empty()) {
        plain_arg("key file", p)?;
        let path = Path::new(p);
        if !path.is_absolute() || !path.is_file() || p.chars().any(char::is_control) {
            return Err(invalid("key file", p));
        }
        if new_passphrase.is_some() {
            return Err(invalid(
                "new master",
                "a key file or a passphrase, not both",
            ));
        }
        argv.push(format!("--new-keyfile={p}"));
    }
    SecretCmd::with(argv, vec![current, new_passphrase])
}

/// The settings the app edits; the CLI refuses any other key too.
/// A dotted settings key such as `performance.upload_concurrency` or `remotes.origin.url`.
/// Which keys exist and what each accepts is the CLI's schema (`config schema`), checked again
/// by `config set`; here only the shape is checked, so a key can never read as a flag.
fn config_key_ok(key: &str) -> Result<(), EngineError> {
    let segs: Vec<&str> = key.split('.').collect();
    let seg_ok = |s: &&str| {
        !s.is_empty()
            && s.len() <= 64
            && !s.starts_with('-')
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
    };
    if (2..=5).contains(&segs.len()) && segs.iter().all(seg_ok) {
        Ok(())
    } else {
        Err(invalid("setting", key))
    }
}

/// Settings whose value is a secret (the last part of the key). Their value goes on stdin only.
const SECRET_LEAVES: [&str; 5] = [
    "token",
    "api_key",
    "access_key_id",
    "secret_access_key",
    "password",
];

fn is_secret_key(key: &str) -> bool {
    key.rsplit('.')
        .next()
        .is_some_and(|leaf| SECRET_LEAVES.contains(&leaf))
}

/// `config set -- <key> [<value>]`. A secret setting takes its value on stdin (`secret`), never
/// in argv; a known secret key refused as plain text, whatever the caller says.
fn config_set_cmd(key: &str, value: String, secret: bool) -> Result<SecretCmd, EngineError> {
    config_key_ok(key)?;
    if is_secret_key(key) && !secret {
        return Err(invalid(
            "setting",
            "this value is a secret and is sent privately",
        ));
    }
    let argv = vec!["config".to_owned(), "set".into(), "--".into(), key.into()];
    if secret {
        return SecretCmd::with(argv, vec![Some(value)]);
    }
    if value.is_empty() || value.len() > 1024 || value.chars().any(char::is_control) {
        return Err(invalid("value", &value));
    }
    let mut argv = argv;
    argv.push(value);
    Ok(SecretCmd::plain(argv))
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

/// `rebase|cherry-pick --continue|--abort|--skip`.
fn control_argv(cmd: &str, action: &str) -> Result<Vec<String>, EngineError> {
    if !matches!(cmd, "rebase" | "cherry-pick") || !matches!(action, "continue" | "abort" | "skip")
    {
        return Err(invalid("action", action));
    }
    Ok(vec![cmd.into(), format!("--{action}")])
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
    options: GcOptions,
    passphrase: Option<String>,
    on_progress: Channel<Progress>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    gc_guard(dry_run, confirmed)?;
    let cmd = with_passphrase(gc_argv(dry_run, &options), passphrase)?;
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
    options: FsckOptions,
    passphrase: Option<String>,
    on_progress: Channel<Progress>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let cmd = with_passphrase(fsck_argv(&options)?, passphrase)?;
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
    options: VerifyOptions,
    passphrase: Option<String>,
    on_progress: Channel<Progress>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let cmd = with_passphrase(verify_argv(&options)?, passphrase)?;
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

/// Create an account on a server that allows sign-up. The password goes on stdin only.
#[tauri::command]
pub async fn auth_register(
    engine: tauri::State<'_, Engine>,
    server: Option<String>,
    username: String,
    email: String,
    password: String,
) -> Result<Value, EngineError> {
    let cmd = auth_register_cmd(server.as_deref(), &username, &email, password)?;
    run(&engine, None, &cmd).await
}

/// Change your own password. Both passwords go on stdin only.
#[tauri::command]
pub async fn auth_passwd(
    engine: tauri::State<'_, Engine>,
    server: Option<String>,
    current: String,
    new: String,
) -> Result<Value, EngineError> {
    let cmd = auth_passwd_cmd(server.as_deref(), current, new)?;
    run(&engine, None, &cmd).await
}

/// Re-lock the repository's key under a new master key (a key file or a passphrase).
#[tauri::command]
pub async fn key_rotate_master(
    engine: tauri::State<'_, Engine>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    new_keyfile: Option<String>,
    current: Option<String>,
    new_passphrase: Option<String>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let cmd = key_rotate_cmd(new_keyfile.as_deref(), current, new_passphrase)?;
    queues.write(&repo, run(&engine, Some(&repo), &cmd)).await
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
    secret: bool,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let cmd = config_set_cmd(&key, value, secret)?;
    queues.write(&repo, run(&engine, Some(&repo), &cmd)).await
}

/// Every setting the CLI knows: key, type, default, documentation, allowed values, secret or not.
#[tauri::command]
pub async fn config_schema(engine: tauri::State<'_, Engine>) -> Result<Value, EngineError> {
    run(
        &engine,
        None,
        &SecretCmd::plain(vec!["config".into(), "schema".into()]),
    )
    .await
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
#[allow(clippy::too_many_arguments)]
pub async fn rebase_start(
    engine: tauri::State<'_, Engine>,
    ops: tauri::State<'_, Ops>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    upstream: String,
    branch: Option<String>,
    keep_empty: Option<bool>,
    on_progress: Channel<Progress>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let argv = rebase_start_argv(&upstream, branch.as_deref(), keep_empty.unwrap_or(false))?;
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
    opts: Option<PickOpts>,
    on_progress: Channel<Progress>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let argv = pick_start_argv(&commits, &opts.unwrap_or_default())?;
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
#[allow(clippy::too_many_arguments)]
pub async fn bisect_start(
    engine: tauri::State<'_, Engine>,
    ops: tauri::State<'_, Ops>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    bad: String,
    good: Option<String>,
    restart: Option<bool>,
    confirmed: Option<bool>,
    on_progress: Channel<Progress>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let argv = bisect_start_argv(
        &bad,
        good.as_deref(),
        restart.unwrap_or(false),
        confirmed.unwrap_or(false),
    )?;
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
    commit: Option<String>,
    confirmed: Option<bool>,
    on_progress: Channel<Progress>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let argv = bisect_reset_argv(commit.as_deref(), confirmed.unwrap_or(false))?;
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

/// Replay a saved bisect log (an absolute path to a file).
#[tauri::command]
pub async fn bisect_replay(
    engine: tauri::State<'_, Engine>,
    ops: tauri::State<'_, Ops>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    logfile: String,
    on_progress: Channel<Progress>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let argv = bisect_replay_argv(&logfile)?;
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
            gc_argv(true, &GcOptions::default()),
            fsck_argv(&FsckOptions::default()).unwrap(),
            verify_argv(&VerifyOptions::default()).unwrap(),
        ] {
            clean(&with_passphrase(argv, Some(PASS.into())).unwrap(), &[PASS]);
        }
        // No secret at all: no stdin lines, so the engine will not add --secret-stdin.
        assert!(
            with_passphrase(fsck_argv(&FsckOptions::default()).unwrap(), None)
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
        let gc = GcOptions::default();
        assert!(matches!(
            gc_guard(false, false),
            Err(EngineError::InvalidArgument(_))
        ));
        assert!(gc_guard(false, true).is_ok());
        assert!(gc_guard(true, false).is_ok());
        assert_eq!(gc_argv(true, &gc), ["gc", "--dry-run"]);
        assert_eq!(gc_argv(false, &gc), ["gc", "--yes"]);
        let fsck = |f: &dyn Fn(&mut FsckOptions)| {
            let mut o = FsckOptions::default();
            f(&mut o);
            fsck_argv(&o)
        };
        assert_eq!(fsck(&|_| {}).unwrap(), ["fsck", "--quick"]);
        assert_eq!(fsck(&|o| o.deep = true).unwrap(), ["fsck", "--full"]);
        let verify = |f: &dyn Fn(&mut VerifyOptions)| {
            let mut o = VerifyOptions::default();
            f(&mut o);
            verify_argv(&o)
        };
        assert_eq!(verify(&|o| o.quick = true).unwrap(), ["verify", "--quick"]);
        assert_eq!(verify(&|_| {}).unwrap(), ["verify"]);
    }

    #[test]
    fn gc_options_are_plain_flags_and_a_real_run_still_says_yes() {
        let o = GcOptions {
            no_prune: true,
            auto: true,
            repack: true,
        };
        assert_eq!(
            gc_argv(false, &o),
            ["gc", "--yes", "--no-prune", "--auto", "--repack"]
        );
        assert_eq!(
            gc_argv(
                true,
                &GcOptions {
                    repack: true,
                    ..GcOptions::default()
                }
            ),
            ["gc", "--dry-run", "--repack"]
        );
    }

    #[test]
    fn fsck_repair_needs_confirmation_and_a_preview_needs_repair() {
        let o = |repair, dry_run, confirmed| FsckOptions {
            repair,
            dry_run,
            confirmed,
            ..FsckOptions::default()
        };
        // Repair changes the repository: refused until the person has agreed.
        assert!(bad(fsck_argv(&o(true, false, false))));
        assert_eq!(
            fsck_argv(&o(true, false, true)).unwrap(),
            ["fsck", "--quick", "--repair"]
        );
        // A preview changes nothing, so it needs no confirmation, but it needs --repair.
        assert_eq!(
            fsck_argv(&o(true, true, false)).unwrap(),
            ["fsck", "--quick", "--repair", "--dry-run"]
        );
        assert!(bad(fsck_argv(&o(false, true, false))));
        // Confirmation alone repairs nothing.
        assert_eq!(
            fsck_argv(&o(false, false, true)).unwrap(),
            ["fsck", "--quick"]
        );
    }

    #[test]
    fn fsck_listing_options_are_flags() {
        let o = FsckOptions {
            deep: true,
            all: true,
            lost_found: true,
            no_dangling: true,
            ..FsckOptions::default()
        };
        assert_eq!(
            fsck_argv(&o).unwrap(),
            ["fsck", "--full", "--all", "--lost-found", "--no-dangling"]
        );
    }

    #[test]
    fn verify_one_version_or_a_range_never_both_and_never_a_flag() {
        let v = |commit: Option<&str>, start: Option<&str>, end: Option<&str>| VerifyOptions {
            commit: commit.map(Into::into),
            start: start.map(Into::into),
            end: end.map(Into::into),
            ..VerifyOptions::default()
        };
        assert_eq!(
            verify_argv(&v(Some("main"), None, None)).unwrap(),
            ["verify", "--", "main"]
        );
        assert_eq!(
            verify_argv(&v(None, Some("abc1234"), Some("HEAD"))).unwrap(),
            ["verify", "--start=abc1234", "--end=HEAD"]
        );
        assert_eq!(
            verify_argv(&VerifyOptions {
                checksums: true,
                detailed: true,
                ..VerifyOptions::default()
            })
            .unwrap(),
            ["verify", "--checksums", "--detailed"]
        );
        assert!(bad(verify_argv(&v(Some("main"), Some("abc1234"), None))));
        for x in ["--all", "a b", "x..y"] {
            assert!(bad(verify_argv(&v(Some(x), None, None))), "{x:?}");
            assert!(bad(verify_argv(&v(None, Some(x), None))), "{x:?}");
        }
        // Blank fields mean "not set".
        assert_eq!(
            verify_argv(&v(Some("  "), None, Some(""))).unwrap(),
            ["verify"]
        );
    }

    #[test]
    fn register_and_passwd_keep_passwords_off_argv() {
        let r =
            auth_register_cmd(Some("https://s.example"), "mira", "m@x.io", PASS.into()).unwrap();
        assert_eq!(
            r.argv,
            [
                "auth",
                "register",
                "--server=https://s.example",
                "--username=mira",
                "--email=m@x.io"
            ]
        );
        clean(&r, &[PASS]);
        assert_eq!(r.secrets.len(), 1);
        for (u, e) in [
            ("--x", "a@b.c"),
            ("mira", "no-at"),
            ("mira", "a b@c.d"),
            ("", "a@b.c"),
            ("mira", "a\nb@c.d"),
        ] {
            assert!(
                bad(auth_register_cmd(None, u, e, PASS.into())),
                "{u:?} {e:?}"
            );
        }
        assert!(bad(auth_register_cmd(
            Some("ftp://h"),
            "mira",
            "m@x.io",
            PASS.into()
        )));
        let p = auth_passwd_cmd(None, "old-pass".into(), "new-pass".into()).unwrap();
        assert_eq!(p.argv, ["auth", "passwd"]);
        assert_eq!(p.secrets.len(), 2);
        clean(&p, &["old-pass", "new-pass"]);
        assert!(bad(auth_passwd_cmd(None, "a\nb".into(), "x".into())));
    }

    #[test]
    fn rotate_master_takes_a_real_key_file_or_a_passphrase_never_both() {
        let tmp = tempfile::tempdir().unwrap();
        let key = tmp.path().join("new.key");
        std::fs::write(&key, b"k").unwrap();
        let k = key.to_str().unwrap();
        let by_file = key_rotate_cmd(Some(k), None, None).unwrap();
        assert_eq!(
            by_file.argv,
            ["key", "rotate-master", &format!("--new-keyfile={k}")]
        );
        assert!(by_file.secrets.is_empty());
        let by_pass = key_rotate_cmd(None, Some("old".into()), Some("new".into())).unwrap();
        assert_eq!(by_pass.argv, ["key", "rotate-master"]);
        assert_eq!(by_pass.secrets.len(), 2);
        clean(&by_pass, &["old", "new"]);
        assert!(bad(key_rotate_cmd(Some(k), None, Some("new".into()))));
        for p in ["relative.key", "--x", "/no/such/file.key"] {
            assert!(bad(key_rotate_cmd(Some(p), None, None)), "{p}");
        }
        // A blank key file is "not set"; an empty passphrase is refused rather than sent.
        assert!(
            key_rotate_cmd(Some("  "), None, None)
                .unwrap()
                .secrets
                .is_empty()
        );
        assert!(key_rotate_cmd(None, None, Some(String::new())).is_err());
    }

    #[test]
    fn config_and_sparse_argv_keep_values_positional() {
        assert_eq!(
            config_set_cmd("author.name", "-Mira Okafor".into(), false)
                .unwrap()
                .argv,
            ["config", "set", "--", "author.name", "-Mira Okafor"]
        );
        assert_eq!(
            config_unset_argv("author.email").unwrap(),
            ["config", "unset", "--", "author.email"]
        );
        for k in [
            "--all",
            "x",
            "",
            "author.name --global",
            "a..b",
            "-a.b",
            "a.b/c",
            "a.b.c.d.e.f",
        ] {
            assert!(bad(config_set_cmd(k, "v".into(), false)), "{k}");
            assert!(bad(config_unset_argv(k)), "{k}");
        }
        assert!(bad(config_set_cmd("author.name", String::new(), false)));
        assert!(bad(config_set_cmd("author.name", "a\nb".into(), false)));
    }

    #[test]
    fn every_schema_shaped_key_is_accepted_not_just_four() {
        for k in [
            "performance.pack_workers",
            "storage.create_dirs",
            "storage.credentials_path",
            "remotes.origin.url",
            "remotes.studio-2.fetch",
        ] {
            assert!(config_key_ok(k).is_ok(), "{k}");
            assert!(config_unset_argv(k).is_ok(), "{k}");
        }
        assert_eq!(
            config_set_cmd("storage.credentials_path", "C:/keys/sa.json".into(), false)
                .unwrap()
                .argv,
            [
                "config",
                "set",
                "--",
                "storage.credentials_path",
                "C:/keys/sa.json"
            ]
        );
    }

    #[test]
    fn secret_settings_travel_on_stdin_and_a_secret_key_is_never_plain() {
        let c = config_set_cmd("remotes.origin.token", PASS.into(), true).unwrap();
        assert_eq!(c.argv, ["config", "set", "--", "remotes.origin.token"]);
        clean(&c, &[PASS]);
        assert_eq!(c.secrets.len(), 1);
        for k in [
            "remotes.origin.token",
            "remotes.o.api_key",
            "storage.access_key_id",
            "storage.secret_access_key",
        ] {
            assert!(is_secret_key(k), "{k}");
            // Even if the caller claims it is not a secret, the value is not put in argv.
            assert!(bad(config_set_cmd(k, PASS.into(), false)), "{k}");
        }
        assert!(!is_secret_key("storage.region"));
        assert!(bad(config_set_cmd(
            "remotes.origin.token",
            String::new(),
            true
        )));
        assert!(bad(config_set_cmd(
            "remotes.origin.token",
            "a\nb".into(),
            true
        )));
    }

    #[test]
    fn sparse_argv_keeps_patterns_positional() {
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
        assert_eq!(
            rebase_start_argv("main", None, false).unwrap(),
            ["rebase", "--", "main"]
        );
        assert!(bad(rebase_start_argv("--abort", None, false)));
        assert!(bad(rebase_start_argv("a b", None, false)));
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
            pick_start_argv(&[oid.into(), "HEAD".into()], &PickOpts::default()).unwrap(),
            ["cherry-pick", "--", oid, "HEAD"]
        );
        assert!(bad(pick_start_argv(&[], &PickOpts::default())));
        assert!(bad(pick_start_argv(&["main".into()], &PickOpts::default())));
        assert!(bad(pick_start_argv(
            &["--no-commit".into()],
            &PickOpts::default()
        )));
        assert_eq!(
            bisect_start_argv("HEAD", Some(oid), false, false).unwrap(),
            ["bisect", "start", "--", "HEAD", oid]
        );
        assert_eq!(
            bisect_start_argv("main", None, false, false).unwrap(),
            ["bisect", "start", "--", "main"]
        );
        assert!(bad(bisect_start_argv("--reset", None, false, false)));
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

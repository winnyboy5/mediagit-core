// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Typed commands for the History, Inspector, Branches, Locks, Sync, Merge, Compare,
//! Stash and undo views.
//!
//! Each command builds its argv here (never forwarded from the webview),
//! validates every user-controlled piece, and runs the CLI through the
//! [`Engine`]. Mutating commands go through the per-repo write queue; reads
//! run concurrently. Only transfers register for cancellation.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::future::Future;
use std::hash::{Hash, Hasher};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;
use tauri::Manager;
use tauri::ipc::Channel;
use tokio::sync::Mutex as AsyncMutex;

use super::{Ops, Throttle, clone_source_ok, plain_arg, repo_path, repo_relative};
use crate::engine::{Engine, EngineError, Progress, Secret};

/// One mutating operation per repository at a time; reads are not queued.
#[derive(Default)]
pub struct RepoQueues(Mutex<HashMap<PathBuf, Arc<AsyncMutex<()>>>>);

impl RepoQueues {
    fn lane(&self, repo: &Path) -> Arc<AsyncMutex<()>> {
        let mut m = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(m.entry(repo.to_path_buf()).or_default())
    }

    /// Run `work` once every earlier write on `repo` has finished.
    pub async fn write<T>(&self, repo: &Path, work: impl Future<Output = T>) -> T {
        let _turn = self.lane(repo).lock_owned().await;
        work.await
    }
}

fn invalid(what: &str, value: &str) -> EngineError {
    EngineError::InvalidArgument(format!("bad {what}: {value:?}"))
}

/// Directory for extracted blobs; the asset protocol scope allows exactly this.
pub fn preview_dir() -> PathBuf {
    std::env::temp_dir().join("mediagit-desktop-preview")
}

// ---------------------------------------------------------------------------
// argv builders (pure, unit-tested)
// ---------------------------------------------------------------------------

/// A revision the UI may pass: `HEAD` or a hex object id.
fn rev_ok(rev: &str) -> Result<(), EngineError> {
    let hex = (4..=64).contains(&rev.len()) && rev.bytes().all(|b| b.is_ascii_hexdigit());
    if rev == "HEAD" || hex {
        Ok(())
    } else {
        Err(invalid("revision", rev))
    }
}

/// Branch and remote names: no flags, spaces, control characters or git-reserved punctuation.
fn ref_name_ok(what: &str, name: &str) -> Result<(), EngineError> {
    plain_arg(what, name)?;
    let bad = name.len() > 255
        || name.starts_with('/')
        || name.ends_with('/')
        || name.ends_with('.')
        || name.ends_with(".lock")
        || name.contains("..")
        || name.contains("@{")
        || name.contains("//")
        || name
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || "~^:?*[\\".contains(c));
    if bad {
        Err(invalid(what, name))
    } else {
        Ok(())
    }
}

fn log_argv(
    skip: u32,
    max: u32,
    all: bool,
    path: Option<&str>,
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
    if all {
        argv.push("--all".into());
    }
    if let Some(p) = path {
        // The revision must come first: the first positional after `--` would
        // otherwise be read as one.
        argv.extend(["HEAD".into(), "--".into(), repo_relative(repo, p)?]);
    }
    Ok(argv)
}

fn show_argv(rev: &str, tree: bool) -> Result<Vec<String>, EngineError> {
    rev_ok(rev)?;
    let mut argv = vec!["show".to_owned(), rev.to_owned()];
    if tree {
        argv.push("--tree".into());
    }
    Ok(argv)
}

fn extract_argv(
    repo: &Path,
    rev: &str,
    path: &str,
    out: &Path,
) -> Result<Vec<String>, EngineError> {
    rev_ok(rev)?;
    let rel = repo_relative(repo, path)?;
    Ok(vec![
        "show".into(),
        format!("{rev}:{rel}"),
        "--output".into(),
        out.to_string_lossy().into_owned(),
    ])
}

fn inside_preview(p: &Path) -> bool {
    p.starts_with(preview_dir()) && !p.components().any(|c| matches!(c, Component::ParentDir))
}

/// A file `media info` may read: repo-relative, or an already-extracted blob.
fn media_target(repo: &Path, path: &str) -> Result<String, EngineError> {
    if Path::new(path).is_absolute() {
        plain_arg("path", path)?;
        return if inside_preview(Path::new(path)) {
            Ok(path.to_owned())
        } else {
            Err(invalid("path", path))
        };
    }
    repo_relative(repo, path)
}

fn media_info_argv(repo: &Path, path: &str) -> Result<Vec<String>, EngineError> {
    Ok(vec![
        "media".into(),
        "info".into(),
        "--".into(),
        media_target(repo, path)?,
    ])
}

fn branch_argv(sub: &str, names: &[&str], force: bool) -> Result<Vec<String>, EngineError> {
    for n in names {
        ref_name_ok("branch name", n)?;
    }
    let mut argv = vec!["branch".to_owned(), sub.to_owned()];
    if force {
        argv.push("-D".into());
    }
    argv.push("--".into());
    argv.extend(names.iter().map(|n| (*n).to_owned()));
    Ok(argv)
}

fn lock_create_argv(repo: &Path, path: &str) -> Result<Vec<String>, EngineError> {
    Ok(vec![
        "lock".into(),
        "create".into(),
        "--".into(),
        repo_relative(repo, path)?,
    ])
}

fn lock_unlock_argv(lock_id: &str, force: bool) -> Result<Vec<String>, EngineError> {
    plain_arg("lock id", lock_id)?;
    let mut argv = vec![
        "lock".to_owned(),
        "unlock".into(),
        format!("--id={lock_id}"),
    ];
    if force {
        argv.push("--force".into());
    }
    Ok(argv)
}

fn remote_add_argv(name: &str, url: &str) -> Result<Vec<String>, EngineError> {
    ref_name_ok("remote name", name)?;
    if !clone_source_ok(url) {
        return Err(invalid("remote address", url));
    }
    Ok(vec![
        "remote".into(),
        "add".into(),
        "--".into(),
        name.into(),
        url.into(),
    ])
}

fn remote_remove_argv(name: &str) -> Result<Vec<String>, EngineError> {
    ref_name_ok("remote name", name)?;
    Ok(vec![
        "remote".into(),
        "remove".into(),
        "--".into(),
        name.into(),
    ])
}

fn transfer_argv(verb: &str, remote: Option<&str>) -> Result<Vec<String>, EngineError> {
    let mut argv = vec![verb.to_owned()];
    if let Some(r) = remote {
        ref_name_ok("remote name", r)?;
        argv.push("--".into());
        argv.push(r.into());
    }
    Ok(argv)
}

/// `merge --strategy=<s> [--no-ff] -- <branch>`. Strategy is the only merge-wide
/// conflict knob the CLI honours; per-file choices go through [`file_at_argv`].
fn merge_argv(branch: &str, strategy: &str, no_ff: bool) -> Result<Vec<String>, EngineError> {
    ref_name_ok("branch name", branch)?;
    if !matches!(strategy, "recursive" | "ours" | "theirs") {
        return Err(invalid("merge strategy", strategy));
    }
    let mut argv = vec!["merge".to_owned(), format!("--strategy={strategy}")];
    if no_ff {
        argv.push("--no-ff".into());
    }
    argv.extend(["--".into(), branch.into()]);
    Ok(argv)
}

/// `merge --continue` or `merge --abort`.
fn merge_control_argv(action: &str) -> Result<Vec<String>, EngineError> {
    match action {
        "continue" | "abort" => Ok(vec!["merge".into(), format!("--{action}")]),
        _ => Err(invalid("merge action", action)),
    }
}

/// Refuse `out` when writing it would land outside `repo`: the file itself is a link, or a
/// folder on the way is a symlink / junction pointing elsewhere. (`repo_relative` only checks
/// the spelling of the path; the CLI's `File::create` follows links.)
fn stays_in_repo(repo: &Path, out: &Path) -> Result<(), EngineError> {
    let escapes = || invalid("path", &out.to_string_lossy());
    if std::fs::symlink_metadata(out).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(escapes());
    }
    // The nearest folder that exists is where the file would really be created.
    let Some(dir) = out.ancestors().skip(1).find(|d| d.is_dir()) else {
        return Err(escapes());
    };
    match dunce::canonicalize(dir) {
        Ok(real) if real.starts_with(repo) => Ok(()),
        _ => Err(escapes()),
    }
}

/// `show <rev>:<path> --output <repo>/<path>`: put a file as it was at `rev` into the
/// working folder. Returns the repo-relative path too, for the follow-up `add`.
fn file_at_argv(repo: &Path, rev: &str, path: &str) -> Result<(String, Vec<String>), EngineError> {
    rev_ok(rev)?;
    let rel = repo_relative(repo, path)?;
    if rel == "." {
        return Err(invalid("path", path));
    }
    let out = repo.join(&rel);
    stays_in_repo(repo, &out)?;
    let argv = vec![
        "show".into(),
        format!("{rev}:{rel}"),
        "--output".into(),
        out.to_string_lossy().into_owned(),
    ];
    Ok((rel, argv))
}

/// Resolve one conflicted file: take our (`HEAD`) or their version of it, then stage it.
fn merge_resolve_argvs(
    repo: &Path,
    path: &str,
    side: &str,
    theirs: &str,
) -> Result<[Vec<String>; 2], EngineError> {
    let rev = match side {
        "ours" => "HEAD",
        "theirs" => theirs,
        _ => return Err(invalid("side", side)),
    };
    let (rel, show) = file_at_argv(repo, rev, path)?;
    Ok([show, vec!["add".into(), "--".into(), rel]])
}

/// `diff` (working folder against HEAD) or `diff <from> <to>`; never half of each.
fn diff_argv(from: Option<&str>, to: Option<&str>) -> Result<Vec<String>, EngineError> {
    match (from, to) {
        (None, None) => Ok(vec!["diff".into()]),
        (Some(a), Some(b)) => {
            rev_ok(a)?;
            rev_ok(b)?;
            Ok(vec!["diff".into(), a.into(), b.into()])
        }
        _ => Err(invalid("comparison", "needs two versions or none")),
    }
}

fn stash_save_argv(
    message: Option<&str>,
    include_untracked: bool,
) -> Result<Vec<String>, EngineError> {
    let mut argv = vec!["stash".to_owned(), "save".into()];
    if let Some(m) = message.map(str::trim).filter(|m| !m.is_empty()) {
        if m.contains('\0') {
            return Err(invalid("stash message", m));
        }
        argv.push(format!("--message={m}"));
    }
    if include_untracked {
        argv.push("--include-untracked".into());
    }
    Ok(argv)
}

/// `stash <show|apply|pop|drop> <index>`.
fn stash_entry_argv(sub: &str, index: u32) -> Result<Vec<String>, EngineError> {
    if !matches!(sub, "show" | "apply" | "pop" | "drop") {
        return Err(invalid("stash action", sub));
    }
    Ok(vec!["stash".into(), sub.into(), index.to_string()])
}

/// `stash clear --force`: the UI has already asked, and under `--machine` the CLI's own
/// confirmation would fail `needs_input`.
fn stash_clear_argv() -> Vec<String> {
    vec!["stash".into(), "clear".into(), "--force".into()]
}

/// `reset --soft|--mixed|--hard -- <rev>`. The mode is always spelled out.
fn reset_argv(mode: &str, rev: &str) -> Result<Vec<String>, EngineError> {
    rev_ok(rev)?;
    if !matches!(mode, "soft" | "mixed" | "hard") {
        return Err(invalid("reset mode", mode));
    }
    Ok(vec![
        "reset".into(),
        format!("--{mode}"),
        "--".into(),
        rev.into(),
    ])
}

/// `tag create [-a --message=<m>] -- <name> [<commit>]`; a message makes the tag annotated.
fn tag_create_argv(
    name: &str,
    commit: Option<&str>,
    message: Option<&str>,
) -> Result<Vec<String>, EngineError> {
    ref_name_ok("tag name", name)?;
    let mut argv = vec!["tag".to_owned(), "create".into()];
    if let Some(m) = message.map(str::trim).filter(|m| !m.is_empty()) {
        if m.contains('\0') {
            return Err(invalid("tag message", m));
        }
        argv.extend(["--annotated".into(), format!("--message={m}")]);
    }
    argv.extend(["--".into(), name.into()]);
    if let Some(c) = commit {
        rev_ok(c)?;
        argv.push(c.into());
    }
    Ok(argv)
}

fn tag_delete_argv(name: &str) -> Result<Vec<String>, EngineError> {
    ref_name_ok("tag name", name)?;
    Ok(vec![
        "tag".into(),
        "delete".into(),
        "--".into(),
        name.into(),
    ])
}

fn revert_argv(rev: &str) -> Result<Vec<String>, EngineError> {
    rev_ok(rev)?;
    Ok(vec!["revert".into(), "--".into(), rev.into()])
}

fn revert_control_argv(action: &str) -> Result<Vec<String>, EngineError> {
    match action {
        "continue" | "abort" | "skip" => Ok(vec!["revert".into(), format!("--{action}")]),
        _ => Err(invalid("revert action", action)),
    }
}

/// A full http(s) URL, or a path relative to the repository root (no `..`).
fn remote_path_ok(p: &str) -> bool {
    if plain_arg("remote path", p).is_err() || p.chars().any(char::is_control) {
        return false;
    }
    if p.starts_with("http://") || p.starts_with("https://") {
        return clone_source_ok(p);
    }
    !Path::new(p).is_absolute()
        && !p.contains('\\')
        && p.split('/').all(|s| !s.is_empty() && s != "..")
}

/// `download [--ref=<r>] --output=<file> -- <remote path>`.
fn download_argv(
    remote_path: &str,
    git_ref: Option<&str>,
    output: &str,
) -> Result<Vec<String>, EngineError> {
    if !remote_path_ok(remote_path) {
        return Err(invalid("remote path", remote_path));
    }
    crate::abs_dest(output)?;
    let mut argv = vec!["download".to_owned()];
    if let Some(r) = git_ref {
        ref_name_ok("ref", r)?;
        argv.push(format!("--ref={r}"));
    }
    argv.push(format!("--output={output}"));
    argv.extend(["--".into(), remote_path.into()]);
    Ok(argv)
}

/// `remote ls-tree [--ref=<r>] -- <remote> [<dir>]`: one folder level of a remote repository.
fn ls_tree_argv(
    remote: &str,
    dir: Option<&str>,
    git_ref: Option<&str>,
) -> Result<Vec<String>, EngineError> {
    ref_name_ok("remote name", remote)?;
    let mut argv = vec!["remote".to_owned(), "ls-tree".into()];
    if let Some(r) = git_ref {
        ref_name_ok("ref", r)?;
        argv.push(format!("--ref={r}"));
    }
    argv.extend(["--".into(), remote.into()]);
    if let Some(d) = dir.filter(|d| !d.is_empty()) {
        if !remote_path_ok(d) || d.starts_with("http") {
            return Err(invalid("folder", d));
        }
        argv.push(d.into());
    }
    Ok(argv)
}

/// Stable cache file name for an extracted blob: hash of rev + path, extension kept
/// so the webview and `media info` can tell the type.
fn preview_name(rev: &str, path: &str) -> String {
    let mut h = DefaultHasher::new();
    rev.hash(&mut h);
    path.hash(&mut h);
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| e.len() <= 12 && e.bytes().all(|b| b.is_ascii_alphanumeric()))
        .map(|e| format!(".{}", e.to_ascii_lowercase()))
        .unwrap_or_default();
    format!("{:016x}{ext}", h.finish())
}

// ---------------------------------------------------------------------------
// runners
// ---------------------------------------------------------------------------

fn as_args(argv: &[String]) -> Vec<&str> {
    argv.iter().map(String::as_str).collect()
}

async fn read(engine: &Engine, repo: &Path, argv: &[String]) -> Result<Value, EngineError> {
    engine.run(Some(repo), &as_args(argv), |_| {}).await
}

async fn write(
    engine: &Engine,
    queues: &RepoQueues,
    repo: &Path,
    argv: &[String],
) -> Result<Value, EngineError> {
    queues.write(repo, read(engine, repo, argv)).await
}

/// A queued write that streams progress. `cancellable` ops register under
/// `op_id`; `cancel_op` then drops the run, which kills the child.
#[allow(clippy::too_many_arguments)]
async fn stream(
    engine: &Engine,
    ops: &Ops,
    queues: &RepoQueues,
    repo: &Path,
    argv: &[String],
    op_id: &str,
    cancellable: bool,
    channel: &Channel<Progress>,
) -> Result<Value, EngineError> {
    stream_with(
        engine,
        ops,
        queues,
        repo,
        argv,
        &[],
        op_id,
        cancellable,
        channel,
    )
    .await
}

/// [`stream`] for a command that may be asked for secrets (passed on its stdin).
#[allow(clippy::too_many_arguments)]
async fn stream_with(
    engine: &Engine,
    ops: &Ops,
    queues: &RepoQueues,
    repo: &Path,
    argv: &[String],
    secrets: &[Secret],
    op_id: &str,
    cancellable: bool,
    channel: &Channel<Progress>,
) -> Result<Value, EngineError> {
    let mut throttle = Throttle::new(Duration::from_millis(100));
    let args = as_args(argv);
    let run = queues.write(
        repo,
        engine.run_with(Some(repo), &args, secrets, |p| {
            if throttle.allow(p.id, p.done, Instant::now()) {
                // A closed channel means the window went away; nobody to tell.
                let _ = channel.send(p);
            }
        }),
    );
    if !cancellable {
        return run.await;
    }
    let cancel = ops.register(op_id);
    let result = tokio::select! {
        r = run => r,
        () = cancel.notified() => Err(EngineError::Cli { code: "cancelled".into(), message: "Cancelled".into() }),
    };
    ops.finish(op_id);
    result
}

// ---------------------------------------------------------------------------
// commands
// ---------------------------------------------------------------------------

/// Only a folder that holds a `.mediagit` directory is a repository worth exposing.
fn require_repo(dir: &Path) -> Result<(), EngineError> {
    if dir.join(".mediagit").is_dir() {
        Ok(())
    } else {
        Err(invalid("repository", &dir.to_string_lossy()))
    }
}

/// Allow the webview to load files under an opened repository (previews), but never
/// its `.mediagit` store. Tauri scopes cannot shrink (a forbid is permanent), so a
/// previously opened repo stays allowed for the session.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // tauri injects AppHandle by value
pub fn repo_allow(app: tauri::AppHandle, repo: String) -> Result<(), EngineError> {
    let repo = repo_path(&repo)?;
    require_repo(&repo)?;
    let scope = app.asset_protocol_scope();
    let spawn = |e: tauri::Error| EngineError::Spawn(e.to_string());
    scope
        .forbid_directory(repo.join(".mediagit"), true)
        .map_err(spawn)?;
    scope.allow_directory(&repo, true).map_err(spawn)
}

/// Commit list page (`log`), newest first, plus all refs.
#[tauri::command]
pub async fn repo_log(
    engine: tauri::State<'_, Engine>,
    repo: String,
    skip: u32,
    max: u32,
    all: bool,
    path: Option<String>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    read(
        &engine,
        &repo,
        &log_argv(skip, max, all, path.as_deref(), &repo)?,
    )
    .await
}

/// One commit with its changed files; `tree` adds the full file list at that commit.
#[tauri::command]
pub async fn repo_show(
    engine: tauri::State<'_, Engine>,
    repo: String,
    rev: String,
    tree: bool,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    read(&engine, &repo, &show_argv(&rev, tree)?).await
}

/// Where an extracted blob lives and how big it is.
#[derive(Serialize)]
pub struct Extracted {
    path: String,
    size: u64,
}

static PART_SEQ: AtomicU64 = AtomicU64::new(0);

/// Write `<rev>:<path>` to the preview dir (cached by rev + path). Progress is the
/// bytes on disk so far against `expected`, the size the tree reported.
#[tauri::command]
pub async fn repo_extract(
    engine: tauri::State<'_, Engine>,
    repo: String,
    rev: String,
    path: String,
    expected: Option<u64>,
    on_progress: Channel<Progress>,
) -> Result<Extracted, EngineError> {
    let repo = repo_path(&repo)?;
    let dir = preview_dir();
    let dest = dir.join(preview_name(&rev, &path));
    let cached = |p: &Path| {
        std::fs::metadata(p)
            .ok()
            .filter(|m| m.len() > 0 || expected == Some(0))
    };
    if let Some(m) = cached(&dest).filter(|m| expected.is_none_or(|e| e == m.len())) {
        return Ok(Extracted {
            path: dest.to_string_lossy().into_owned(),
            size: m.len(),
        });
    }
    std::fs::create_dir_all(&dir).map_err(|e| EngineError::Spawn(e.to_string()))?;
    // Written under a private name and renamed on success: a killed run must not
    // leave something the cache check would accept.
    let part = dir.join(format!(
        "{}.{}.part",
        std::process::id(),
        PART_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let argv = extract_argv(&repo, &rev, &path, &part)?;
    let name = path.rsplit('/').next().unwrap_or(&path).to_owned();

    let args = as_args(&argv);
    let run = engine.run(Some(&repo), &args, |_| {});
    tokio::pin!(run);
    let mut tick = tokio::time::interval(Duration::from_millis(150));
    let result = loop {
        tokio::select! {
            r = &mut run => break r,
            _ = tick.tick() => {
                if let Ok(m) = std::fs::metadata(&part) {
                    let _ = on_progress.send(Progress { id: 0, msg: format!("Extracting {name}"), pos: m.len(), len: expected, unit: "bytes".into(), done: false });
                }
            }
        }
    };
    let finished = result.and_then(|_| {
        std::fs::rename(&part, &dest).map_err(|e| EngineError::Spawn(e.to_string()))?;
        std::fs::metadata(&dest).map_err(|e| EngineError::Spawn(e.to_string()))
    });
    let m = match finished {
        Ok(m) => m,
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            return Err(e);
        }
    };
    let _ = on_progress.send(Progress {
        id: 0,
        msg: format!("Extracting {name}"),
        pos: m.len(),
        len: Some(m.len()),
        unit: "bytes".into(),
        done: true,
    });
    Ok(Extracted {
        path: dest.to_string_lossy().into_owned(),
        size: m.len(),
    })
}

/// Media metadata for a working file or an extracted blob.
#[tauri::command]
pub async fn media_info(
    engine: tauri::State<'_, Engine>,
    repo: String,
    path: String,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    read(&engine, &repo, &media_info_argv(&repo, &path)?).await
}

/// Local branches with ahead/behind.
#[tauri::command]
pub async fn branch_list(
    engine: tauri::State<'_, Engine>,
    repo: String,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    read(&engine, &repo, &["branch".to_owned(), "list".into()]).await
}

/// Create a branch at HEAD.
#[tauri::command]
pub async fn branch_create(
    engine: tauri::State<'_, Engine>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    name: String,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    write(
        &engine,
        &queues,
        &repo,
        &branch_argv("create", &[name.as_str()], false)?,
    )
    .await
}

/// Switch the working tree to a branch; streams files-written progress.
#[tauri::command]
pub async fn branch_switch(
    engine: tauri::State<'_, Engine>,
    ops: tauri::State<'_, Ops>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    name: String,
    on_progress: Channel<Progress>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let argv = branch_argv("switch", &[name.as_str()], false)?;
    stream(
        &engine,
        &ops,
        &queues,
        &repo,
        &argv,
        "",
        false,
        &on_progress,
    )
    .await
}

/// Delete a branch; `force` also removes unmerged work.
#[tauri::command]
pub async fn branch_delete(
    engine: tauri::State<'_, Engine>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    name: String,
    force: bool,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    write(
        &engine,
        &queues,
        &repo,
        &branch_argv("delete", &[name.as_str()], force)?,
    )
    .await
}

/// Rename a branch.
#[tauri::command]
pub async fn branch_rename(
    engine: tauri::State<'_, Engine>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    from: String,
    to: String,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    write(
        &engine,
        &queues,
        &repo,
        &branch_argv("rename", &[from.as_str(), to.as_str()], false)?,
    )
    .await
}

/// Tags with the commit each points at.
#[tauri::command]
pub async fn tag_list(
    engine: tauri::State<'_, Engine>,
    repo: String,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    read(&engine, &repo, &["tag".to_owned(), "list".into()]).await
}

/// Name a version. `commit` defaults to HEAD; a message makes the tag annotated.
#[tauri::command]
pub async fn tag_create(
    engine: tauri::State<'_, Engine>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    name: String,
    commit: Option<String>,
    message: Option<String>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let argv = tag_create_argv(&name, commit.as_deref(), message.as_deref())?;
    write(&engine, &queues, &repo, &argv).await
}

/// Remove a tag name; the version it named stays.
#[tauri::command]
pub async fn tag_delete(
    engine: tauri::State<'_, Engine>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    name: String,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    write(&engine, &queues, &repo, &tag_delete_argv(&name)?).await
}

/// All file locks on the default remote.
#[tauri::command]
pub async fn lock_list(
    engine: tauri::State<'_, Engine>,
    repo: String,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    read(&engine, &repo, &["lock".to_owned(), "list".into()]).await
}

/// Lock one file.
#[tauri::command]
pub async fn lock_create(
    engine: tauri::State<'_, Engine>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    path: String,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    write(&engine, &queues, &repo, &lock_create_argv(&repo, &path)?).await
}

/// Release a lock by id; `force` releases someone else's.
#[tauri::command]
pub async fn lock_unlock(
    engine: tauri::State<'_, Engine>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    lock_id: String,
    force: bool,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    write(&engine, &queues, &repo, &lock_unlock_argv(&lock_id, force)?).await
}

/// Configured remotes.
#[tauri::command]
pub async fn remote_list(
    engine: tauri::State<'_, Engine>,
    repo: String,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    read(&engine, &repo, &["remote".to_owned(), "list".into()]).await
}

/// Add a remote; returns the resulting remote set.
#[tauri::command]
pub async fn remote_add(
    engine: tauri::State<'_, Engine>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    name: String,
    url: String,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    write(&engine, &queues, &repo, &remote_add_argv(&name, &url)?).await
}

/// Remove a remote; returns the resulting remote set.
#[tauri::command]
pub async fn remote_remove(
    engine: tauri::State<'_, Engine>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    name: String,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    write(&engine, &queues, &repo, &remote_remove_argv(&name)?).await
}

/// Download new work from a remote without changing files (`fetch`); cancellable.
#[tauri::command]
pub async fn repo_fetch(
    engine: tauri::State<'_, Engine>,
    ops: tauri::State<'_, Ops>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    remote: Option<String>,
    op_id: String,
    on_progress: Channel<Progress>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let argv = transfer_argv("fetch", remote.as_deref())?;
    stream(
        &engine,
        &ops,
        &queues,
        &repo,
        &argv,
        &op_id,
        true,
        &on_progress,
    )
    .await
}

/// Get the latest work from a remote and apply it (`pull`); cancellable. A killed pull
/// is safe to retry since issues 93 and 95 (kill-safety gate G3).
#[tauri::command]
pub async fn repo_pull(
    engine: tauri::State<'_, Engine>,
    ops: tauri::State<'_, Ops>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    remote: Option<String>,
    op_id: String,
    on_progress: Channel<Progress>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let argv = transfer_argv("pull", remote.as_deref())?;
    stream(
        &engine,
        &ops,
        &queues,
        &repo,
        &argv,
        &op_id,
        true,
        &on_progress,
    )
    .await
}

/// Send local work to a remote (`push`); cancellable.
#[tauri::command]
pub async fn repo_push(
    engine: tauri::State<'_, Engine>,
    ops: tauri::State<'_, Ops>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    remote: Option<String>,
    op_id: String,
    on_progress: Channel<Progress>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let argv = transfer_argv("push", remote.as_deref())?;
    stream(
        &engine,
        &ops,
        &queues,
        &repo,
        &argv,
        &op_id,
        true,
        &on_progress,
    )
    .await
}

// ---------------------------------------------------------------------------
// W2: merge, compare, stash, undo, download
// ---------------------------------------------------------------------------

/// Start a merge; streams checkout progress. A conflict is a normal result
/// (`status: "conflict"`, with the paths), not an error.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn repo_merge(
    engine: tauri::State<'_, Engine>,
    ops: tauri::State<'_, Ops>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    branch: String,
    strategy: String,
    no_ff: bool,
    on_progress: Channel<Progress>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let argv = merge_argv(&branch, &strategy, no_ff)?;
    stream(
        &engine,
        &ops,
        &queues,
        &repo,
        &argv,
        "",
        false,
        &on_progress,
    )
    .await
}

/// Finish (`continue`) or give up on (`abort`) the merge in progress.
#[tauri::command]
pub async fn repo_merge_control(
    engine: tauri::State<'_, Engine>,
    ops: tauri::State<'_, Ops>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    action: String,
    on_progress: Channel<Progress>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let argv = merge_control_argv(&action)?;
    stream(
        &engine,
        &ops,
        &queues,
        &repo,
        &argv,
        "",
        false,
        &on_progress,
    )
    .await
}

/// Resolve one conflicted file with our version (`ours`) or `theirs` (a commit id), and stage it.
#[tauri::command]
pub async fn merge_resolve(
    engine: tauri::State<'_, Engine>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    path: String,
    side: String,
    theirs: String,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let [show, add] = merge_resolve_argvs(&repo, &path, &side, &theirs)?;
    queues
        .write(&repo, async {
            read(&engine, &repo, &show).await?;
            read(&engine, &repo, &add).await
        })
        .await
}

/// Changed files between two versions, or the working folder against HEAD when both are absent.
#[tauri::command]
pub async fn repo_diff(
    engine: tauri::State<'_, Engine>,
    repo: String,
    from: Option<String>,
    to: Option<String>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    read(&engine, &repo, &diff_argv(from.as_deref(), to.as_deref())?).await
}

/// All stash entries, newest first.
#[tauri::command]
pub async fn stash_list(
    engine: tauri::State<'_, Engine>,
    repo: String,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    read(&engine, &repo, &["stash".to_owned(), "list".into()]).await
}

/// One stash entry.
#[tauri::command]
pub async fn stash_show(
    engine: tauri::State<'_, Engine>,
    repo: String,
    index: u32,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    read(&engine, &repo, &stash_entry_argv("show", index)?).await
}

/// Park the unsaved changes; streams progress.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn stash_save(
    engine: tauri::State<'_, Engine>,
    ops: tauri::State<'_, Ops>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    message: Option<String>,
    include_untracked: bool,
    on_progress: Channel<Progress>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let argv = stash_save_argv(message.as_deref(), include_untracked)?;
    stream(
        &engine,
        &ops,
        &queues,
        &repo,
        &argv,
        "",
        false,
        &on_progress,
    )
    .await
}

/// Bring a stash back, keeping the entry; streams files-written progress.
#[tauri::command]
pub async fn stash_apply(
    engine: tauri::State<'_, Engine>,
    ops: tauri::State<'_, Ops>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    index: u32,
    on_progress: Channel<Progress>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let argv = stash_entry_argv("apply", index)?;
    stream(
        &engine,
        &ops,
        &queues,
        &repo,
        &argv,
        "",
        false,
        &on_progress,
    )
    .await
}

/// Bring a stash back and remove the entry.
#[tauri::command]
pub async fn stash_pop(
    engine: tauri::State<'_, Engine>,
    ops: tauri::State<'_, Ops>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    index: u32,
    on_progress: Channel<Progress>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let argv = stash_entry_argv("pop", index)?;
    stream(
        &engine,
        &ops,
        &queues,
        &repo,
        &argv,
        "",
        false,
        &on_progress,
    )
    .await
}

/// Throw one stash entry away.
#[tauri::command]
pub async fn stash_drop(
    engine: tauri::State<'_, Engine>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    index: u32,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    write(&engine, &queues, &repo, &stash_entry_argv("drop", index)?).await
}

/// Throw every stash entry away.
#[tauri::command]
pub async fn stash_clear(
    engine: tauri::State<'_, Engine>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    write(&engine, &queues, &repo, &stash_clear_argv()).await
}

/// Move HEAD to `rev`, keeping files (`soft`: staged too; `mixed`: unstaged). Never touches
/// working files; `reset --hard` is its own command.
#[tauri::command]
pub async fn repo_reset(
    engine: tauri::State<'_, Engine>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    rev: String,
    mode: String,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    if mode == "hard" {
        return Err(invalid("reset mode", &mode));
    }
    write(&engine, &queues, &repo, &reset_argv(&mode, &rev)?).await
}

/// Move HEAD to `rev` and overwrite the working folder with it. Unsaved work is lost.
#[tauri::command]
pub async fn repo_reset_hard(
    engine: tauri::State<'_, Engine>,
    ops: tauri::State<'_, Ops>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    rev: String,
    on_progress: Channel<Progress>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let argv = reset_argv("hard", &rev)?;
    stream(
        &engine,
        &ops,
        &queues,
        &repo,
        &argv,
        "",
        false,
        &on_progress,
    )
    .await
}

/// Undo a version by adding a new one that reverses it.
#[tauri::command]
pub async fn repo_revert(
    engine: tauri::State<'_, Engine>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    rev: String,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    write(&engine, &queues, &repo, &revert_argv(&rev)?).await
}

/// `continue`, `abort` or `skip` a revert that stopped on a conflict.
#[tauri::command]
pub async fn repo_revert_control(
    engine: tauri::State<'_, Engine>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    action: String,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    write(&engine, &queues, &repo, &revert_control_argv(&action)?).await
}

/// Overwrite one working file with its content at `rev`. The file's unsaved edits are lost.
#[tauri::command]
pub async fn repo_restore_file(
    engine: tauri::State<'_, Engine>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    rev: String,
    path: String,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let (_, argv) = file_at_argv(&repo, &rev, &path)?;
    write(&engine, &queues, &repo, &argv).await
}

/// Download one file from the remote without cloning; cancellable.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn repo_download(
    engine: tauri::State<'_, Engine>,
    ops: tauri::State<'_, Ops>,
    repo: String,
    remote_path: String,
    git_ref: Option<String>,
    output: String,
    op_id: String,
    on_progress: Channel<Progress>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let argv = download_argv(&remote_path, git_ref.as_deref(), &output)?;
    // A download does not touch the repository, so it must not wait behind (or block)
    // its writes: a private lane per call.
    stream(
        &engine,
        &ops,
        &RepoQueues::default(),
        &repo,
        &argv,
        &op_id,
        true,
        &on_progress,
    )
    .await
}

/// One folder level of a remote repository at a ref (the server default when absent).
#[tauri::command]
pub async fn remote_ls_tree(
    engine: tauri::State<'_, Engine>,
    repo: String,
    remote: String,
    dir: Option<String>,
    git_ref: Option<String>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    read(
        &engine,
        &repo,
        &ls_tree_argv(&remote, dir.as_deref(), git_ref.as_deref())?,
    )
    .await
}

/// Cache key of a remote preview: the repository and the ref it was asked at. Only a full
/// commit id names fixed content; a branch (or no ref, the server default) moves, so those
/// get a key no other call shares and are fetched again every time.
fn remote_preview_key(repo: &Path, git_ref: Option<&str>) -> String {
    match git_ref {
        Some(r) if matches!(r.len(), 40 | 64) && r.bytes().all(|b| b.is_ascii_hexdigit()) => {
            format!("remote:{}:{r}", repo.display())
        }
        _ => format!(
            "remote:{}:fresh:{}:{}",
            repo.display(),
            std::process::id(),
            PART_SEQ.fetch_add(1, Ordering::Relaxed)
        ),
    }
}

/// Download one remote file into the preview folder (cached by repository + commit id + path;
/// a moving ref is never served from the cache) so the webview can show it; cancellable. Written under a private name and renamed on success.
#[tauri::command]
pub async fn repo_download_preview(
    engine: tauri::State<'_, Engine>,
    ops: tauri::State<'_, Ops>,
    repo: String,
    remote_path: String,
    git_ref: Option<String>,
    op_id: String,
    on_progress: Channel<Progress>,
) -> Result<Extracted, EngineError> {
    let repo = repo_path(&repo)?;
    let dir = preview_dir();
    let key = remote_preview_key(&repo, git_ref.as_deref());
    let dest = dir.join(preview_name(&key, &remote_path));
    if let Some(m) = std::fs::metadata(&dest).ok().filter(|m| m.len() > 0) {
        return Ok(Extracted {
            path: dest.to_string_lossy().into_owned(),
            size: m.len(),
        });
    }
    std::fs::create_dir_all(&dir).map_err(|e| EngineError::Spawn(e.to_string()))?;
    let part = dir.join(format!(
        "{}.{}.part",
        std::process::id(),
        PART_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let argv = download_argv(&remote_path, git_ref.as_deref(), &part.to_string_lossy())?;
    let result = stream(
        &engine,
        &ops,
        &RepoQueues::default(),
        &repo,
        &argv,
        &op_id,
        true,
        &on_progress,
    )
    .await;
    let finished = result.and_then(|_| {
        std::fs::rename(&part, &dest).map_err(|e| EngineError::Spawn(e.to_string()))?;
        std::fs::metadata(&dest).map_err(|e| EngineError::Spawn(e.to_string()))
    });
    match finished {
        Ok(m) => Ok(Extracted {
            path: dest.to_string_lossy().into_owned(),
            size: m.len(),
        }),
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

pub mod admin;
pub mod server_admin;

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize};

    #[allow(clippy::needless_pass_by_value)] // call sites pass temporaries
    fn bad(r: Result<Vec<String>, EngineError>) -> bool {
        matches!(r, Err(EngineError::InvalidArgument(_)))
    }

    #[test]
    fn log_argv_pages_and_puts_the_revision_before_the_path() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        assert_eq!(
            log_argv(0, 200, true, None, repo).unwrap(),
            ["log", "--skip", "0", "-n", "200", "--all"]
        );
        assert_eq!(
            log_argv(5, 10, false, Some("art/hero.psd"), repo).unwrap(),
            [
                "log",
                "--skip",
                "5",
                "-n",
                "10",
                "HEAD",
                "--",
                "art/hero.psd"
            ]
        );
        assert!(bad(log_argv(0, 0, false, None, repo)));
        assert!(bad(log_argv(0, 5000, false, None, repo)));
        for p in ["--all", "../x", "-n", "a/../../b"] {
            assert!(bad(log_argv(0, 10, false, Some(p), repo)), "{p}");
        }
    }

    #[test]
    fn show_and_extract_only_accept_hex_revisions() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        let oid = "0123456789abcdef0123456789abcdef";
        assert_eq!(show_argv(oid, true).unwrap(), ["show", oid, "--tree"]);
        assert_eq!(show_argv("HEAD", false).unwrap(), ["show", "HEAD"]);
        for r in ["--tree", "main", "abc", "HEAD~1", "zz11zz11", "", "a b c d"] {
            assert!(bad(show_argv(r, false)), "{r}");
        }
        let out = tmp.path().join("o.png");
        assert_eq!(
            extract_argv(repo, oid, "a/b.png", &out).unwrap(),
            vec![
                "show".to_owned(),
                format!("{oid}:a/b.png"),
                "--output".into(),
                out.to_string_lossy().into_owned()
            ]
        );
        assert!(bad(extract_argv(repo, "main", "a.png", &out)));
        assert!(bad(extract_argv(repo, oid, "../a.png", &out)));
        assert!(bad(extract_argv(repo, oid, "-x", &out)));
    }

    #[test]
    fn media_info_reads_repo_files_or_extracted_blobs_only() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        assert_eq!(
            media_info_argv(repo, "a/b.wav").unwrap(),
            ["media", "info", "--", "a/b.wav"]
        );
        let blob = preview_dir().join("0123.png");
        assert!(media_info_argv(repo, blob.to_str().unwrap()).is_ok());
        let sneaky = preview_dir().join("..").join("secret.png");
        assert!(bad(media_info_argv(repo, sneaky.to_str().unwrap())));
        let other = tempfile::tempdir().unwrap();
        assert!(bad(media_info_argv(
            repo,
            other.path().join("x.png").to_str().unwrap()
        )));
        assert!(bad(media_info_argv(repo, "--json")));
    }

    #[test]
    fn branch_argv_rejects_flags_and_reserved_names() {
        assert_eq!(
            branch_argv("create", &["feature/grade"], false).unwrap(),
            ["branch", "create", "--", "feature/grade"]
        );
        assert_eq!(
            branch_argv("delete", &["old"], true).unwrap(),
            ["branch", "delete", "-D", "--", "old"]
        );
        assert_eq!(
            branch_argv("rename", &["a", "b"], false).unwrap(),
            ["branch", "rename", "--", "a", "b"]
        );
        for n in [
            "--force", "-x", "a b", "a..b", "a~1", "a^", "a:b", "x/", "/x", "x.lock", "x.",
            "a@{1}", "a\\b", "", "a//b", "a\u{7}b",
        ] {
            assert!(bad(branch_argv("create", &[n], false)), "{n:?}");
        }
        assert!(
            bad(branch_argv("rename", &["ok", "--bad"], false)),
            "one bad name poisons the call"
        );
    }

    #[test]
    fn lock_remote_and_transfer_argv_validate() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        assert_eq!(
            lock_create_argv(repo, "art/hero.psd").unwrap(),
            ["lock", "create", "--", "art/hero.psd"]
        );
        assert!(bad(lock_create_argv(repo, "../x")));
        assert_eq!(
            lock_unlock_argv("abc-1", true).unwrap(),
            ["lock", "unlock", "--id=abc-1", "--force"]
        );
        assert!(bad(lock_unlock_argv("--force", false)));
        assert_eq!(
            remote_add_argv("origin", "https://h.example/r").unwrap(),
            ["remote", "add", "--", "origin", "https://h.example/r"]
        );
        assert!(bad(remote_add_argv("origin", "--upload-pack=x")));
        assert!(bad(remote_add_argv("-o", "https://h.example/r")));
        assert!(bad(remote_add_argv("o k", "https://h.example/r")));
        assert_eq!(
            remote_remove_argv("origin").unwrap(),
            ["remote", "remove", "--", "origin"]
        );
        assert_eq!(transfer_argv("push", None).unwrap(), ["push"]);
        assert_eq!(
            transfer_argv("pull", Some("origin")).unwrap(),
            ["pull", "--", "origin"]
        );
        assert!(bad(transfer_argv("push", Some("--force"))));
        assert!(bad(transfer_argv("fetch", Some("a b"))));
    }

    #[test]
    fn merge_argv_spells_out_strategy_and_guards_the_branch() {
        assert_eq!(
            merge_argv("feature/fx", "recursive", false).unwrap(),
            ["merge", "--strategy=recursive", "--", "feature/fx"]
        );
        assert_eq!(
            merge_argv("fx", "theirs", true).unwrap(),
            ["merge", "--strategy=theirs", "--no-ff", "--", "fx"]
        );
        assert!(bad(merge_argv("--abort", "ours", false)));
        assert!(bad(merge_argv("a b", "ours", false)));
        assert!(bad(merge_argv("fx", "octopus", false)));
        assert!(bad(merge_argv("fx", "--strategy=ours", false)));
        assert_eq!(
            merge_control_argv("continue").unwrap(),
            ["merge", "--continue"]
        );
        assert_eq!(merge_control_argv("abort").unwrap(), ["merge", "--abort"]);
        assert!(bad(merge_control_argv("--abort")));
        assert!(bad(merge_control_argv("")));
    }

    #[test]
    fn merge_resolve_extracts_one_side_then_stages_the_path() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        let theirs = "0123456789abcdef0123456789abcdef";
        let out = repo.join("art/hero.psd").to_string_lossy().into_owned();
        let [show, add] = merge_resolve_argvs(repo, "art/hero.psd", "ours", theirs).unwrap();
        assert_eq!(
            show,
            ["show", "HEAD:art/hero.psd", "--output", out.as_str()]
        );
        assert_eq!(add, ["add", "--", "art/hero.psd"]);
        let [show, _] = merge_resolve_argvs(repo, "art/hero.psd", "theirs", theirs).unwrap();
        assert_eq!(show[1], format!("{theirs}:art/hero.psd"));
        assert!(bad_pair(merge_resolve_argvs(repo, "../x", "ours", theirs)));
        assert!(bad_pair(merge_resolve_argvs(repo, "-x", "ours", theirs)));
        assert!(bad_pair(merge_resolve_argvs(repo, ".", "ours", theirs)));
        assert!(bad_pair(merge_resolve_argvs(repo, "a.psd", "both", theirs)));
        assert!(bad_pair(merge_resolve_argvs(
            repo, "a.psd", "theirs", "main"
        )));
        assert!(bad_pair(merge_resolve_argvs(repo, "a.psd", "theirs", "")));
    }

    #[allow(clippy::needless_pass_by_value)] // call sites pass temporaries
    fn bad_pair(r: Result<[Vec<String>; 2], EngineError>) -> bool {
        matches!(r, Err(EngineError::InvalidArgument(_)))
    }

    #[test]
    fn diff_needs_two_versions_or_none() {
        let a = "0123456789abcdef";
        let b = "fedcba9876543210";
        assert_eq!(diff_argv(None, None).unwrap(), ["diff"]);
        assert_eq!(diff_argv(Some(a), Some(b)).unwrap(), ["diff", a, b]);
        assert_eq!(
            diff_argv(Some("HEAD"), Some(a)).unwrap(),
            ["diff", "HEAD", a]
        );
        assert!(bad(diff_argv(Some(a), None)));
        assert!(bad(diff_argv(None, Some(a))));
        assert!(bad(diff_argv(Some("--cached"), Some(a))));
        assert!(bad(diff_argv(Some("main"), Some(a))));
    }

    #[test]
    fn stash_argv_keeps_messages_as_values_and_indexes_numeric() {
        assert_eq!(stash_save_argv(None, false).unwrap(), ["stash", "save"]);
        assert_eq!(
            stash_save_argv(Some("  WIP --lighting "), true).unwrap(),
            [
                "stash",
                "save",
                "--message=WIP --lighting",
                "--include-untracked"
            ]
        );
        assert_eq!(
            stash_save_argv(Some("   "), false).unwrap(),
            ["stash", "save"]
        );
        assert!(bad(stash_save_argv(Some("a\0b"), false)));
        assert_eq!(stash_entry_argv("pop", 2).unwrap(), ["stash", "pop", "2"]);
        for sub in ["show", "apply", "pop", "drop"] {
            assert!(stash_entry_argv(sub, 0).is_ok(), "{sub}");
        }
        assert_eq!(stash_clear_argv(), ["stash", "clear", "--force"]);
        assert!(bad(stash_entry_argv("clear", 0)));
        assert!(bad(stash_entry_argv("--index", 0)));
    }

    #[test]
    fn reset_always_names_its_mode_and_revert_guards_the_revision() {
        let oid = "0123456789abcdef";
        assert_eq!(
            reset_argv("soft", oid).unwrap(),
            ["reset", "--soft", "--", oid]
        );
        assert_eq!(
            reset_argv("mixed", "HEAD").unwrap(),
            ["reset", "--mixed", "--", "HEAD"]
        );
        assert_eq!(
            reset_argv("hard", oid).unwrap(),
            ["reset", "--hard", "--", oid]
        );
        assert!(bad(reset_argv("", oid)));
        assert!(bad(reset_argv("--hard", oid)));
        assert!(bad(reset_argv("hard", "main")));
        assert!(bad(reset_argv("hard", "--soft")));
        assert_eq!(revert_argv(oid).unwrap(), ["revert", "--", oid]);
        assert!(bad(revert_argv("--no-commit")));
        for a in ["continue", "abort", "skip"] {
            assert_eq!(
                revert_control_argv(a).unwrap(),
                ["revert", format!("--{a}").as_str()]
            );
        }
        assert!(bad(revert_control_argv("--abort")));
    }

    #[test]
    fn restoring_a_file_writes_it_inside_the_repo_only() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        let oid = "0123456789abcdef";
        let (rel, argv) = file_at_argv(repo, oid, "a/b.png").unwrap();
        assert_eq!(rel, "a/b.png");
        assert_eq!(argv[..2], ["show".to_owned(), format!("{oid}:a/b.png")]);
        assert_eq!(argv[2], "--output");
        assert_eq!(argv[3], repo.join("a/b.png").to_string_lossy());
        for p in ["../x", "-x", ".", "a/../../x"] {
            assert!(file_at_argv(repo, oid, p).is_err(), "{p}");
        }
        assert!(file_at_argv(repo, "main", "a.png").is_err());
    }

    #[test]
    fn download_argv_takes_urls_or_repo_relative_paths_only() {
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("logo.png");
        let out = out.to_str().unwrap();
        assert_eq!(
            download_argv("assets/logo.png", Some("v1.0"), out).unwrap(),
            [
                "download".to_owned(),
                "--ref=v1.0".into(),
                format!("--output={out}"),
                "--".into(),
                "assets/logo.png".into()
            ]
        );
        assert!(download_argv("http://s:3000/proj/assets/logo.png", None, out).is_ok());
        for p in [
            "--output=x",
            "../x",
            "a/../b",
            "/etc/passwd",
            "a//b",
            "a\\b",
            "ftp://h/x",
            "",
            "a\u{7}b",
        ] {
            assert!(bad(download_argv(p, None, out)), "{p:?}");
        }
        assert!(bad(download_argv("a.png", Some("--ref"), out)));
        assert!(bad(download_argv("a.png", Some("a b"), out)));
        assert!(bad(download_argv("a.png", None, "relative.png")));
        assert!(bad(download_argv("a.png", None, "-o")));
    }

    #[test]
    fn tag_argv_guards_names_commits_and_messages() {
        let oid = "0123456789abcdef";
        assert_eq!(
            tag_create_argv("v1.0", None, None).unwrap(),
            ["tag", "create", "--", "v1.0"]
        );
        assert_eq!(
            tag_create_argv("v1.0", Some(oid), Some(" Final ")).unwrap(),
            [
                "tag",
                "create",
                "--annotated",
                "--message=Final",
                "--",
                "v1.0",
                oid
            ]
        );
        for n in ["--force", "a b", "x..y", ""] {
            assert!(bad(tag_create_argv(n, None, None)), "{n:?}");
            assert!(bad(tag_delete_argv(n)), "{n:?}");
        }
        assert!(bad(tag_create_argv("v1", Some("main"), None)));
        assert!(bad(tag_create_argv("v1", None, Some("a\0b"))));
        assert_eq!(
            tag_delete_argv("v1.0").unwrap(),
            ["tag", "delete", "--", "v1.0"]
        );
    }

    #[test]
    fn ls_tree_argv_guards_remote_ref_and_folder() {
        assert_eq!(
            ls_tree_argv("origin", None, None).unwrap(),
            ["remote", "ls-tree", "--", "origin"]
        );
        assert_eq!(
            ls_tree_argv("origin", Some("video/shots"), Some("v0.3")).unwrap(),
            [
                "remote",
                "ls-tree",
                "--ref=v0.3",
                "--",
                "origin",
                "video/shots"
            ]
        );
        assert_eq!(
            ls_tree_argv("origin", Some(""), None).unwrap(),
            ["remote", "ls-tree", "--", "origin"]
        );
        assert!(bad(ls_tree_argv("--all", None, None)));
        assert!(bad(ls_tree_argv("origin", None, Some("--x"))));
        for d in ["../x", "/abs", "-x", "a//b", "a\\b", "http://h/x"] {
            assert!(bad(ls_tree_argv("origin", Some(d), None)), "{d}");
        }
    }

    #[test]
    fn only_folders_with_a_store_may_be_exposed() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(matches!(
            require_repo(tmp.path()),
            Err(EngineError::InvalidArgument(_))
        ));
        std::fs::create_dir(tmp.path().join(".mediagit")).unwrap();
        assert!(require_repo(tmp.path()).is_ok());
    }

    #[test]
    fn preview_names_are_stable_distinct_and_keep_a_safe_extension() {
        let a = preview_name("abcd", "x/hero.PNG");
        assert_eq!(a, preview_name("abcd", "x/hero.PNG"));
        assert_ne!(a, preview_name("abce", "x/hero.PNG"));
        assert_ne!(a, preview_name("abcd", "y/hero.PNG"));
        assert!(a.ends_with(".png"), "{a}");
        assert!(!preview_name("abcd", "x/no_ext").contains('.'));
        assert!(!preview_name("abcd", "x/a.we ird").contains(' '));
    }

    #[test]
    fn restoring_through_a_link_that_leaves_the_repo_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = repo_path(tmp.path().to_str().unwrap()).unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let outside = elsewhere.path();
        std::fs::create_dir(repo.join("real")).unwrap();
        let link = repo.join("link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside, &link).unwrap();
        #[cfg(windows)]
        assert!(
            std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&link)
                .arg(outside)
                .output()
                .unwrap()
                .status
                .success()
        );
        assert!(file_at_argv(&repo, "HEAD", "real/a.psd").is_ok());
        assert!(
            file_at_argv(&repo, "HEAD", "link/a.psd").is_err(),
            "folder is a link out"
        );
    }

    #[test]
    fn remote_preview_keys_pin_repo_and_commit_and_never_cache_a_moving_ref() {
        let sha = "a".repeat(64);
        let (a, b) = (Path::new("/repos/a"), Path::new("/repos/b"));
        // A commit id is fixed content: shared per repository, never across repositories.
        assert_eq!(
            remote_preview_key(a, Some(&sha)),
            remote_preview_key(a, Some(&sha))
        );
        assert_ne!(
            remote_preview_key(a, Some(&sha)),
            remote_preview_key(b, Some(&sha))
        );
        // A branch, or the server default, can move: each call is fetched afresh.
        for r in [None, Some("main"), Some("abc123")] {
            assert_ne!(remote_preview_key(a, r), remote_preview_key(a, r), "{r:?}");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_cancelled_write_finishes_dropping_its_work_before_the_lane_is_free() {
        // Stands in for a child process that takes a moment to die when its future is dropped.
        struct SlowDrop(Arc<AtomicBool>);
        impl Drop for SlowDrop {
            fn drop(&mut self) {
                std::thread::sleep(Duration::from_millis(150));
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let q = Arc::new(RepoQueues::default());
        let gone = Arc::new(AtomicBool::new(false));
        let first = tokio::spawn({
            let (q, gone) = (Arc::clone(&q), Arc::clone(&gone));
            async move {
                q.write(Path::new("/r/a"), async {
                    let _slow = SlowDrop(gone);
                    std::future::pending::<()>().await;
                })
                .await;
            }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        let second = tokio::spawn({
            let (q, gone) = (Arc::clone(&q), Arc::clone(&gone));
            async move {
                q.write(Path::new("/r/a"), async { gone.load(Ordering::SeqCst) })
                    .await
            }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        first.abort();
        assert!(
            second.await.unwrap(),
            "the next write started while the cancelled one was still dying"
        );
    }

    #[tokio::test]
    async fn writes_to_one_repo_never_overlap_but_other_repos_do() {
        let q = Arc::new(RepoQueues::default());
        let live = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let job = |q: Arc<RepoQueues>,
                   repo: &'static str,
                   live: Arc<AtomicUsize>,
                   peak: Arc<AtomicUsize>| async move {
            q.write(Path::new(repo), async {
                let now = live.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(40)).await;
                live.fetch_sub(1, Ordering::SeqCst);
            })
            .await;
        };
        let same: Vec<_> = (0..4)
            .map(|_| {
                tokio::spawn(job(
                    Arc::clone(&q),
                    "/r/a",
                    Arc::clone(&live),
                    Arc::clone(&peak),
                ))
            })
            .collect();
        for h in same {
            h.await.unwrap();
        }
        assert_eq!(peak.load(Ordering::SeqCst), 1, "same repo serializes");

        peak.store(0, Ordering::SeqCst);
        let a = tokio::spawn(job(
            Arc::clone(&q),
            "/r/a",
            Arc::clone(&live),
            Arc::clone(&peak),
        ));
        let b = tokio::spawn(job(
            Arc::clone(&q),
            "/r/b",
            Arc::clone(&live),
            Arc::clone(&peak),
        ));
        a.await.unwrap();
        b.await.unwrap();
        assert_eq!(
            peak.load(Ordering::SeqCst),
            2,
            "different repos run together"
        );
    }
}

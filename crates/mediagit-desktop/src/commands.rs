// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Typed commands for the History, Inspector, Branches, Locks and Sync views.
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
use crate::engine::{Engine, EngineError, Progress};

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
    let mut throttle = Throttle::new(Duration::from_millis(100));
    let args = as_args(argv);
    let run = queues.write(
        repo,
        engine.run(Some(repo), &args, |p| {
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

/// Allow the webview to load files under an opened repository (previews).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // tauri injects AppHandle by value
pub fn repo_allow(app: tauri::AppHandle, repo: String) -> Result<(), EngineError> {
    let repo = repo_path(&repo)?;
    app.asset_protocol_scope()
        .allow_directory(&repo, true)
        .map_err(|e| EngineError::Spawn(e.to_string()))
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

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
    fn preview_names_are_stable_distinct_and_keep_a_safe_extension() {
        let a = preview_name("abcd", "x/hero.PNG");
        assert_eq!(a, preview_name("abcd", "x/hero.PNG"));
        assert_ne!(a, preview_name("abce", "x/hero.PNG"));
        assert_ne!(a, preview_name("abcd", "y/hero.PNG"));
        assert!(a.ends_with(".png"), "{a}");
        assert!(!preview_name("abcd", "x/no_ext").contains('.'));
        assert!(!preview_name("abcd", "x/a.we ird").contains(' '));
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

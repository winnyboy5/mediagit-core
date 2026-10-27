// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! MediaGit desktop app: a Tauri shell whose commands run the `mediagit` CLI.

mod commands;
pub mod engine;

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use commands::RepoQueues;
use engine::{Engine, EngineError, Progress};
use serde_json::Value;
use tauri::Manager;
use tauri::ipc::Channel;
use tokio::sync::Notify;

/// How long exit waits for cancelled transfers to wind down.
const EXIT_GRACE: Duration = Duration::from_secs(3);

/// Reject anything but an existing absolute directory before it reaches argv.
/// Returns the canonical path so every spelling of a repo shares one write lane.
fn repo_path(repo: &str) -> Result<PathBuf, EngineError> {
    let p = PathBuf::from(repo);
    if p.is_absolute()
        && p.is_dir()
        && let Ok(canon) = dunce::canonicalize(&p)
    {
        Ok(canon)
    } else {
        Err(EngineError::InvalidArgument(format!(
            "not a directory: {repo}"
        )))
    }
}

/// Reject strings that could be read as a flag or smuggle a NUL.
fn plain_arg(what: &str, s: &str) -> Result<(), EngineError> {
    if s.is_empty() || s.starts_with('-') || s.contains('\0') {
        return Err(EngineError::InvalidArgument(format!("bad {what}: {s:?}")));
    }
    Ok(())
}

/// Make `path` relative to `repo`: refuses flags, `..`, and absolute paths
/// outside the repo. Returns a forward-slash path.
fn repo_relative(repo: &Path, path: &str) -> Result<String, EngineError> {
    plain_arg("path", path)?;
    let p = Path::new(path);
    let rel = if p.is_absolute() {
        // `repo` is canonical; the caller may have built `path` from another spelling.
        let canon = dunce::canonicalize(p).ok();
        canon
            .as_deref()
            .and_then(|c| c.strip_prefix(repo).ok())
            .or_else(|| p.strip_prefix(repo).ok())
            .map(Path::to_path_buf)
            .ok_or_else(|| EngineError::InvalidArgument(format!("outside repository: {path}")))?
    } else {
        p.to_path_buf()
    };
    let mut parts = Vec::new();
    for c in rel.components() {
        match c {
            Component::Normal(n) => parts.push(n.to_string_lossy().into_owned()),
            Component::CurDir => {}
            _ => return Err(EngineError::InvalidArgument(format!("bad path: {path}"))),
        }
    }
    Ok(if parts.is_empty() {
        ".".into()
    } else {
        parts.join("/")
    })
}

/// `add -- <paths>`; the `--` keeps every path positional.
fn add_argv(repo: &Path, paths: &[String]) -> Result<Vec<String>, EngineError> {
    if paths.is_empty() {
        return Err(EngineError::InvalidArgument("no paths to add".into()));
    }
    let mut argv = vec!["add".to_owned(), "--".to_owned()];
    for p in paths {
        argv.push(repo_relative(repo, p)?);
    }
    Ok(argv)
}

/// `commit --message=<msg>`: one element, so a message starting with `-`
/// is still a value, never a flag.
fn commit_argv(message: &str) -> Result<Vec<String>, EngineError> {
    if message.trim().is_empty() || message.contains('\0') {
        return Err(EngineError::InvalidArgument("empty commit message".into()));
    }
    Ok(vec!["commit".into(), format!("--message={message}")])
}

fn abs_dest(dest: &str) -> Result<(), EngineError> {
    plain_arg("destination", dest)?;
    if Path::new(dest).is_absolute() {
        Ok(())
    } else {
        Err(EngineError::InvalidArgument(format!(
            "not an absolute path: {dest}"
        )))
    }
}

/// `init -- <path>` for a new repository folder (may not exist yet).
fn init_argv(path: &str) -> Result<Vec<String>, EngineError> {
    abs_dest(path)?;
    Ok(vec!["init".into(), "--".into(), path.into()])
}

/// http(s) URL with a host, or an existing absolute local path.
fn clone_source_ok(url: &str) -> bool {
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) || url.starts_with('-') {
        return false;
    }
    if let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    {
        return rest
            .split('/')
            .next()
            .is_some_and(|h| !h.is_empty() && !h.starts_with('@'));
    }
    let p = Path::new(url);
    p.is_absolute() && p.exists()
}

/// `clone -- <url> <dest>`.
fn clone_argv(url: &str, dest: &str) -> Result<Vec<String>, EngineError> {
    if !clone_source_ok(url) {
        return Err(EngineError::InvalidArgument(format!(
            "not a clone source: {url}"
        )));
    }
    abs_dest(dest)?;
    Ok(vec!["clone".into(), "--".into(), url.into(), dest.into()])
}

/// Forwards at most one update per bar per interval; the final update
/// (`done`) always passes so a bar never gets stuck below 100%.
struct Throttle {
    every: Duration,
    last: HashMap<u64, Instant>,
}

impl Throttle {
    fn new(every: Duration) -> Self {
        Self {
            every,
            last: HashMap::new(),
        }
    }

    fn allow(&mut self, id: u64, done: bool, now: Instant) -> bool {
        let due = self
            .last
            .get(&id)
            .is_none_or(|t| now.duration_since(*t) >= self.every);
        if due || done {
            self.last.insert(id, now);
        }
        due || done
    }
}

#[derive(Default)]
struct OpsState {
    running: HashMap<String, Arc<Notify>>,
    /// Cancels that arrived before their op registered; honoured at `register`.
    early: HashSet<String>,
}

/// Running cancellable operations, by frontend-chosen op id.
#[derive(Default)]
struct Ops(Mutex<OpsState>);

impl Ops {
    fn register(&self, id: &str) -> Arc<Notify> {
        let n = Arc::new(Notify::new());
        if let Ok(mut s) = self.0.lock() {
            if s.early.remove(id) {
                n.notify_one(); // stores a permit: `notified()` fires at once
            }
            s.running.insert(id.to_owned(), Arc::clone(&n));
        }
        n
    }

    fn finish(&self, id: &str) {
        if let Ok(mut s) = self.0.lock() {
            s.running.remove(id);
            s.early.remove(id);
        }
    }

    /// True when `id` was running; otherwise the cancel is remembered for its `register`.
    fn cancel(&self, id: &str) -> bool {
        let Ok(mut s) = self.0.lock() else {
            return false;
        };
        if let Some(n) = s.running.get(id) {
            n.notify_one();
            true
        } else {
            s.early.insert(id.to_owned());
            false
        }
    }

    fn cancel_all(&self) {
        if let Ok(s) = self.0.lock() {
            s.running.values().for_each(|n| n.notify_one());
        }
    }

    fn is_idle(&self) -> bool {
        self.0.lock().is_ok_and(|s| s.running.is_empty())
    }

    async fn idle(&self) {
        while !self.is_idle() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

/// CLI version + contract, checked by the frontend at startup.
#[tauri::command]
async fn engine_info(engine: tauri::State<'_, Engine>) -> Result<Value, EngineError> {
    engine.run(None, &["version"], |_| {}).await
}

/// Working-copy status (`mediagit status`).
#[tauri::command]
async fn repo_status(engine: tauri::State<'_, Engine>, repo: String) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    engine.run(Some(&repo), &["status"], |_| {}).await
}

/// Stage files (`mediagit add`).
#[tauri::command]
async fn repo_add(
    engine: tauri::State<'_, Engine>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    paths: Vec<String>,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let argv = add_argv(&repo, &paths)?;
    let args: Vec<&str> = argv.iter().map(String::as_str).collect();
    queues
        .write(&repo, engine.run(Some(&repo), &args, |_| {}))
        .await
}

/// Save a version (`mediagit commit`).
#[tauri::command]
async fn repo_commit(
    engine: tauri::State<'_, Engine>,
    queues: tauri::State<'_, RepoQueues>,
    repo: String,
    message: String,
) -> Result<Value, EngineError> {
    let repo = repo_path(&repo)?;
    let argv = commit_argv(&message)?;
    let args: Vec<&str> = argv.iter().map(String::as_str).collect();
    queues
        .write(&repo, engine.run(Some(&repo), &args, |_| {}))
        .await
}

/// New repository (`mediagit init`).
#[tauri::command]
async fn repo_init(engine: tauri::State<'_, Engine>, path: String) -> Result<Value, EngineError> {
    let argv = init_argv(&path)?;
    let args: Vec<&str> = argv.iter().map(String::as_str).collect();
    engine.run(None, &args, |_| {}).await
}

/// Clone with streamed progress; `op_id` lets `cancel_op` stop it.
#[tauri::command]
async fn repo_clone(
    engine: tauri::State<'_, Engine>,
    ops: tauri::State<'_, Ops>,
    url: String,
    dest: String,
    op_id: String,
    on_progress: Channel<Progress>,
) -> Result<Value, EngineError> {
    let argv = clone_argv(&url, &dest)?;
    let args: Vec<&str> = argv.iter().map(String::as_str).collect();
    let cancel = ops.register(&op_id);
    let mut throttle = Throttle::new(Duration::from_millis(100));
    let run = engine.run(None, &args, |p| {
        if throttle.allow(p.id, p.done, Instant::now()) {
            // A closed channel means the window went away; nobody to tell.
            let _ = on_progress.send(p);
        }
    });
    // Dropping `run` kills the child (`kill_on_drop`); kill-safety is gate G3.
    let result = tokio::select! {
        r = run => r,
        () = cancel.notified() => Err(EngineError::Cli {
            code: "cancelled".into(),
            message: "Cancelled".into(),
        }),
    };
    ops.finish(&op_id);
    result
}

/// Stop a running cancellable operation.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // tauri injects State by value
fn cancel_op(ops: tauri::State<'_, Ops>, op_id: String) -> bool {
    ops.cancel(&op_id)
}

/// Build and run the app. A missing CLI is reported by the first command,
/// so the window still opens and can show the error.
pub fn run() {
    // Extracted previews are a per-session cache; start each session clean.
    let _ = std::fs::remove_dir_all(commands::preview_dir());
    let engine = Engine::locate().unwrap_or_else(|_| Engine::new(PathBuf::from("mediagit")));
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(engine)
        .manage(Ops::default())
        .manage(RepoQueues::default())
        .invoke_handler(tauri::generate_handler![
            engine_info,
            repo_status,
            repo_add,
            repo_commit,
            repo_init,
            repo_clone,
            cancel_op,
            commands::repo_allow,
            commands::repo_log,
            commands::repo_show,
            commands::repo_extract,
            commands::media_info,
            commands::branch_list,
            commands::branch_create,
            commands::branch_switch,
            commands::branch_delete,
            commands::branch_rename,
            commands::tag_list,
            commands::lock_list,
            commands::lock_create,
            commands::lock_unlock,
            commands::remote_list,
            commands::remote_add,
            commands::remote_remove,
            commands::repo_fetch,
            commands::repo_pull,
            commands::repo_push
        ])
        .build(tauri::generate_context!())
        .unwrap_or_else(|e| panic!("failed to start MediaGit desktop: {e}"));
    // Windows leaves children running when their parent exits, and `kill_on_drop`
    // only fires if the runs are dropped: cancel transfers, wait briefly, then exit.
    app.run(|app, event| {
        if let tauri::RunEvent::ExitRequested {
            api, code: None, ..
        } = event
        {
            let ops = app.state::<Ops>();
            if ops.is_idle() {
                return;
            }
            api.prevent_exit();
            ops.cancel_all();
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let _ = tokio::time::timeout(EXIT_GRACE, app.state::<Ops>().idle()).await;
                app.exit(0);
            });
        }
    });
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[allow(clippy::needless_pass_by_value)] // call sites pass temporaries
    fn bad(r: Result<Vec<String>, EngineError>) -> bool {
        matches!(r, Err(EngineError::InvalidArgument(_)))
    }

    #[test]
    fn repo_path_rejects_relative_missing_and_flags() {
        assert!(repo_path("relative/dir").is_err());
        assert!(repo_path("--machine").is_err());
        assert!(repo_path("-C").is_err());
        let tmp = tempfile::tempdir().unwrap();
        assert!(repo_path(tmp.path().join("missing").to_str().unwrap()).is_err());
        assert_eq!(
            repo_path(tmp.path().to_str().unwrap()).unwrap(),
            dunce::canonicalize(tmp.path()).unwrap()
        );
    }

    #[test]
    fn repo_path_gives_every_spelling_of_a_dir_one_key() {
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let plain = repo_path(tmp.path().to_str().unwrap()).unwrap();
        let dotted = repo_path(sub.join("..").to_str().unwrap()).unwrap();
        let trailing = repo_path(&format!(
            "{}{}",
            tmp.path().display(),
            std::path::MAIN_SEPARATOR
        ))
        .unwrap();
        assert_eq!(plain, dotted);
        assert_eq!(plain, trailing);
    }

    #[test]
    fn repo_relative_accepts_a_noncanonical_spelling_of_an_inside_file() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = repo_path(tmp.path().to_str().unwrap()).unwrap();
        std::fs::create_dir(repo.join("a")).unwrap();
        std::fs::write(repo.join("a").join("f.psd"), b"x").unwrap();
        let spelled = repo.join("a").join("..").join("a").join("f.psd");
        assert_eq!(
            repo_relative(&repo, spelled.to_str().unwrap()).unwrap(),
            "a/f.psd"
        );
    }

    #[test]
    fn add_argv_keeps_paths_relative_and_positional() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        let inside = repo.join("a").join("b.psd");
        let argv = add_argv(
            repo,
            &[
                "x/y.exr".into(),
                inside.to_str().unwrap().into(),
                "./z".into(),
            ],
        )
        .unwrap();
        assert_eq!(argv, ["add", "--", "x/y.exr", "a/b.psd", "z"]);
        assert_eq!(add_argv(repo, &[".".into()]).unwrap(), ["add", "--", "."]);
    }

    #[test]
    fn add_argv_rejects_injection() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        let outside = tempfile::tempdir().unwrap();
        for p in ["--machine", "-A", "../x", "a/../../x", "", "a\u{0}b"] {
            assert!(bad(add_argv(repo, &[p.into()])), "{p:?}");
        }
        let abs_outside = outside.path().join("f");
        assert!(bad(add_argv(repo, &[abs_outside.to_str().unwrap().into()])));
        assert!(bad(add_argv(repo, &[])));
        // one bad path poisons the whole call
        assert!(bad(add_argv(repo, &["ok".into(), "--force".into()])));
    }

    #[test]
    fn commit_argv_is_one_element_even_for_flaglike_messages() {
        assert_eq!(
            commit_argv("fix --machine").unwrap(),
            ["commit", "--message=fix --machine"]
        );
        assert_eq!(commit_argv("-m").unwrap(), ["commit", "--message=-m"]);
        assert!(bad(commit_argv("   ")));
        assert!(bad(commit_argv("")));
    }

    #[test]
    fn init_and_clone_argv_validate_sources_and_destinations() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_str().unwrap();
        assert_eq!(init_argv(dir).unwrap(), ["init", "--", dir]);
        assert!(bad(init_argv("--bare")));
        assert!(bad(init_argv("relative")));
        assert!(clone_argv("https://h.example/r", dir).is_ok());
        assert!(clone_argv(dir, dir).is_ok());
        for u in [
            "--upload-pack=x",
            "ftp://h/r",
            "https://",
            "http:///x",
            "https://h/ r",
            "relative/path",
            "/no/such/dir/xyz",
        ] {
            assert!(bad(clone_argv(u, dir)), "{u}");
        }
        assert!(bad(clone_argv("https://h/r", "-x")));
        assert!(bad(clone_argv("https://h/r", "rel")));
    }

    #[test]
    fn throttle_limits_each_bar_but_passes_the_final_update() {
        let mut t = Throttle::new(Duration::from_millis(100));
        let t0 = Instant::now();
        assert!(t.allow(1, false, t0));
        assert!(!t.allow(1, false, t0 + Duration::from_millis(50)));
        assert!(
            t.allow(2, false, t0 + Duration::from_millis(50)),
            "bars are independent"
        );
        assert!(t.allow(1, false, t0 + Duration::from_millis(100)));
        assert!(
            t.allow(1, true, t0 + Duration::from_millis(110)),
            "done always passes"
        );
    }

    #[test]
    fn ops_cancel_only_known_ids() {
        let ops = Ops::default();
        assert!(!ops.cancel("nope"));
        let _n = ops.register("a");
        assert!(ops.cancel("a"));
        ops.finish("a");
        assert!(!ops.cancel("a"));
    }

    #[tokio::test]
    async fn a_cancel_that_beats_register_is_not_lost() {
        let ops = Ops::default();
        assert!(!ops.cancel("early"));
        let n = ops.register("early");
        tokio::time::timeout(Duration::from_millis(500), n.notified())
            .await
            .unwrap(); // an early cancel must fire at register
        ops.finish("early");
        // consumed once: a later op reusing the id is not pre-cancelled
        let n = ops.register("early");
        assert!(
            tokio::time::timeout(Duration::from_millis(50), n.notified())
                .await
                .is_err()
        );
    }

    #[test]
    fn cancel_all_wakes_every_running_op() {
        let ops = Ops::default();
        assert!(ops.is_idle());
        let _a = ops.register("a");
        assert!(!ops.is_idle());
        ops.cancel_all();
        ops.finish("a");
        assert!(ops.is_idle());
    }
}

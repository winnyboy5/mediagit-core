// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Owns the optional local `mediagit-server`: first-run setup, start, stop, health, log.
//!
//! Only the process lifecycle lives here. Everything the server *does* (users, keys,
//! repositories) is administered through the bundled CLI, so the app never speaks HTTP
//! beyond one `GET /healthz`. The server binds `127.0.0.1` unless the person turns on the
//! network setting; authentication is always on for a server this app sets up, and the
//! admin password travels on stdin only.
//!
//! Layout under the app data dir: `server/{mediagit-server.toml, repos/, auth/, desktop.json,
//! server.log}`.

use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use crate::engine::{EngineError, Secret};

/// Override for the server binary path (tests, dev against another build).
pub const SERVER_PATH_ENV: &str = "MEDIAGIT_DESKTOP_SERVER";
/// Override for the server data folder (tests, the real-app e2e).
pub const SERVER_DIR_ENV: &str = "MEDIAGIT_DESKTOP_SERVER_DIR";

/// How long a started server has to answer `/healthz`.
const HEALTH_BUDGET: Duration = Duration::from_secs(3);
/// How long the server gets to exit after its stdin closes, before it is killed.
const STOP_GRACE: Duration = Duration::from_secs(10);
/// Log file size past which the next start begins a fresh one.
const LOG_MAX_BYTES: u64 = 1 << 20;

fn err(code: &str, message: impl Into<String>) -> EngineError {
    EngineError::Cli {
        code: code.into(),
        message: message.into(),
    }
}

/// What the Server view shows for "This computer".
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LocalStatus {
    /// `not_set_up`, `stopped`, `running` (started by this app) or `elsewhere`
    /// (something already answers on the port; this app did not start it).
    pub state: &'static str,
    /// Base URL clients use, when running.
    pub url: Option<String>,
    /// Port the server listens on, once set up.
    pub port: Option<u16>,
    /// Process id, only for a server this app started.
    pub pid: Option<u32>,
    /// Folder holding the server config, repos and auth store.
    pub data_dir: String,
    /// File capturing the server's stdout and stderr.
    pub log_path: String,
    /// Listening on the network instead of this computer only.
    pub lan: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Saved {
    port: u16,
    #[serde(default)]
    lan: bool,
}

struct Running {
    child: Child,
    pid: u32,
}

/// The one local server this app may own.
pub struct ServerManager {
    root: std::sync::RwLock<PathBuf>,
    /// Where the chosen folder is remembered; `None` = the folder cannot be changed.
    located: Option<Located>,
    /// Explicit server binary (tests); else [`ServerManager::locate_bin`].
    bin: Option<PathBuf>,
    running: Mutex<Option<Running>>,
    /// Held for a whole `start`, so overlapping starts cannot both spawn.
    starting: Mutex<()>,
}

/// The app data dir: home of the saved folder choice, and a folder the server must not swallow.
struct Located {
    app_data: PathBuf,
}

impl Located {
    fn file(&self) -> PathBuf {
        self.app_data.join("server-location.json")
    }

    fn saved_folder(&self) -> Option<PathBuf> {
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(self.file()).ok()?).ok()?;
        v["path"].as_str().map(PathBuf::from)
    }
}

impl ServerManager {
    /// `default_root` is `<app data>/server`. [`SERVER_DIR_ENV`] overrides it and pins it;
    /// otherwise a folder saved by [`ServerManager::set_folder`] wins.
    pub fn new(default_root: PathBuf) -> Self {
        let pinned = std::env::var_os(SERVER_DIR_ENV).map(PathBuf::from);
        let located = default_root
            .parent()
            .map(|p| Located {
                app_data: p.to_path_buf(),
            })
            .filter(|_| pinned.is_none());
        let root = pinned
            .or_else(|| located.as_ref().and_then(Located::saved_folder))
            .unwrap_or(default_root);
        Self {
            root: root.into(),
            located,
            bin: None,
            running: Mutex::new(None),
            starting: Mutex::new(()),
        }
    }

    /// Use `bin` as the server instead of looking for the bundled one.
    pub fn with_bin(root: PathBuf, bin: PathBuf) -> Self {
        Self {
            root: root.into(),
            located: None,
            bin: Some(bin),
            running: Mutex::new(None),
            starting: Mutex::new(()),
        }
    }

    /// Allow [`ServerManager::set_folder`], remembering the choice under `app_data`.
    #[must_use]
    pub fn located(mut self, app_data: PathBuf) -> Self {
        self.located = Some(Located { app_data });
        self
    }

    fn root(&self) -> PathBuf {
        self.root
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Move the server to another folder (`None` = back to `<app data>/server`).
    ///
    /// The folder must be absolute, writable, empty or already a server folder, outside any
    /// repository, and must not contain the app data dir. Refused while the server runs.
    pub async fn set_folder(&self, path: Option<&str>) -> Result<LocalStatus, EngineError> {
        let loc = self
            .located
            .as_ref()
            .ok_or_else(|| err("pinned", "the server folder is fixed for this session"))?;
        if self.is_running().await {
            return Err(err("running", "stop the server before changing its folder"));
        }
        let new_root = if let Some(p) = path {
            let dir = PathBuf::from(p);
            check_server_folder(&dir, &loc.app_data)?;
            std::fs::create_dir_all(&loc.app_data).map_err(|e| err("io", e.to_string()))?;
            std::fs::write(loc.file(), serde_json::json!({ "path": dir }).to_string())
                .map_err(|e| err("io", e.to_string()))?;
            dir
        } else {
            let _ = std::fs::remove_file(loc.file());
            loc.app_data.join("server")
        };
        *self
            .root
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = new_root;
        Ok(self.status().await)
    }

    fn server_bin(&self) -> Result<PathBuf, EngineError> {
        self.bin.clone().map_or_else(Self::locate_bin, Ok)
    }

    fn config_path(&self) -> PathBuf {
        self.root().join("mediagit-server.toml")
    }
    fn saved_path(&self) -> PathBuf {
        self.root().join("desktop.json")
    }
    fn log_path(&self) -> PathBuf {
        self.root().join("server.log")
    }

    fn saved(&self) -> Option<Saved> {
        let text = std::fs::read_to_string(self.saved_path()).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// Find the server: [`SERVER_PATH_ENV`], else next to the running exe (see `Engine::locate`).
    pub fn locate_bin() -> Result<PathBuf, EngineError> {
        if let Some(p) = std::env::var_os(SERVER_PATH_ENV) {
            let p = PathBuf::from(p);
            return if p.is_file() {
                Ok(p)
            } else {
                Err(err("server_not_found", p.display().to_string()))
            };
        }
        let exe = std::env::current_exe().map_err(|e| err("server_not_found", e.to_string()))?;
        let name = format!("mediagit-server{}", std::env::consts::EXE_SUFFIX);
        exe.ancestors()
            .skip(1)
            .take(2)
            .map(|dir| dir.join(&name))
            .find(|p| p.is_file())
            .ok_or_else(|| {
                err(
                    "server_not_found",
                    format!("{name} beside {}", exe.display()),
                )
            })
    }

    /// Current state. Reaps a server that exited on its own.
    pub async fn status(&self) -> LocalStatus {
        let mut guard = self.running.lock().await;
        if let Some(r) = guard.as_mut()
            && !matches!(r.child.try_wait(), Ok(None))
        {
            *guard = None;
        }
        let saved = self.saved();
        let port = saved.as_ref().map(|s| s.port);
        let pid = guard.as_ref().map(|r| r.pid);
        let state = if pid.is_some() {
            "running"
        } else if saved.is_none() || !self.config_path().is_file() {
            "not_set_up"
        } else if healthy(port.unwrap_or(0)).await {
            "elsewhere"
        } else {
            "stopped"
        };
        LocalStatus {
            state,
            url: port
                .filter(|_| state != "not_set_up")
                .map(|p| format!("http://127.0.0.1:{p}")),
            port,
            pid,
            data_dir: self.root().display().to_string(),
            log_path: self.log_path().display().to_string(),
            lan: saved.is_some_and(|s| s.lan),
        }
    }

    /// First run: write the config and the first admin. The password is one line on the
    /// server's stdin, never argv or env.
    pub async fn setup(
        &self,
        username: &str,
        email: &str,
        password: &Secret,
        storage: Option<&StorageOpts>,
    ) -> Result<LocalStatus, EngineError> {
        if self.config_path().exists() {
            return Err(err("already_set_up", "the local server is already set up"));
        }
        for (what, v) in [("username", username), ("email", email)] {
            if v.is_empty() || v.starts_with('-') || v.chars().any(char::is_control) {
                return Err(EngineError::InvalidArgument(format!("bad {what}")));
            }
        }
        let (storage_argv, storage_lines) =
            storage.map_or_else(|| Ok((vec![], vec![])), StorageOpts::to_args)?;
        let bin = self.server_bin()?;
        std::fs::create_dir_all(self.root()).map_err(|e| err("io", e.to_string()))?;
        let port = free_port().await?;
        let mut cmd = Command::new(bin);
        cmd.arg("init")
            .args([
                "--non-interactive",
                "--enable-auth",
                "--admin-password-stdin",
            ])
            .arg("--config")
            .arg(self.config_path())
            .args(["--host", "127.0.0.1", "--port"])
            .arg(port.to_string())
            .arg("--data-dir")
            .arg(self.root().join("repos"))
            .args(["--admin-username", username, "--admin-email", email])
            .args(storage_argv)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        hide_window(&mut cmd);
        let mut child = cmd.spawn().map_err(|e| EngineError::Spawn(e.to_string()))?;
        if let Some(mut stdin) = child.stdin.take() {
            // A closed pipe means init died early; its exit status says why.
            let _ = stdin.write_all(password.as_bytes()).await;
            let _ = stdin.write_all(b"\n").await;
            // Storage credentials follow the password line, as `name=value` lines.
            for line in &storage_lines {
                let _ = stdin.write_all(line.as_bytes()).await;
                let _ = stdin.write_all(b"\n").await;
            }
        }
        let out = child
            .wait_with_output()
            .await
            .map_err(|e| EngineError::Spawn(e.to_string()))?;
        if !out.status.success() {
            let _ = std::fs::remove_file(self.config_path());
            let text = String::from_utf8_lossy(&out.stderr);
            return Err(err("setup_failed", last_lines(&text, 5)));
        }
        let saved = Saved { port, lan: false };
        std::fs::write(
            self.saved_path(),
            serde_json::to_vec(&saved).map_err(|e| err("io", e.to_string()))?,
        )
        .map_err(|e| err("io", e.to_string()))?;
        Ok(self.status().await)
    }

    /// Start the server and wait for it to answer `/healthz`.
    pub async fn start(&self) -> Result<LocalStatus, EngineError> {
        let _starting = self.starting.lock().await;
        let status = self.status().await;
        match status.state {
            "not_set_up" => return Err(err("needs_setup", "set up the local server first")),
            "running" => return Ok(status),
            "elsewhere" => {
                return Err(err(
                    "already_running",
                    format!(
                        "a server is already answering on port {}; this app did not start it",
                        status.port.unwrap_or(0)
                    ),
                ));
            }
            _ => {}
        }
        let saved = self
            .saved()
            .ok_or_else(|| err("needs_setup", "set up the local server first"))?;
        let host = if saved.lan { "0.0.0.0" } else { "127.0.0.1" };
        // A port held by something that is not a MediaGit server.
        if TcpListener::bind((host, saved.port)).await.is_err() {
            return Err(err(
                "port_in_use",
                format!("port {} is used by another program", saved.port),
            ));
        }
        let bin = self.server_bin()?;
        let log = self.open_log()?;
        let log_err = log.try_clone().map_err(|e| err("io", e.to_string()))?;
        let mut cmd = Command::new(bin);
        cmd.arg("--config")
            .arg(self.config_path())
            // The server exits when this pipe closes: that is how stop (and an app crash) ends it.
            .arg("--exit-on-stdin-eof")
            .args(["--host", host, "--port"])
            .arg(saved.port.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(log_err))
            .kill_on_drop(true);
        hide_window(&mut cmd);
        let mut child = cmd.spawn().map_err(|e| EngineError::Spawn(e.to_string()))?;
        let pid = child.id().unwrap_or(0);

        let deadline = Instant::now() + HEALTH_BUDGET;
        loop {
            if let Ok(Some(code)) = child.try_wait() {
                return Err(err(
                    "start_failed",
                    format!("the server exited ({code}): {}", self.log_tail(5)),
                ));
            }
            if healthy(saved.port).await {
                break;
            }
            if Instant::now() >= deadline {
                let _ = child.kill().await;
                return Err(err(
                    "health_timeout",
                    format!(
                        "the server did not answer within {} seconds: {}",
                        HEALTH_BUDGET.as_secs(),
                        self.log_tail(5)
                    ),
                ));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        *self.running.lock().await = Some(Running { child, pid });
        Ok(self.status().await)
    }

    /// Stop the server this app started. Never touches one it did not start.
    pub async fn stop(&self) -> Result<LocalStatus, EngineError> {
        let taken = self.running.lock().await.take();
        let Some(mut r) = taken else {
            return Err(err(
                "not_owned",
                "this app did not start a local server, so it will not stop one",
            ));
        };
        terminate(&mut r).await;
        Ok(self.status().await)
    }

    /// For app exit: stop the owned server if any; a no-op otherwise.
    pub async fn stop_owned(&self) {
        if let Some(mut r) = self.running.lock().await.take() {
            terminate(&mut r).await;
        }
    }

    /// Whether this app has a server child to stop.
    pub async fn is_running(&self) -> bool {
        self.running.lock().await.is_some()
    }

    /// Listen on the network (`true`) or this computer only. Only while stopped.
    pub async fn set_lan(&self, lan: bool) -> Result<LocalStatus, EngineError> {
        if self.is_running().await {
            return Err(err("running", "stop the server before changing this"));
        }
        let mut saved = self
            .saved()
            .ok_or_else(|| err("needs_setup", "set up the local server first"))?;
        saved.lan = lan;
        std::fs::write(
            self.saved_path(),
            serde_json::to_vec(&saved).map_err(|e| err("io", e.to_string()))?,
        )
        .map_err(|e| err("io", e.to_string()))?;
        Ok(self.status().await)
    }

    /// The last `n` lines the server printed.
    pub fn log_tail(&self, n: usize) -> String {
        let text = std::fs::read_to_string(self.log_path()).unwrap_or_default();
        last_lines(&text, n)
    }

    fn open_log(&self) -> Result<std::fs::File, EngineError> {
        let path = self.log_path();
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > LOG_MAX_BYTES) {
            let _ = std::fs::rename(&path, path.with_extension("log.1"));
        }
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| err("io", e.to_string()))
    }
}

fn last_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

#[allow(unused_variables)] // only Windows needs the flag
fn hide_window(cmd: &mut Command) {
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
}

async fn free_port() -> Result<u16, EngineError> {
    let l = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|e| err("io", e.to_string()))?;
    l.local_addr()
        .map(|a| a.port())
        .map_err(|e| err("io", e.to_string()))
}

/// `GET /healthz` on loopback answers 200.
async fn healthy(port: u16) -> bool {
    if port == 0 {
        return false;
    }
    let probe = async {
        let mut s = TcpStream::connect(("127.0.0.1", port)).await.ok()?;
        s.write_all(b"GET /healthz HTTP/1.0\r\nHost: localhost\r\n\r\n")
            .await
            .ok()?;
        let mut buf = [0u8; 12];
        s.read_exact(&mut buf).await.ok()?;
        // "HTTP/1.x 200"
        Some(buf.starts_with(b"HTTP/1.") && &buf[9..12] == b"200")
    };
    tokio::time::timeout(Duration::from_millis(500), probe)
        .await
        .ok()
        .flatten()
        .unwrap_or(false)
}

/// Close the server's stdin (it exits on EOF), then kill it if it has not exited within [`STOP_GRACE`].
async fn terminate(r: &mut Running) {
    drop(r.child.stdin.take());
    if tokio::time::timeout(STOP_GRACE, r.child.wait())
        .await
        .is_err()
    {
        let _ = r.child.kill().await;
        let _ = r.child.wait().await;
    }
}

/// Why `dir` cannot be the server folder; creates it when it is fine.
fn check_server_folder(dir: &Path, app_data: &Path) -> Result<(), EngineError> {
    let bad = |m: &str| EngineError::InvalidArgument(m.into());
    if !dir.is_absolute() || dir.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(bad("the server folder must be an absolute path"));
    }
    if app_data.starts_with(dir) {
        return Err(bad("that folder contains the app's own data"));
    }
    if dir.ancestors().any(|a| a.join(".mediagit").is_dir()) {
        return Err(bad("that folder is inside a repository"));
    }
    if dir.exists() && !dir.is_dir() {
        return Err(bad("that path is not a folder"));
    }
    let is_server = dir.join("mediagit-server.toml").is_file();
    let empty = std::fs::read_dir(dir).is_ok_and(|mut d| d.next().is_none());
    if dir.exists() && !is_server && !empty {
        return Err(err(
            "not_empty",
            "that folder is not empty and is not a server folder",
        ));
    }
    std::fs::create_dir_all(dir).map_err(|e| err("not_writable", e.to_string()))?;
    let probe = dir.join(".write-test");
    std::fs::write(&probe, b"").map_err(|e| err("not_writable", e.to_string()))?;
    let _ = std::fs::remove_file(probe);
    Ok(())
}

/// Where the server keeps repository data. Secrets travel on stdin, never argv.
#[derive(Debug, Clone, Default, Deserialize)]
#[allow(missing_docs)] // field names are the documentation
pub struct StorageOpts {
    /// `filesystem` (default), `s3`, `gcs` or `azure`.
    pub kind: String,
    pub bucket: Option<String>,
    pub region: Option<String>,
    pub endpoint: Option<String>,
    pub prefix: Option<String>,
    pub container: Option<String>,
    pub account: Option<String>,
    pub project: Option<String>,
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
    pub account_key: Option<String>,
    pub connection_string: Option<String>,
    pub credentials_path: Option<String>,
}

impl StorageOpts {
    /// `(argv, stdin lines)` for `mediagit-server init|storage-test`. Empty for the filesystem default.
    fn to_args(&self) -> Result<(Vec<String>, Vec<String>), EngineError> {
        if !["filesystem", "s3", "gcs", "azure"].contains(&self.kind.as_str()) {
            return Err(EngineError::InvalidArgument("unknown storage kind".into()));
        }
        if self.kind == "filesystem" {
            return Ok((vec![], vec![]));
        }
        let clean = |what: &str, v: &Option<String>| -> Result<Option<String>, EngineError> {
            match v.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                Some(s) if s.chars().any(char::is_control) => {
                    Err(EngineError::InvalidArgument(format!("bad {what}")))
                }
                s => Ok(s.map(str::to_owned)),
            }
        };
        let mut argv = vec![format!("--storage={}", self.kind)];
        for (flag, v) in [
            ("bucket", &self.bucket),
            ("region", &self.region),
            ("endpoint", &self.endpoint),
            ("prefix", &self.prefix),
            ("container", &self.container),
            ("account", &self.account),
            ("project", &self.project),
        ] {
            if let Some(v) = clean(flag, v)? {
                argv.push(format!("--storage-{flag}={v}"));
            }
        }
        let mut lines = vec![];
        for (name, v) in [
            ("access-key-id", &self.access_key_id),
            ("secret-access-key", &self.secret_access_key),
            ("account-key", &self.account_key),
            ("connection-string", &self.connection_string),
            ("credentials-path", &self.credentials_path),
        ] {
            if let Some(v) = clean(name, v)? {
                lines.push(format!("{name}={v}"));
            }
        }
        if !lines.is_empty() {
            argv.push("--storage-secret-stdin".into());
        }
        Ok((argv, lines))
    }
}

impl ServerManager {
    /// Check the storage credentials: the server writes, reads and deletes a probe object.
    /// Returns `{ok, error?}`.
    pub async fn storage_test(&self, opts: &StorageOpts) -> Result<serde_json::Value, EngineError> {
        let (argv, lines) = opts.to_args()?;
        if argv.is_empty() {
            return Err(EngineError::InvalidArgument(
                "pick S3, Google Cloud or Azure storage to test".into(),
            ));
        }
        let mut cmd = Command::new(self.server_bin()?);
        cmd.arg("storage-test")
            .args(argv)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        hide_window(&mut cmd);
        let mut child = cmd.spawn().map_err(|e| EngineError::Spawn(e.to_string()))?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(lines.join("\n").as_bytes()).await;
        }
        let out = tokio::time::timeout(STORAGE_TEST_BUDGET, child.wait_with_output())
            .await
            .map_err(|_| err("storage_test_timeout", "the storage did not answer in time"))?
            .map_err(|e| EngineError::Spawn(e.to_string()))?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        serde_json::from_str(stdout.lines().last().unwrap_or_default()).map_err(|_| {
            err(
                "storage_test_failed",
                last_lines(&String::from_utf8_lossy(&out.stderr), 5),
            )
        })
    }
}

/// How long `storage_test` may take (cloud retries are slow to give up).
const STORAGE_TEST_BUDGET: Duration = Duration::from_secs(120);

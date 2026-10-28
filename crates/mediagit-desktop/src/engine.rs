// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Runs the bundled `mediagit` binary in machine mode.
//!
//! Every operation is one child process: `mediagit -C <repo> --machine <cmd>`.
//! stdout carries exactly one envelope (`{"v":1,"ok":..}`); stderr carries
//! NDJSON progress lines plus ordinary log text. Running the CLI out of
//! process keeps its process-global state (at-rest key, cwd, prompts) per
//! operation, and means the app exercises the same binary the test suites do.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

/// Machine contract version this app understands. Must match the CLI's
/// `MACHINE_CONTRACT_VERSION`; a mismatch means app and CLI were built apart.
pub const CONTRACT_VERSION: u64 = 1;

/// Override for the CLI path (tests, dev against another build).
pub const CLI_PATH_ENV: &str = "MEDIAGIT_DESKTOP_CLI";

/// How much trailing stderr to keep for error context.
const STDERR_TAIL_LINES: usize = 20;

/// Errors surfaced to the frontend as `{code, message}`.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum EngineError {
    /// The `mediagit` binary could not be found next to the app.
    #[error("mediagit binary not found: {0}")]
    CliNotFound(String),
    /// The child process could not be started or waited on.
    #[error("failed to run mediagit: {0}")]
    Spawn(String),
    /// stdout was not a single machine envelope.
    #[error("unexpected output from mediagit: {0}")]
    Protocol(String),
    /// App and CLI speak different contract versions.
    #[error("mediagit contract v{found}, app expects v{CONTRACT_VERSION}")]
    ContractMismatch {
        /// Version the CLI reported.
        found: u64,
    },
    /// The CLI reported a failure (`ok: false`).
    #[error("{code}: {message}")]
    Cli {
        /// CLI error code (`needs_input`, `json_unsupported`, `error`, ...).
        code: String,
        /// Human-readable error chain.
        message: String,
    },
    /// The caller passed an argument the app refuses to forward.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
}

impl EngineError {
    /// Stable machine code for the frontend; CLI failures keep the CLI's code.
    pub fn code(&self) -> &str {
        match self {
            Self::CliNotFound(_) => "cli_not_found",
            Self::Spawn(_) => "spawn_failed",
            Self::Protocol(_) => "protocol",
            Self::ContractMismatch { .. } => "contract_mismatch",
            Self::Cli { code, .. } => code,
            Self::InvalidArgument(_) => "invalid_argument",
        }
    }
}

/// Tauri returns command errors as JSON: `{code, message}`.
impl Serialize for EngineError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let message = match self {
            Self::Cli { message, .. } => message.clone(),
            other => other.to_string(),
        };
        let mut st = s.serialize_struct("EngineError", 2)?;
        st.serialize_field("code", self.code())?;
        st.serialize_field("message", &message)?;
        st.end()
    }
}

/// One sampled progress bar, as the CLI prints it on stderr.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Progress {
    /// Bar id, stable for the life of one CLI run.
    pub id: u64,
    /// Bar label (phase or current file).
    pub msg: String,
    /// Units completed so far.
    pub pos: u64,
    /// Total units; `None` while unknown (render indeterminate).
    pub len: Option<u64>,
    /// `bytes` or `items`.
    pub unit: String,
    /// Bar finished.
    pub done: bool,
}

#[derive(Deserialize)]
struct Envelope {
    v: u64,
    ok: bool,
    #[serde(default)]
    data: Value,
    error: Option<CliError>,
}

#[derive(Deserialize)]
struct CliError {
    code: String,
    message: String,
}

/// A passphrase, password or recovery code on its way to a child's stdin. It is never
/// serialized, printed or compared, and its text is overwritten when it is dropped.
pub struct Secret(String);

impl Secret {
    /// One line of at most 1 KiB: no line breaks or NUL (they would split or end the prompt).
    pub fn new(text: String) -> Result<Self, EngineError> {
        if text.is_empty() || text.len() > 1024 || text.contains(['\n', '\r', '\0']) {
            // The message names no part of the value.
            return Err(EngineError::InvalidArgument("bad secret".into()));
        }
        Ok(Self(text))
    }

    fn line(&self) -> String {
        format!("{}\n", self.0)
    }

    /// Whether `needle` is this secret (for tests that check it never leaks elsewhere).
    pub fn is(&self, needle: &str) -> bool {
        self.0 == needle
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(***)")
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        // Best effort without `unsafe`: overwrite in place, then release.
        let n = self.0.len();
        self.0 = "x".repeat(n);
        self.0.clear();
    }
}

/// Handle to the `mediagit` binary.
#[derive(Debug, Clone)]
pub struct Engine {
    bin: PathBuf,
}

impl Engine {
    /// Use an explicit binary path.
    pub fn new(bin: PathBuf) -> Self {
        Self { bin }
    }

    /// Path of the CLI binary this engine runs.
    pub fn bin(&self) -> &Path {
        &self.bin
    }

    /// Find the CLI:`MEDIAGIT_DESKTOP_CLI`, else next to the running exe.
    ///
    /// Tauri's `externalBin` installs the sidecar beside the app executable,
    /// and in a cargo build `target/<profile>/mediagit(.exe)` sits beside
    /// `mediagit-desktop(.exe)`, so one rule covers bundle and dev. The parent
    /// directory is also checked for test binaries in `target/<profile>/deps`.
    pub fn locate() -> Result<Self, EngineError> {
        if let Some(p) = std::env::var_os(CLI_PATH_ENV) {
            let p = PathBuf::from(p);
            return if p.is_file() {
                Ok(Self::new(p))
            } else {
                Err(EngineError::CliNotFound(p.display().to_string()))
            };
        }
        let exe = std::env::current_exe().map_err(|e| EngineError::CliNotFound(e.to_string()))?;
        let name = format!("mediagit{}", std::env::consts::EXE_SUFFIX);
        exe.ancestors()
            .skip(1)
            .take(2)
            .map(|dir| dir.join(&name))
            .find(|p| p.is_file())
            .map(Self::new)
            .ok_or_else(|| EngineError::CliNotFound(format!("{name} beside {}", exe.display())))
    }

    /// Run one command and return the envelope's `data`.
    ///
    /// `repo` becomes `-C <repo>`; `args` is the subcommand and its flags,
    /// built by typed Tauri commands — never forwarded from the webview.
    pub async fn run(
        &self,
        repo: Option<&Path>,
        args: &[&str],
        on_progress: impl FnMut(Progress),
    ) -> Result<Value, EngineError> {
        self.run_with(repo, args, &[], on_progress).await
    }

    /// [`Engine::run`] that also answers the command's secret prompts. Each secret is
    /// written to the child's stdin as one line (`--secret-stdin`), in prompt order, and
    /// stdin is then closed. Secrets never go in argv or the environment and are not logged.
    pub async fn run_with(
        &self,
        repo: Option<&Path>,
        args: &[&str],
        secrets: &[Secret],
        mut on_progress: impl FnMut(Progress),
    ) -> Result<Value, EngineError> {
        let mut cmd = Command::new(&self.bin);
        if let Some(repo) = repo {
            cmd.arg("-C").arg(repo);
        }
        cmd.arg("--machine");
        if !secrets.is_empty() {
            cmd.arg("--secret-stdin");
        }
        cmd.args(args)
            .stdin(if secrets.is_empty() {
                Stdio::null()
            } else {
                Stdio::piped()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console flash

        let mut child = cmd.spawn().map_err(|e| EngineError::Spawn(e.to_string()))?;
        if !secrets.is_empty() {
            let mut stdin = child
                .stdin
                .take()
                .ok_or_else(|| EngineError::Spawn("no stdin".into()))?;
            // A fixed message: whatever the OS says must not carry the secret's neighbours.
            let fail = |_| EngineError::Spawn("could not pass the secret to mediagit".into());
            for secret in secrets {
                stdin
                    .write_all(secret.line().as_bytes())
                    .await
                    .map_err(fail)?;
            }
            drop(stdin); // EOF: a prompt that wants more input fails instead of hanging
        }
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| EngineError::Spawn("no stdout".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| EngineError::Spawn("no stderr".into()))?;

        // Drain stdout concurrently so a large envelope cannot block the child
        // while we are still reading stderr.
        let out_task = tokio::spawn(async move {
            let mut buf = String::new();
            stdout.read_to_string(&mut buf).await.map(|_| buf)
        });

        let mut tail: Vec<String> = Vec::new();
        // Raw bytes, not `lines()`: log text need not be UTF-8 (paths in the
        // active code page), and that must not abort the run and kill the child.
        let mut reader = BufReader::new(stderr);
        let mut raw = Vec::new();
        loop {
            raw.clear();
            let n = reader
                .read_until(b'\n', &mut raw)
                .await
                .map_err(|e| EngineError::Spawn(e.to_string()))?;
            if n == 0 {
                break;
            }
            let line = lossy_line(&raw);
            match parse_progress(&line) {
                Some(p) => on_progress(p),
                None => {
                    tail.push(line);
                    if tail.len() > STDERR_TAIL_LINES {
                        tail.remove(0);
                    }
                }
            }
        }

        child
            .wait()
            .await
            .map_err(|e| EngineError::Spawn(e.to_string()))?;
        let out = out_task
            .await
            .map_err(|e| EngineError::Spawn(e.to_string()))?
            .map_err(|e| EngineError::Spawn(e.to_string()))?;
        parse_envelope(&out).map_err(|e| match e {
            EngineError::Protocol(m) if !tail.is_empty() => {
                EngineError::Protocol(format!("{m}; stderr: {}", tail.join("\n")))
            }
            other => other,
        })
    }
}

/// One stderr line as text: invalid UTF-8 is replaced, the line ending dropped.
fn lossy_line(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw)
        .trim_end_matches(['\r', '\n'])
        .to_owned()
}

/// Parse an NDJSON progress line; anything else is ordinary log text.
pub fn parse_progress(line: &str) -> Option<Progress> {
    #[derive(Deserialize)]
    struct Tagged {
        #[serde(rename = "type")]
        kind: String,
        #[serde(flatten)]
        progress: Progress,
    }
    let t: Tagged = serde_json::from_str(line.trim()).ok()?;
    (t.kind == "progress").then_some(t.progress)
}

/// Parse the single stdout envelope.
pub fn parse_envelope(stdout: &str) -> Result<Value, EngineError> {
    let line = stdout.trim();
    if line.is_empty() {
        return Err(EngineError::Protocol("empty stdout".into()));
    }
    if line.contains('\n') {
        return Err(EngineError::Protocol("more than one line on stdout".into()));
    }
    let env: Envelope =
        serde_json::from_str(line).map_err(|e| EngineError::Protocol(e.to_string()))?;
    if env.v != CONTRACT_VERSION {
        return Err(EngineError::ContractMismatch { found: env.v });
    }
    match (env.ok, env.error) {
        (true, _) => Ok(env.data),
        (false, Some(err)) => Err(EngineError::Cli {
            code: err.code,
            message: err.message,
        }),
        (false, None) => Err(EngineError::Protocol("ok:false without error".into())),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn envelope_ok_returns_data() {
        let v = parse_envelope(r#"{"v":1,"ok":true,"data":{"a":1}}"#).unwrap();
        assert_eq!(v["a"], 1);
    }

    #[test]
    fn envelope_error_maps_code() {
        let e = parse_envelope(
            r#"{"v":1,"ok":false,"error":{"code":"needs_input","message":"passphrase"}}"#,
        )
        .unwrap_err();
        assert_eq!(
            e,
            EngineError::Cli {
                code: "needs_input".into(),
                message: "passphrase".into()
            }
        );
    }

    #[test]
    fn envelope_rejects_other_versions_and_noise() {
        assert_eq!(
            parse_envelope(r#"{"v":2,"ok":true,"data":null}"#).unwrap_err(),
            EngineError::ContractMismatch { found: 2 }
        );
        assert!(matches!(parse_envelope(""), Err(EngineError::Protocol(_))));
        assert!(matches!(
            parse_envelope("On branch main\n{}"),
            Err(EngineError::Protocol(_))
        ));
        assert!(matches!(
            parse_envelope(r#"{"v":1,"ok":false}"#),
            Err(EngineError::Protocol(_))
        ));
    }

    #[test]
    fn non_utf8_stderr_is_text_not_an_error() {
        let line = lossy_line(b"WARN bad path caf\xe9.psd\r\n");
        assert_eq!(line, "WARN bad path caf\u{fffd}.psd");
        assert!(parse_progress(&line).is_none());
        let p = lossy_line(b"{\"type\":\"progress\",\"id\":1,\"msg\":\"caf\xe9\",\"pos\":1,\"len\":2,\"unit\":\"items\",\"done\":false}\n");
        assert_eq!(parse_progress(&p).unwrap().msg, "caf\u{fffd}");
    }

    #[test]
    fn progress_lines_parse_and_logs_do_not() {
        let p = parse_progress(r#"{"type":"progress","id":3,"msg":"Uploading","pos":5,"len":10,"unit":"bytes","done":false}"#)
            .unwrap();
        assert_eq!(
            (p.id, p.pos, p.len, p.unit.as_str()),
            (3, 5, Some(10), "bytes")
        );
        assert!(parse_progress(r#"{"type":"progress","id":1,"msg":"","pos":0,"len":null,"unit":"items","done":true}"#)
            .is_some_and(|p| p.len.is_none() && p.done));
        assert!(parse_progress("WARN something happened").is_none());
        assert!(parse_progress(r#"{"type":"log","msg":"x"}"#).is_none());
    }
}

// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! AU-10: one server process per directory, enforced at boot.
//!
//! Two `mediagit-server` processes sharing a directory **silently discard each
//! other's writes**, and not in one place:
//!
//! | State | Consequence of a second instance |
//! |---|---|
//! | `users.jsonl` / `grants.jsonl` | each loads at boot and full-rewrites on mutation; the slower writer's snapshot wins and the other's users/grants vanish |
//! | `locks.jsonl` | same full-rewrite, and the double-lock 409 guard is per-process, so two clients can both hold one path |
//! | `delta_written_pairs` | the **circular chunk-delta guard** is in-process; two instances can write A→B and B→A and make the repo unpushable |
//! | `pack_verify_inflight` | the same pack gets verified twice over the WAN |
//! | `RevokedTokens` | logout does not propagate; the sibling keeps accepting a revoked JWT |
//! | rate limiter | N instances serve N× the configured budget |
//! | `want_cache` | negotiation state is per-process, so clone/push negotiation breaks |
//! | `.pending` durability sweep | assumes the process that left `.pending` is dead — false while a sibling is live |
//!
//! None of those fail loudly. Patching one leaves the other seven, so the fix
//! is at the only place that covers all of them: refuse to be the second
//! process.
//!
//! The mechanism is an **advisory exclusive lock held on an open file handle**,
//! not a pid file. The OS drops the lock when the process exits *for any
//! reason* — clean shutdown, panic, SIGKILL, power loss — so there is no such
//! thing as a stale lock to reclaim, no liveness probe to get wrong, and no
//! staleness window that delays a legitimate restart after a crash. A pid file
//! would need all three.
//!
//! This is deliberately not [`crate::setup::probe_server_running`], which
//! answers "is something already listening on that port" for the setup wizard.
//! Different question, different answer: two instances can share a directory
//! while binding different ports, which is exactly the case that eats data.

use anyhow::{Context, Result};
use std::fs::File;
use std::path::{Path, PathBuf};

/// Env escape hatch. Set to `1` to downgrade the refusal to a warning.
pub const ALLOW_MULTI_INSTANCE_ENV: &str = "MEDIAGIT_ALLOW_MULTI_INSTANCE";

const LOCK_FILE_NAME: &str = ".mediagit-server.lock";

/// Who holds the lock, in plain text, for a human reading a refusal.
///
/// A separate file because on Windows an exclusive `LockFileEx` range also
/// blocks *reads* from other handles — so the one process that needs these
/// details, the one being refused, is precisely the one that cannot read them
/// out of the lock file. The lock file therefore stays empty and carries only
/// the lock; this one carries only information and is never consulted for
/// control flow.
const OWNER_FILE_NAME: &str = ".mediagit-server.owner";

/// A held exclusive lock on one directory. Releasing happens when this value
/// is dropped, and — because the lock lives on the file handle rather than in
/// the file's contents — also when the process dies without dropping it.
#[derive(Debug)]
pub struct InstanceLock {
    path: PathBuf,
    owner_path: PathBuf,
    /// Held purely for its lock. Deliberately empty — see [`OWNER_FILE_NAME`].
    _file: File,
}

impl InstanceLock {
    /// How long to keep retrying before declaring the directory taken.
    ///
    /// A restart — supervisor, container, or `Restart-QaServer` in the QA
    /// harness — starts the replacement the moment the old process is signalled,
    /// which can be marginally before the OS has torn down its handles and
    /// released the lock. Without this window that ordinary restart becomes an
    /// intermittent refusal to start, i.e. this guard's most likely effect in
    /// practice would be a flake rather than the protection it exists for.
    ///
    /// Bounded deliberately, and short: a genuinely live sibling holds the lock
    /// for its whole lifetime, so no amount of waiting would help and three
    /// seconds is not worth adding to an operator's feedback loop.
    const RETRY_WINDOW: std::time::Duration = std::time::Duration::from_secs(3);
    const RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

    /// Take the lock for `dir`, or fail describing who holds it.
    ///
    /// `dir` is created if missing. `bind_addr` is recorded beside the lock so
    /// an operator staring at a refusal can find the other process.
    pub fn acquire(dir: &Path, bind_addr: &str) -> Result<Self> {
        let deadline = std::time::Instant::now() + Self::RETRY_WINDOW;
        loop {
            match Self::try_acquire_once(dir, bind_addr) {
                Ok(lock) => return Ok(lock),
                Err(e) if std::time::Instant::now() < deadline => {
                    tracing::debug!(
                        "instance lock on {} not yet available, retrying: {e:#}",
                        dir.display()
                    );
                    std::thread::sleep(Self::RETRY_INTERVAL);
                }
                Err(e) => return Err(e),
            }
        }
    }

    fn try_acquire_once(dir: &Path, bind_addr: &str) -> Result<Self> {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("creating directory for instance lock: {}", dir.display()))?;
        let path = dir.join(LOCK_FILE_NAME);
        let owner_path = dir.join(OWNER_FILE_NAME);

        // Not `create_new`: the file legitimately survives a previous run (the
        // *lock* does not), so demanding its absence would turn every crash
        // into a manual cleanup.
        let file = File::options()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("opening instance lock: {}", path.display()))?;

        // `File::try_lock` (std, stable since 1.89) is `flock(LOCK_EX|LOCK_NB)`
        // on unix and `LockFileEx` on Windows. `Err(Error)` is a real I/O
        // problem and is worth surfacing distinctly from "someone holds it";
        // only `WouldBlock` means a live sibling.
        match file.try_lock() {
            Ok(()) => {
                write_owner(&owner_path, bind_addr)?;
                return Ok(Self {
                    path,
                    owner_path,
                    _file: file,
                });
            }
            Err(std::fs::TryLockError::WouldBlock) => {}
            Err(std::fs::TryLockError::Error(e)) => {
                // Not "someone holds it" — the lock could not be *tested*. The
                // realistic cause is a filesystem with no lock support (some
                // NFS/SMB mounts return ENOLCK), which is a working deployment
                // this check would otherwise break. Still refuses, because
                // starting anyway would silently drop the protection on exactly
                // the shared mounts where two instances are most likely — but
                // the message has to name the cause and the way out, or an
                // operator is left guessing at an error they cannot act on.
                return Err(anyhow::Error::new(e).context(format!(
                    "could not test the instance lock at {} — refusing to start rather than \
                     assume this directory is unused. If this is a network filesystem without \
                     lock support, either put `repos_dir` on local storage or set \
                     {ALLOW_MULTI_INSTANCE_ENV}=1 (and make sure only one process uses it).",
                    path.display()
                )));
            }
        }

        // Lock held by a live sibling. The owner file names it; it is untrusted
        // text and only ever reaches an error message.
        let holder = std::fs::read_to_string(&owner_path).unwrap_or_default();
        let holder = holder.trim();
        let holder = if holder.is_empty() {
            "unknown (no owner file)".to_string()
        } else {
            holder.replace('\n', "; ")
        };
        anyhow::bail!(
            "refusing to start: another mediagit-server process already owns {}\n  \
             holder: {holder}\n  \
             Two servers sharing a directory silently discard each other's auth writes, \
             lock records and delta-chain guards. Run one process per directory, give this \
             instance its own `repos_dir`/`auth_store_dir`, or set {ALLOW_MULTI_INSTANCE_ENV}=1 \
             to override at your own risk.",
            dir.display(),
        );
    }

    /// Path of the lock file, for messages and tests.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        // Best-effort tidy-up. The lock itself is released by the handle
        // closing right after this, so a failure here costs nothing but a
        // leftover file — which the next start reuses.
        let _ = std::fs::remove_file(&self.owner_path);
        let _ = std::fs::remove_file(&self.path);
    }
}

fn write_owner(owner_path: &Path, bind_addr: &str) -> Result<()> {
    // Epoch seconds rather than a formatted timestamp: this crate has no
    // date-formatting dependency and one is not worth adding for a line a
    // human reads next to a pid.
    let started = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    std::fs::write(
        owner_path,
        format!(
            "pid={}\nbind={bind_addr}\nstarted_epoch={started}\n",
            std::process::id()
        ),
    )
    .with_context(|| format!("recording instance lock owner: {}", owner_path.display()))
}

/// [`InstanceLock::acquire`], with the escape hatch applied.
///
/// With `MEDIAGIT_ALLOW_MULTI_INSTANCE=1` a failure to take the lock becomes a
/// warning that names what is at risk, and startup continues without a lock.
/// It exists for the operator who has genuinely separated every piece of shared
/// state and only trips over a lock file on a shared mount — and so the QA
/// drill can exercise the refusal path deliberately.
pub fn acquire_or_warn(dir: &Path, bind_addr: &str) -> Result<Option<InstanceLock>> {
    match InstanceLock::acquire(dir, bind_addr) {
        Ok(lock) => Ok(Some(lock)),
        Err(e) if std::env::var(ALLOW_MULTI_INSTANCE_ENV).as_deref() == Ok("1") => {
            tracing::warn!(
                "{ALLOW_MULTI_INSTANCE_ENV}=1: starting a second instance on {} anyway. \
                 At risk, all silently: auth users/grants, repo locks, the circular \
                 chunk-delta guard, pack verification, token revocation, the rate-limit \
                 budget, want-cache negotiation and the .pending durability sweep. \
                 Underlying: {e:#}",
                dir.display()
            );
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn second_acquire_on_the_same_dir_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let first = InstanceLock::acquire(dir.path(), "127.0.0.1:8080").unwrap();

        let second = InstanceLock::acquire(dir.path(), "127.0.0.1:9090");
        let err = format!(
            "{:#}",
            second.expect_err("a second instance on one dir must be refused")
        );
        assert!(
            err.contains(&std::process::id().to_string()),
            "the refusal must name the holder's pid so an operator can find it, got: {err}"
        );
        assert!(
            err.contains("127.0.0.1:8080"),
            "the refusal must name the HOLDER's bind address, not the newcomer's — \
             a second instance on a different port is exactly the case that eats \
             data, got: {err}"
        );

        drop(first);
    }

    #[test]
    fn refusal_still_happens_promptly_when_the_holder_is_live() {
        // The retry window must not turn a real refusal into a long hang, and
        // must not extend past its own bound.
        let dir = tempfile::tempdir().unwrap();
        let _held = InstanceLock::acquire(dir.path(), "127.0.0.1:8080").unwrap();

        let started = std::time::Instant::now();
        assert!(InstanceLock::acquire(dir.path(), "127.0.0.1:9090").is_err());
        let waited = started.elapsed();
        assert!(
            waited >= InstanceLock::RETRY_WINDOW,
            "a refusal before the window elapses means the retry is not actually \
             running, so an ordinary restart race would still be refused: {waited:?}"
        );
        assert!(
            waited < InstanceLock::RETRY_WINDOW * 3,
            "the window must stay bounded; a live sibling never releases, so \
             waiting longer only delays a message the operator needs: {waited:?}"
        );
    }

    #[test]
    fn releasing_the_lock_lets_the_next_process_in() {
        let dir = tempfile::tempdir().unwrap();
        let first = InstanceLock::acquire(dir.path(), "127.0.0.1:8080").unwrap();
        drop(first);

        InstanceLock::acquire(dir.path(), "127.0.0.1:8080").expect(
            "the lock must be reusable once released — otherwise a clean \
                    restart is blocked by its own predecessor",
        );
    }

    #[test]
    fn a_leftover_lock_file_alone_does_not_block_startup() {
        // The crash case. A pid-file design would see this file and have to
        // decide whether the pid is alive; here the file carries no authority,
        // only the held handle does — so a file left by a killed process is
        // simply reused.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(LOCK_FILE_NAME), "pid=999999\nbind=stale\n").unwrap();

        InstanceLock::acquire(dir.path(), "127.0.0.1:8080")
            .expect("a lock file with no live holder must not block a restart");
    }

    #[test]
    fn separate_dirs_do_not_contend() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let _la = InstanceLock::acquire(a.path(), "127.0.0.1:8080").unwrap();
        InstanceLock::acquire(b.path(), "127.0.0.1:9090")
            .expect("the lock is per-directory; two servers with their own dirs are supported");
    }
}

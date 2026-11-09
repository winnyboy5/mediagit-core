// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Server panel commands: the local server's lifecycle, and users, access, API keys and
//! hosted repositories on any server (this computer's or a remote one).
//!
//! Administration is the CLI's job: every command here builds argv for
//! `auth admin ...`, `auth key ...` or `remote repos ...` with `--server=<url>` and runs it
//! through the engine. Passwords go to the child's stdin as secrets, never argv. Deletions
//! and revocations need `confirm: true` from the webview (the CLI needs `--yes` in machine
//! mode); deleting a repository also needs its name typed back.

use serde_json::Value;
use tauri::Manager;

use super::admin::{SecretCmd, run, server_flag};
use super::{invalid, plain_arg};
use crate::engine::{Engine, EngineError, Secret};
use crate::server_manager::{LocalStatus, ServerManager, StorageOpts};

// ---------------------------------------------------------------------------
// argv builders (pure, unit-tested)
// ---------------------------------------------------------------------------

/// `["auth", "admin", ...]` style prefix plus the required `--server=<url>`.
fn base(prefix: &[&str], server: &str) -> Result<Vec<String>, EngineError> {
    if server.is_empty() {
        return Err(invalid("server address", server));
    }
    let mut argv: Vec<String> = prefix.iter().map(|s| (*s).to_owned()).collect();
    server_flag(&mut argv, Some(server))?;
    Ok(argv)
}

/// A user, key or label: one printable line that cannot read as a flag.
fn name_ok(what: &str, v: &str) -> Result<(), EngineError> {
    plain_arg(what, v)?;
    if v.len() > 128 || v.chars().any(char::is_control) {
        return Err(invalid(what, v));
    }
    Ok(())
}

/// A hosted repository name: what the server accepts on its path.
fn repo_name_ok(v: &str) -> Result<(), EngineError> {
    let ok = !v.is_empty()
        && v.len() <= 100
        && !v.starts_with(['.', '-'])
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'))
        && !v.contains("..")
        && !v.ends_with('/')
        && !v.contains("//");
    if ok {
        Ok(())
    } else {
        Err(invalid("repository name", v))
    }
}

fn role_ok(role: &str) -> Result<(), EngineError> {
    if matches!(role, "read" | "write" | "admin") {
        Ok(())
    } else {
        Err(invalid("role", role))
    }
}

pub(crate) fn users_list_argv(server: &str) -> Result<Vec<String>, EngineError> {
    base(&["auth", "admin", "list-users"], server)
}

/// The new account's password is a secret (stdin); the email is a flag.
pub(crate) fn user_create_cmd(
    server: &str,
    user: &str,
    email: &str,
    role: &str,
    password: String,
) -> Result<SecretCmd, EngineError> {
    name_ok("username", user)?;
    name_ok("email", email)?;
    role_ok(role)?;
    let mut argv = base(&["auth", "admin", "create-user"], server)?;
    argv.extend([
        format!("--role={role}"),
        format!("--email={email}"),
        "--".into(),
        user.into(),
    ]);
    SecretCmd::with(argv, vec![Some(password)])
}

pub(crate) fn set_role_argv(
    server: &str,
    user: &str,
    role: &str,
) -> Result<Vec<String>, EngineError> {
    name_ok("username", user)?;
    role_ok(role)?;
    let mut argv = base(&["auth", "admin", "set-role"], server)?;
    argv.extend(["--".into(), user.into(), role.into()]);
    Ok(argv)
}

pub(crate) fn reset_password_cmd(
    server: &str,
    user: &str,
    password: String,
) -> Result<SecretCmd, EngineError> {
    name_ok("username", user)?;
    let mut argv = base(&["auth", "admin", "reset-password"], server)?;
    argv.extend(["--".into(), user.into()]);
    SecretCmd::with(argv, vec![Some(password)])
}

/// Disabling blocks sign-in without deleting the account; needs `confirm`.
pub(crate) fn disable_user_argv(
    server: &str,
    user: &str,
    confirm: bool,
) -> Result<Vec<String>, EngineError> {
    confirmed(confirm)?;
    name_ok("username", user)?;
    let mut argv = base(&["auth", "admin", "disable-user"], server)?;
    argv.extend(["--yes".into(), "--".into(), user.into()]);
    Ok(argv)
}

pub(crate) fn enable_user_argv(server: &str, user: &str) -> Result<Vec<String>, EngineError> {
    name_ok("username", user)?;
    let mut argv = base(&["auth", "admin", "enable-user"], server)?;
    argv.extend(["--".into(), user.into()]);
    Ok(argv)
}

/// What one person can reach.
pub(crate) fn list_grants_argv(server: &str, user: &str) -> Result<Vec<String>, EngineError> {
    name_ok("username", user)?;
    let mut argv = base(&["auth", "admin", "list-grants"], server)?;
    argv.extend(["--".into(), user.into()]);
    Ok(argv)
}

/// Who can reach one repository.
pub(crate) fn repo_access_argv(server: &str, repo: &str) -> Result<Vec<String>, EngineError> {
    repo_name_ok(repo)?;
    let mut argv = base(&["auth", "admin", "repo-access"], server)?;
    argv.extend(["--".into(), repo.into()]);
    Ok(argv)
}

pub(crate) fn grant_argv(
    server: &str,
    user: &str,
    repo: &str,
    level: &str,
) -> Result<Vec<String>, EngineError> {
    name_ok("username", user)?;
    repo_name_ok(repo)?;
    role_ok(level)?;
    let mut argv = base(&["auth", "admin", "grant"], server)?;
    argv.extend(["--".into(), user.into(), repo.into(), level.into()]);
    Ok(argv)
}

pub(crate) fn revoke_grant_argv(
    server: &str,
    user: &str,
    repo: &str,
    confirm: bool,
) -> Result<Vec<String>, EngineError> {
    confirmed(confirm)?;
    name_ok("username", user)?;
    repo_name_ok(repo)?;
    let mut argv = base(&["auth", "admin", "revoke-grant"], server)?;
    argv.extend(["--yes".into(), "--".into(), user.into(), repo.into()]);
    Ok(argv)
}

pub(crate) fn keys_list_argv(server: &str) -> Result<Vec<String>, EngineError> {
    base(&["auth", "key", "list"], server)
}

pub(crate) fn key_create_argv(
    server: &str,
    name: &str,
    permissions: &[String],
) -> Result<Vec<String>, EngineError> {
    name_ok("key name", name)?;
    let mut argv = base(&["auth", "key", "create"], server)?;
    argv.push(format!("--name={name}"));
    if !permissions.is_empty() {
        for p in permissions {
            let ok = !p.is_empty()
                && p.len() <= 64
                && p.chars()
                    .all(|c| c.is_ascii_lowercase() || matches!(c, ':' | '_' | '.' | '-' | '*'));
            if !ok || p.starts_with('-') {
                return Err(invalid("permission", p));
            }
        }
        argv.push(format!("--permissions={}", permissions.join(",")));
    }
    Ok(argv)
}

pub(crate) fn key_revoke_argv(
    server: &str,
    id: &str,
    confirm: bool,
) -> Result<Vec<String>, EngineError> {
    confirmed(confirm)?;
    name_ok("key id", id)?;
    let mut argv = base(&["auth", "key", "revoke"], server)?;
    argv.extend(["--yes".into(), "--".into(), id.into()]);
    Ok(argv)
}

pub(crate) fn repos_list_argv(server: &str) -> Result<Vec<String>, EngineError> {
    base(&["remote", "repos", "list"], server)
}

pub(crate) fn repo_create_argv(server: &str, name: &str) -> Result<Vec<String>, EngineError> {
    repo_name_ok(name)?;
    let mut argv = base(&["remote", "repos", "create"], server)?;
    argv.extend(["--".into(), name.into()]);
    Ok(argv)
}

/// Deleting needs `confirm` and the repository's name typed back exactly.
pub(crate) fn repo_delete_argv(
    server: &str,
    name: &str,
    typed_name: &str,
    force: bool,
    confirm: bool,
) -> Result<Vec<String>, EngineError> {
    confirmed(confirm)?;
    repo_name_ok(name)?;
    if typed_name != name {
        return Err(invalid("confirmation name", typed_name));
    }
    let mut argv = base(&["remote", "repos", "delete"], server)?;
    if force {
        argv.push("--force".into());
    }
    argv.extend(["--yes".into(), "--".into(), name.into()]);
    Ok(argv)
}

fn confirmed(confirm: bool) -> Result<(), EngineError> {
    if confirm {
        Ok(())
    } else {
        Err(EngineError::InvalidArgument("not confirmed".into()))
    }
}

// ---------------------------------------------------------------------------
// commands
// ---------------------------------------------------------------------------

async fn go_plain(engine: &Engine, argv: Vec<String>) -> Result<Value, EngineError> {
    run(engine, None, &SecretCmd::plain(argv)).await
}

#[tauri::command]
pub async fn server_users_list(
    engine: tauri::State<'_, Engine>,
    server: String,
) -> Result<Value, EngineError> {
    go_plain(&engine, users_list_argv(&server)?).await
}

#[tauri::command]
pub async fn server_user_create(
    engine: tauri::State<'_, Engine>,
    server: String,
    user: String,
    email: String,
    role: String,
    password: String,
) -> Result<Value, EngineError> {
    let cmd = user_create_cmd(&server, &user, &email, &role, password)?;
    run(&engine, None, &cmd).await
}

#[tauri::command]
pub async fn server_user_set_role(
    engine: tauri::State<'_, Engine>,
    server: String,
    user: String,
    role: String,
) -> Result<Value, EngineError> {
    go_plain(&engine, set_role_argv(&server, &user, &role)?).await
}

#[tauri::command]
pub async fn server_user_reset_password(
    engine: tauri::State<'_, Engine>,
    server: String,
    user: String,
    password: String,
) -> Result<Value, EngineError> {
    let cmd = reset_password_cmd(&server, &user, password)?;
    run(&engine, None, &cmd).await
}

#[tauri::command]
pub async fn server_user_disable(
    engine: tauri::State<'_, Engine>,
    server: String,
    user: String,
    confirm: bool,
) -> Result<Value, EngineError> {
    go_plain(&engine, disable_user_argv(&server, &user, confirm)?).await
}

#[tauri::command]
pub async fn server_user_enable(
    engine: tauri::State<'_, Engine>,
    server: String,
    user: String,
) -> Result<Value, EngineError> {
    go_plain(&engine, enable_user_argv(&server, &user)?).await
}

#[tauri::command]
pub async fn server_user_grants(
    engine: tauri::State<'_, Engine>,
    server: String,
    user: String,
) -> Result<Value, EngineError> {
    go_plain(&engine, list_grants_argv(&server, &user)?).await
}

#[tauri::command]
pub async fn server_repo_access(
    engine: tauri::State<'_, Engine>,
    server: String,
    repo: String,
) -> Result<Value, EngineError> {
    go_plain(&engine, repo_access_argv(&server, &repo)?).await
}

#[tauri::command]
pub async fn server_grant_set(
    engine: tauri::State<'_, Engine>,
    server: String,
    user: String,
    repo: String,
    level: String,
) -> Result<Value, EngineError> {
    go_plain(&engine, grant_argv(&server, &user, &repo, &level)?).await
}

#[tauri::command]
pub async fn server_grant_remove(
    engine: tauri::State<'_, Engine>,
    server: String,
    user: String,
    repo: String,
    confirm: bool,
) -> Result<Value, EngineError> {
    go_plain(&engine, revoke_grant_argv(&server, &user, &repo, confirm)?).await
}

#[tauri::command]
pub async fn server_keys_list(
    engine: tauri::State<'_, Engine>,
    server: String,
) -> Result<Value, EngineError> {
    go_plain(&engine, keys_list_argv(&server)?).await
}

#[tauri::command]
pub async fn server_key_create(
    engine: tauri::State<'_, Engine>,
    server: String,
    name: String,
    permissions: Vec<String>,
) -> Result<Value, EngineError> {
    go_plain(&engine, key_create_argv(&server, &name, &permissions)?).await
}

#[tauri::command]
pub async fn server_key_revoke(
    engine: tauri::State<'_, Engine>,
    server: String,
    id: String,
    confirm: bool,
) -> Result<Value, EngineError> {
    go_plain(&engine, key_revoke_argv(&server, &id, confirm)?).await
}

#[tauri::command]
pub async fn server_repos_list(
    engine: tauri::State<'_, Engine>,
    server: String,
) -> Result<Value, EngineError> {
    go_plain(&engine, repos_list_argv(&server)?).await
}

#[tauri::command]
pub async fn server_repo_create(
    engine: tauri::State<'_, Engine>,
    server: String,
    name: String,
) -> Result<Value, EngineError> {
    go_plain(&engine, repo_create_argv(&server, &name)?).await
}

#[tauri::command]
pub async fn server_repo_delete(
    engine: tauri::State<'_, Engine>,
    server: String,
    name: String,
    typed_name: String,
    force: bool,
    confirm: bool,
) -> Result<Value, EngineError> {
    go_plain(
        &engine,
        repo_delete_argv(&server, &name, &typed_name, force, confirm)?,
    )
    .await
}

// -- local server lifecycle --------------------------------------------------

#[tauri::command]
pub async fn server_local_status(
    mgr: tauri::State<'_, ServerManager>,
) -> Result<LocalStatus, EngineError> {
    Ok(mgr.status().await)
}

/// First run: create the config and the first admin (password on stdin only).
#[tauri::command]
pub async fn server_local_setup(
    mgr: tauri::State<'_, ServerManager>,
    username: String,
    email: String,
    password: String,
    storage: Option<StorageOpts>,
) -> Result<LocalStatus, EngineError> {
    let password = Secret::new(password)?;
    mgr.setup(&username, &email, &password, storage.as_ref())
        .await
}

#[tauri::command]
pub async fn server_local_start(
    mgr: tauri::State<'_, ServerManager>,
) -> Result<LocalStatus, EngineError> {
    mgr.start().await
}

#[tauri::command]
pub async fn server_local_stop(
    mgr: tauri::State<'_, ServerManager>,
) -> Result<LocalStatus, EngineError> {
    mgr.stop().await
}

#[tauri::command]
pub async fn server_local_log(
    mgr: tauri::State<'_, ServerManager>,
    lines: usize,
) -> Result<String, EngineError> {
    Ok(mgr.log_tail(lines.clamp(1, 500)))
}

/// Network access for the local server: on needs auth (always on for a server set up here).
#[tauri::command]
pub async fn server_local_set_network(
    mgr: tauri::State<'_, ServerManager>,
    enabled: bool,
) -> Result<LocalStatus, EngineError> {
    mgr.set_lan(enabled).await
}

/// Move the local server to another folder (`path` omitted = the default). Stopped server only.
#[tauri::command]
pub async fn server_local_set_folder(
    mgr: tauri::State<'_, ServerManager>,
    path: Option<String>,
) -> Result<LocalStatus, EngineError> {
    mgr.set_folder(path.as_deref()).await
}

/// Check storage credentials with a throwaway write, read and delete. Returns `{ok, error?}`.
#[tauri::command]
pub async fn server_storage_test(
    mgr: tauri::State<'_, ServerManager>,
    storage: StorageOpts,
) -> Result<Value, EngineError> {
    mgr.storage_test(&storage).await
}

/// Stop the owned server when the app exits.
pub async fn stop_on_exit(app: &tauri::AppHandle) {
    app.state::<ServerManager>().stop_owned().await;
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const S: &str = "http://127.0.0.1:4100";
    const PW: &str = "correct horse battery 7";

    #[allow(clippy::needless_pass_by_value)] // call sites pass temporaries
    fn bad<T>(r: Result<T, EngineError>) -> bool {
        matches!(r, Err(EngineError::InvalidArgument(_)))
    }

    #[test]
    fn every_op_targets_the_server_with_a_flag() {
        assert_eq!(
            users_list_argv(S).unwrap(),
            [
                "auth",
                "admin",
                "list-users",
                "--server=http://127.0.0.1:4100"
            ]
        );
        assert_eq!(
            repos_list_argv(S).unwrap(),
            ["remote", "repos", "list", "--server=http://127.0.0.1:4100"]
        );
        assert!(bad(users_list_argv("")));
        assert!(bad(users_list_argv("ftp://x")));
        assert!(bad(users_list_argv("--server=x")));
    }

    #[test]
    fn passwords_are_stdin_secrets_never_argv() {
        let c = user_create_cmd(S, "ana", "ana@x.io", "write", PW.into()).unwrap();
        assert!(c.argv().iter().all(|a| !a.contains(PW)));
        assert_eq!(c.secret_count(), 1);
        assert!(c.argv().contains(&"--email=ana@x.io".to_owned()));
        let r = reset_password_cmd(S, "ana", PW.into()).unwrap();
        assert!(r.argv().iter().all(|a| !a.contains(PW)));
        assert_eq!(r.secret_count(), 1);
        assert_eq!(r.argv().last().unwrap(), "ana");
    }

    #[test]
    fn names_cannot_become_flags() {
        for u in ["--role", "-x", "", "a\nb"] {
            assert!(bad(set_role_argv(S, u, "read")), "{u:?}");
            assert!(
                bad(user_create_cmd(S, u, "a@b.c", "read", PW.into())),
                "{u:?}"
            );
        }
        for r in ["../x", "-r", ".hidden", "a//b", "", "a b", "x/"] {
            assert!(bad(repo_create_argv(S, r)), "{r:?}");
            assert!(bad(grant_argv(S, "ana", r, "read")), "{r:?}");
        }
        assert!(bad(set_role_argv(S, "ana", "root")));
        assert!(bad(grant_argv(S, "ana", "film", "owner")));
        assert!(repo_create_argv(S, "client/film-1").is_ok());
    }

    #[test]
    fn destructive_ops_need_confirm_and_pass_yes_after_positionals_guard() {
        assert!(bad(key_revoke_argv(S, "k1", false)));
        assert!(bad(revoke_grant_argv(S, "ana", "film", false)));
        assert!(bad(repo_delete_argv(S, "film", "film", false, false)));
        assert!(bad(repo_delete_argv(S, "film", "fil", false, true)));
        let d = repo_delete_argv(S, "film", "film", true, true).unwrap();
        assert_eq!(
            d,
            [
                "remote",
                "repos",
                "delete",
                "--server=http://127.0.0.1:4100",
                "--force",
                "--yes",
                "--",
                "film"
            ]
        );
        assert!(
            key_revoke_argv(S, "k1", true)
                .unwrap()
                .contains(&"--yes".to_owned())
        );
    }

    #[test]
    fn disable_needs_confirm_and_enable_does_not() {
        assert!(bad(disable_user_argv(S, "ana", false)));
        assert!(bad(disable_user_argv(S, "-x", true)));
        assert_eq!(
            disable_user_argv(S, "ana", true).unwrap(),
            [
                "auth",
                "admin",
                "disable-user",
                "--server=http://127.0.0.1:4100",
                "--yes",
                "--",
                "ana"
            ]
        );
        assert_eq!(
            enable_user_argv(S, "ana").unwrap(),
            [
                "auth",
                "admin",
                "enable-user",
                "--server=http://127.0.0.1:4100",
                "--",
                "ana"
            ]
        );
        assert!(bad(enable_user_argv(S, "--x")));
    }

    #[test]
    fn grant_lists_validate_names() {
        assert_eq!(
            list_grants_argv(S, "ana").unwrap(),
            [
                "auth",
                "admin",
                "list-grants",
                "--server=http://127.0.0.1:4100",
                "--",
                "ana"
            ]
        );
        assert_eq!(
            repo_access_argv(S, "film").unwrap(),
            [
                "auth",
                "admin",
                "repo-access",
                "--server=http://127.0.0.1:4100",
                "--",
                "film"
            ]
        );
        assert!(bad(list_grants_argv(S, "-x")));
        assert!(bad(repo_access_argv(S, "../x")));
    }

    #[test]
    fn key_permissions_are_validated() {
        let a = key_create_argv(S, "ci", &["repo:read".into(), "repo:write".into()]).unwrap();
        assert!(a.contains(&"--permissions=repo:read,repo:write".to_owned()));
        assert!(
            !key_create_argv(S, "ci", &[])
                .unwrap()
                .iter()
                .any(|a| a.starts_with("--permissions"))
        );
        assert!(bad(key_create_argv(S, "ci", &["--x".into()])));
        assert!(bad(key_create_argv(S, "ci", &["a b".into()])));
        assert!(bad(key_create_argv(S, "-n", &[])));
    }
}

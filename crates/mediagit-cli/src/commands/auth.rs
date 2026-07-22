// MediaGit - Git for Media Files
// Copyright (C) 2025 MediaGit Contributors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU Affero General Public License for more details.

//! Interactive client-side authentication: `mediagit auth <subcommand>`.
//!
//! Talks to the server's `/auth/*` HTTP surface (see
//! `crates/mediagit-security/src/auth/handlers.rs` and
//! `crates/mediagit-server/src/handlers/admin.rs`) directly via `reqwest`
//! rather than through `ProtocolClient`: those routes hang off the server
//! root (`{origin}/auth/...`), whereas `ProtocolClient::base_url` is always
//! `{origin}/{repo}` — there is no repo to scope these requests to.
//!
//! Request/response DTOs are re-declared locally rather than imported,
//! mirroring how `mediagit-protocol` mirrors server DTOs for the git
//! protocol (e.g. `LockInfo`): the concrete `Serialize`/`Deserialize` types
//! in `mediagit-security`/`mediagit-server` either aren't public from a
//! production dependency (`mediagit-server` is a dev-dependency here) or
//! only derive the side we don't need (the server's own request types only
//! derive `Deserialize`, since the server never serializes them). `Role` and
//! `GrantLevel` *are* reused directly from `mediagit-security`, since both
//! derive both traits and reusing them keeps role/level wire encoding from
//! drifting between client and server.
//!
//! `enable_auth` defaults to `false`, and every `/auth/*` route (including
//! `/auth/login`) is conditionally mounted only when the server constructs
//! an `AuthService` — see `create_router`/`create_rate_limited_router` in
//! `mediagit-server/src/lib.rs`. A 404 on any of these paths therefore means
//! "this server has authentication disabled", never "route doesn't exist for
//! some other reason" — every handler here treats it that way rather than
//! surfacing a raw HTTP error.

use crate::output;
use crate::repo::{CredentialSource, find_repo_root};
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use dialoguer::{Input, Password};
use mediagit_protocol::Credentials;
use mediagit_security::auth::{AuthResponse, GrantLevel, user::Role};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Manage authentication with a MediaGit server
#[derive(Parser, Debug)]
pub struct AuthCmd {
    #[command(subcommand)]
    pub subcommand: AuthSubcommand,
}

#[derive(Subcommand, Debug)]
pub enum AuthSubcommand {
    /// Log in to a MediaGit server and store the credential
    Login(LoginOpts),

    /// Register a new account on a MediaGit server
    Register(ServerOpts),

    /// Show which credential tier is active and the server's auth mode
    Status(ServerOpts),

    /// Delete the stored credential for a server
    Logout(LogoutOpts),

    /// Change your own password
    Passwd(ServerOpts),

    /// Show your identity, role, and granted repos
    Whoami(ServerOpts),

    /// Manage your own API keys
    Key(KeyCmd),

    /// Administrative user/grant management (requires the admin role)
    Admin(AdminCmd),
}

/// Options shared by every subcommand that needs only a server target.
#[derive(Parser, Debug)]
pub struct ServerOpts {
    /// Server URL (scheme://host[:port]). Defaults to the current repo's
    /// `origin` remote when omitted.
    #[arg(long, value_name = "URL")]
    pub server: Option<String>,
}

/// `mediagit auth login`
#[derive(Parser, Debug)]
pub struct LoginOpts {
    #[arg(long, value_name = "URL")]
    pub server: Option<String>,

    /// Username or email; prompted for if omitted
    #[arg(long)]
    pub username: Option<String>,

    /// Store this bearer token directly instead of prompting for a password
    #[arg(long, conflicts_with = "api_key")]
    pub token: Option<String>,

    /// Store this API key directly instead of prompting for a password
    #[arg(long, conflicts_with = "token")]
    pub api_key: Option<String>,
}

/// `mediagit auth logout`
#[derive(Parser, Debug)]
pub struct LogoutOpts {
    #[arg(long, value_name = "URL")]
    pub server: Option<String>,

    /// Remove every locally known credential, not just this server's
    #[arg(long)]
    pub all: bool,
}

/// `mediagit auth key <subcommand>`
#[derive(Parser, Debug)]
pub struct KeyCmd {
    #[command(subcommand)]
    pub subcommand: KeySubcommand,
}

#[derive(Subcommand, Debug)]
pub enum KeySubcommand {
    /// Mint a new API key for yourself
    Create(KeyCreateOpts),

    /// List your own API keys
    #[command(alias = "ls")]
    List(ServerOpts),

    /// Revoke one of your own API keys (or, as admin, anyone's)
    Revoke(KeyRevokeOpts),
}

#[derive(Parser, Debug)]
pub struct KeyCreateOpts {
    #[arg(long, value_name = "URL")]
    pub server: Option<String>,

    /// A memorable label for the key (e.g. "ci")
    #[arg(long)]
    pub name: String,

    /// Comma-separated permission list; defaults to your own full permission set
    #[arg(long, value_delimiter = ',')]
    pub permissions: Option<Vec<String>>,
}

#[derive(Parser, Debug)]
pub struct KeyRevokeOpts {
    #[arg(long, value_name = "URL")]
    pub server: Option<String>,

    /// Key id, as shown by `auth key list`
    pub id: String,
}

/// `mediagit auth admin <subcommand>`
#[derive(Parser, Debug)]
pub struct AdminCmd {
    #[command(subcommand)]
    pub subcommand: AdminSubcommand,
}

#[derive(Subcommand, Debug)]
pub enum AdminSubcommand {
    /// List every user account
    #[command(name = "list-users")]
    ListUsers(ServerOpts),

    /// Change a user's role
    #[command(name = "set-role")]
    SetRole(SetRoleOpts),

    /// Create a user with an explicit role (for closed-registration servers)
    #[command(name = "create-user")]
    CreateUser(CreateUserOpts),

    /// Reset a user's password (forgot-password recovery path)
    #[command(name = "reset-password")]
    ResetPassword(UserOpts),

    /// Grant a user access to a repo
    Grant(GrantOpts),

    /// Remove a user's grant on a repo
    #[command(name = "revoke-grant")]
    RevokeGrant(RevokeGrantOpts),
}

#[derive(Parser, Debug)]
pub struct SetRoleOpts {
    #[arg(long, value_name = "URL")]
    pub server: Option<String>,

    /// Username or user id
    pub user: String,

    /// New role: read|write|admin
    pub role: String,
}

#[derive(Parser, Debug)]
pub struct CreateUserOpts {
    #[arg(long, value_name = "URL")]
    pub server: Option<String>,

    /// Username for the new account
    pub user: String,

    /// Role: read|write|admin
    #[arg(long)]
    pub role: String,
}

#[derive(Parser, Debug)]
pub struct UserOpts {
    #[arg(long, value_name = "URL")]
    pub server: Option<String>,

    /// Username or user id
    pub user: String,
}

#[derive(Parser, Debug)]
pub struct GrantOpts {
    #[arg(long, value_name = "URL")]
    pub server: Option<String>,

    /// Username or user id
    pub user: String,

    /// Repo name
    pub repo: String,

    /// Access level: read|write|admin
    pub level: String,
}

#[derive(Parser, Debug)]
pub struct RevokeGrantOpts {
    #[arg(long, value_name = "URL")]
    pub server: Option<String>,

    /// Username or user id
    pub user: String,

    /// Repo name
    pub repo: String,
}

/// Message printed (never as a raw HTTP error) whenever a 404 on `/auth/*`
/// means the whole router is unmounted rather than a specific resource
/// missing (see the module doc comment).
const AUTH_DISABLED_MSG: &str = "this server has authentication disabled - no login is required";

impl AuthCmd {
    pub async fn execute(&self) -> Result<()> {
        match &self.subcommand {
            AuthSubcommand::Login(opts) => login(opts).await,
            AuthSubcommand::Register(opts) => register(opts).await,
            AuthSubcommand::Status(opts) => status(opts).await,
            AuthSubcommand::Logout(opts) => logout(opts).await,
            AuthSubcommand::Passwd(opts) => passwd(opts).await,
            AuthSubcommand::Whoami(opts) => whoami(opts).await,
            AuthSubcommand::Key(cmd) => cmd.execute().await,
            AuthSubcommand::Admin(cmd) => cmd.execute().await,
        }
    }
}

impl KeyCmd {
    pub async fn execute(&self) -> Result<()> {
        match &self.subcommand {
            KeySubcommand::Create(opts) => key_create(opts).await,
            KeySubcommand::List(opts) => key_list(opts).await,
            KeySubcommand::Revoke(opts) => key_revoke(opts).await,
        }
    }
}

impl AdminCmd {
    pub async fn execute(&self) -> Result<()> {
        match &self.subcommand {
            AdminSubcommand::ListUsers(opts) => admin_list_users(opts).await,
            AdminSubcommand::SetRole(opts) => admin_set_role(opts).await,
            AdminSubcommand::CreateUser(opts) => admin_create_user(opts).await,
            AdminSubcommand::ResetPassword(opts) => admin_reset_password(opts).await,
            AdminSubcommand::Grant(opts) => admin_grant(opts).await,
            AdminSubcommand::RevokeGrant(opts) => admin_revoke_grant(opts).await,
        }
    }
}

// ---------------------------------------------------------------------
// Server-target resolution: where to send requests, and how to
// resolve/persist/forget credentials for that destination.
// ---------------------------------------------------------------------

/// Either an explicit `--server` origin (no local repo/config context) or
/// the current repo's `origin` remote (full tiered resolution, config
/// included). Every subcommand resolves one of these first.
enum ServerTarget {
    Explicit {
        origin: String,
    },
    FromRepo {
        repo_root: PathBuf,
        config: Box<mediagit_config::Config>,
        remote: String,
        origin: String,
    },
}

impl ServerTarget {
    fn origin(&self) -> &str {
        match self {
            ServerTarget::Explicit { origin } => origin,
            ServerTarget::FromRepo { origin, .. } => origin,
        }
    }

    /// Resolve credentials for this target: the full env/config/keychain
    /// tier chain when we have a repo+remote to consult, or the smaller
    /// env/keychain-by-origin chain when we don't (see
    /// [`crate::repo::resolve_credentials_for_origin`]).
    fn resolve_credentials(&self) -> (Credentials, CredentialSource) {
        match self {
            ServerTarget::Explicit { origin } => {
                crate::repo::resolve_credentials_for_origin(origin)
            }
            ServerTarget::FromRepo {
                repo_root,
                config,
                remote,
                ..
            } => crate::repo::resolve_credentials_tiered(repo_root, config, remote),
        }
    }

    fn remember(&self, creds: &Credentials) {
        match self {
            ServerTarget::Explicit { origin } => {
                crate::repo::remember_credentials_for_origin(origin, creds)
            }
            ServerTarget::FromRepo { config, remote, .. } => {
                crate::repo::remember_credentials(config, remote, creds)
            }
        }
    }

    /// Returns `true` if a stored credential was actually removed.
    fn forget(&self) -> bool {
        match self {
            ServerTarget::Explicit { origin } => crate::repo::forget_credentials_for_origin(origin),
            ServerTarget::FromRepo { config, remote, .. } => {
                crate::repo::forget_credentials(config, remote)
            }
        }
    }
}

async fn resolve_server_target(server: &Option<String>) -> Result<ServerTarget> {
    if let Some(url) = server {
        let origin = crate::repo::remote_origin(url).with_context(|| {
            format!("--server '{url}' is not a valid URL (need scheme://host[:port])")
        })?;
        return Ok(ServerTarget::Explicit { origin });
    }

    let repo_root =
        find_repo_root().context("not inside a mediagit repository; pass --server <URL>")?;
    let config = mediagit_config::Config::load(&repo_root)
        .await
        .context("failed to load repository config")?;
    let remote_url = config
        .resolve_remote_url("origin")
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("no 'origin' remote configured; pass --server <URL>")?;
    let origin = crate::repo::remote_origin(&remote_url)
        .with_context(|| format!("remote 'origin' URL '{remote_url}' is not a valid URL"))?;

    Ok(ServerTarget::FromRepo {
        repo_root,
        config: Box::new(config),
        remote: "origin".to_string(),
        origin,
    })
}

// ---------------------------------------------------------------------
// HTTP helpers
// ---------------------------------------------------------------------

fn attach_credentials(
    req: reqwest::RequestBuilder,
    creds: &Credentials,
) -> reqwest::RequestBuilder {
    match creds {
        Credentials::Bearer(t) => req.bearer_auth(t),
        Credentials::ApiKey(k) => req.header("x-api-key", k),
        Credentials::None => req,
    }
}

#[derive(Deserialize)]
struct ErrorBody {
    error: String,
}

/// Extract a clean error message from a non-2xx JSON error body, falling
/// back to the bare status code if the body isn't the expected shape.
async fn error_message(resp: reqwest::Response) -> String {
    let status = resp.status();
    match resp.json::<ErrorBody>().await {
        Ok(body) => body.error,
        Err(_) => format!("request failed with status {status}"),
    }
}

fn format_duration_secs(secs: i64) -> String {
    if secs <= 0 {
        return "expired".to_string();
    }
    format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
}

fn parse_role(s: &str) -> Result<Role> {
    match s.to_lowercase().as_str() {
        "read" => Ok(Role::Read),
        "write" => Ok(Role::Write),
        "admin" => Ok(Role::Admin),
        other => anyhow::bail!("invalid role '{other}' (expected read|write|admin)"),
    }
}

fn parse_grant_level(s: &str) -> Result<GrantLevel> {
    match s.to_lowercase().as_str() {
        "read" => Ok(GrantLevel::Read),
        "write" => Ok(GrantLevel::Write),
        "admin" => Ok(GrantLevel::Admin),
        other => anyhow::bail!("invalid level '{other}' (expected read|write|admin)"),
    }
}

#[derive(Deserialize)]
struct WhoAmI {
    #[allow(dead_code)]
    id: String,
    username: String,
    email: String,
    role: Role,
    #[allow(dead_code)]
    permissions: Vec<String>,
    grants: Vec<GrantInfoResp>,
}

#[derive(Deserialize)]
struct GrantInfoResp {
    repo: String,
    level: GrantLevel,
}

fn print_whoami_body(who: &WhoAmI) {
    output::detail("Username", &who.username);
    output::detail("Role", &format!("{:?}", who.role));
    if who.grants.is_empty() {
        output::detail("Grants", "(none)");
    } else {
        for g in &who.grants {
            output::detail(&format!("Grant: {}", g.repo), &format!("{:?}", g.level));
        }
    }
}

/// GET `/auth/whoami`, print identity/role/grants, and return the parsed
/// identity (`None` when the server has auth disabled). Shared by `whoami`,
/// `status`, and the post-login identity check for `--token`/`--api-key`.
async fn fetch_and_print_whoami(
    client: &reqwest::Client,
    origin: &str,
    creds: &Credentials,
) -> Result<Option<WhoAmI>> {
    let url = format!("{origin}/auth/whoami");
    let resp = attach_credentials(client.get(&url), creds)
        .send()
        .await
        .with_context(|| format!("failed to reach {url}"))?;

    if resp.status().as_u16() == 404 {
        output::info(AUTH_DISABLED_MSG);
        return Ok(None);
    }
    if resp.status().as_u16() == 401 {
        anyhow::bail!("credential was rejected (401 Unauthorized)");
    }
    if !resp.status().is_success() {
        anyhow::bail!("{}", error_message(resp).await);
    }
    let who: WhoAmI = resp
        .json()
        .await
        .context("failed to parse /auth/whoami response")?;
    print_whoami_body(&who);
    Ok(Some(who))
}

/// Record the authenticated identity as the current repo's commit author
/// (config `[author]`), so commits after `auth login` are attributed to the
/// logged-in user. No-op when run outside a repository (e.g. `auth login
/// --server` before cloning). Best-effort — never fails the login itself.
async fn set_login_author(name: &str, email: &str) {
    let name = name.trim();
    let email = email.trim();
    let Ok(repo_root) = find_repo_root() else {
        return;
    };
    let Ok(mut config) = mediagit_config::Config::load(&repo_root).await else {
        return;
    };
    // Skip the write when the author already matches the logged-in identity.
    if config.author.name.as_deref() == Some(name) && config.author.email.as_deref() == Some(email)
    {
        return;
    }
    config.author.name = Some(name.to_string());
    config.author.email = Some(email.to_string());
    if config.save(&repo_root).is_ok() {
        output::detail("Commit author", &format!("{name} <{email}>"));
    }
}

// ---------------------------------------------------------------------
// login / register
// ---------------------------------------------------------------------

async fn login(opts: &LoginOpts) -> Result<()> {
    let target = resolve_server_target(&opts.server).await?;
    let client = reqwest::Client::new();

    if let Some(creds) = direct_credentials(opts.token.as_deref(), opts.api_key.as_deref()) {
        target.remember(&creds);
        output::success(&format!("Stored credential for {}", target.origin()));
        match fetch_and_print_whoami(&client, target.origin(), &creds).await {
            Ok(Some(who)) => set_login_author(&who.username, &who.email).await,
            Ok(None) => {}
            Err(e) => output::warning(&format!("stored, but could not verify identity: {e:#}")),
        }
        return Ok(());
    }

    let username = match &opts.username {
        Some(u) => u.clone(),
        None => {
            let u: String = Input::new()
                .with_prompt("Username or email")
                .interact_text()?;
            u
        }
    };
    let password = Password::new().with_prompt("Password").interact()?;

    #[derive(Serialize)]
    struct LoginBody<'a> {
        identifier: &'a str,
        password: &'a str,
    }

    let url = format!("{}/auth/login", target.origin());
    let resp = client
        .post(&url)
        .json(&LoginBody {
            identifier: &username,
            password: &password,
        })
        .send()
        .await
        .with_context(|| format!("failed to reach {url}"))?;

    if resp.status().as_u16() == 404 {
        output::info(AUTH_DISABLED_MSG);
        return Ok(());
    }
    if !resp.status().is_success() {
        anyhow::bail!("login failed: {}", error_message(resp).await);
    }
    let auth: AuthResponse = resp
        .json()
        .await
        .context("failed to parse login response")?;
    let creds = Credentials::Bearer(auth.tokens.access_token.clone());
    target.remember(&creds);

    output::success(&format!("Logged in as {}", auth.user.username));
    output::detail("Server", target.origin());
    output::detail("Role", &format!("{:?}", auth.user.role));
    output::detail(
        "Token expires in",
        &format_duration_secs(auth.tokens.expires_in),
    );
    set_login_author(&auth.user.username, &auth.user.email).await;
    Ok(())
}

fn direct_credentials(token: Option<&str>, api_key: Option<&str>) -> Option<Credentials> {
    if let Some(t) = token {
        return Some(Credentials::Bearer(t.to_string()));
    }
    if let Some(k) = api_key {
        return Some(Credentials::ApiKey(k.to_string()));
    }
    None
}

async fn register(opts: &ServerOpts) -> Result<()> {
    let target = resolve_server_target(&opts.server).await?;
    let client = reqwest::Client::new();

    let username: String = Input::new().with_prompt("Username").interact_text()?;
    let email: String = Input::new().with_prompt("Email").interact_text()?;
    let password = Password::new()
        .with_prompt("Password")
        .with_confirmation("Confirm password", "Passwords don't match")
        .interact()?;

    #[derive(Serialize)]
    struct RegisterBody<'a> {
        username: &'a str,
        email: &'a str,
        password: &'a str,
    }

    let url = format!("{}/auth/register", target.origin());
    let resp = client
        .post(&url)
        .json(&RegisterBody {
            username: &username,
            email: &email,
            password: &password,
        })
        .send()
        .await
        .with_context(|| format!("failed to reach {url}"))?;

    if resp.status().as_u16() == 404 {
        output::info(AUTH_DISABLED_MSG);
        return Ok(());
    }
    if !resp.status().is_success() {
        anyhow::bail!("registration failed: {}", error_message(resp).await);
    }
    let auth: AuthResponse = resp
        .json()
        .await
        .context("failed to parse registration response")?;
    let creds = Credentials::Bearer(auth.tokens.access_token.clone());
    target.remember(&creds);

    output::success(&format!(
        "Registered and logged in as {}",
        auth.user.username
    ));
    output::detail("Server", target.origin());
    output::detail("Role", &format!("{:?}", auth.user.role));
    output::detail(
        "Token expires in",
        &format_duration_secs(auth.tokens.expires_in),
    );
    set_login_author(&auth.user.username, &auth.user.email).await;
    Ok(())
}

// ---------------------------------------------------------------------
// status / whoami / logout / passwd
// ---------------------------------------------------------------------

async fn status(opts: &ServerOpts) -> Result<()> {
    let target = resolve_server_target(&opts.server).await?;
    let (creds, source) = target.resolve_credentials();
    let client = reqwest::Client::new();

    let url = format!("{}/auth/whoami", target.origin());
    let resp = attach_credentials(client.get(&url), &creds)
        .send()
        .await
        .with_context(|| format!("failed to reach {url}"))?;

    // Server auth mode is reported first, before any local tier info.
    if resp.status().as_u16() == 404 {
        output::info("Server auth mode: disabled - no login is required");
        return Ok(());
    }
    output::info("Server auth mode: enabled");

    let tier = match source {
        CredentialSource::Env => "environment variable",
        CredentialSource::Config => "config.toml (remotes.<name>.token/api_key)",
        CredentialSource::Keychain => "OS keychain",
        CredentialSource::None => "none (no credential found)",
    };
    output::detail("Credential tier", tier);
    output::detail("Account", target.origin());

    if resp.status().as_u16() == 401 {
        output::warning("Not logged in (no valid credential for this server)");
        return Ok(());
    }
    if !resp.status().is_success() {
        anyhow::bail!("{}", error_message(resp).await);
    }
    let who: WhoAmI = resp
        .json()
        .await
        .context("failed to parse /auth/whoami response")?;
    print_whoami_body(&who);
    Ok(())
}

async fn whoami(opts: &ServerOpts) -> Result<()> {
    let target = resolve_server_target(&opts.server).await?;
    let (creds, _source) = target.resolve_credentials();
    let client = reqwest::Client::new();
    fetch_and_print_whoami(&client, target.origin(), &creds).await?;
    Ok(())
}

async fn logout(opts: &LogoutOpts) -> Result<()> {
    let mut removed = 0usize;

    // A single-server logout needs a target; `--all` is a best-effort sweep and
    // must not require a repo or --server (it's the escape hatch for clearing a
    // stale credential from anywhere).
    match resolve_server_target(&opts.server).await {
        Ok(target) => {
            if target.forget() {
                removed += 1;
            }
        }
        Err(e) => {
            if !opts.all {
                return Err(e);
            }
        }
    }

    if opts.all {
        // ponytail: the `keyring` crate has no cross-platform "list every
        // entry for this service" API, so `--all` covers every origin
        // MediaGit can locally enumerate (the current repo's configured
        // remotes) rather than a true keychain-wide sweep. Upgrade path:
        // switch to a keyring backend that supports enumeration, if a truly
        // exhaustive sweep is ever needed.
        if let Ok(repo_root) = find_repo_root()
            && let Ok(config) = mediagit_config::Config::load(&repo_root).await
        {
            for name in config.list_remotes() {
                if crate::repo::forget_credentials(&config, &name) {
                    removed += 1;
                }
            }
        }
    }

    if removed == 0 {
        output::info("No stored credential found");
    } else {
        output::success(&format!("Removed {removed} stored credential(s)"));
    }
    Ok(())
}

async fn passwd(opts: &ServerOpts) -> Result<()> {
    let target = resolve_server_target(&opts.server).await?;
    let (creds, _source) = target.resolve_credentials();
    if creds == Credentials::None {
        anyhow::bail!("not logged in; run `mediagit auth login` first");
    }
    let client = reqwest::Client::new();

    let current_password = Password::new().with_prompt("Current password").interact()?;
    let new_password = Password::new()
        .with_prompt("New password")
        .with_confirmation("Confirm new password", "Passwords don't match")
        .interact()?;

    #[derive(Serialize)]
    struct Body<'a> {
        current_password: &'a str,
        new_password: &'a str,
    }
    #[derive(Deserialize)]
    struct Resp {
        note: String,
    }

    let url = format!("{}/auth/password", target.origin());
    let resp = attach_credentials(client.post(&url), &creds)
        .json(&Body {
            current_password: &current_password,
            new_password: &new_password,
        })
        .send()
        .await
        .with_context(|| format!("failed to reach {url}"))?;

    if resp.status().as_u16() == 404 {
        anyhow::bail!(AUTH_DISABLED_MSG);
    }
    if resp.status().as_u16() == 401 {
        anyhow::bail!("current password was rejected");
    }
    if !resp.status().is_success() {
        anyhow::bail!("password change failed: {}", error_message(resp).await);
    }
    let body: Resp = resp
        .json()
        .await
        .context("failed to parse password-change response")?;
    output::success("Password changed");
    output::info(&body.note);
    Ok(())
}

// ---------------------------------------------------------------------
// key create / list / revoke
// ---------------------------------------------------------------------

async fn key_create(opts: &KeyCreateOpts) -> Result<()> {
    let target = resolve_server_target(&opts.server).await?;
    let (creds, _source) = target.resolve_credentials();
    if creds == Credentials::None {
        anyhow::bail!("not logged in; run `mediagit auth login` first");
    }
    let client = reqwest::Client::new();

    #[derive(Serialize)]
    struct Body<'a> {
        name: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        permissions: Option<&'a Vec<String>>,
    }
    #[derive(Deserialize)]
    struct Resp {
        id: String,
        key: String,
        name: String,
        permissions: Vec<String>,
    }

    let url = format!("{}/auth/keys", target.origin());
    let resp = attach_credentials(client.post(&url), &creds)
        .json(&Body {
            name: &opts.name,
            permissions: opts.permissions.as_ref(),
        })
        .send()
        .await
        .with_context(|| format!("failed to reach {url}"))?;

    // A 404 here is unambiguous: /auth/keys is mounted unconditionally
    // whenever the admin router is (see the module doc comment).
    if resp.status().as_u16() == 404 {
        anyhow::bail!(AUTH_DISABLED_MSG);
    }
    if !resp.status().is_success() {
        anyhow::bail!("key creation failed: {}", error_message(resp).await);
    }
    let body: Resp = resp
        .json()
        .await
        .context("failed to parse key-creation response")?;

    output::success(&format!("Created key '{}' (id {})", body.name, body.id));
    output::warning("Store this key now - it will not be shown again:");
    println!("{}", body.key);
    output::detail("Permissions", &body.permissions.join(", "));
    Ok(())
}

async fn key_list(opts: &ServerOpts) -> Result<()> {
    let target = resolve_server_target(&opts.server).await?;
    let (creds, _source) = target.resolve_credentials();
    let client = reqwest::Client::new();

    #[derive(Deserialize)]
    struct KeyInfo {
        id: String,
        name: String,
        created_at: i64,
    }

    let url = format!("{}/auth/keys/mine", target.origin());
    let resp = attach_credentials(client.get(&url), &creds)
        .send()
        .await
        .with_context(|| format!("failed to reach {url}"))?;

    if resp.status().as_u16() == 404 {
        anyhow::bail!(AUTH_DISABLED_MSG);
    }
    if !resp.status().is_success() {
        anyhow::bail!("{}", error_message(resp).await);
    }
    let keys: Vec<KeyInfo> = resp
        .json()
        .await
        .context("failed to parse key list response")?;
    if keys.is_empty() {
        output::info("No API keys");
        return Ok(());
    }
    for k in &keys {
        println!("{}\t{}\t{}", k.id, k.name, k.created_at);
    }
    Ok(())
}

async fn key_revoke(opts: &KeyRevokeOpts) -> Result<()> {
    let target = resolve_server_target(&opts.server).await?;
    let (creds, _source) = target.resolve_credentials();
    let client = reqwest::Client::new();

    let url = format!("{}/auth/keys/{}", target.origin(), opts.id);
    let resp = attach_credentials(client.delete(&url), &creds)
        .send()
        .await
        .with_context(|| format!("failed to reach {url}"))?;

    // Unlike the admin `set-role`/`grant`/etc. paths, there is no prior
    // call here to prove the admin router is mounted, so a 404 could
    // (rarely) mean either "auth disabled" or "no such key id" -- say both
    // rather than assert either with false confidence.
    if resp.status().as_u16() == 404 {
        anyhow::bail!("key not found, or this server has authentication disabled");
    }
    if resp.status().as_u16() == 403 {
        anyhow::bail!("you can only revoke your own API keys (or be an admin)");
    }
    if !resp.status().is_success() {
        anyhow::bail!("key revocation failed: {}", error_message(resp).await);
    }
    output::success(&format!("Revoked key {}", opts.id));
    Ok(())
}

// ---------------------------------------------------------------------
// admin: list-users / set-role / create-user / reset-password / grant / revoke-grant
// ---------------------------------------------------------------------

#[derive(Deserialize)]
struct AdminUserInfo {
    id: String,
    username: String,
    role: Role,
}

/// Resolve a CLI-supplied `<user>` argument (username or literal user id) to
/// a server-side user id via `GET /auth/users` (admin-only). Doing this
/// lookup first also proves the admin router is reachable, so a 404 from
/// the *mutating* call that follows unambiguously means "target not found"
/// rather than needing to also explain "or auth might be disabled".
async fn resolve_user_id(
    client: &reqwest::Client,
    origin: &str,
    creds: &Credentials,
    user: &str,
) -> Result<String> {
    let users = fetch_users(client, origin, creds).await?;
    if let Some(u) = users.iter().find(|u| u.id == user) {
        return Ok(u.id.clone());
    }
    users
        .into_iter()
        .find(|u| u.username == user)
        .map(|u| u.id)
        .with_context(|| format!("no user found matching '{user}'"))
}

async fn fetch_users(
    client: &reqwest::Client,
    origin: &str,
    creds: &Credentials,
) -> Result<Vec<AdminUserInfo>> {
    let url = format!("{origin}/auth/users");
    let resp = attach_credentials(client.get(&url), creds)
        .send()
        .await
        .with_context(|| format!("failed to reach {url}"))?;
    if resp.status().as_u16() == 404 {
        anyhow::bail!(AUTH_DISABLED_MSG);
    }
    if resp.status().as_u16() == 403 {
        anyhow::bail!("administrator access required (user:manage permission)");
    }
    if !resp.status().is_success() {
        anyhow::bail!("{}", error_message(resp).await);
    }
    resp.json()
        .await
        .context("failed to parse user list response")
}

async fn admin_list_users(opts: &ServerOpts) -> Result<()> {
    let target = resolve_server_target(&opts.server).await?;
    let (creds, _source) = target.resolve_credentials();
    let client = reqwest::Client::new();

    let users = fetch_users(&client, target.origin(), &creds).await?;
    for u in &users {
        println!("{}\t{}\t{:?}", u.id, u.username, u.role);
    }
    Ok(())
}

async fn admin_set_role(opts: &SetRoleOpts) -> Result<()> {
    let target = resolve_server_target(&opts.server).await?;
    let (creds, _source) = target.resolve_credentials();
    let role = parse_role(&opts.role)?;
    let client = reqwest::Client::new();

    let id = resolve_user_id(&client, target.origin(), &creds, &opts.user).await?;

    #[derive(Serialize)]
    struct Body {
        role: Role,
    }
    #[derive(Deserialize)]
    struct Resp {
        role: Role,
        note: String,
    }

    let url = format!("{}/auth/users/{}/role", target.origin(), id);
    let resp = attach_credentials(client.patch(&url), &creds)
        .json(&Body { role })
        .send()
        .await
        .with_context(|| format!("failed to reach {url}"))?;

    if resp.status().as_u16() == 404 {
        anyhow::bail!("user not found (may have been removed)");
    }
    if resp.status().as_u16() == 409 {
        anyhow::bail!("cannot demote the last remaining admin");
    }
    if resp.status().as_u16() == 403 {
        anyhow::bail!("administrator access required (user:manage permission)");
    }
    if !resp.status().is_success() {
        anyhow::bail!("role change failed: {}", error_message(resp).await);
    }
    let body: Resp = resp
        .json()
        .await
        .context("failed to parse role-change response")?;
    output::success(&format!("Set {}'s role to {:?}", opts.user, body.role));
    output::info(&body.note);
    Ok(())
}

async fn admin_create_user(opts: &CreateUserOpts) -> Result<()> {
    let target = resolve_server_target(&opts.server).await?;
    let (creds, _source) = target.resolve_credentials();
    let role = parse_role(&opts.role)?;
    let client = reqwest::Client::new();

    let email: String = Input::new().with_prompt("Email").interact_text()?;
    let password = Password::new()
        .with_prompt("Password")
        .with_confirmation("Confirm password", "Passwords don't match")
        .interact()?;

    #[derive(Serialize)]
    struct Body<'a> {
        username: &'a str,
        email: &'a str,
        password: &'a str,
        role: Role,
    }
    #[derive(Deserialize)]
    struct Resp {
        id: String,
        username: String,
        role: Role,
    }

    let url = format!("{}/auth/users", target.origin());
    let resp = attach_credentials(client.post(&url), &creds)
        .json(&Body {
            username: &opts.user,
            email: &email,
            password: &password,
            role,
        })
        .send()
        .await
        .with_context(|| format!("failed to reach {url}"))?;

    if resp.status().as_u16() == 404 {
        anyhow::bail!(AUTH_DISABLED_MSG);
    }
    if resp.status().as_u16() == 403 {
        anyhow::bail!("administrator access required (user:manage permission)");
    }
    if !resp.status().is_success() {
        anyhow::bail!("user creation failed: {}", error_message(resp).await);
    }
    let body: Resp = resp
        .json()
        .await
        .context("failed to parse user-creation response")?;
    output::success(&format!(
        "Created user {} (id {}, role {:?})",
        body.username, body.id, body.role
    ));
    Ok(())
}

async fn admin_reset_password(opts: &UserOpts) -> Result<()> {
    let target = resolve_server_target(&opts.server).await?;
    let (creds, _source) = target.resolve_credentials();
    let client = reqwest::Client::new();

    let id = resolve_user_id(&client, target.origin(), &creds, &opts.user).await?;

    let new_password = Password::new()
        .with_prompt("New password")
        .with_confirmation("Confirm new password", "Passwords don't match")
        .interact()?;

    #[derive(Serialize)]
    struct Body<'a> {
        new_password: &'a str,
    }

    let url = format!("{}/auth/users/{}/password", target.origin(), id);
    let resp = attach_credentials(client.patch(&url), &creds)
        .json(&Body {
            new_password: &new_password,
        })
        .send()
        .await
        .with_context(|| format!("failed to reach {url}"))?;

    if resp.status().as_u16() == 404 {
        anyhow::bail!("user not found (may have been removed)");
    }
    if resp.status().as_u16() == 403 {
        anyhow::bail!("administrator access required (user:manage permission)");
    }
    if !resp.status().is_success() {
        anyhow::bail!("password reset failed: {}", error_message(resp).await);
    }
    output::success(&format!("Password reset for {}", opts.user));
    // reset_password returns a bare 204 with no body, unlike the
    // self-service change (`ChangePasswordResponse.note`) -- state the same
    // no-revocation caveat client-side so the operator isn't misled either way.
    output::info(
        "This does not invalidate the user's existing sessions - any active token \
         remains valid until it expires (up to 24h).",
    );
    Ok(())
}

async fn admin_grant(opts: &GrantOpts) -> Result<()> {
    let target = resolve_server_target(&opts.server).await?;
    let (creds, _source) = target.resolve_credentials();
    let level = parse_grant_level(&opts.level)?;
    let client = reqwest::Client::new();

    let id = resolve_user_id(&client, target.origin(), &creds, &opts.user).await?;

    #[derive(Serialize)]
    struct Body<'a> {
        repo: &'a str,
        level: GrantLevel,
    }

    let url = format!("{}/auth/users/{}/grants", target.origin(), id);
    let resp = attach_credentials(client.post(&url), &creds)
        .json(&Body {
            repo: &opts.repo,
            level,
        })
        .send()
        .await
        .with_context(|| format!("failed to reach {url}"))?;

    if resp.status().as_u16() == 404 {
        anyhow::bail!("user not found (may have been removed)");
    }
    if resp.status().as_u16() == 403 {
        anyhow::bail!("administrator access required (user:manage permission)");
    }
    if !resp.status().is_success() {
        anyhow::bail!("grant failed: {}", error_message(resp).await);
    }
    output::success(&format!(
        "Granted {} {:?} on '{}'",
        opts.user, level, opts.repo
    ));
    Ok(())
}

async fn admin_revoke_grant(opts: &RevokeGrantOpts) -> Result<()> {
    let target = resolve_server_target(&opts.server).await?;
    let (creds, _source) = target.resolve_credentials();
    let client = reqwest::Client::new();

    let id = resolve_user_id(&client, target.origin(), &creds, &opts.user).await?;

    #[derive(Serialize)]
    struct Body<'a> {
        repo: &'a str,
    }

    let url = format!("{}/auth/users/{}/grants", target.origin(), id);
    let resp = attach_credentials(client.delete(&url), &creds)
        .json(&Body { repo: &opts.repo })
        .send()
        .await
        .with_context(|| format!("failed to reach {url}"))?;

    if resp.status().as_u16() == 404 {
        anyhow::bail!("user not found (may have been removed)");
    }
    if resp.status().as_u16() == 403 {
        anyhow::bail!("administrator access required (user:manage permission)");
    }
    if !resp.status().is_success() {
        anyhow::bail!("grant revocation failed: {}", error_message(resp).await);
    }
    output::success(&format!("Revoked {}'s grant on '{}'", opts.user, opts.repo));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_duration_secs_formats_hours_and_minutes() {
        assert_eq!(format_duration_secs(3600 * 23 + 60 * 59), "23h59m");
        assert_eq!(format_duration_secs(0), "expired");
        assert_eq!(format_duration_secs(-5), "expired");
    }

    #[test]
    fn parse_role_accepts_case_insensitive_known_values() {
        assert_eq!(parse_role("Admin").unwrap(), Role::Admin);
        assert_eq!(parse_role("read").unwrap(), Role::Read);
        assert_eq!(parse_role("WRITE").unwrap(), Role::Write);
        assert!(parse_role("superuser").is_err());
    }

    #[test]
    fn parse_grant_level_accepts_case_insensitive_known_values() {
        assert_eq!(parse_grant_level("Read").unwrap(), GrantLevel::Read);
        assert_eq!(parse_grant_level("admin").unwrap(), GrantLevel::Admin);
        assert!(parse_grant_level("nope").is_err());
    }

    #[test]
    fn direct_credentials_prefers_token_when_both_given() {
        let creds = direct_credentials(Some("tok"), Some("key")).unwrap();
        assert_eq!(creds, Credentials::Bearer("tok".to_string()));
    }

    #[test]
    fn direct_credentials_none_when_neither_given() {
        assert!(direct_credentials(None, None).is_none());
    }
}

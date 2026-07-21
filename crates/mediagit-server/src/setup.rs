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

//! `mediagit-server init` wizard and `mediagit-server admin` subcommands.
//!
//! Both are dispatched from `main.rs` before `ServerConfig::load` runs the
//! normal serve path (see `main.rs`), so neither touches the request-serving
//! code at all — they only write a config file and/or mutate the auth store
//! directly on disk via `CredentialsStore`.

use anyhow::{Context, Result};
use clap::{Args, Subcommand, ValueEnum};
use mediagit_security::auth::{
    user::Role, validate_password_strength, validate_registration_input, CredentialsStore, User,
};
use mediagit_server::ServerConfig;
use std::net::ToSocketAddrs;
use std::path::{Path, PathBuf};
use std::time::Duration;

// ---------------------------------------------------------------------
// init
// ---------------------------------------------------------------------

#[derive(Args, Debug)]
pub struct InitArgs {
    /// Skip all prompts; use the flags below (and their defaults) instead.
    /// For scripted / CI setup.
    #[arg(long)]
    pub non_interactive: bool,

    /// Overwrite an existing config file.
    #[arg(long)]
    pub force: bool,

    /// Path to write the config file to.
    #[arg(long, default_value = "mediagit-server.toml")]
    pub config: String,

    #[arg(long)]
    pub host: Option<String>,
    #[arg(long)]
    pub port: Option<u16>,
    #[arg(long)]
    pub data_dir: Option<PathBuf>,

    /// Enable authentication on the new server (default: off, matching the
    /// current product default). When on: a random JWT secret is generated,
    /// registration defaults closed, and rate limiting is enabled.
    #[arg(long)]
    pub enable_auth: bool,

    #[arg(long)]
    pub admin_username: Option<String>,
    #[arg(long)]
    pub admin_email: Option<String>,
    #[arg(long)]
    pub admin_password: Option<String>,
}

pub async fn run_init(args: &InitArgs) -> Result<()> {
    let config_path = Path::new(&args.config);
    if config_path.exists() && !args.force {
        anyhow::bail!(
            "config file '{}' already exists; pass --force to overwrite it",
            args.config
        );
    }

    let (host, port, data_dir, enable_auth) = if args.non_interactive {
        (
            args.host.clone().unwrap_or_else(|| "127.0.0.1".to_string()),
            args.port.unwrap_or(3000),
            args.data_dir
                .clone()
                .unwrap_or_else(|| PathBuf::from("./repos")),
            args.enable_auth,
        )
    } else {
        prompt_basics(args)?
    };

    // Mirrors the runtime refusal in main.rs (non-loopback host + auth off) —
    // surface it at wizard time instead of at first boot.
    if !enable_auth
        && !crate::is_loopback_host(&host)
        && std::env::var("MEDIAGIT_ALLOW_INSECURE_BIND").as_deref() != Ok("1")
    {
        anyhow::bail!(
            "refusing to write a config the server will refuse to boot from: host '{}' is not \
             loopback-only and authentication is off. Choose one: bind to 127.0.0.1/localhost, \
             enable authentication (--enable-auth, or answer 'yes' at the prompt), or set \
             MEDIAGIT_ALLOW_INSECURE_BIND=1 to override at your own risk.",
            host
        );
    }

    let mut config = ServerConfig {
        host,
        port,
        repos_dir: data_dir,
        ..ServerConfig::default()
    };

    let admin: Option<(String, String, String)> = if enable_auth {
        config.enable_auth = true;
        config.jwt_secret = Some(generate_jwt_secret());
        // Secure-by-default for new installs; existing configs are
        // untouched (ServerConfig::allow_open_registration defaults true).
        config.allow_open_registration = false;
        config.enable_rate_limiting = true;

        if args.non_interactive {
            match (
                &args.admin_username,
                &args.admin_email,
                &args.admin_password,
            ) {
                (Some(u), Some(e), Some(p)) => Some((u.clone(), e.clone(), p.clone())),
                _ => {
                    eprintln!(
                        "auth enabled but --admin-username/--admin-email/--admin-password were \
                         not all supplied; skipping admin creation. Run `mediagit-server admin \
                         create` separately before starting the server."
                    );
                    None
                }
            }
        } else {
            Some(prompt_admin()?)
        }
    } else {
        None
    };

    write_config_file(config_path, &config)?;
    std::fs::create_dir_all(&config.repos_dir)
        .with_context(|| format!("creating repos dir {:?}", config.repos_dir))?;

    if enable_auth {
        let auth_store_dir = config.resolved_auth_store_dir();
        std::fs::create_dir_all(&auth_store_dir)
            .with_context(|| format!("creating auth store dir {:?}", auth_store_dir))?;

        if let Some((username, email, password)) = admin {
            create_user(&auth_store_dir, username, email, password, Role::Admin).await?;
        }
    }

    println!("Wrote {}", args.config);
    if enable_auth {
        println!(
            "Authentication is ON. Start the server with: mediagit-server --config {}",
            args.config
        );
    } else {
        println!(
            "Authentication is OFF (local/single-user mode). Start with: mediagit-server --config {}",
            args.config
        );
    }

    Ok(())
}

fn prompt_basics(args: &InitArgs) -> Result<(String, u16, PathBuf, bool)> {
    use dialoguer::{Confirm, Input};

    let host: String = Input::new()
        .with_prompt("Host to bind to")
        .default(args.host.clone().unwrap_or_else(|| "127.0.0.1".to_string()))
        .interact_text()?;
    let port: u16 = Input::new()
        .with_prompt("Port")
        .default(args.port.unwrap_or(3000))
        .interact_text()?;
    let data_dir: String = Input::new()
        .with_prompt("Repository storage directory")
        .default(
            args.data_dir
                .clone()
                .unwrap_or_else(|| PathBuf::from("./repos"))
                .display()
                .to_string(),
        )
        .interact_text()?;
    let enable_auth = Confirm::new()
        .with_prompt("Enable authentication?")
        .default(args.enable_auth)
        .interact()?;

    Ok((host, port, PathBuf::from(data_dir), enable_auth))
}

fn prompt_admin() -> Result<(String, String, String)> {
    use dialoguer::Input;

    println!("Create the first admin user:");
    let username: String = Input::new().with_prompt("Admin username").interact_text()?;
    let email: String = Input::new().with_prompt("Admin email").interact_text()?;
    let password = prompt_new_password()?;
    Ok((username, email, password))
}

/// Masked password + confirmation, re-prompting on mismatch.
fn prompt_new_password() -> Result<String> {
    use dialoguer::Password;

    loop {
        let password = Password::new().with_prompt("Password").interact()?;
        let confirm = Password::new().with_prompt("Confirm password").interact()?;
        if password != confirm {
            eprintln!("Passwords do not match, try again.");
            continue;
        }
        return Ok(password);
    }
}

/// 32 random bytes, hex-encoded, for use as a JWT signing secret.
// ponytail: mediagit-server has no `rand` dependency; `uuid` (already a
// dependency, §B) generates each v4 UUID via the OS RNG, so two concatenated
// UUIDs give 32 bytes of randomness without adding a new crate.
fn generate_jwt_secret() -> String {
    let mut bytes = Vec::with_capacity(32);
    bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn write_config_file(path: &Path, config: &ServerConfig) -> Result<()> {
    let toml_str = toml::to_string_pretty(config).context("serializing config")?;
    std::fs::write(path, toml_str).with_context(|| format!("writing {:?}", path))
}

// ---------------------------------------------------------------------
// admin
// ---------------------------------------------------------------------

#[derive(Args, Debug)]
pub struct AdminArgs {
    #[command(subcommand)]
    pub action: AdminAction,

    /// Path to the server config file (resolves the auth store dir and the
    /// host:port used for the running-server guard below).
    #[arg(short, long, default_value = "mediagit-server.toml")]
    pub config: String,

    /// Proceed even if the server appears to be running on the configured
    /// host:port. A running server holds users.jsonl in memory and
    /// full-rewrites it on its own next mutation, silently discarding this
    /// change unless the server is restarted afterward.
    #[arg(long)]
    pub force: bool,
}

#[derive(Subcommand, Debug)]
pub enum AdminAction {
    /// Create an admin user (the first-admin bootstrap).
    Create {
        username: String,
        email: String,
        /// Non-interactive password (omit to be prompted, masked, with confirmation).
        #[arg(long)]
        password: Option<String>,
    },
    /// List all users (id, username, role).
    List,
    /// Promote a user (by id or username) to Admin.
    Promote { user: String },
    /// Demote a user (by id or username) to Write.
    Demote { user: String },
    /// Create a user with an explicit role.
    CreateUser {
        username: String,
        email: String,
        #[arg(long, value_enum, default_value_t = RoleArg::Write)]
        role: RoleArg,
        #[arg(long)]
        password: Option<String>,
    },
    /// Reset a user's password (no current-password check — recovery path).
    ResetPassword {
        user: String,
        #[arg(long)]
        password: Option<String>,
    },
}

#[derive(ValueEnum, Clone, Copy, Debug)]
pub enum RoleArg {
    Read,
    Write,
    Admin,
}

impl From<RoleArg> for Role {
    fn from(r: RoleArg) -> Role {
        match r {
            RoleArg::Read => Role::Read,
            RoleArg::Write => Role::Write,
            RoleArg::Admin => Role::Admin,
        }
    }
}

pub async fn run_admin(args: &AdminArgs) -> Result<()> {
    let config = ServerConfig::load(&args.config)?;
    let auth_store_dir = config.resolved_auth_store_dir();

    let mutates = !matches!(args.action, AdminAction::List);
    if mutates && !args.force && probe_server_running(&config) {
        anyhow::bail!(
            "server appears to be running on {} - changes would be lost at its next write \
             (users.jsonl is rewritten in full on every mutation); stop it first, or pass \
             --force and restart the server afterward for the change to take effect.",
            config.bind_addr()
        );
    }
    if mutates && !config.enable_auth {
        eprintln!(
            "WARNING: `enable_auth` is false in {}; the user you are about to create/modify will \
             not be usable until authentication is enabled.",
            args.config
        );
    }

    match &args.action {
        AdminAction::Create {
            username,
            email,
            password,
        } => {
            let password = match password.clone() {
                Some(p) => p,
                None => prompt_new_password()?,
            };
            create_user(
                &auth_store_dir,
                username.clone(),
                email.clone(),
                password,
                Role::Admin,
            )
            .await?;
        }
        AdminAction::CreateUser {
            username,
            email,
            role,
            password,
        } => {
            let password = match password.clone() {
                Some(p) => p,
                None => prompt_new_password()?,
            };
            create_user(
                &auth_store_dir,
                username.clone(),
                email.clone(),
                password,
                (*role).into(),
            )
            .await?;
        }
        AdminAction::List => {
            let store = CredentialsStore::load_or_new(&auth_store_dir)?;
            for user in store.list_users().await {
                println!("{}\t{}\t{:?}", user.id, user.username, user.role);
            }
        }
        AdminAction::Promote { user } => {
            set_role_by_lookup(&auth_store_dir, user, Role::Admin).await?;
        }
        AdminAction::Demote { user } => {
            set_role_by_lookup(&auth_store_dir, user, Role::Write).await?;
        }
        AdminAction::ResetPassword { user, password } => {
            let password = match password.clone() {
                Some(p) => p,
                None => prompt_new_password()?,
            };
            validate_password_strength(&password).map_err(|e| anyhow::anyhow!(e))?;
            let store = CredentialsStore::load_or_new(&auth_store_dir)?;
            let target = find_user(&store, user).await?;
            store.update_password(&target.id, &password).await?;
            println!("Password reset for '{}'.", target.username);
        }
    }

    Ok(())
}

async fn create_user(
    auth_store_dir: &Path,
    username: String,
    email: String,
    password: String,
    role: Role,
) -> Result<()> {
    validate_registration_input(&username, &email, &password).map_err(|e| anyhow::anyhow!(e))?;
    let store = CredentialsStore::load_or_new(auth_store_dir)?;
    let user_id = uuid::Uuid::new_v4().to_string();
    store
        .create_user_with_role(user_id, username.clone(), email, &password, role)
        .await?;
    println!("Created user '{}' with role {:?}", username, role);
    Ok(())
}

/// Look up a user by id or username via a linear scan of `list_users()`.
// ponytail: CredentialsStore has no by-username lookup (§A only added
// id-keyed set_role/count_by_role); user counts on an offline admin CLI are
// small, so an O(n) scan here isn't worth a new store method.
async fn find_user(store: &CredentialsStore, id_or_username: &str) -> Result<User> {
    store
        .list_users()
        .await
        .into_iter()
        .find(|u| u.id == id_or_username || u.username == id_or_username)
        .ok_or_else(|| anyhow::anyhow!("no user found with id or username '{}'", id_or_username))
}

async fn set_role_by_lookup(auth_store_dir: &Path, id_or_username: &str, role: Role) -> Result<()> {
    let store = CredentialsStore::load_or_new(auth_store_dir)?;
    let target = find_user(&store, id_or_username).await?;
    if target.role == Role::Admin && role != Role::Admin {
        let admin_count = store.count_by_role(Role::Admin).await;
        if admin_count <= 1 {
            anyhow::bail!(
                "refusing to demote '{}': they are the only remaining Admin",
                target.username
            );
        }
    }
    store.set_role(&target.id, role).await?;
    println!("'{}' is now {:?}.", target.username, role);
    Ok(())
}

/// TCP-probes `config`'s bind address with a short timeout. Deliberately not
/// a pid/lock file: that invents new runtime lifecycle state and a
/// stale-lock failure class for a check a two-line connect covers.
fn probe_server_running(config: &ServerConfig) -> bool {
    let addr = config.bind_addr();
    match addr.to_socket_addrs().ok().and_then(|mut a| a.next()) {
        Some(sock_addr) => {
            std::net::TcpStream::connect_timeout(&sock_addr, Duration::from_millis(300)).is_ok()
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wizard_config_roundtrips_through_load() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("mediagit-server.toml");

        let mut config = ServerConfig {
            host: "127.0.0.1".to_string(),
            port: 4000,
            repos_dir: tmp.path().join("repos"),
            ..ServerConfig::default()
        };
        config.enable_auth = true;
        config.jwt_secret = Some(generate_jwt_secret());
        config.allow_open_registration = false;
        config.enable_rate_limiting = true;

        write_config_file(&config_path, &config).unwrap();

        let reloaded = ServerConfig::load(config_path.to_str().unwrap()).unwrap();
        assert_eq!(reloaded.host, config.host);
        assert_eq!(reloaded.port, config.port);
        assert_eq!(reloaded.repos_dir, config.repos_dir);
        assert!(reloaded.enable_auth);
        assert_eq!(reloaded.jwt_secret, config.jwt_secret);
        assert!(!reloaded.allow_open_registration);
        assert!(reloaded.enable_rate_limiting);
    }

    #[test]
    fn test_generate_jwt_secret_is_32_bytes_hex() {
        let secret = generate_jwt_secret();
        assert_eq!(secret.len(), 64);
        assert!(secret.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(secret, generate_jwt_secret());
    }

    #[test]
    fn test_probe_server_running_true_while_listening() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let config = ServerConfig {
            host: "127.0.0.1".to_string(),
            port,
            ..Default::default()
        };
        assert!(probe_server_running(&config));
    }

    #[test]
    fn test_probe_server_running_false_after_listener_dropped() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let config = ServerConfig {
            host: "127.0.0.1".to_string(),
            port,
            ..Default::default()
        };
        assert!(!probe_server_running(&config));
    }

    #[test]
    fn test_role_arg_conversion() {
        assert_eq!(Role::from(RoleArg::Read), Role::Read);
        assert_eq!(Role::from(RoleArg::Write), Role::Write);
        assert_eq!(Role::from(RoleArg::Admin), Role::Admin);
    }
}

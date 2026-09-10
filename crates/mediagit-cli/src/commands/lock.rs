// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Manage server-enforced file locks (B4/B5).
//!
//! Thin CLI wrapper over `ProtocolClient::{create_lock,list_locks,delete_lock}`
//! (see `crates/mediagit-protocol/src/client/locks.rs`), which itself mirrors
//! the server's `crates/mediagit-server/src/handlers/locks.rs` HTTP surface.

use super::super::repo::find_repo_root;
use crate::output;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use mediagit_protocol::ProtocolClient;

/// Manage server-enforced file locks
#[derive(Parser, Debug)]
pub struct LockCmd {
    #[command(subcommand)]
    pub subcommand: LockSubcommand,
}

#[derive(Subcommand, Debug)]
pub enum LockSubcommand {
    /// Acquire a lock on a file
    Create(CreateOpts),

    /// Release a lock
    Unlock(UnlockOpts),

    /// List active locks
    #[command(alias = "ls")]
    List(ListOpts),
}

/// Acquire a lock on a file
#[derive(Parser, Debug)]
pub struct CreateOpts {
    /// Repo-relative path to lock
    #[arg(value_name = "PATH")]
    pub path: String,

    /// Identity to attribute the lock to. Required by no-auth servers;
    /// authenticated servers derive the owner from the credential and
    /// ignore this. Defaults to MEDIAGIT_AUTHOR_NAME, then config.toml
    /// [author].name, then $USER.
    #[arg(long)]
    pub owner: Option<String>,

    /// Remote to talk to (defaults to origin)
    #[arg(long, value_name = "REMOTE")]
    pub remote: Option<String>,
}

/// Release a lock
#[derive(Parser, Debug)]
pub struct UnlockOpts {
    /// Repo-relative path of the lock to release (looked up via `lock list`
    /// since the server deletes by lock id, not path)
    #[arg(value_name = "PATH")]
    pub path: Option<String>,

    /// Lock id to release directly, instead of resolving by path
    #[arg(long = "id", value_name = "LOCK_ID")]
    pub lock_id: Option<String>,

    /// Force-release someone else's lock (requires repo:admin; required on
    /// no-auth servers, which have no owner identity to match against)
    #[arg(long)]
    pub force: bool,

    /// Remote to talk to (defaults to origin)
    #[arg(long, value_name = "REMOTE")]
    pub remote: Option<String>,
}

/// List active locks
#[derive(Parser, Debug)]
pub struct ListOpts {
    /// Output as JSON
    #[arg(long)]
    pub json: bool,

    /// Remote to talk to (defaults to origin)
    #[arg(long, value_name = "REMOTE")]
    pub remote: Option<String>,
}

impl LockCmd {
    pub async fn execute(&self) -> Result<()> {
        match &self.subcommand {
            LockSubcommand::Create(opts) => self.create(opts).await,
            LockSubcommand::Unlock(opts) => self.unlock(opts).await,
            LockSubcommand::List(opts) => self.list(opts).await,
        }
    }

    /// Build a `ProtocolClient` for `remote_opt` (defaults to "origin"),
    /// following the same remote-URL + credential resolution as
    /// push/pull/fetch. Also returns the config, resolved remote name, repo
    /// root, and the credentials used (with their [`crate::repo::CredentialSource`])
    /// so callers can write them through to the keychain
    /// (`crate::repo::remember_credentials`) after their first request
    /// succeeds, or invalidate-and-retry on a 401 (I11).
    #[allow(clippy::type_complexity)]
    async fn build_client(
        &self,
        remote_opt: &Option<String>,
    ) -> Result<(
        ProtocolClient,
        mediagit_config::Config,
        std::path::PathBuf,
        String,
        mediagit_protocol::Credentials,
        crate::repo::CredentialSource,
    )> {
        let repo_root = find_repo_root()?;
        let config = mediagit_config::Config::load(&repo_root).await?;
        let remote = remote_opt.as_deref().unwrap_or("origin").to_string();
        let remote_url = config
            .resolve_remote_url(&remote)
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        let (credentials, cred_source) =
            crate::repo::resolve_credentials_tiered(&repo_root, &config, &remote);
        let client = ProtocolClient::new(remote_url).with_credentials(credentials.clone());
        Ok((client, config, repo_root, remote, credentials, cred_source))
    }

    /// On a 401 from `err` (whose credential came from `cred_source`),
    /// invalidate the keychain entry and rebuild a fresh client + resolved
    /// credentials for one retry. Shared by every `lock` subcommand so the
    /// invalidate-and-retry logic lives in one place (I11).
    fn refresh_client_on_unauthorized(
        &self,
        config: &mediagit_config::Config,
        repo_root: &std::path::Path,
        remote: &str,
        cred_source: crate::repo::CredentialSource,
        err: &anyhow::Error,
    ) -> Option<(ProtocolClient, mediagit_protocol::Credentials)> {
        if !crate::repo::invalidate_on_unauthorized(config, remote, cred_source, err) {
            return None;
        }
        let remote_url = config.resolve_remote_url(remote).ok()?;
        let credentials = crate::repo::resolve_credentials(repo_root, config, remote);
        let client = ProtocolClient::new(remote_url).with_credentials(credentials.clone());
        Some((client, credentials))
    }

    /// Resolve the owner identity for a new lock: `--owner` >
    /// `MEDIAGIT_AUTHOR_NAME` > `config.toml [author].name` > `$USER`.
    async fn resolve_owner(&self, explicit: Option<String>) -> Result<String> {
        if let Some(owner) = explicit {
            return Ok(owner);
        }
        if let Ok(name) = std::env::var("MEDIAGIT_AUTHOR_NAME")
            && !name.trim().is_empty()
        {
            return Ok(name);
        }
        let repo_root = find_repo_root()?;
        let config = mediagit_config::Config::load(&repo_root).await?;
        if let Some(name) = config.author.name.clone() {
            return Ok(name);
        }
        Ok(std::env::var("USER").unwrap_or_else(|_| "unknown".to_string()))
    }

    async fn create(&self, opts: &CreateOpts) -> Result<()> {
        let (mut client, config, repo_root, remote, mut credentials, cred_source) =
            self.build_client(&opts.remote).await?;
        let owner = self.resolve_owner(opts.owner.clone()).await?;

        // First (and only, for this subcommand) authenticated call — a
        // cached keychain credential may have expired; on a 401, invalidate
        // it and retry once with the next tier (I11).
        let info = match client.create_lock(&opts.path, Some(owner.clone())).await {
            Ok(i) => i,
            Err(e) => match self.refresh_client_on_unauthorized(
                &config,
                &repo_root,
                &remote,
                cred_source,
                &e,
            ) {
                Some((new_client, new_creds)) => {
                    client = new_client;
                    credentials = new_creds;
                    client.create_lock(&opts.path, Some(owner)).await?
                }
                None => return Err(e),
            },
        };
        crate::repo::remember_credentials(&config, &remote, &credentials);
        output::success(&format!(
            "Locked '{}' as {} (id {})",
            info.path, info.owner, info.lock_id
        ));
        Ok(())
    }

    async fn unlock(&self, opts: &UnlockOpts) -> Result<()> {
        if opts.path.is_none() && opts.lock_id.is_none() {
            anyhow::bail!("mediagit lock unlock requires a PATH or --id <LOCK_ID>");
        }

        let (mut client, config, repo_root, remote, mut credentials, cred_source) =
            self.build_client(&opts.remote).await?;

        let lock_id = if let Some(id) = &opts.lock_id {
            id.clone()
        } else {
            let path = opts.path.as_ref().unwrap();
            // First authenticated call of this command — see `create` above.
            let locks = match client.list_locks().await {
                Ok(l) => l,
                Err(e) => match self.refresh_client_on_unauthorized(
                    &config,
                    &repo_root,
                    &remote,
                    cred_source,
                    &e,
                ) {
                    Some((new_client, new_creds)) => {
                        client = new_client;
                        credentials = new_creds;
                        client.list_locks().await?
                    }
                    None => return Err(e),
                },
            };
            crate::repo::remember_credentials(&config, &remote, &credentials);
            locks
                .into_iter()
                .find(|l| &l.path == path)
                .map(|l| l.lock_id)
                .with_context(|| format!("No active lock found for '{}'", path))?
        };

        match client.delete_lock(&lock_id, opts.force).await {
            Ok(()) => {}
            Err(e) => match self.refresh_client_on_unauthorized(
                &config,
                &repo_root,
                &remote,
                cred_source,
                &e,
            ) {
                Some((new_client, new_creds)) => {
                    client = new_client;
                    credentials = new_creds;
                    client.delete_lock(&lock_id, opts.force).await?
                }
                None => return Err(e),
            },
        }
        crate::repo::remember_credentials(&config, &remote, &credentials);
        output::success(&format!("Unlocked {}", lock_id));
        Ok(())
    }

    async fn list(&self, opts: &ListOpts) -> Result<()> {
        let (mut client, config, repo_root, remote, mut credentials, cred_source) =
            self.build_client(&opts.remote).await?;
        // First authenticated call of this command — see `create` above.
        let locks = match client.list_locks().await {
            Ok(l) => l,
            Err(e) => match self.refresh_client_on_unauthorized(
                &config,
                &repo_root,
                &remote,
                cred_source,
                &e,
            ) {
                Some((new_client, new_creds)) => {
                    client = new_client;
                    credentials = new_creds;
                    client.list_locks().await?
                }
                None => return Err(e),
            },
        };
        crate::repo::remember_credentials(&config, &remote, &credentials);

        if opts.json {
            let json: Vec<_> = locks
                .iter()
                .map(|l| {
                    serde_json::json!({
                        "lock_id": l.lock_id,
                        "path": l.path,
                        "owner": l.owner,
                        "created_at": l.created_at,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&json)?);
            return Ok(());
        }

        if locks.is_empty() {
            output::info("No active locks");
            return Ok(());
        }
        for l in &locks {
            println!("{}\t{}\t{}\t{}", l.path, l.owner, l.lock_id, l.created_at);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn resolve_owner_returns_explicit_owner_without_touching_repo_state() {
        // An explicit --owner short-circuits before any find_repo_root()/config
        // lookup, so this is safe to run outside a repository and in parallel
        // with other tests that change cwd.
        let cmd = LockCmd {
            subcommand: LockSubcommand::List(ListOpts {
                json: false,
                remote: None,
            }),
        };
        let owner = cmd.resolve_owner(Some("alice".to_string())).await.unwrap();
        assert_eq!(owner, "alice");
    }

    #[tokio::test]
    async fn unlock_requires_path_or_id() {
        let cmd = LockCmd {
            subcommand: LockSubcommand::List(ListOpts {
                json: false,
                remote: None,
            }),
        };
        let opts = UnlockOpts {
            path: None,
            lock_id: None,
            force: false,
            remote: None,
        };
        let err = cmd.unlock(&opts).await.unwrap_err();
        assert!(err.to_string().contains("requires a PATH or --id"));
    }
}

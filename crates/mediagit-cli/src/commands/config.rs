// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Read and write repository configuration.
//!
//! Exists because `commit` now refuses an unconfigured author identity rather
//! than recording `Unknown <unknown@localhost>` — telling a user to hand-edit
//! TOML to get past that is not an acceptable answer. `init` and `commit` help
//! text also referenced a `mediagit-config(1)` that did not exist.
//!
//! Deliberately a small, closed set of keys rather than a general TOML editor:
//! an unrecognised key is a typo, and silently accepting `auther.name` would
//! reproduce exactly the "looks configured, isn't" failure this is here to fix.

use super::super::repo::find_repo_root;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use console::style;
use mediagit_config::Config;

/// Get and set repository configuration
#[derive(Parser, Debug)]
#[command(after_help = "EXAMPLES:
    # Set the identity commits are recorded under
    mediagit config set author.name \"Your Name\"
    mediagit config set author.email you@example.com

    # Read one value, or list everything settable
    mediagit config get author.email
    mediagit config list

    # Remove a setting
    mediagit config unset performance.upload_concurrency

SEE ALSO:
    mediagit-init(1), mediagit-commit(1), mediagit-remote(1)")]
pub struct ConfigCmd {
    #[command(subcommand)]
    pub subcommand: ConfigSubcommand,
}

#[derive(Subcommand, Debug)]
pub enum ConfigSubcommand {
    /// Print a single value
    Get(GetOpts),
    /// Set a value
    Set(SetOpts),
    /// Remove a value, restoring the default
    Unset(UnsetOpts),
    /// List every settable key and its current value
    List,
}

#[derive(Parser, Debug)]
pub struct GetOpts {
    /// Key, e.g. author.email
    #[arg(value_name = "KEY")]
    pub key: String,
}

#[derive(Parser, Debug)]
pub struct SetOpts {
    /// Key, e.g. author.email
    #[arg(value_name = "KEY")]
    pub key: String,
    /// Value to store
    #[arg(value_name = "VALUE")]
    pub value: String,
}

#[derive(Parser, Debug)]
pub struct UnsetOpts {
    /// Key, e.g. performance.upload_concurrency
    #[arg(value_name = "KEY")]
    pub key: String,
}

/// Every key this command understands.
///
/// Listed once so `list`, `get`, `set`, `unset` and the unknown-key error can
/// never disagree about what is supported.
const KEYS: &[&str] = &[
    "author.name",
    "author.email",
    "performance.upload_concurrency",
    "performance.download_concurrency",
];

impl ConfigCmd {
    pub async fn execute(&self) -> Result<()> {
        let repo_root = find_repo_root()?;
        let mut config = Config::load(&repo_root).await?;

        match &self.subcommand {
            ConfigSubcommand::List => {
                for key in KEYS {
                    let shown = read_key(&config, key)
                        .unwrap_or_default()
                        .unwrap_or_else(|| "(unset)".to_string());
                    println!("{:<34} {}", key, shown);
                }
                Ok(())
            }

            ConfigSubcommand::Get(opts) => {
                match read_key(&config, &opts.key)? {
                    Some(value) => {
                        println!("{}", value);
                        Ok(())
                    }
                    // Exit non-zero so `mediagit config get x || setup` works
                    // in a script; printing nothing and succeeding would make
                    // "unset" indistinguishable from "empty".
                    None => anyhow::bail!("{} is not set", opts.key),
                }
            }

            ConfigSubcommand::Set(opts) => {
                write_key(&mut config, &opts.key, Some(opts.value.trim()))?;
                config
                    .save(&repo_root)
                    .with_context(|| format!("Failed to save {}", opts.key))?;
                println!(
                    "{} {} = {}",
                    style("✓").green().bold(),
                    opts.key,
                    opts.value.trim()
                );
                Ok(())
            }

            ConfigSubcommand::Unset(opts) => {
                write_key(&mut config, &opts.key, None)?;
                config
                    .save(&repo_root)
                    .with_context(|| format!("Failed to save {}", opts.key))?;
                println!("{} {} unset", style("✓").green().bold(), opts.key);
                Ok(())
            }
        }
    }
}

fn unknown_key(key: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "unknown config key {key:?}. Supported keys:\n{}",
        KEYS.iter()
            .map(|k| format!("  {k}"))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

fn read_key(config: &Config, key: &str) -> Result<Option<String>> {
    Ok(match key {
        "author.name" => config.author.name.clone(),
        "author.email" => config.author.email.clone(),
        "performance.upload_concurrency" => {
            config.performance.upload_concurrency.map(|n| n.to_string())
        }
        "performance.download_concurrency" => config
            .performance
            .download_concurrency
            .map(|n| n.to_string()),
        _ => return Err(unknown_key(key)),
    })
}

/// `None` clears the key.
///
/// Values are validated here rather than at use time: a config file is written
/// once and read for months, so an empty name or a concurrency of zero should
/// fail while the user is looking at the command that caused it.
fn write_key(config: &mut Config, key: &str, value: Option<&str>) -> Result<()> {
    match key {
        "author.name" => {
            config.author.name = match value {
                Some("") => anyhow::bail!("author.name cannot be empty"),
                Some(v) => Some(v.to_string()),
                None => None,
            };
        }
        "author.email" => {
            config.author.email = match value {
                Some(v) => Some(validate_email(v)?.to_string()),
                None => None,
            };
        }
        "performance.upload_concurrency" => {
            config.performance.upload_concurrency = parse_concurrency(key, value)?;
        }
        "performance.download_concurrency" => {
            config.performance.download_concurrency = parse_concurrency(key, value)?;
        }
        _ => return Err(unknown_key(key)),
    }
    Ok(())
}

/// Deliberately minimal: one `@`, something either side, no whitespace.
///
/// Not RFC 5322 — the goal is to catch a shell mangling or a swapped
/// name/email argument, not to adjudicate exotic addresses.
fn validate_email(value: &str) -> Result<&str> {
    let ok = match value.split_once('@') {
        Some((local, domain)) => !local.is_empty() && !domain.is_empty() && domain.contains('.'),
        None => false,
    };
    if !ok || value.split_whitespace().count() != 1 {
        anyhow::bail!(
            "author.email must look like an address (e.g. you@example.com); got {value:?}"
        );
    }
    Ok(value)
}

fn parse_concurrency(key: &str, value: Option<&str>) -> Result<Option<usize>> {
    let Some(raw) = value else { return Ok(None) };
    let n: usize = raw
        .parse()
        .with_context(|| format!("{key} must be a whole number; got {raw:?}"))?;
    if n == 0 {
        anyhow::bail!("{key} must be at least 1; 0 would stall every transfer");
    }
    Ok(Some(n))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank() -> Config {
        Config::default()
    }

    #[test]
    fn set_then_read_round_trips() {
        let mut c = blank();
        write_key(&mut c, "author.name", Some("Ada Lovelace")).unwrap();
        write_key(&mut c, "author.email", Some("ada@example.com")).unwrap();
        assert_eq!(
            read_key(&c, "author.name").unwrap().as_deref(),
            Some("Ada Lovelace")
        );
        assert_eq!(
            read_key(&c, "author.email").unwrap().as_deref(),
            Some("ada@example.com")
        );
    }

    #[test]
    fn unset_clears_the_value() {
        let mut c = blank();
        write_key(&mut c, "author.name", Some("Ada")).unwrap();
        write_key(&mut c, "author.name", None).unwrap();
        assert_eq!(read_key(&c, "author.name").unwrap(), None);
    }

    /// A typo'd key must be rejected. Silently accepting `auther.name` would
    /// leave the user believing they had configured an identity when they had
    /// not — the exact failure `commit`'s new refusal exists to prevent.
    #[test]
    fn unknown_keys_are_rejected_and_list_what_is_supported() {
        let mut c = blank();
        let err = write_key(&mut c, "auther.name", Some("Ada")).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("unknown config key"), "{msg}");
        assert!(msg.contains("author.name"), "must list valid keys: {msg}");

        assert!(read_key(&c, "user.email").is_err());
    }

    #[test]
    fn obviously_wrong_email_is_rejected() {
        let mut c = blank();
        for bad in ["ada", "ada@", "@example.com", "ada@localhost", "a b@c.com"] {
            assert!(
                write_key(&mut c, "author.email", Some(bad)).is_err(),
                "{bad:?} should be rejected"
            );
        }
        assert!(write_key(&mut c, "author.email", Some("ada@example.co.uk")).is_ok());
    }

    #[test]
    fn empty_name_and_zero_concurrency_are_rejected() {
        let mut c = blank();
        assert!(write_key(&mut c, "author.name", Some("")).is_err());
        assert!(write_key(&mut c, "performance.upload_concurrency", Some("0")).is_err());
        assert!(write_key(&mut c, "performance.upload_concurrency", Some("x")).is_err());
        assert!(write_key(&mut c, "performance.upload_concurrency", Some("8")).is_ok());
    }

    /// `KEYS` drives `list`, `get`, `set` and the error message, so every entry
    /// must actually be readable and writable — otherwise `list` advertises a
    /// key that `set` rejects.
    #[test]
    fn every_advertised_key_is_readable_and_writable() {
        let mut c = blank();
        for key in KEYS {
            read_key(&c, key).unwrap_or_else(|e| panic!("{key} not readable: {e}"));
            write_key(&mut c, key, None).unwrap_or_else(|e| panic!("{key} not writable: {e}"));
        }
    }
}

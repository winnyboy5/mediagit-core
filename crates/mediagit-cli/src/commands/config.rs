// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Read and write repository configuration.
//!
//! Exists because `commit` refuses an unconfigured author identity rather than
//! recording `Unknown <unknown@localhost>`, and so the GUI can edit the real
//! config instead of a hand-picked four keys.
//!
//! Every settable key is one row of [`SCHEMA`]. `get`, `set`, `unset`, `list`
//! and `schema` all read that table, so they cannot disagree about what is
//! supported, and a unit test fails when `mediagit_config::Config` grows a field
//! that has neither a row nor an explicit exclusion.
//!
//! Edits go through `toml_edit`, never through `toml::Value`: it keeps comments
//! and unknown keys, and `Value` would overflow `cdc_seed` (a `u64`) into `i64`.
//! The edited text is parsed into the real `Config` (`deny_unknown_fields`, then
//! `validate()`) before it replaces the file, so a bad edit leaves it untouched.

use super::super::repo::find_repo_root;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use console::style;
use mediagit_config::{Config, ConfigFormat, ConfigLoader};
use serde::Serialize;
use std::path::Path;
use toml_edit::{DocumentMut, Item, Table};

use crate::output;

/// `--machine` data for `config get|set|unset|list`. `author.name` and
/// `author.email` are the commit identity (compare with lock owners).
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../../ui/src/api/generated/"))]
pub struct ConfigReport {
    /// `get`, `set`, `unset` or `list`.
    pub action: String,
    pub entries: Vec<ConfigEntry>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../../ui/src/api/generated/"))]
pub struct ConfigEntry {
    pub key: String,
    /// `None` when unset. A secret key reports `"set"` here, never its value.
    pub value: Option<String>,
    pub secret: bool,
}

/// `--machine` data for `config schema`.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../../ui/src/api/generated/"))]
pub struct ConfigSchemaReport {
    pub keys: Vec<ConfigKeySchema>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../../ui/src/api/generated/"))]
pub struct ConfigKeySchema {
    /// Dotted path. `<name>` stands for any one name, e.g. `remotes.<name>.url`.
    pub key: String,
    /// `string`, `int`, `bool`, `enum` or `path`.
    #[serde(rename = "type")]
    pub ty: String,
    /// What applies when the key is unset; `None` when there is no fixed value.
    pub default: Option<String>,
    pub doc: String,
    /// `enum` keys: the allowed values.
    pub enum_values: Vec<String>,
    /// Never printed by `get`/`list`; set via `--secret-stdin` under `--machine`.
    pub secret: bool,
    #[cfg_attr(test, ts(type = "number | null"))]
    pub min: Option<i64>,
    #[cfg_attr(test, ts(type = "number | null"))]
    pub max: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Ty {
    String,
    Int,
    Bool,
    // No enum-valued key exists yet; adding one is a table row.
    #[allow(dead_code)]
    Enum,
    Path,
}

impl Ty {
    fn name(self) -> &'static str {
        match self {
            Ty::String => "string",
            Ty::Int => "int",
            Ty::Bool => "bool",
            Ty::Enum => "enum",
            Ty::Path => "path",
        }
    }
}

struct KeySpec {
    key: &'static str,
    ty: Ty,
    default: Option<&'static str>,
    doc: &'static str,
    values: &'static [&'static str],
    secret: bool,
    min: Option<i64>,
    max: Option<i64>,
    check: Option<fn(&str) -> Result<()>>,
}

const fn spec(key: &'static str, ty: Ty, doc: &'static str) -> KeySpec {
    KeySpec {
        key,
        ty,
        default: None,
        doc,
        values: &[],
        secret: false,
        min: None,
        max: None,
        check: None,
    }
}

/// Every key `config` can read and write. One row each.
///
/// Left out on purpose (see `EXCLUDED_TOP_LEVEL` for the checked list): the
/// storage identity (`backend`, `bucket`, `base_path`, `prefix`, ...) and the
/// repo identity fields, because changing them orphans stored objects; and the
/// tables other commands own (`branches`, `protected_branches`).
static SCHEMA: &[KeySpec] = &[
    KeySpec {
        check: Some(validate_email),
        ..spec(
            "author.email",
            Ty::String,
            "Email recorded on commits you create.",
        )
    },
    spec(
        "author.name",
        Ty::String,
        "Display name recorded on commits you create.",
    ),
    KeySpec {
        min: Some(1),
        ..spec(
            "performance.upload_concurrency",
            Ty::Int,
            "Parallel chunk uploads on push. Unset: MEDIAGIT_UPLOAD_CONCURRENCY, then 32.",
        )
    },
    KeySpec {
        min: Some(1),
        ..spec(
            "performance.download_concurrency",
            Ty::Int,
            "Parallel chunk downloads on pull/fetch. Unset: MEDIAGIT_DOWNLOAD_CONCURRENCY, then 24.",
        )
    },
    KeySpec {
        min: Some(1),
        ..spec(
            "performance.pack_workers",
            Ty::Int,
            "Server-side pack-write workers. Unset: MEDIAGIT_PACK_WORKERS, then 8.",
        )
    },
    KeySpec {
        default: Some("true"),
        ..spec(
            "storage.create_dirs",
            Ty::Bool,
            "Filesystem backend: create missing directories.",
        )
    },
    KeySpec {
        default: Some("false"),
        ..spec(
            "storage.sync",
            Ty::Bool,
            "Filesystem backend: sync writes to disk (slower, safer).",
        )
    },
    KeySpec {
        default: Some("0644"),
        check: Some(validate_octal),
        ..spec(
            "storage.file_permissions",
            Ty::String,
            "Filesystem backend: octal permission string for stored files.",
        )
    },
    spec(
        "storage.region",
        Ty::String,
        "S3 backend: region of the bucket.",
    ),
    spec(
        "storage.endpoint",
        Ty::String,
        "S3 backend: custom endpoint URL (MinIO and other S3-compatible services).",
    ),
    KeySpec {
        secret: true,
        ..spec(
            "storage.access_key_id",
            Ty::String,
            "S3 backend: access key id.",
        )
    },
    KeySpec {
        secret: true,
        ..spec(
            "storage.secret_access_key",
            Ty::String,
            "S3 backend: secret access key.",
        )
    },
    spec(
        "storage.credentials_path",
        Ty::Path,
        "GCS backend: service-account JSON file. Unset: Application Default Credentials.",
    ),
    spec(
        "remotes.<name>.url",
        Ty::String,
        "Remote server URL (http:// or https://).",
    ),
    spec(
        "remotes.<name>.fetch",
        Ty::String,
        "Fetch URL when it differs from the remote URL.",
    ),
    spec(
        "remotes.<name>.push",
        Ty::String,
        "Push URL when it differs from the remote URL.",
    ),
    KeySpec {
        secret: true,
        ..spec(
            "remotes.<name>.token",
            Ty::String,
            "Bearer token for this remote. Stored in the OS keychain, never in config.toml.",
        )
    },
    KeySpec {
        secret: true,
        ..spec(
            "remotes.<name>.api_key",
            Ty::String,
            "API key for this remote. Stored in plain text in config.toml.",
        )
    },
];

/// Top-level `Config` fields with no (or not only) `SCHEMA` rows, and why.
/// Checked by `every_config_field_has_a_schema_row_or_an_exclusion`.
#[cfg(test)]
const EXCLUDED_TOP_LEVEL: &[(&str, &str)] = &[
    (
        "branches",
        "owned by `branch` and `pull` (upstream tracking)",
    ),
    ("protected_branches", "owned by `branch protect`"),
    ("custom", "free-form user table; edit config.toml"),
    ("cdc_seed", "set once at init; changing it breaks dedup"),
    (
        "repo_namespace",
        "set once at init; changing it orphans objects",
    ),
    (
        "layout_version",
        "storage layout marker, migrated automatically",
    ),
    ("repo_id", "generated at init"),
    ("config_version", "schema version, migrated automatically"),
];

fn validate_email(value: &str) -> Result<()> {
    let ok = match value.split_once('@') {
        Some((local, domain)) => !local.is_empty() && !domain.is_empty() && domain.contains('.'),
        None => false,
    };
    if !ok || value.split_whitespace().count() != 1 {
        anyhow::bail!(
            "author.email must look like an address (e.g. you@example.com); got {value:?}"
        );
    }
    Ok(())
}

fn validate_octal(value: &str) -> Result<()> {
    if value.is_empty() || !value.chars().all(|c| ('0'..='7').contains(&c)) {
        anyhow::bail!(
            "storage.file_permissions must be an octal string like \"0644\"; got {value:?}"
        );
    }
    Ok(())
}

/// Get and set repository configuration
#[derive(Parser, Debug)]
#[command(after_help = "EXAMPLES:
    # Set the identity commits are recorded under
    mediagit config set author.name \"Your Name\"
    mediagit config set author.email you@example.com

    # Read one value, or list everything settable
    mediagit config get author.email
    mediagit config list

    # See every key, its type, default and documentation
    mediagit config schema

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
    /// Describe every settable key: type, default, documentation
    Schema,
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
    /// Value to store. For a secret key, omit it: with `--machine` it is read
    /// from stdin (`--secret-stdin`), otherwise you are prompted.
    #[arg(value_name = "VALUE")]
    pub value: Option<String>,
}

#[derive(Parser, Debug)]
pub struct UnsetOpts {
    /// Key, e.g. performance.upload_concurrency
    #[arg(value_name = "KEY")]
    pub key: String,
}

impl ConfigCmd {
    pub async fn execute(&self) -> Result<()> {
        if matches!(self.subcommand, ConfigSubcommand::Schema) {
            return schema_command();
        }
        let repo_root = find_repo_root()?;
        // A config that cannot be loaded is reported, not edited around.
        let cfg = Config::load(&repo_root).await?;
        let path = repo_root.join(".mediagit/config.toml");
        let mut doc = read_doc(&path)?;

        match &self.subcommand {
            ConfigSubcommand::Schema => unreachable!("handled above"),
            ConfigSubcommand::List => {
                let entries = list_entries(&doc);
                if output::machine_mode() {
                    return output::emit_ok(&ConfigReport {
                        action: "list".into(),
                        entries,
                    });
                }
                for e in &entries {
                    let shown = e.value.as_deref().unwrap_or("(unset)");
                    println!("{:<44} {}", e.key, shown);
                }
                Ok(())
            }

            ConfigSubcommand::Get(opts) => {
                let (spec, segs) = lookup(&opts.key)?;
                let value = shown_value(spec, read_value(&doc, &segs));
                if output::machine_mode() {
                    return output::emit_ok(&ConfigReport {
                        action: "get".into(),
                        entries: vec![ConfigEntry {
                            key: opts.key.clone(),
                            value,
                            secret: spec.secret,
                        }],
                    });
                }
                match value {
                    Some(v) => {
                        println!("{v}");
                        Ok(())
                    }
                    // Exit non-zero so `mediagit config get x || setup` works
                    // in a script; printing nothing and succeeding would make
                    // "unset" indistinguishable from "empty".
                    None => anyhow::bail!("{} is not set", opts.key),
                }
            }

            ConfigSubcommand::Set(opts) => {
                let (spec, segs) = lookup(&opts.key)?;
                let raw = value_for_set(spec, opts)?;
                let item = coerce(spec, raw.trim())?;
                store_value(&cfg, &mut doc, spec, &segs, item, raw.trim())?;
                commit_doc(&repo_root, &path, &doc)
                    .with_context(|| format!("Failed to save {}", opts.key))?;
                let shown = shown_value(spec, Some(raw.trim().to_string()));
                if output::machine_mode() {
                    return output::emit_ok(&ConfigReport {
                        action: "set".into(),
                        entries: vec![ConfigEntry {
                            key: opts.key.clone(),
                            value: shown,
                            secret: spec.secret,
                        }],
                    });
                }
                println!(
                    "{} {} = {}",
                    style("✓").green().bold(),
                    opts.key,
                    shown.unwrap_or_default()
                );
                Ok(())
            }

            ConfigSubcommand::Unset(opts) => {
                let (spec, segs) = lookup(&opts.key)?;
                clear_value(&cfg, &mut doc, spec, &segs);
                commit_doc(&repo_root, &path, &doc)
                    .with_context(|| format!("Failed to save {}", opts.key))?;
                if output::machine_mode() {
                    return output::emit_ok(&ConfigReport {
                        action: "unset".into(),
                        entries: vec![ConfigEntry {
                            key: opts.key.clone(),
                            value: None,
                            secret: spec.secret,
                        }],
                    });
                }
                println!("{} {} unset", style("✓").green().bold(), opts.key);
                Ok(())
            }
        }
    }
}

fn schema_command() -> Result<()> {
    let keys: Vec<ConfigKeySchema> = SCHEMA
        .iter()
        .map(|s| ConfigKeySchema {
            key: s.key.to_string(),
            ty: s.ty.name().to_string(),
            default: s.default.map(str::to_string),
            doc: s.doc.to_string(),
            enum_values: s.values.iter().map(|v| v.to_string()).collect(),
            secret: s.secret,
            min: s.min,
            max: s.max,
        })
        .collect();
    if output::machine_mode() {
        return output::emit_ok(&ConfigSchemaReport { keys });
    }
    for k in &keys {
        let default = k.default.as_deref().unwrap_or("-");
        let secret = if k.secret { " [secret]" } else { "" };
        println!("{:<40} {:<7} default {:<8}{}", k.key, k.ty, default, secret);
        println!("    {}", k.doc);
    }
    Ok(())
}

/// The row for `key`, and the key split into path segments.
fn lookup(key: &str) -> Result<(&'static KeySpec, Vec<String>)> {
    let segs: Vec<String> = key.split('.').map(str::to_string).collect();
    for s in SCHEMA {
        let pat: Vec<&str> = s.key.split('.').collect();
        if pat.len() == segs.len()
            && pat
                .iter()
                .zip(&segs)
                .all(|(p, g)| (*p == "<name>" && !g.is_empty()) || p == g)
        {
            return Ok((s, segs));
        }
    }
    Err(unknown_key(key))
}

fn unknown_key(key: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "unknown config key {key:?}. Supported keys:\n{}",
        SCHEMA
            .iter()
            .map(|s| format!("  {}", s.key))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

/// What the user typed, or for a secret in machine mode what arrived on stdin.
/// A secret never travels on argv under `--machine`.
fn value_for_set(spec: &KeySpec, opts: &SetOpts) -> Result<String> {
    if spec.secret {
        if output::machine_mode() {
            if opts.value.is_some() {
                return Err(output::CodedError::new(
                    "invalid_args",
                    format!(
                        "{} is a secret: pass it on stdin with --secret-stdin, not as an argument",
                        spec.key
                    ),
                )
                .into());
            }
            if let Some(line) = output::read_secret_line(spec.key)? {
                return Ok(line.to_string());
            }
            output::ensure_interactive(spec.key)?;
        }
        return match &opts.value {
            Some(v) => Ok(v.clone()),
            None => Ok(dialoguer::Password::new()
                .with_prompt(spec.key)
                .interact()?),
        };
    }
    opts.value
        .clone()
        .ok_or_else(|| anyhow::anyhow!("{} needs a VALUE", spec.key))
}

/// Type-check `raw` against the row and build the TOML value.
fn coerce(spec: &KeySpec, raw: &str) -> Result<Item> {
    let key = spec.key;
    let item = match spec.ty {
        Ty::String | Ty::Path => {
            if raw.is_empty() {
                anyhow::bail!("{key} cannot be empty");
            }
            toml_edit::value(raw)
        }
        Ty::Enum => {
            if !spec.values.contains(&raw) {
                anyhow::bail!(
                    "{key} must be one of {}; got {raw:?}",
                    spec.values.join(", ")
                );
            }
            toml_edit::value(raw)
        }
        Ty::Bool => match raw {
            "true" => toml_edit::value(true),
            "false" => toml_edit::value(false),
            _ => anyhow::bail!("{key} must be true or false; got {raw:?}"),
        },
        Ty::Int => {
            let n: i64 = raw
                .parse()
                .with_context(|| format!("{key} must be a whole number; got {raw:?}"))?;
            if let Some(min) = spec.min
                && n < min
            {
                anyhow::bail!("{key} must be at least {min}; got {n}");
            }
            if let Some(max) = spec.max
                && n > max
            {
                anyhow::bail!("{key} must be at most {max}; got {n}");
            }
            toml_edit::value(n)
        }
    };
    if let Some(check) = spec.check {
        check(raw)?;
    }
    Ok(item)
}

/// A secret is reported as `"set"`; everything else as its text.
fn shown_value(spec: &KeySpec, value: Option<String>) -> Option<String> {
    if spec.secret {
        value.map(|_| "set".to_string())
    } else {
        value
    }
}

fn read_doc(path: &Path) -> Result<DocumentMut> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    mask_big_ints(&text)
        .parse::<DocumentMut>()
        .with_context(|| format!("{} is not valid TOML", path.display()))
}

/// `toml_edit` integers are `i64`, but `cdc_seed` is a random `u64`: about half
/// of all repos hold one above `i64::MAX`, which `toml_edit` refuses to parse.
/// Such a value (always a bare `key = digits` line) is carried through the edit
/// as a marked string and written back as the same digits by [`unmask_big_ints`].
const BIG_INT: &str = "mediagit-u64:";

fn mask_big_ints(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let masked = line.split_once('=').and_then(|(k, rest)| {
            let digits = rest.trim_start();
            let end = digits
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(digits.len());
            let (num, tail) = digits.split_at(end);
            let tail_ok = tail.trim_start().is_empty() || tail.trim_start().starts_with('#');
            (!k.trim().is_empty()
                && tail_ok
                && num.parse::<i64>().is_err()
                && num.parse::<u64>().is_ok())
            .then(|| format!("{k}= \"{BIG_INT}{num}\"{tail}"))
        });
        out.push_str(masked.as_deref().unwrap_or(line));
    }
    out
}

fn unmask_big_ints(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find(&format!("\"{BIG_INT}")) {
        let after = &rest[i + 1 + BIG_INT.len()..];
        let end = after
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(after.len());
        if end > 0 && after[end..].starts_with('"') {
            out.push_str(&rest[..i]);
            out.push_str(&after[..end]);
            rest = &after[end + 1..];
        } else {
            out.push_str(&rest[..=i]);
            rest = &rest[i + 1..];
        }
    }
    out.push_str(rest);
    out
}

fn read_value(doc: &DocumentMut, segs: &[String]) -> Option<String> {
    let mut cur: &Item = doc.as_item();
    for s in segs {
        cur = cur.get(s)?;
    }
    let v = cur.as_value()?;
    Some(match v.as_str() {
        Some(s) => s.strip_prefix(BIG_INT).unwrap_or(s).to_string(),
        None => v.to_string().trim().to_string(),
    })
}

/// `remotes.<name>.token` -> `<name>`. That one key lives in the OS keychain.
fn remote_token_name(segs: &[String]) -> Option<&str> {
    match segs {
        [r, name, t] if r == "remotes" && t == "token" => Some(name),
        _ => None,
    }
}

/// The one place a `config set` value is written. `remotes.<name>.token` goes
/// to the OS keychain (an error if there is none), never into `config.toml`.
fn store_value(
    cfg: &Config,
    doc: &mut DocumentMut,
    _spec: &KeySpec,
    segs: &[String],
    item: Item,
    raw: &str,
) -> Result<()> {
    if let Some(name) = remote_token_name(segs) {
        crate::repo::store_remote_token(cfg, name, raw)?;
        remove_item(doc, segs); // drop any stale plaintext copy
        return Ok(());
    }
    set_item(doc, segs, item)
}

/// The one place a `config unset` is applied; see [`store_value`].
fn clear_value(cfg: &Config, doc: &mut DocumentMut, _spec: &KeySpec, segs: &[String]) {
    if let Some(name) = remote_token_name(segs) {
        crate::repo::forget_credentials(cfg, name);
    }
    remove_item(doc, segs);
}

fn set_item(doc: &mut DocumentMut, segs: &[String], item: Item) -> Result<()> {
    let (last, parents) = segs.split_last().expect("a key has at least one segment");
    let mut table: &mut Table = doc.as_table_mut();
    for seg in parents {
        if !table.contains_key(seg) {
            let mut fresh = Table::new();
            fresh.set_implicit(true);
            table.insert(seg, Item::Table(fresh));
        }
        table = table
            .get_mut(seg)
            .and_then(Item::as_table_mut)
            .ok_or_else(|| anyhow::anyhow!("{seg} is not a table in config.toml"))?;
    }
    table.insert(last, item);
    Ok(())
}

fn remove_item(doc: &mut DocumentMut, segs: &[String]) {
    let (last, parents) = segs.split_last().expect("a key has at least one segment");
    let mut table: &mut Table = doc.as_table_mut();
    for seg in parents {
        match table.get_mut(seg).and_then(Item::as_table_mut) {
            Some(t) => table = t,
            None => return,
        }
    }
    table.remove(last);
}

/// Every concrete key with its current value; `<name>` rows expand to the
/// remotes that exist.
fn list_entries(doc: &DocumentMut) -> Vec<ConfigEntry> {
    let remotes: Vec<String> = doc
        .get("remotes")
        .and_then(Item::as_table)
        .map(|t| t.iter().map(|(k, _)| k.to_string()).collect())
        .unwrap_or_default();
    let mut out = Vec::new();
    for s in SCHEMA {
        let keys: Vec<String> = if s.key.contains("<name>") {
            remotes.iter().map(|r| s.key.replace("<name>", r)).collect()
        } else {
            vec![s.key.to_string()]
        };
        for key in keys {
            let segs: Vec<String> = key.split('.').map(str::to_string).collect();
            out.push(ConfigEntry {
                value: shown_value(s, read_value(doc, &segs)),
                secret: s.secret,
                key,
            });
        }
    }
    out
}

/// Validate the edited text as a real `Config`, then replace the file
/// atomically. On any error the file is untouched.
fn commit_doc(repo_root: &Path, path: &Path, doc: &DocumentMut) -> Result<()> {
    let text = unmask_big_ints(&doc.to_string());
    ConfigLoader::new()
        .load_from_string(&text, ConfigFormat::Toml)
        .map_err(|e| anyhow::anyhow!("that change makes config.toml invalid: {e}"))?;
    let dir = repo_root.join(".mediagit");
    std::fs::create_dir_all(&dir)?;
    let tmp = path.with_file_name(format!(
        "config.toml.{}.{:?}.tmp",
        std::process::id(),
        std::thread::current().id()
    ));
    {
        use std::io::Write;
        let mut oo = std::fs::OpenOptions::new();
        oo.write(true).create(true).truncate(true);
        // Owner-only from creation: the file can hold other secrets.
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut oo, 0o600);
        let mut f = oo.open(&tmp)?;
        #[cfg(not(unix))]
        mediagit_security::fsperm::restrict_to_owner_or_warn(&tmp);
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn doc(s: &str) -> DocumentMut {
        mask_big_ints(s).parse().unwrap()
    }

    fn set(d: &mut DocumentMut, key: &str, raw: &str) -> Result<()> {
        let (spec, segs) = lookup(key)?;
        set_item(d, &segs, coerce(spec, raw)?)
    }

    fn get(d: &DocumentMut, key: &str) -> Option<String> {
        let (_, segs) = lookup(key).unwrap();
        read_value(d, &segs)
    }

    #[test]
    fn set_then_read_round_trips_and_unset_clears() {
        let mut d = doc("");
        set(&mut d, "author.name", "Ada Lovelace").unwrap();
        set(&mut d, "performance.upload_concurrency", "8").unwrap();
        set(&mut d, "storage.sync", "true").unwrap();
        assert_eq!(get(&d, "author.name").as_deref(), Some("Ada Lovelace"));
        assert_eq!(
            get(&d, "performance.upload_concurrency").as_deref(),
            Some("8")
        );
        assert_eq!(get(&d, "storage.sync").as_deref(), Some("true"));
        let (_, segs) = lookup("author.name").unwrap();
        remove_item(&mut d, &segs);
        assert_eq!(get(&d, "author.name"), None);
    }

    #[test]
    fn unknown_keys_are_rejected_and_list_what_is_supported() {
        let msg = lookup("auther.name").err().unwrap().to_string();
        assert!(msg.contains("unknown config key"), "{msg}");
        assert!(msg.contains("author.name"), "must list valid keys: {msg}");
        assert!(lookup("user.email").is_err());
        // The wildcard is exactly one name, and never empty.
        assert!(lookup("remotes.origin.url").is_ok());
        assert!(lookup("remotes.url").is_err());
        assert!(lookup("remotes..url").is_err());
        assert!(lookup("remotes.a.b.url").is_err());
    }

    #[test]
    fn values_are_type_checked_against_the_schema() {
        let mut d = doc("");
        for (key, bad) in [
            ("author.email", "ada"),
            ("author.email", "ada@localhost"),
            ("author.name", ""),
            ("performance.upload_concurrency", "0"),
            ("performance.upload_concurrency", "x"),
            ("performance.upload_concurrency", "-3"),
            ("storage.sync", "yes"),
            ("storage.file_permissions", "rwx"),
        ] {
            assert!(set(&mut d, key, bad).is_err(), "{key}={bad:?} should fail");
        }
        assert!(set(&mut d, "author.email", "ada@example.co.uk").is_ok());
        assert!(set(&mut d, "performance.upload_concurrency", "8").is_ok());
        assert!(set(&mut d, "storage.file_permissions", "0755").is_ok());
    }

    #[test]
    fn enum_keys_accept_only_listed_values() {
        let spec = KeySpec {
            values: &["a", "b"],
            ..spec("x.mode", Ty::Enum, "test")
        };
        assert!(coerce(&spec, "a").is_ok());
        assert!(coerce(&spec, "c").is_err());
    }

    #[test]
    fn secrets_are_never_shown() {
        let (spec, segs) = lookup("remotes.origin.token").unwrap();
        let d = doc("[remotes.origin]\nurl = \"http://h/r\"\ntoken = \"sekrit\"\n");
        let shown = shown_value(spec, read_value(&d, &segs));
        assert_eq!(shown.as_deref(), Some("set"));
        let listed = list_entries(&d);
        let t = listed
            .iter()
            .find(|e| e.key == "remotes.origin.token")
            .unwrap();
        assert_eq!(t.value.as_deref(), Some("set"));
        assert!(listed.iter().all(|e| e.value.as_deref() != Some("sekrit")));
    }

    /// The memory trap: `toml::Value` would turn this u64 into an i64 error.
    /// Editing a different key must leave it, comments and unknown keys alone.
    #[test]
    fn editing_keeps_comments_unknown_keys_and_a_u64_seed() {
        let mut d = doc(
            "# keep me\ncdc_seed = 18446744073709551615\n\n[author]\nname = \"A\" # trailing\n",
        );
        set(&mut d, "author.email", "a@b.co").unwrap();
        assert_eq!(
            read_value(&d, &["cdc_seed".to_string()]).as_deref(),
            Some("18446744073709551615")
        );
        let out = unmask_big_ints(&d.to_string());
        assert!(out.contains("# keep me"), "{out}");
        assert!(out.contains("\ncdc_seed = 18446744073709551615\n"), "{out}");
        assert!(!out.contains(BIG_INT), "{out}");
        assert!(out.contains("# trailing"), "{out}");
        assert!(out.contains("a@b.co"), "{out}");
    }

    #[test]
    fn a_new_remote_key_creates_its_table() {
        let mut d = doc("");
        set(&mut d, "remotes.origin.url", "http://h/r").unwrap();
        assert_eq!(get(&d, "remotes.origin.url").as_deref(), Some("http://h/r"));
        assert!(
            list_entries(&d)
                .iter()
                .any(|e| e.key == "remotes.origin.url" && e.value.is_some())
        );
    }

    /// Edits that would not deserialize into the real `Config` are refused and
    /// leave the file as it was.
    #[test]
    fn invalid_result_is_rejected_and_the_file_is_untouched() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        let path = root.join(".mediagit/config.toml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let before = "[author]\nname = \"A\"\n";
        std::fs::write(&path, before).unwrap();

        // A token without the remote's required url cannot become a `Config`.
        let mut d = read_doc(&path).unwrap();
        set(&mut d, "remotes.origin.token", "t").unwrap();
        assert!(commit_doc(root, &path, &d).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);

        let mut d = read_doc(&path).unwrap();
        set(&mut d, "author.name", "B").unwrap();
        commit_doc(root, &path, &d).unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains("\"B\""));
    }

    fn struct_fields(src: &str, name: &str) -> Vec<String> {
        let start = src
            .find(&format!("pub struct {name} {{"))
            .unwrap_or_else(|| panic!("struct {name} not found"));
        let body = &src[start..];
        let end = body.find("\n}\n").unwrap();
        body[..end]
            .lines()
            .filter_map(|l| l.strip_prefix("    pub "))
            .filter_map(|l| l.split(':').next())
            .map(str::to_string)
            .collect()
    }

    /// A new `Config` field must reach the GUI (a `SCHEMA` row) or be excluded
    /// with a reason; it cannot silently go missing.
    #[test]
    fn every_config_field_has_a_schema_row_or_an_exclusion() {
        let src = include_str!("../../../mediagit-config/src/schema.rs");
        let keys: BTreeSet<&str> = SCHEMA.iter().map(|s| s.key).collect();
        let excluded: BTreeSet<&str> = EXCLUDED_TOP_LEVEL.iter().map(|(k, _)| *k).collect();
        let mut missing = Vec::new();

        for field in struct_fields(src, "Config") {
            if field.starts_with("deprecated_") {
                continue;
            }
            let covered = keys.iter().any(|k| k.split('.').next() == Some(&field))
                || excluded.contains(field.as_str());
            if !covered {
                missing.push(format!("Config.{field}"));
            }
        }
        for (strukt, prefix) in [
            ("AuthorConfig", "author."),
            ("PerformanceConfig", "performance."),
            ("RemoteConfig", "remotes.<name>."),
        ] {
            for field in struct_fields(src, strukt) {
                if field.starts_with("deprecated_") {
                    continue;
                }
                if !keys.contains(format!("{prefix}{field}").as_str()) {
                    missing.push(format!("{strukt}.{field}"));
                }
            }
        }
        // And nothing excluded may have vanished from Config.
        let fields: BTreeSet<String> = struct_fields(src, "Config").into_iter().collect();
        for (k, _) in EXCLUDED_TOP_LEVEL {
            assert!(fields.contains(*k), "stale exclusion {k}");
        }
        assert!(
            missing.is_empty(),
            "add a SCHEMA row (or an EXCLUDED_TOP_LEVEL reason) for: {missing:?}"
        );
    }

    #[test]
    fn every_schema_key_is_unique_and_resolves() {
        let mut seen = BTreeSet::new();
        for s in SCHEMA {
            assert!(seen.insert(s.key), "duplicate {}", s.key);
            let concrete = s.key.replace("<name>", "origin");
            assert_eq!(lookup(&concrete).unwrap().0.key, s.key);
        }
    }

    /// With no keychain, setting a remote token fails and writes nothing to
    /// config.toml; unset still drops any stale plaintext copy.
    #[test]
    #[allow(unsafe_code)]
    fn remote_token_goes_to_keychain_never_the_doc() {
        let _g = crate::commands::utils::test_support::REPO_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var("MEDIAGIT_NO_KEYRING", "1") };
        let cfg = Config::default();
        let mut d = doc("[remotes.origin]\nurl = \"http://h/r\"\ntoken = \"old\"\n");
        let (spec, segs) = lookup("remotes.origin.token").unwrap();
        let item = coerce(spec, "s3cret").unwrap();
        let r = store_value(&cfg, &mut d, spec, &segs, item, "s3cret");
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::remove_var("MEDIAGIT_NO_KEYRING") };
        assert!(r.is_err(), "token must not fall back to plaintext");
        assert!(!d.to_string().contains("s3cret"));
        clear_value(&cfg, &mut d, spec, &segs);
        assert!(!d.to_string().contains("old"));
    }

    #[cfg(unix)]
    #[test]
    fn committed_config_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(".mediagit/config.toml");
        commit_doc(tmp.path(), &path, &doc("")).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

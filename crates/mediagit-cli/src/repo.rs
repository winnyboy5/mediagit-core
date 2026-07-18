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

//! Repository utilities for MediaGit CLI
//!
//! Shared utilities for repository discovery, path handling, and storage backend creation.

use anyhow::{Context, Result};
use mediagit_storage::StorageBackend;
use mediagit_versioning::{ObjectDatabase, RefDatabase};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Normalize a file path to a repo-relative path with forward slashes.
///
/// Handles `./`, `.\`, backslash separators, and non-canonical paths by:
/// 1. Attempting `dunce::canonicalize()` + `strip_prefix(repo_root)`
/// 2. Falling back to manual normalization if canonicalization fails
///
/// # Arguments
/// * `path` - The path to normalize (absolute or relative)
/// * `repo_root` - The canonicalized repository root
///
/// # Returns
/// A repo-relative `PathBuf` with forward slashes (e.g. `dir/file.txt`)
pub fn normalize_path(path: &Path, repo_root: &Path) -> PathBuf {
    // Try canonicalize + strip_prefix first (handles symlinks, 8.3 names, etc.)
    if let Ok(abs) = dunce::canonicalize(path) {
        if let Ok(rel) = abs.strip_prefix(repo_root) {
            return PathBuf::from(rel.to_string_lossy().replace('\\', "/"));
        }
    }

    // Fallback: manual normalization
    let s = path.to_string_lossy();

    // Strip leading ./ or .\ prefix
    let stripped = s
        .strip_prefix("./")
        .or_else(|| s.strip_prefix(".\\"))
        .unwrap_or(&s);

    // Normalize separators and collect components to resolve .. etc.
    let normalized = stripped.replace('\\', "/");
    let clean: PathBuf = Path::new(&normalized).components().collect();

    // If still absolute, try strip_prefix as last resort
    if clean.is_absolute() {
        clean
            .strip_prefix(repo_root)
            .map(|p| PathBuf::from(p.to_string_lossy().replace('\\', "/")))
            .unwrap_or(clean)
    } else {
        PathBuf::from(clean.to_string_lossy().replace('\\', "/"))
    }
}

/// Find the root of the MediaGit repository by walking up from current directory.
///
/// # Returns
/// - `Ok(PathBuf)` - Path to repository root (directory containing `.mediagit`)
/// - `Err` - If not inside a MediaGit repository
///
/// # Example
/// ```no_run
/// use mediagit_cli::repo::find_repo_root;
///
/// let root = find_repo_root()?;
/// println!("Repository at: {}", root.display());
/// # Ok::<(), anyhow::Error>(())
/// ```
pub fn find_repo_root() -> Result<PathBuf> {
    // Honor -C flag (stored as MEDIAGIT_REPO env var by main.rs)
    if let Ok(repo_path) = std::env::var("MEDIAGIT_REPO") {
        let path = PathBuf::from(&repo_path);
        // Try as-is first
        if path.join(".mediagit").exists() {
            return Ok(path);
        }
        // Try canonicalized
        if let Ok(canonical) = dunce::canonicalize(&path) {
            if canonical.join(".mediagit").exists() {
                return Ok(canonical);
            }
        }
        // Walk up from the given path
        return find_repo_root_from(&path);
    }

    let mut current = std::env::current_dir()?;

    loop {
        if current.join(".mediagit").exists() {
            return Ok(current);
        }

        if !current.pop() {
            anyhow::bail!("Not a mediagit repository (or any parent up to mount point)");
        }
    }
}

/// Find repository root from a specific starting path.
///
/// # Arguments
/// * `start` - Starting directory to search from
///
/// # Returns
/// - `Ok(PathBuf)` - Path to repository root
/// - `Err` - If not inside a MediaGit repository
pub fn find_repo_root_from(start: &std::path::Path) -> Result<PathBuf> {
    let mut current = start.to_path_buf();

    loop {
        if current.join(".mediagit").exists() {
            return Ok(current);
        }

        if !current.pop() {
            anyhow::bail!("Not a mediagit repository (or any parent up to mount point)");
        }
    }
}

/// Resolve client credentials for talking to `remote_name` (M2 client auth,
/// OS-keychain tier added for I10).
///
/// Precedence: env `MEDIAGIT_TOKEN` / `MEDIAGIT_API_KEY` → OS keychain
/// (skipped entirely if `MEDIAGIT_NO_KEYRING` is set) → per-remote config
/// (`remotes.<name>.token` / `.api_key` in config.toml). `token` wins over
/// `api_key` within the config tier if a remote somehow has both set.
/// Threaded into `ProtocolClient::with_credentials` by every remote command
/// (fetch, pull, push, clone, download, lock).
///
/// Missing/no remote entry (e.g. `clone`, which has no repo config yet) or
/// unknown `remote_name` falls straight through to the next tier.
///
/// Callers should follow up with [`remember_credentials`] once these
/// credentials have been accepted by the server, so the next invocation can
/// skip straight to the (faster, no file/env lookup) keychain tier.
pub fn resolve_credentials(
    repo_root: &Path,
    config: &mediagit_config::Config,
    remote_name: &str,
) -> mediagit_protocol::Credentials {
    if let Some(token) = std::env::var("MEDIAGIT_TOKEN")
        .ok()
        .filter(|t| !t.trim().is_empty())
    {
        return mediagit_protocol::Credentials::Bearer(token);
    }
    if let Some(key) = std::env::var("MEDIAGIT_API_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty())
    {
        return mediagit_protocol::Credentials::ApiKey(key);
    }
    if !keyring_disabled() {
        if let Some(creds) = keyring_read(&keyring_account(config, remote_name)) {
            return creds;
        }
    }
    if let Some(remote) = config.remotes.get(remote_name) {
        if let Some(token) = remote.token.as_deref().filter(|t| !t.trim().is_empty()) {
            warn_if_config_world_readable(repo_root);
            return mediagit_protocol::Credentials::Bearer(token.to_string());
        }
        if let Some(key) = remote.api_key.as_deref().filter(|k| !k.trim().is_empty()) {
            warn_if_config_world_readable(repo_root);
            return mediagit_protocol::Credentials::ApiKey(key.to_string());
        }
    }
    mediagit_protocol::Credentials::None
}

/// True if the OS-keychain tier should be skipped entirely (opt-out knob,
/// I10) — resolution behaves exactly as it did before this feature existed.
fn keyring_disabled() -> bool {
    std::env::var_os("MEDIAGIT_NO_KEYRING").is_some()
}

/// Service name every MediaGit keychain entry is stored under.
const KEYRING_SERVICE: &str = "mediagit";

/// Keychain account key for a remote: the resolved remote URL when one is
/// configured, else the bare remote name. The URL (not just "origin") keeps
/// entries unambiguous across different repos/servers that both happen to
/// name a remote "origin".
fn keyring_account(config: &mediagit_config::Config, remote_name: &str) -> String {
    config
        .resolve_remote_url(remote_name)
        .unwrap_or_else(|_| remote_name.to_string())
}

/// Read a previously-stored credential from the OS keychain. Any failure —
/// locked/unavailable keychain service, missing entry, corrupt payload —
/// degrades silently to `None`: a broken keyring must never break a
/// push/fetch that would otherwise work off env or config.toml.
fn keyring_read(account: &str) -> Option<mediagit_protocol::Credentials> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, account).ok()?;
    let stored = entry.get_password().ok()?;
    let (kind, value) = stored.split_once(':')?;
    match kind {
        "bearer" => Some(mediagit_protocol::Credentials::Bearer(value.to_string())),
        "apikey" => Some(mediagit_protocol::Credentials::ApiKey(value.to_string())),
        _ => None,
    }
}

/// Write-through: persist `creds` into the OS keychain after the server has
/// accepted a request made with them, so the next command for this remote
/// resolves straight from the keychain tier instead of env/config.toml.
///
/// Call only after a successful (non-error) response — never speculatively
/// — since the keychain tier is checked *before* config.toml: caching an
/// untested or bad credential would silently shadow a subsequently-fixed
/// `config.toml`/env value on every future run. No-op if `MEDIAGIT_NO_KEYRING`
/// is set, `creds` is `Credentials::None`, or the keychain write fails
/// (best-effort, same "never break a working command" rule as the read side).
///
/// ponytail: re-writing a value that was itself just read from the keychain
/// (env/config-sourced vs. keychain-sourced credentials aren't distinguished
/// by the caller) is a harmless idempotent no-op, so every call site can
/// call this unconditionally after success rather than threading an extra
/// "which tier did this come from" flag through every command.
pub fn remember_credentials(
    config: &mediagit_config::Config,
    remote_name: &str,
    creds: &mediagit_protocol::Credentials,
) {
    if keyring_disabled() {
        return;
    }
    let payload = match creds {
        mediagit_protocol::Credentials::Bearer(t) => format!("bearer:{t}"),
        mediagit_protocol::Credentials::ApiKey(k) => format!("apikey:{k}"),
        mediagit_protocol::Credentials::None => return,
    };
    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, &keyring_account(config, remote_name))
    {
        let _ = entry.set_password(&payload);
    }
}

/// True if `typed_url` and `remote_url` point at the same host for
/// credential-attachment purposes (F4): same scheme (case-insensitive),
/// same host (case-insensitive), and same effective port — an explicit
/// port if given, else the scheme's well-known default (80/443/...).
/// Path, query, and userinfo are ignored; either URL failing to parse is
/// treated as "no match" (fail closed).
///
/// Used only to gate credential attachment in `download`'s full-URL mode:
/// env/config credentials are attached only if the user-typed URL's host
/// matches one of the repo's configured remotes, so a typo'd or malicious
/// host never receives a token meant for the real remote.
pub fn host_matches_remote(typed_url: &str, remote_url: &str) -> bool {
    let (Ok(a), Ok(b)) = (url::Url::parse(typed_url), url::Url::parse(remote_url)) else {
        return false;
    };
    if !a.scheme().eq_ignore_ascii_case(b.scheme()) {
        return false;
    }
    match (a.host_str(), b.host_str()) {
        (Some(ha), Some(hb)) if ha.eq_ignore_ascii_case(hb) => {}
        _ => return false,
    }
    a.port_or_known_default() == b.port_or_known_default()
}

/// Best-effort warning when a credential was just read from a config.toml
/// that's readable by group/other. Unix-only (real permission bits); on
/// Windows this is a no-op rather than shelling out to `icacls` for a
/// one-line advisory — add real ACL inspection if this ever matters more
/// than a nudge.
#[cfg(unix)]
fn warn_if_config_world_readable(repo_root: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let config_path = repo_root.join(".mediagit").join("config.toml");
    if let Ok(meta) = std::fs::metadata(&config_path) {
        if meta.permissions().mode() & 0o077 != 0 {
            eprintln!(
                "warning: {} is readable by group/other and may contain a remote token or API key — consider `chmod 600` on it",
                config_path.display()
            );
        }
    }
}

#[cfg(not(unix))]
fn warn_if_config_world_readable(_repo_root: &Path) {
    // ponytail: best-effort only, see doc comment above.
}

/// Determine the effective repo namespace (layout v2): env override wins,
/// then the value persisted in config.toml at init/clone time, then a
/// sanitized basename of the repo root as a last-resort fallback for repos
/// whose config predates `repo_namespace` (never written on disk in that
/// case — recomputed identically every time, since repo root doesn't move).
fn resolve_repo_namespace(repo_root: &Path, config: &mediagit_config::Config) -> String {
    if let Ok(ns) = std::env::var("MEDIAGIT_REPO_NAMESPACE") {
        if !ns.trim().is_empty() {
            return mediagit_storage::sanitize_namespace(&ns);
        }
    }
    if let Some(ns) = &config.repo_namespace {
        if !ns.trim().is_empty() {
            return mediagit_storage::sanitize_namespace(ns);
        }
    }
    let basename = repo_root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".to_string());
    mediagit_storage::sanitize_namespace(&basename)
}

/// Resolve this repository's identity (namespace-collision guard, M2).
///
/// Returns the `repo_id` persisted in `config.repo_id`. For configs written
/// before this field existed (`None`), generates a fresh one and writes it
/// back to `config.toml` immediately — "generate-and-write on first open" —
/// so the value is stable across subsequent invocations rather than being
/// silently unowned or regenerated every run (which would make the
/// collision guard useless: a marker adopted with a throwaway id would
/// mismatch on the very next open).
async fn resolve_repo_id(repo_root: &Path, config: &mediagit_config::Config) -> Result<String> {
    if let Some(id) = &config.repo_id {
        if !id.trim().is_empty() {
            return Ok(id.clone());
        }
    }
    let id = mediagit_storage::generate_repo_id();
    let mut updated = config.clone();
    updated.repo_id = Some(id.clone());
    updated
        .save(repo_root)
        .context("Failed to persist newly generated repo_id to config.toml")?;
    Ok(id)
}

/// Create the appropriate storage backend based on repository config.
///
/// Reads `.mediagit/config.toml` to determine backend type (filesystem, S3, Azure, GCS).
/// Falls back to local filesystem if config is missing or uses default storage.
///
/// Layout v2: the returned backend is always wrapped in
/// [`mediagit_storage::NamespacedBackend`] so every key is transparently
/// prefixed `"<repo_namespace>/"` — this is one of exactly two production
/// construction sites (the other is the server's `build_storage_backend`);
/// every backend arm below funnels through the same wrap-before-return path.
///
/// # Arguments
/// * `repo_root` - Root of the mediagit repository (parent of .mediagit/)
///
/// # Returns
/// An `Arc<dyn StorageBackend>` configured per the repository's config.toml
pub async fn create_storage_backend(repo_root: &Path) -> Result<Arc<dyn StorageBackend>> {
    let mediagit_dir = repo_root.join(".mediagit");

    // Load config (returns default if config.toml doesn't exist)
    let config = mediagit_config::Config::load(repo_root)
        .await
        .unwrap_or_default();

    let ns = resolve_repo_namespace(repo_root, &config);
    let repo_id = resolve_repo_id(repo_root, &config).await?;
    let inner = create_inner_storage_backend(repo_root, &mediagit_dir, &config).await?;
    let namespaced = mediagit_storage::NamespacedBackend::new(inner, ns)
        .context("Failed to construct namespaced storage backend")?;

    mediagit_storage::check_or_write_layout_marker(
        &namespaced,
        mediagit_config::CURRENT_LAYOUT_VERSION,
        &repo_id,
    )
    .await
    .context("Layout version check failed")?;

    Ok(Arc::new(namespaced))
}

async fn create_inner_storage_backend(
    repo_root: &Path,
    mediagit_dir: &Path,
    config: &mediagit_config::Config,
) -> Result<Arc<dyn StorageBackend>> {
    match &config.storage {
        mediagit_config::StorageConfig::FileSystem(fs_config) => {
            let storage_path = if std::path::Path::new(&fs_config.base_path).is_absolute() {
                PathBuf::from(&fs_config.base_path)
            } else if fs_config.base_path == "./data" {
                // Default config value - use .mediagit
                mediagit_dir.to_path_buf()
            } else {
                repo_root.join(&fs_config.base_path)
            };
            let storage = mediagit_storage::LocalBackend::new(&storage_path)
                .await
                .context("Failed to initialize filesystem storage backend")?;
            Ok(Arc::new(storage))
        }
        mediagit_config::StorageConfig::S3(s3_config) => {
            if let Some(endpoint) = &s3_config.endpoint {
                // S3-compatible (MinIO, DigitalOcean Spaces, etc.)
                let storage = mediagit_storage::MinIOBackend::new_with_prefix(
                    endpoint,
                    &s3_config.bucket,
                    s3_config.access_key_id.as_deref().unwrap_or(""),
                    s3_config.secret_access_key.as_deref().unwrap_or(""),
                    &s3_config.prefix,
                )
                .await
                .context("Failed to initialize S3-compatible storage backend")?;
                Ok(Arc::new(storage))
            } else {
                // AWS S3
                let aws_endpoint = format!("https://s3.{}.amazonaws.com", s3_config.region);
                let storage = mediagit_storage::MinIOBackend::new_with_prefix(
                    &aws_endpoint,
                    &s3_config.bucket,
                    s3_config.access_key_id.as_deref().unwrap_or(""),
                    s3_config.secret_access_key.as_deref().unwrap_or(""),
                    &s3_config.prefix,
                )
                .await
                .context("Failed to initialize AWS S3 storage backend")?;
                Ok(Arc::new(storage))
            }
        }
        mediagit_config::StorageConfig::Azure(azure_config) => {
            let storage = if let Some(conn_str) = &azure_config.connection_string {
                mediagit_storage::AzureBackend::with_connection_string_and_prefix(
                    &azure_config.container,
                    conn_str,
                    &azure_config.prefix,
                )
                .await
                .context("Failed to initialize Azure storage backend")?
            } else if let Some(account_key) = &azure_config.account_key {
                mediagit_storage::AzureBackend::with_account_key_and_prefix(
                    &azure_config.account_name,
                    &azure_config.container,
                    account_key,
                    &azure_config.prefix,
                )
                .await
                .context("Failed to initialize Azure storage backend")?
            } else {
                anyhow::bail!("Azure backend requires either connection_string or account_key");
            };
            Ok(Arc::new(storage))
        }
        mediagit_config::StorageConfig::GCS(gcs_config) => {
            let credentials_path = gcs_config.credentials_path.as_deref().unwrap_or("");

            let mut config =
                mediagit_storage::GcsConfig::new(&gcs_config.project_id, &gcs_config.bucket);
            if !gcs_config.prefix.is_empty() {
                config.prefix = Some(gcs_config.prefix.clone());
            }

            let storage = if credentials_path.is_empty() {
                mediagit_storage::GcsBackend::with_default_credentials_and_config(config)
                    .await
                    .context("Failed to initialize GCS storage backend")?
            } else {
                mediagit_storage::GcsBackend::with_config(config, credentials_path)
                    .await
                    .context("Failed to initialize GCS storage backend")?
            };
            Ok(Arc::new(storage))
        }
        mediagit_config::StorageConfig::Multi(_) => {
            anyhow::bail!("Multi-backend storage is not yet implemented");
        }
    }
}

/// Collect the tip OIDs of every local ref (heads, tags, remotes) as a
/// deduplicated list of hex strings suitable for use as the `have` field in a
/// pack-negotiation request.
///
/// This is the *small* shape of have-set negotiation: rather than shipping
/// the full object closure across the wire, we send only ref tips (tens of
/// OIDs even for large repos) and let the server expand the closure locally
/// from its own ODB. Any symbolic refs are resolved to their underlying OID;
/// any refs that fail to read are skipped silently — stale or corrupted
/// local state must not block a fetch.
///
/// Used by `fetch` and `pull` to enable incremental object transfer.
///
/// **BUG-008 correctness note:** every OID emitted here must be present in
/// the local ODB. Clone pre-populates `refs/remotes/origin/<branch>` for
/// **every** advertised branch but only ships objects reachable from the
/// default branch. If we advertise an OID as "have" without verifying the
/// object is actually present, the server will prune its closure and send
/// nothing — leaving the user unable to pull the non-default branch.
///
/// We filter via [`ObjectDatabase::exists`] which does a cache probe plus
/// a single storage-existence check (no content read), so the added cost
/// is O(number_of_refs) cheap I/O with zero memory overhead.
pub async fn collect_local_have(refdb: &RefDatabase, odb: &ObjectDatabase) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<String> = Vec::new();

    // Walk every namespace that can plausibly mirror objects on the server:
    // - heads: local branches the user may have pushed
    // - remotes: tracking refs (guaranteed to exist on the server modulo race)
    // - tags: lightweight + annotated tag tips
    for namespace in ["heads", "remotes", "tags"] {
        let ref_names = match refdb.list(namespace).await {
            Ok(names) => names,
            Err(_) => continue,
        };

        for name in ref_names {
            // Prefer `resolve` so symbolic refs (HEAD → refs/heads/main) get
            // followed to their direct OID.
            if let Ok(oid) = refdb.resolve(&name).await {
                let hex = oid.to_hex();
                if !seen.insert(hex.clone()) {
                    continue;
                }
                // Only advertise as "have" if the object really is in our ODB.
                // Skipping silently on `exists()` errors is deliberate: a
                // transient storage hiccup should trigger a full re-download
                // rather than claim to have data we can't read.
                if odb.exists(&oid).await.unwrap_or(false) {
                    out.push(hex);
                }
            }
        }
    }

    out
}

/// Like [`collect_local_have`] but skips the object-presence filter.
///
/// Useful for push paths where the client has just written the objects
/// locally and is trying to tell the server which ones not to send back.
/// Fetch/pull callers must use [`collect_local_have`] to avoid the
/// advertised-but-missing remote-tracking OID problem described there.
#[allow(dead_code)]
pub async fn collect_local_have_unchecked(refdb: &RefDatabase) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for namespace in ["heads", "remotes", "tags"] {
        let ref_names = match refdb.list(namespace).await {
            Ok(names) => names,
            Err(_) => continue,
        };
        for name in ref_names {
            if let Ok(oid) = refdb.resolve(&name).await {
                let hex = oid.to_hex();
                if seen.insert(hex.clone()) {
                    out.push(hex);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_find_repo_root_from() {
        let temp = TempDir::new().unwrap();
        let repo_root = temp.path();

        // Create .mediagit directory
        std::fs::create_dir(repo_root.join(".mediagit")).unwrap();

        // Create nested directory
        let nested = repo_root.join("src").join("commands");
        std::fs::create_dir_all(&nested).unwrap();

        // Should find root from nested path
        let found = find_repo_root_from(&nested).unwrap();
        assert_eq!(found, repo_root);
    }

    #[test]
    fn test_find_repo_root_from_not_found() {
        let temp = TempDir::new().unwrap();
        // No .mediagit directory
        let result = find_repo_root_from(temp.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_normalize_path_strips_dot_slash() {
        let temp = TempDir::new().unwrap();
        let repo_root = dunce::canonicalize(temp.path()).unwrap();

        // Create a file to test with
        let file_path = repo_root.join("test.ai");
        std::fs::write(&file_path, "test").unwrap();

        // ./test.ai should normalize to test.ai
        let dot_slash = Path::new("./test.ai");
        let result = normalize_path(dot_slash, &repo_root);
        assert_eq!(result, PathBuf::from("test.ai"));
    }

    #[test]
    fn test_normalize_path_absolute() {
        let temp = TempDir::new().unwrap();
        let repo_root = dunce::canonicalize(temp.path()).unwrap();

        let file_path = repo_root.join("subdir").join("file.psd");
        std::fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        std::fs::write(&file_path, "test").unwrap();

        let result = normalize_path(&file_path, &repo_root);
        assert_eq!(result, PathBuf::from("subdir/file.psd"));
    }

    #[test]
    fn test_normalize_path_backslash() {
        let temp = TempDir::new().unwrap();
        let repo_root = dunce::canonicalize(temp.path()).unwrap();

        // Simulate Windows-style path
        let result = normalize_path(Path::new(".\\test.ai"), &repo_root);
        assert_eq!(result, PathBuf::from("test.ai"));
    }

    #[test]
    fn host_matches_remote_exact_match() {
        assert!(host_matches_remote(
            "http://host.example/repo/file.png",
            "http://host.example/repo"
        ));
    }

    #[test]
    fn host_matches_remote_case_insensitive_host() {
        assert!(host_matches_remote(
            "http://Host.Example/repo/file.png",
            "http://host.example/repo"
        ));
    }

    #[test]
    fn host_matches_remote_explicit_matches_default_port_http() {
        assert!(host_matches_remote(
            "http://host.example:80/repo/file.png",
            "http://host.example/repo"
        ));
    }

    #[test]
    fn host_matches_remote_explicit_matches_default_port_https() {
        assert!(host_matches_remote(
            "https://host.example:443/repo/file.png",
            "https://host.example/repo"
        ));
    }

    #[test]
    fn host_matches_remote_different_port_no_match() {
        assert!(!host_matches_remote(
            "http://host.example:8080/repo/file.png",
            "http://host.example/repo"
        ));
    }

    #[test]
    fn host_matches_remote_different_scheme_no_match() {
        assert!(!host_matches_remote(
            "https://host.example/repo/file.png",
            "http://host.example/repo"
        ));
    }

    #[test]
    fn host_matches_remote_ignores_path_and_userinfo() {
        assert!(host_matches_remote(
            "http://user:pass@host.example/some/other/path",
            "http://host.example/completely-different-repo-path"
        ));
    }

    #[test]
    fn host_matches_remote_different_host_no_match() {
        assert!(!host_matches_remote(
            "http://evil.example/repo/file.png",
            "http://host.example/repo"
        ));
    }
}

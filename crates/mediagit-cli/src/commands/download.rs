// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Download a single file from a remote repository by path (#6).
//!
//! Plain streaming GET against the server's existing browse endpoint
//! (`GET /{repo}/files/{*path}?ref=<ref>`) — deliberately not
//! `StreamingDownloader`, which assumes HEAD+Range support the browse
//! endpoint doesn't offer. Works without a local repository when given a
//! full URL, which is the point: CI/scripting can pull one asset without
//! cloning.

use anyhow::{Context, Result};
use clap::Parser;
use mediagit_versioning::{Commit, ObjectDatabase, RefDatabase};
use std::path::{Path, PathBuf};

/// Download a single file from a remote repository by path
///
/// Streams the file straight to disk without buffering it fully in memory.
/// Works two ways:
///   - Full URL (no local repository needed): the first path segment after
///     the host is the repository name, everything after it is the file
///     path within that repository.
///   - Repo-relative path (run inside a MediaGit repository): resolved
///     against the `origin` remote configured for that repository.
#[derive(Parser, Debug)]
#[command(after_help = "EXAMPLES:
    # Download a file by full URL — no local repository required (CI/scripting)
    mediagit download http://server:3000/my-project/assets/logo.png

    # Download at a specific ref
    mediagit download http://server:3000/my-project/assets/logo.png --ref v1.0

    # Download to a specific output path
    mediagit download http://server:3000/my-project/assets/logo.png -o logo.png

    # Inside a repo, resolves against the 'origin' remote
    mediagit download assets/logo.png

SEE ALSO:
    mediagit-clone(1), mediagit-pull(1)")]
pub struct DownloadCmd {
    /// Remote file to download: a full URL
    /// (`scheme://host[:port]/<repo>/<path...>`), or — when run inside a
    /// MediaGit repository — a path relative to that repo's root, resolved
    /// against the `origin` remote.
    #[arg(value_name = "REMOTE_PATH")]
    pub remote_path: String,

    /// Ref to download the file from (branch, tag, or commit OID). Defaults
    /// to the server's advertised default branch (`main`, else `master`,
    /// else the first `refs/heads/*` entry) rather than the literal string
    /// "HEAD" — repos populated purely via `push` have no server-side HEAD
    /// ref for the browse endpoint to resolve, so defaulting to literal
    /// HEAD would make this command fail out of the box on exactly the
    /// repos it's meant for.
    #[arg(long, value_name = "REF")]
    pub r#ref: Option<String>,

    /// Output file path (defaults to the file's base name in the current directory)
    #[arg(short, long, value_name = "PATH")]
    pub output: Option<String>,

    /// Quiet mode - minimal output
    #[arg(short, long)]
    pub quiet: bool,
}

impl DownloadCmd {
    pub async fn execute(&self) -> Result<()> {
        use crate::output;

        // Local case: repo-relative path, run inside a repository — resolve
        // `--ref` and read the blob straight out of local history, no
        // server contact required. Any local miss (unresolvable ref, path
        // not in the tree, no repo, ...) falls through unchanged to the
        // existing remote/URL path below.
        let is_url =
            self.remote_path.starts_with("http://") || self.remote_path.starts_with("https://");
        if !is_url {
            validate_no_path_traversal(&self.remote_path)?;
            if let Ok(repo_root) = crate::repo::find_repo_root()
                && let Some(bytes) = self.try_local_extract(&repo_root).await
            {
                let out_path = self.output_path(&self.remote_path);
                if let Some(parent) = out_path.parent()
                    && !parent.as_os_str().is_empty()
                {
                    std::fs::create_dir_all(parent).context("Failed to create output directory")?;
                }
                std::fs::write(&out_path, &bytes).with_context(|| {
                    format!("Failed to write output file '{}'", out_path.display())
                })?;
                if !self.quiet {
                    output::success(&format!(
                        "Downloaded '{}' ({} bytes) to {}",
                        self.remote_path,
                        bytes.len(),
                        out_path.display()
                    ));
                }
                return Ok(());
            }
        }

        let (base_url, file_path, config, repo_root, attach_credentials) =
            self.resolve_source().await?;
        validate_no_path_traversal(&file_path)?;

        let (mut credentials, cred_source) = if attach_credentials {
            crate::repo::resolve_credentials_tiered(&repo_root, &config, "origin")
        } else {
            // F4: full-URL mode with no repo, or with a repo whose configured
            // remotes don't match the typed host — never fall through to
            // MEDIAGIT_TOKEN/MEDIAGIT_API_KEY, which would otherwise ship a
            // token to whatever host the user typed.
            if !self.quiet
                && (std::env::var("MEDIAGIT_TOKEN")
                    .ok()
                    .filter(|t| !t.trim().is_empty())
                    .is_some()
                    || std::env::var("MEDIAGIT_API_KEY")
                        .ok()
                        .filter(|k| !k.trim().is_empty())
                        .is_some())
            {
                output::info(
                    "Not attaching MEDIAGIT_TOKEN/MEDIAGIT_API_KEY: URL host doesn't match a configured remote",
                );
            }
            (
                mediagit_protocol::Credentials::None,
                crate::repo::CredentialSource::None,
            )
        };
        let mut client = mediagit_protocol::ProtocolClient::new(base_url.clone())
            .with_credentials(credentials.clone());

        let ref_name = match &self.r#ref {
            Some(r) => r.clone(),
            None => self.default_ref(&client).await?,
        };

        let out_path = self.output_path(&file_path);
        if let Some(parent) = out_path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).context("Failed to create output directory")?;
        }

        let mut file = tokio::fs::File::create(&out_path)
            .await
            .with_context(|| format!("Failed to create output file '{}'", out_path.display()))?;

        let download_result = client
            .download_file_by_path(&file_path, &ref_name, &mut file)
            .await
            .context("Download failed");
        let bytes = match download_result {
            Ok(bytes) => bytes,
            // First (and only, for this command) authenticated call — a
            // cached keychain credential may have expired; on a 401,
            // invalidate it and retry once with the next tier (I11). The
            // partially-written file must be truncated before retrying.
            Err(e)
                if attach_credentials
                    && crate::repo::invalidate_on_unauthorized(
                        &config,
                        "origin",
                        cred_source,
                        &e,
                    ) =>
            {
                credentials = crate::repo::resolve_credentials(&repo_root, &config, "origin");
                client = mediagit_protocol::ProtocolClient::new(base_url.clone())
                    .with_credentials(credentials.clone());
                file = tokio::fs::File::create(&out_path).await.with_context(|| {
                    format!("Failed to recreate output file '{}'", out_path.display())
                })?;
                match client
                    .download_file_by_path(&file_path, &ref_name, &mut file)
                    .await
                    .context("Download failed")
                {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        drop(file);
                        let _ = tokio::fs::remove_file(&out_path).await;
                        return Err(e);
                    }
                }
            }
            Err(e) => {
                // Don't leave a stray empty/partial file behind on failure.
                drop(file);
                let _ = tokio::fs::remove_file(&out_path).await;
                return Err(e);
            }
        };
        if attach_credentials {
            crate::repo::remember_credentials(&config, "origin", &credentials);
        }

        if !self.quiet {
            output::success(&format!(
                "Downloaded '{}' ({} bytes) to {}",
                file_path,
                bytes,
                out_path.display()
            ));
        }

        Ok(())
    }

    /// Determine which ref to request when `--ref` wasn't given: the
    /// server's advertised `main`, else `master`, else the first
    /// `refs/heads/*` entry. Falls back to the literal string `"HEAD"` only
    /// if the server advertises no branches at all (e.g. `GET /info/refs`
    /// unsupported/empty) — some server-side repo really might have an
    /// explicit HEAD file in that case.
    async fn default_ref(&self, client: &mediagit_protocol::ProtocolClient) -> Result<String> {
        let refs = client
            .get_refs()
            .await
            .context("Failed to fetch remote refs to determine the default branch")?;
        let has_branch = |name: &str| {
            refs.refs
                .iter()
                .any(|r| r.name == format!("refs/heads/{name}"))
        };
        if has_branch("main") {
            return Ok("refs/heads/main".to_string());
        }
        if has_branch("master") {
            return Ok("refs/heads/master".to_string());
        }
        if let Some(first) = refs.refs.iter().find(|r| r.name.starts_with("refs/heads/")) {
            return Ok(first.name.clone());
        }
        Ok("HEAD".to_string())
    }

    /// Resolve the server base URL, the repo-relative file path, a config +
    /// repo root, and whether credential attachment is allowed (F4).
    ///
    /// Full-URL mode (match-remote-else-strip): best-effort `find_repo_root`
    /// — if inside a repo, its real `Config` is loaded and credentials are
    /// allowed only if the typed URL's host matches one of that repo's
    /// configured remotes (scheme + host + effective port). No repo, or no
    /// matching remote, and credentials are disallowed outright: `execute`
    /// then skips `resolve_credentials` entirely, so `MEDIAGIT_TOKEN`/
    /// `MEDIAGIT_API_KEY` never gets attached to an arbitrary host the user
    /// happened to type. This intentionally departs from the old behavior,
    /// which returned a default `Config` and let credentials fall through
    /// to those env vars regardless of host — a token-exfiltration bug.
    ///
    /// Repo-relative mode is unchanged: credentials are always allowed,
    /// resolved the normal way (per-remote config, then env fallback) since
    /// the target is always the repo's own `origin` remote.
    async fn resolve_source(
        &self,
    ) -> Result<(String, String, mediagit_config::Config, PathBuf, bool)> {
        if self.remote_path.starts_with("http://") || self.remote_path.starts_with("https://") {
            let (base_url, file_path) = split_repo_url(&self.remote_path)?;
            let cwd = std::env::current_dir().context("Failed to get current directory")?;

            if let Ok(repo_root) = crate::repo::find_repo_root() {
                let config = mediagit_config::Config::load(&repo_root)
                    .await
                    .unwrap_or_default();
                let host_match = config
                    .remotes
                    .values()
                    .any(|r| crate::repo::host_matches_remote(&base_url, &r.url));
                return Ok((base_url, file_path, config, repo_root, host_match));
            }
            return Ok((
                base_url,
                file_path,
                mediagit_config::Config::default(),
                cwd,
                false,
            ));
        }

        let repo_root = crate::repo::find_repo_root().context(
            "Not inside a MediaGit repository — pass a full URL to download without one, \
             e.g. `mediagit download http://server:3000/my-project/path/to/file`",
        )?;
        let config = mediagit_config::Config::load(&repo_root).await?;
        let base_url = config
            .resolve_remote_url("origin")
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        Ok((base_url, self.remote_path.clone(), config, repo_root, true))
    }

    /// Try to resolve `--ref` (default `HEAD`) and extract `remote_path`
    /// from local repository history — no server contact. Reuses
    /// `ShowCmd`'s revision-peeling and tree-walk helpers (same logic as
    /// `show <rev>:<path>`). Returns `None` on any local miss (unresolvable
    /// ref, path not found in the tree, storage error, ...) so `execute`
    /// falls back to the existing remote-download path unchanged.
    async fn try_local_extract(&self, repo_root: &Path) -> Option<Vec<u8>> {
        let storage = crate::repo::create_storage_backend(repo_root).await.ok()?;
        let refdb = RefDatabase::new(repo_root.join(".mediagit"));
        let odb = ObjectDatabase::with_smart_compression(storage, 1000);

        let ref_name = self.r#ref.as_deref().unwrap_or("HEAD");
        let oid = mediagit_versioning::resolve_revision(ref_name, &refdb, &odb)
            .await
            .ok()?;
        let commit_oid = super::show::ShowCmd::peel_to_commit(oid, &odb).await.ok()?;
        let commit_data = odb.read(&commit_oid).await.ok()?;
        let commit = Commit::deserialize(&commit_data).ok()?;
        let file_oid =
            super::show::ShowCmd::find_path_in_tree(&odb, &commit.tree, &self.remote_path)
                .await
                .ok()?;
        odb.read(&file_oid).await.ok()
    }

    fn output_path(&self, file_path: &str) -> PathBuf {
        if let Some(o) = &self.output {
            return PathBuf::from(o);
        }
        let name = file_path.rsplit('/').next().unwrap_or(file_path);
        PathBuf::from(name)
    }
}

/// Split a full remote URL into `(base_url_including_repo, file_path)`.
/// The repo name is exactly the first path segment (matching the server's
/// `/{repo}/files/{*path}` route); everything after it is the file path.
///
/// Deliberately parsed with plain string splitting rather than a URL crate
/// — mediagit-cli doesn't otherwise depend on one, and a mediagit remote
/// URL's shape (`scheme://host[:port]/repo/path...`) doesn't need general
/// URL parsing to split at the first two `/`.
fn split_repo_url(url_str: &str) -> Result<(String, String)> {
    let (scheme, rest) = url_str
        .split_once("://")
        .ok_or_else(|| anyhow::anyhow!("Invalid URL '{url_str}': missing scheme"))?;
    let (authority, path) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx + 1..]),
        None => (rest, ""),
    };
    let mut segments = path.split('/').filter(|s| !s.is_empty());
    let repo = segments.next().ok_or_else(|| {
        anyhow::anyhow!(
            "URL must include a repository segment and a file path, \
             e.g. http://host/my-repo/path/to/file (got '{}')",
            url_str
        )
    })?;
    let file_path: String = segments.collect::<Vec<_>>().join("/");
    if file_path.is_empty() {
        anyhow::bail!(
            "URL must include a file path after the repository segment '{}' (got '{}')",
            repo,
            url_str
        );
    }
    Ok((format!("{scheme}://{authority}/{repo}"), file_path))
}

/// Reject `..` path-traversal components client-side, in addition to
/// whatever the server validates. Backslashes are rejected too since a
/// server on a Windows host could otherwise reinterpret them as separators.
fn validate_no_path_traversal(file_path: &str) -> Result<()> {
    if file_path.is_empty() {
        anyhow::bail!("Empty remote file path");
    }
    if file_path.contains('\\') {
        anyhow::bail!("Remote file path must not contain backslashes: '{file_path}'");
    }
    if file_path.split('/').any(|part| part == "..") {
        anyhow::bail!("Remote file path must not contain '..': '{file_path}'");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_repo_url_extracts_repo_and_path() {
        let (base, path) =
            split_repo_url("http://localhost:3000/my-project/assets/logo.png").unwrap();
        assert_eq!(base, "http://localhost:3000/my-project");
        assert_eq!(path, "assets/logo.png");
    }

    #[test]
    fn split_repo_url_with_port_and_nested_path() {
        let (base, path) = split_repo_url("https://host.example:8443/repo/a/b/c.bin").unwrap();
        assert_eq!(base, "https://host.example:8443/repo");
        assert_eq!(path, "a/b/c.bin");
    }

    #[test]
    fn split_repo_url_rejects_missing_file_path() {
        assert!(split_repo_url("http://localhost:3000/my-project").is_err());
    }

    #[test]
    fn split_repo_url_rejects_missing_repo() {
        assert!(split_repo_url("http://localhost:3000/").is_err());
    }

    #[test]
    fn validate_no_path_traversal_rejects_dotdot() {
        assert!(validate_no_path_traversal("../etc/passwd").is_err());
        assert!(validate_no_path_traversal("assets/../../etc/passwd").is_err());
    }

    #[test]
    fn validate_no_path_traversal_rejects_backslash() {
        assert!(validate_no_path_traversal("assets\\logo.png").is_err());
    }

    #[test]
    fn validate_no_path_traversal_accepts_normal_path() {
        assert!(validate_no_path_traversal("assets/logo.png").is_ok());
    }

    #[test]
    fn output_path_defaults_to_basename() {
        let cmd = DownloadCmd {
            remote_path: "http://x/repo/a/b/c.bin".to_string(),
            r#ref: None,
            output: None,
            quiet: false,
        };
        assert_eq!(cmd.output_path("a/b/c.bin"), PathBuf::from("c.bin"));
    }

    #[test]
    fn output_path_honors_explicit_output() {
        let cmd = DownloadCmd {
            remote_path: "http://x/repo/a/b/c.bin".to_string(),
            r#ref: None,
            output: Some("out/here.bin".to_string()),
            quiet: false,
        };
        assert_eq!(cmd.output_path("a/b/c.bin"), PathBuf::from("out/here.bin"));
    }
}

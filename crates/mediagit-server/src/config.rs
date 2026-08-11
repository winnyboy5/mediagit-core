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

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[cfg(feature = "tls")]
use mediagit_security::TlsConfig;

/// Server-side at-rest encryption.
///
/// This is the switch. An earlier `security.encryption_at_rest` existed in
/// `mediagit-config` and was documented as the server's setting, but the server
/// loads [`ServerConfig`] and never read it — setting it did nothing at all. It
/// has been removed rather than left to look load-bearing.
///
/// What `enabled` turns on is the server's willingness to *hold* repository
/// keys: the escrow endpoints, and per-repo sealing of what it writes. It is
/// not a mandate — unencrypted repositories keep working on the same server,
/// and clients that send nothing encrypted are unaffected.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncryptionConfig {
    /// Accept escrowed repository keys and permit encrypted repositories.
    #[serde(default)]
    pub enabled: bool,

    /// File holding this server's master key, which wraps every repository key
    /// it stores.
    ///
    /// Required when `enabled` is true: without it the server would have to
    /// keep repository keys in the clear on its own disk, and the threat model
    /// this feature exists for — a compromised object store — usually means the
    /// object store's credentials are on that same disk.
    ///
    /// Same format the client accepts: 64 hex characters or 32 raw bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub master_key_path: Option<PathBuf>,
}

/// Server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    /// Port to listen on (HTTP)
    #[serde(default = "default_port")]
    pub port: u16,

    /// Directory containing repositories
    #[serde(default = "default_repos_dir")]
    pub repos_dir: PathBuf,

    /// Host to bind to
    #[serde(default = "default_host")]
    pub host: String,

    /// Enable HTTPS/TLS
    #[serde(default)]
    pub enable_tls: bool,

    /// HTTPS port (when TLS is enabled)
    #[serde(default = "default_tls_port")]
    pub tls_port: u16,

    /// TLS certificate file path (PEM format)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls_cert_path: Option<PathBuf>,

    /// TLS private key file path (PEM format)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls_key_path: Option<PathBuf>,

    /// Use self-signed certificate for development
    #[serde(default)]
    pub tls_self_signed: bool,

    /// Minimum TLS protocol version to accept: `"1.2"` or `"1.3"`. Defaults
    /// to `"1.3"` when unset. This is an escape hatch for clients/proxies
    /// that only speak TLS 1.2 — an unrecognized value is a hard config
    /// error (see `build_tls_config`), not a silent fallback, matching this
    /// repo's closed-key-set convention for security settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tls_min_version: Option<String>,

    /// At-rest encryption of stored objects.
    ///
    /// Off by default, and absent from existing config files, which parse
    /// unchanged and keep behaving exactly as they did.
    #[serde(default)]
    pub encryption: EncryptionConfig,

    /// Enable authentication
    #[serde(default)]
    pub enable_auth: bool,

    /// JWT secret key (required when enable_auth = true)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jwt_secret: Option<String>,

    /// Whether `POST /auth/register` is open to anonymous callers. Defaults
    /// to `true` so existing configs and drills (which self-register users)
    /// behave exactly as before; new deployments may opt into closed
    /// registration explicitly. Only meaningful when `enable_auth = true`.
    #[serde(default = "default_allow_open_registration")]
    pub allow_open_registration: bool,

    /// TTL (seconds) for presigned PUT URLs issued to clients for direct-to-bucket uploads.
    /// 12 hours by default; may need lowering if credentials use short-lived STS sessions.
    #[serde(default = "default_presigned_url_ttl")]
    pub presigned_url_ttl_seconds: u64,

    /// Enable rate limiting
    #[serde(default)]
    pub enable_rate_limiting: bool,

    /// Rate limiting: requests per second
    #[serde(default = "default_rate_limit_rps")]
    pub rate_limit_rps: u64,

    /// Rate limiting: burst size
    #[serde(default = "default_rate_limit_burst")]
    pub rate_limit_burst: u32,

    /// Directory where auth state (users.jsonl, api_keys.jsonl) is
    /// persisted. Defaults to a sibling `auth/` directory next to
    /// `repos_dir` when unset — see [`ServerConfig::resolved_auth_store_dir`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_store_dir: Option<PathBuf>,

    /// Allowed CORS origins (exact match, e.g. "https://app.example.com").
    /// When unset (the default), no CORS layer is added — the server keeps
    /// today's behavior of emitting no CORS headers at all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cors_allowed_origins: Option<Vec<String>>,

    /// Server-enforced content verification of presigned uploads: chunk
    /// completion (`POST /:repo/chunks/complete`) and pack registration
    /// (`POST /:repo/packs/complete`). Presigned bytes go client→bucket
    /// directly, so this is the only point the server can check them at all.
    ///
    /// Turning it off drops back to existence-only checks, which accept any
    /// bytes under a claimed id — a client holding a valid `repo:write` grant
    /// could poison the store, and nothing would notice until someone
    /// reconstructed the file.
    ///
    /// **Defaults to `true`.** It did not always: `bdfd897` turned it off on
    /// measured evidence, because verifying synchronously ran at an aggregate
    /// **0.226 MiB/s** on real S3 — a 1 GB push was still unfinished after 42
    /// minutes, extrapolating to ~12.6 h for 10 GB against ~20 min without.
    /// The cost was contention, not bandwidth: N concurrent `complete_pack`
    /// requests each dragged a whole 64 MiB pack back over one WAN link, and
    /// per-pack throughput decayed monotonically as they piled up (0.057 →
    /// 0.032 MiB/s) while the last pack, running alone, was 45x faster.
    ///
    /// That cost is now gone. `complete_pack` registers the pack and returns;
    /// verification runs in the background under a serialising semaphore
    /// (`MEDIAGIT_PACK_VERIFY_CONCURRENCY`), durably tracked by a `.pending`
    /// marker so a crash mid-verification is resumed by the startup sweep
    /// rather than silently dropped.
    ///
    /// Defaulting on is safe because **reads are never speculative**: an
    /// unverified pack is verified before any byte of it is served
    /// (`download_chunk`, `batch_get_pack_chunks` verify the requested slice
    /// inline) and before any presigned URL for it is minted
    /// (`presign_pack_downloads` verifies the whole pack, since once a URL is
    /// out the server has no revocation, only a 12 h expiry). A verified pack
    /// mints immediately with zero added cost, so steady-state pulls are
    /// unchanged and media keeps travelling client↔bucket directly.
    ///
    /// The proxy upload path (`PUT /:repo/chunks/:id`) verifies unconditionally
    /// either way — that check is free, the server already holds those bytes.
    #[serde(default = "default_verify_content_on_complete")]
    pub verify_content_on_complete: bool,
}

fn default_port() -> u16 {
    3000
}

fn default_tls_port() -> u16 {
    3443
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}

fn default_repos_dir() -> PathBuf {
    PathBuf::from("./repos")
}

fn default_presigned_url_ttl() -> u64 {
    43200 // 12 hours
}

fn default_rate_limit_rps() -> u64 {
    10 // 10 requests per second
}

fn default_rate_limit_burst() -> u32 {
    20 // Allow bursts up to 20 requests
}

fn default_allow_open_registration() -> bool {
    true
}

fn default_verify_content_on_complete() -> bool {
    // true again, now that verification is off the push critical path.
    //
    // It was flipped to false in bdfd897 on measured evidence: synchronous
    // verification ran at 0.226 MiB/s aggregate on real S3 (~12.6 h extrapolated
    // for a 10 GB push against ~20 min without), because N concurrent
    // complete_pack requests each dragged a whole 64 MiB pack back over one WAN
    // link. That cost is gone: complete_pack now registers and returns, and
    // verification happens in the background under a serialising semaphore.
    //
    // Safe to default on because reads are never speculative -- an unverified
    // pack is verified before any byte of it is served, and before any presigned
    // URL for it is minted. See ServerConfig::verify_content_on_complete.
    true
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: default_port(),
            repos_dir: default_repos_dir(),
            host: default_host(),
            encryption: EncryptionConfig::default(),
            enable_tls: false,
            tls_port: default_tls_port(),
            tls_cert_path: None,
            tls_key_path: None,
            tls_self_signed: false,
            tls_min_version: None,
            enable_auth: false,
            jwt_secret: None,
            allow_open_registration: default_allow_open_registration(),
            presigned_url_ttl_seconds: default_presigned_url_ttl(),
            enable_rate_limiting: false,
            rate_limit_rps: default_rate_limit_rps(),
            rate_limit_burst: default_rate_limit_burst(),
            auth_store_dir: None,
            cors_allowed_origins: None,
            verify_content_on_complete: default_verify_content_on_complete(),
        }
    }
}

impl ServerConfig {
    /// Load configuration from the given path, or use defaults if the file does not exist.
    ///
    /// If `config_path` differs from the default ("mediagit-server.toml") and the file
    /// is missing, return an error instead of silently falling back — this prevents
    /// operators from thinking their S3/TLS/auth config is wired when it isn't.
    pub fn load(config_path: &str) -> Result<Self> {
        let path = PathBuf::from(config_path);
        let is_default = config_path == "mediagit-server.toml";

        if path.exists() {
            let content = std::fs::read_to_string(&path).context("Failed to read config file")?;

            toml::from_str(&content).with_context(|| {
                format!(
                    "Failed to parse config file {} (unknown keys are rejected; \
                     check for typos or deprecated sections)",
                    path.display()
                )
            })
        } else if is_default {
            tracing::warn!(
                "No config file at '{}'; using built-in defaults \
                 (port=3000, host=127.0.0.1, auth=off, rate_limit=off)",
                path.display()
            );
            Ok(Self::default())
        } else {
            anyhow::bail!(
                "config file not found: {} (specified via --config)",
                path.display()
            )
        }
    }

    /// Get the full bind address
    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// Get the full TLS bind address
    pub fn tls_bind_addr(&self) -> String {
        format!("{}:{}", self.host, self.tls_port)
    }

    /// Resolve the directory where auth state (users.jsonl, api_keys.jsonl)
    /// is persisted: `auth_store_dir` if set, otherwise a sibling `auth/`
    /// directory next to `repos_dir`.
    pub fn resolved_auth_store_dir(&self) -> PathBuf {
        self.auth_store_dir.clone().unwrap_or_else(|| {
            self.repos_dir
                .parent()
                .map(|p| p.join("auth"))
                .unwrap_or_else(|| PathBuf::from("auth"))
        })
    }

    /// Build TlsConfig from server configuration
    #[cfg(feature = "tls")]
    pub fn build_tls_config(&self) -> Result<TlsConfig> {
        use mediagit_security::{TlsConfigBuilder, TlsVersion};

        if !self.enable_tls {
            return Ok(TlsConfig::default());
        }

        let mut builder = TlsConfigBuilder::new().enable();

        // Escape hatch for TLS 1.2-only clients/proxies. Default (key absent)
        // stays 1.3. An unrecognized value is a hard error, not a silent
        // fallback — a typo here must not look configured while it isn't.
        if let Some(version) = &self.tls_min_version {
            let version = match version.as_str() {
                "1.2" => TlsVersion::V1_2,
                "1.3" => TlsVersion::V1_3,
                other => anyhow::bail!(
                    "invalid tls_min_version '{other}': accepted values are \"1.2\", \"1.3\""
                ),
            };
            builder = builder.min_tls_version(version);
        }

        if self.tls_self_signed {
            // Use self-signed certificate for development
            builder = builder.self_signed("localhost");
        } else {
            // Use provided certificate paths
            let cert_path = self
                .tls_cert_path
                .as_ref()
                .context("TLS certificate path is required when not using self-signed")?;
            let key_path = self
                .tls_key_path
                .as_ref()
                .context("TLS key path is required when not using self-signed")?;

            builder = builder.certificate_paths(cert_path, key_path);
        }

        builder.build().context("Failed to build TLS configuration")
    }

    /// Build TlsConfig (stub for non-TLS builds)
    #[cfg(not(feature = "tls"))]
    pub fn build_tls_config(&self) -> Result<()> {
        if self.enable_tls {
            anyhow::bail!(
                "TLS is enabled in configuration but not compiled in. Rebuild with --features tls"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rejects_unknown_top_level_keys() {
        // Guard for BUG-004: unknown keys/sections must not be silently dropped.
        let toml_str = r#"
            port = 5061
            [storage]
            backend = "s3"
        "#;
        let err = toml::from_str::<ServerConfig>(toml_str)
            .expect_err("unknown [storage] section must not parse");
        let msg = err.to_string();
        assert!(
            msg.contains("storage") || msg.contains("unknown"),
            "expected unknown-field rejection, got: {msg}"
        );
    }

    #[test]
    fn test_rejects_unknown_leaf_key() {
        let toml_str = r#"
            port = 5061
            enable_s3 = true
        "#;
        let err = toml::from_str::<ServerConfig>(toml_str)
            .expect_err("unknown `enable_s3` must not parse");
        let msg = err.to_string();
        assert!(
            msg.contains("enable_s3") || msg.contains("unknown"),
            "expected unknown-field rejection, got: {msg}"
        );
    }

    #[test]
    fn test_load_explicit_path_missing_bails() {
        let err = ServerConfig::load("nonexistent-config-for-bug-004.toml")
            .expect_err("explicit missing path must bail");
        let msg = err.to_string();
        assert!(
            msg.contains("not found") || msg.contains("nonexistent-config-for-bug-004"),
            "expected missing-path error, got: {msg}"
        );
    }

    #[test]
    fn test_load_default_path_missing_uses_defaults() {
        // Default path "mediagit-server.toml" is allowed to be missing
        // so first-run users get sensible defaults. Run from a temp dir
        // to guarantee the default file is not present.
        let tmp = tempfile::tempdir().expect("tempdir");
        let cwd = std::env::current_dir().expect("cwd");
        std::env::set_current_dir(tmp.path()).expect("chdir");
        let result = ServerConfig::load("mediagit-server.toml");
        std::env::set_current_dir(cwd).expect("restore cwd");
        let cfg = result.expect("default path missing must fall back to defaults");
        assert_eq!(cfg.port, 3000);
        assert!(!cfg.enable_auth);
    }

    #[test]
    fn test_serialize_load_roundtrip() {
        // ServerConfig has `deny_unknown_fields`, and its five Option fields
        // previously had no `skip_serializing_if`, so a naive serialize
        // emitted explicit `None`s that the loader would still accept as
        // TOML nulls are simply absent keys - but the real hazard this
        // guards is any future field losing its `skip_serializing_if`.
        // Round-trip through the exact same `toml::to_string` +
        // `ServerConfig::load`-equivalent path a wizard would use.
        let cfg = ServerConfig::default();
        let serialized = toml::to_string(&cfg).expect("serialize default config");
        let reloaded: ServerConfig =
            toml::from_str(&serialized).expect("wizard-written TOML must parse back");
        assert_eq!(reloaded.port, cfg.port);
        assert_eq!(
            reloaded.allow_open_registration,
            cfg.allow_open_registration
        );
        assert!(reloaded.jwt_secret.is_none());

        // Also round-trip with the Option fields populated, so a config
        // written after `mediagit-server init` with auth enabled parses too.
        let cfg2 = ServerConfig {
            enable_auth: true,
            jwt_secret: Some("a-secret".to_string()),
            auth_store_dir: Some(PathBuf::from("/data/auth")),
            allow_open_registration: false,
            ..Default::default()
        };
        let serialized2 = toml::to_string(&cfg2).expect("serialize populated config");
        let reloaded2: ServerConfig =
            toml::from_str(&serialized2).expect("populated wizard TOML must parse back");
        assert_eq!(reloaded2.jwt_secret, cfg2.jwt_secret);
        assert_eq!(reloaded2.auth_store_dir, cfg2.auth_store_dir);
        assert!(!reloaded2.allow_open_registration);
    }

    #[test]
    #[cfg(feature = "tls")]
    fn test_tls_min_version_absent_defaults_to_1_3() {
        let cfg = ServerConfig {
            enable_tls: true,
            tls_self_signed: true,
            tls_min_version: None,
            ..Default::default()
        };
        let tls_config = cfg.build_tls_config().expect("build_tls_config");
        assert_eq!(
            tls_config.min_tls_version,
            mediagit_security::TlsVersion::V1_3
        );
    }

    #[test]
    #[cfg(feature = "tls")]
    fn test_tls_min_version_1_2_is_honoured() {
        let cfg = ServerConfig {
            enable_tls: true,
            tls_self_signed: true,
            tls_min_version: Some("1.2".to_string()),
            ..Default::default()
        };
        let tls_config = cfg.build_tls_config().expect("build_tls_config");
        assert_eq!(
            tls_config.min_tls_version,
            mediagit_security::TlsVersion::V1_2
        );
    }

    #[test]
    #[cfg(feature = "tls")]
    fn test_tls_min_version_rejects_bogus_value() {
        let cfg = ServerConfig {
            enable_tls: true,
            tls_self_signed: true,
            tls_min_version: Some("1.1".to_string()),
            ..Default::default()
        };
        let err = cfg
            .build_tls_config()
            .expect_err("bogus tls_min_version must be a hard error");
        let msg = err.to_string();
        assert!(
            msg.contains("1.1"),
            "error should name the bad value: {msg}"
        );
        assert!(
            msg.contains("1.2") && msg.contains("1.3"),
            "error should name accepted values: {msg}"
        );
    }
}

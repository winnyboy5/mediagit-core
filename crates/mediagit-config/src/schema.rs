// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Author identity configuration (used when creating commits)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct AuthorConfig {
    /// Author display name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Author email address
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

/// Top-level configuration structure
///
/// This is the **client's per-repo `.mediagit/config.toml`**. The server never
/// loads this type — it has its own `mediagit_server::ServerConfig`. Anything
/// here that looks like a server setting is either a mistake or a second name
/// for a setting that lives somewhere else; see the tombstone below.
///
/// `deny_unknown_fields` is load-bearing. Without it an unrecognised key — a
/// typo, or a setting a document promised and no code ever read — parses,
/// validates, and does nothing. That failure mode has now cost this codebase
/// four separate incidents. `custom` is the sanctioned place for keys the
/// schema does not know.
//
// TOMBSTONE — the dead-config family. Removed 2026-09-18; do not re-add
// without a read site to point at.
//
//   `encryption_at_rest`, `encryption_key_path` (removed earlier)
//       Several documents described these as the server's at-rest encryption
//       switch. Nothing ever read them: the server loads its own
//       `ServerConfig`, and this type's `validate()` is never called on the
//       server path either, so even the "key path must exist" check never ran.
//       The real switch is `[encryption]` in `mediagit-server`'s config.
//
//   `[security]` — the whole struct (https_enabled, tls_cert_path,
//   tls_key_path, api_key, auth_enabled, cors_origins, rate_limiting)
//       Same defect, same struct, one field short of the previous fix. The
//       server enforces CORS, rate limiting and TLS through separately named
//       fields on `ServerConfig` (`cors_allowed_origins`,
//       `enable_rate_limiting`, `rate_limit_rps`, `rate_limit_burst`) and
//       `mediagit-security::TlsConfig`. Client-side credentials live on
//       `RemoteConfig.{token, api_key}`.
//
//   `[app]` (name, version, environment, port, host, debug)
//       A per-repo config has no application to name and no port to bind.
//
//   `[observability]` + `[observability.metrics]`
//       Prometheus is wired in the server from `mediagit-metrics`, whose own
//       `MetricsConfig` is live. Logging is configured by
//       `mediagit-observability`. Neither reads anything from here.
//
//   `[performance.cache]` and `performance.buffer_size`
//       `cache_type` validated a closed set of memory/disk/redis with no
//       dispatcher behind it and no Redis implementation anywhere.
//
// WHY A SYMBOL GREP DID NOT CATCH ANY OF IT: two of the dead types were named
// `RateLimitConfig` and `MetricsConfig`, which are also the names of the
// genuinely live `mediagit_server::security::RateLimitConfig` and
// `mediagit_metrics::MetricsConfig`. Grepping the symbol found a real
// implementation and stopped. The check that works is grepping for a *read
// site outside the defining crate*.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Storage backend configuration
    pub storage: StorageConfig,

    /// Performance tuning
    pub performance: PerformanceConfig,

    /// Author identity (used when creating commits)
    #[serde(default)]
    pub author: AuthorConfig,

    /// Remote repositories configuration
    #[serde(default)]
    pub remotes: HashMap<String, RemoteConfig>,

    /// Branch tracking configuration (upstream branches)
    #[serde(default)]
    pub branches: HashMap<String, BranchConfig>,

    /// Branch protection rules
    #[serde(default)]
    pub protected_branches: HashMap<String, BranchProtection>,

    /// Custom user-defined settings
    #[serde(default)]
    pub custom: HashMap<String, serde_json::Value>,

    /// Per-repo content-defined chunking (CDC) seed. `0` (the default for
    /// repos without this field, e.g. pre-existing configs) reproduces the
    /// original unseeded chunk boundaries exactly. Generated once at `mediagit
    /// init` for new repos and propagated to clones via protocol capabilities.
    #[serde(default)]
    pub cdc_seed: u64,

    /// Per-repo storage namespace (layout v2). All object keys are prefixed
    /// `"<repo_namespace>/"` by `NamespacedBackend` so one storage
    /// root/bucket can safely host multiple repos. `None` for pre-v2 repos
    /// (never written) — the storage factory falls back to a sanitized
    /// basename of the repo root at open time. Set once at `init`/`clone`
    /// and never changed afterward (changing it would silently orphan every
    /// existing key under the old namespace).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_namespace: Option<String>,

    /// Physical storage layout version. `1` = pre-namespace flat layout
    /// (implicit, absent from old configs); `2` = per-repo namespace +
    /// true hash fanout (this cycle). Mirrored in the `LAYOUT` marker file
    /// at the storage root so a repo opened with the wrong client version
    /// fails fast instead of silently corrupting the physical layout.
    #[serde(default = "default_layout_version")]
    pub layout_version: u32,

    /// Unique identifier for *this* repository, distinct from
    /// `repo_namespace` (which defaults to a sanitized directory basename
    /// and can collide across independently-created repos sharing a
    /// storage root/bucket). Generated once at `init`/`clone` and recorded
    /// in the `LAYOUT` marker so a namespace collision is a hard error
    /// instead of silently merging two repos' key spaces. `None` for
    /// configs written before this field existed (adopted into the marker
    /// on first open after this fix — see `check_or_write_layout_marker`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_id: Option<String>,

    /// Config schema version (distinct from `layout_version`, which tracks
    /// the on-disk *object storage* layout and is authoritative via the
    /// `LAYOUT` marker — this field is config.toml's own schema version,
    /// migrated by `crate::migration::MigrationManager`). Missing on any
    /// config.toml written before this field existed, which is exactly what
    /// `#[serde(default)]` (-> 0) is for: an absent field means "v0".
    #[serde(default)]
    pub config_version: u32,

    // ---- accepted, discarded, never written -------------------------
    //
    // Sections this schema used to have. They are what makes
    // `deny_unknown_fields` safe for the configs already on disk: every
    // config.toml this tool has ever written carries them, because `save()`
    // serialized them unconditionally, so rejecting them outright would brick
    // every existing repository.
    //
    // The Rust names say `deprecated_`; serde accepts the old TOML key. The
    // contents are `serde_json::Value` because nothing looks at them — they
    // are read in order to be thrown away. `skip_serializing` means `save()`
    // never writes them back, so a config is cleaned the first time any
    // command opens it. They cannot be flattened into one struct:
    // `#[serde(flatten)]` and `deny_unknown_fields` are mutually exclusive,
    // and these keys sit at the top level of the document.
    //
    // THIS IS NOT A PLACE TO PARK A SETTING. Anything added here is a setting
    // that silently does nothing, which is the defect this change exists to
    // end. A genuinely new setting gets a real field and a read site.
    /// Was `AppConfig`.
    #[serde(default, rename = "app", skip_serializing)]
    pub deprecated_app: Option<serde_json::Value>,

    /// Was `ObservabilityConfig`, including its nested `metrics` table.
    #[serde(default, rename = "observability", skip_serializing)]
    pub deprecated_observability: Option<serde_json::Value>,

    /// Was `SecurityConfig`, including its nested `rate_limiting` table.
    #[serde(default, rename = "security", skip_serializing)]
    pub deprecated_security: Option<serde_json::Value>,

    /// Never existed on this schema at any version. Present in real configs
    /// and in this repository's own test fixtures, silently discarded by serde
    /// until `deny_unknown_fields` arrived and would now be an error.
    #[serde(default, rename = "compression", skip_serializing)]
    pub deprecated_compression: Option<serde_json::Value>,
}

/// Current on-disk layout version new repos are initialized with.
pub const CURRENT_LAYOUT_VERSION: u32 = 2;

fn default_layout_version() -> u32 {
    // Configs written before this field existed predate layout v2 entirely
    // (v1 had no namespace, no `LAYOUT` marker) — default to 1, not
    // `CURRENT_LAYOUT_VERSION`, so a pre-existing repo's config doesn't
    // silently claim to be on a layout it was never written with.
    1
}

impl Config {
    /// Get remote URL by name
    pub fn get_remote_url(&self, remote_name: &str) -> Result<String, String> {
        // `fetch_url()`, not `url`: a remote may carry a distinct fetch URL.
        // In practice `set-url` writes both, so this is the same string today —
        // but reading the field is what keeps it from becoming another setting
        // that exists and is never consulted.
        self.remotes
            .get(remote_name)
            .map(|r| r.fetch_url().to_owned())
            .ok_or_else(|| format!("Remote '{}' not found in configuration", remote_name))
    }

    /// Resolve a remote argument that may be either a name or a bare URL.
    /// If `remote_or_url` already starts with a URL scheme it is returned as-is;
    /// otherwise it is looked up as a remote name.
    ///
    /// This is the FETCH-side resolution. Pushes must use
    /// [`Self::resolve_push_url`] — see the note there.
    pub fn resolve_remote_url(&self, remote_or_url: &str) -> Result<String, String> {
        if remote_or_url.starts_with("http://") || remote_or_url.starts_with("https://") {
            return Ok(remote_or_url.to_owned());
        }
        self.get_remote_url(remote_or_url)
    }

    /// Resolve the URL a PUSH to `remote_or_url` should go to.
    ///
    /// Separate from [`Self::resolve_remote_url`] because a remote may have a
    /// distinct push URL — `mediagit remote set-url --push <url>` sets one, the
    /// same way git's `remote.<name>.pushurl` does.
    ///
    /// **This did not exist, and `remote.push` was written by nothing that read
    /// it.** `set-url --push` stored the value, printed "Changed push URL for
    /// 'origin'", and `remote show` displayed it — while `push` resolved
    /// through `resolve_remote_url` and went to `url` regardless. An operator
    /// redirecting pushes at a new server got a success message, a config that
    /// agreed with them, and pushes that kept going to the old one. Same family
    /// as the dead-config removals in `config_version` 4, but worse: those did
    /// nothing, and this one did something other than what it reported.
    pub fn resolve_push_url(&self, remote_or_url: &str) -> Result<String, String> {
        if remote_or_url.starts_with("http://") || remote_or_url.starts_with("https://") {
            return Ok(remote_or_url.to_owned());
        }
        self.remotes
            .get(remote_or_url)
            .map(|r| r.push_url().to_owned())
            .ok_or_else(|| format!("Remote '{}' not found in configuration", remote_or_url))
    }

    /// Add or update a remote
    pub fn set_remote(&mut self, name: impl Into<String>, url: impl Into<String>) {
        self.remotes
            .insert(name.into(), RemoteConfig::new(url.into()));
    }

    /// Remove a remote
    pub fn remove_remote(&mut self, name: &str) -> Option<RemoteConfig> {
        self.remotes.remove(name)
    }

    /// List all remote names
    pub fn list_remotes(&self) -> Vec<String> {
        self.remotes.keys().cloned().collect()
    }

    /// Load config from repository root
    ///
    /// If the loaded config's `config_version` is behind
    /// `migration::CONFIG_VERSION`, runs `MigrationManager` to bring it up to
    /// date, backs up the original to `config.toml.bak`, and writes the
    /// migrated config back before returning it. This never touches object
    /// storage layout (`layout_version` / the `LAYOUT` marker) — only the
    /// config.toml schema.
    /// # Why the migration runs on the TYPED config, not on the raw document
    ///
    /// The obvious way to make `deny_unknown_fields` safe for old configs is to
    /// parse the file permissively into a `toml::Value`, migrate that, and only
    /// then deserialize strictly. **That does not work here, and the failure is
    /// silent and repo-destroying.**
    ///
    /// TOML integers are `i64`. `cdc_seed` is a random `u64`, so roughly half
    /// of all real repositories carry a seed above `i64::MAX` and
    /// `toml::from_str::<toml::Value>` fails outright on them. `Config::load`
    /// then returns `Err`, and several callers treat that as "no config" and
    /// fall back to `Config::default()` — which has no `repo_namespace` and a
    /// fresh `repo_id`, so the next command reports a namespace collision
    /// between the repository and itself. Unit fixtures without a seed stay
    /// green throughout; this was caught only by running `init` for real.
    ///
    /// (`deleted warn_unknown_keys` hit the same wall from the other side and
    /// left a note about it. The note outlived the code; the trap did not.)
    ///
    /// So the order is: parse into `Config` — which deserializes `cdc_seed`
    /// straight to `u64` and never materialises a `toml::Value` — then migrate
    /// through `serde_json::Value`, which represents `u64` natively. Sections
    /// deleted in v4 are absorbed by [`DeprecatedSections`] on the way in and
    /// never written on the way out.
    pub async fn load(repo_root: impl AsRef<std::path::Path>) -> anyhow::Result<Self> {
        use crate::ConfigLoader;
        use crate::migration::{
            CONFIG_VERSION, MigrationManager, MigrationV0ToV1, MigrationV1ToV2, MigrationV2ToV3,
            MigrationV3ToV4,
        };
        let config_path = repo_root.as_ref().join(".mediagit/config.toml");

        if !config_path.exists() {
            // Return default config if file doesn't exist
            return Ok(Self::default());
        }

        let loader = ConfigLoader::new();
        let config: Config = loader.load_file(&config_path).await.map_err(|e| {
            anyhow::anyhow!(
                "Failed to load {}: {}.\n\
                 Unrecognised keys are rejected rather than silently ignored — a setting this \
                 file names but no code reads is worse than one that is absent. Put keys the \
                 schema does not define under [custom].",
                config_path.display(),
                e
            )
        })?;

        config.warn_about_deprecated_keys(&config_path);

        if config.config_version >= CONFIG_VERSION {
            return Ok(config);
        }

        tracing::info!(
            from_version = config.config_version,
            to_version = CONFIG_VERSION,
            path = %config_path.display(),
            "Migrating config.toml to current schema version"
        );

        let backup_path = config_path.with_file_name(format!(
            "{}.bak",
            config_path
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("Invalid config path: {}", config_path.display()))?
                .to_string_lossy()
        ));
        std::fs::copy(&config_path, &backup_path).map_err(|e| {
            anyhow::anyhow!(
                "Failed to back up config.toml to {} before migration: {}",
                backup_path.display(),
                e
            )
        })?;

        let mut manager = MigrationManager::new();
        manager.register(Box::new(MigrationV0ToV1));
        manager.register(Box::new(MigrationV1ToV2));
        manager.register(Box::new(MigrationV2ToV3));
        manager.register(Box::new(MigrationV3ToV4));

        // serde_json, not toml, as the migration medium — see the note above.
        let value = serde_json::to_value(&config)
            .map_err(|e| anyhow::anyhow!("Failed to serialize config for migration: {}", e))?;
        let migrated_value = manager
            .migrate(value, config.config_version, CONFIG_VERSION)
            .map_err(|e| anyhow::anyhow!("Config migration failed: {}", e))?;
        let mut migrated: Config = serde_json::from_value(migrated_value)
            .map_err(|e| anyhow::anyhow!("Failed to deserialize migrated config: {}", e))?;
        migrated.config_version = CONFIG_VERSION;

        migrated.save(repo_root.as_ref())?;

        Ok(migrated)
    }

    /// Name every deprecated section present in the file, once, on load.
    ///
    /// The `deprecated_*` fields exist so an old config still parses, but they
    /// leave a wart: serde's rejection message lists every field it accepts, so
    /// a user who mistypes a key is shown `cache`, `buffer_size`,
    /// `connection_pool` and `timeouts` among the "expected" names. Staying
    /// silent about them would be the original defect all over again — a key
    /// the tool appears to accept and does nothing with.
    ///
    /// So they are accepted, reported, and dropped on the next `save()`.
    pub fn warn_about_deprecated_keys(&self, config_path: &std::path::Path) {
        let present: Vec<&str> = [
            ("app", self.deprecated_app.is_some()),
            ("observability", self.deprecated_observability.is_some()),
            ("security", self.deprecated_security.is_some()),
            ("compression", self.deprecated_compression.is_some()),
            (
                "performance.cache",
                self.performance.deprecated_cache.is_some(),
            ),
            (
                "performance.buffer_size",
                self.performance.deprecated_buffer_size.is_some(),
            ),
            (
                "performance.connection_pool",
                self.performance.deprecated_connection_pool.is_some(),
            ),
            (
                "performance.timeouts",
                self.performance.deprecated_timeouts.is_some(),
            ),
        ]
        .into_iter()
        .filter_map(|(name, present)| present.then_some(name))
        .collect();

        if present.is_empty() {
            return;
        }

        tracing::warn!(
            keys = %present.join(", "),
            path = %config_path.display(),
            "config.toml contains settings that nothing reads. They are ignored and will be \
             dropped the next time this file is written. Nothing needs to be done."
        );
    }

    /// Save config to repository root
    pub fn save(&self, repo_root: impl AsRef<std::path::Path>) -> anyhow::Result<()> {
        let config_path = repo_root.as_ref().join(".mediagit/config.toml");

        // Create .mediagit directory if it doesn't exist
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let toml_str = toml::to_string_pretty(self)?;

        // Atomic: this file holds the author identity and every remote, so a
        // torn write (crash, ENOSPC) leaves a repository that cannot resolve
        // its own remote or commit. Same defect class as the index and pack
        // manifests. Written to a process/thread-unique temp file in the same
        // directory, fsynced, then renamed, so a reader sees either the old
        // contents or the complete new ones.
        let unique = format!(
            "{}.{}.{:?}.tmp",
            config_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("config.toml"),
            std::process::id(),
            std::thread::current().id(),
        );
        let tmp_path = config_path.with_file_name(unique);

        {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp_path)?;
            f.write_all(toml_str.as_bytes())?;
            f.sync_all()?;
        }

        if let Err(e) = std::fs::rename(&tmp_path, &config_path) {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(e.into());
        }
        Ok(())
    }

    /// Get upstream tracking for a branch
    /// Returns (remote_name, remote_branch) if tracked
    pub fn get_branch_upstream(&self, branch: &str) -> Option<(&str, &str)> {
        self.branches
            .get(branch)
            .map(|bc| (bc.remote.as_str(), bc.merge.as_str()))
    }

    /// Set upstream tracking for a branch
    pub fn set_branch_upstream(
        &mut self,
        branch: impl Into<String>,
        remote: impl Into<String>,
        merge: impl Into<String>,
    ) {
        self.branches
            .insert(branch.into(), BranchConfig::new(remote, merge));
    }

    /// Remove upstream tracking for a branch
    pub fn remove_branch_upstream(&mut self, branch: &str) -> Option<BranchConfig> {
        self.branches.remove(branch)
    }

    /// Check if a branch is protected
    pub fn is_branch_protected(&self, branch: &str) -> bool {
        self.protected_branches.contains_key(branch)
    }

    /// Get protection rules for a branch
    pub fn get_branch_protection(&self, branch: &str) -> Option<&BranchProtection> {
        self.protected_branches.get(branch)
    }

    /// Protect a branch with default rules
    pub fn protect_branch(&mut self, branch: impl Into<String>) {
        self.protected_branches
            .insert(branch.into(), BranchProtection::default_protection());
    }

    /// Protect a branch with custom rules
    pub fn protect_branch_with(&mut self, branch: impl Into<String>, protection: BranchProtection) {
        self.protected_branches.insert(branch.into(), protection);
    }

    /// Unprotect a branch
    pub fn unprotect_branch(&mut self, branch: &str) -> Option<BranchProtection> {
        self.protected_branches.remove(branch)
    }

    /// List all protected branches
    pub fn list_protected_branches(&self) -> Vec<&String> {
        self.protected_branches.keys().collect()
    }
}

/// Storage backend configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "backend")]
pub enum StorageConfig {
    /// Filesystem storage
    #[serde(rename = "filesystem")]
    FileSystem(FileSystemStorage),

    /// AWS S3 storage
    #[serde(rename = "s3")]
    S3(S3Storage),

    /// Azure Blob Storage
    #[serde(rename = "azure")]
    Azure(AzureStorage),

    /// Google Cloud Storage
    #[serde(rename = "gcs")]
    GCS(GCSStorage),

    /// Multi-backend configuration
    #[serde(rename = "multi")]
    Multi(MultiBackendStorage),
}

/// Filesystem storage configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FileSystemStorage {
    /// Base directory path
    pub base_path: String,

    /// Create directories if they don't exist
    #[serde(default = "default_true")]
    pub create_dirs: bool,

    /// Sync writes to disk
    #[serde(default)]
    pub sync: bool,

    /// File permissions (octal string like "0755")
    #[serde(default = "default_file_permissions")]
    pub file_permissions: String,
}

/// AWS S3 storage configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct S3Storage {
    /// S3 bucket name
    pub bucket: String,

    /// AWS region
    pub region: String,

    /// AWS access key ID (can be overridden via env)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_key_id: Option<String>,

    /// AWS secret access key (can be overridden via env)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret_access_key: Option<String>,

    /// S3 endpoint (for S3-compatible services)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,

    /// Object prefix
    #[serde(default)]
    pub prefix: String,
}

/// How to authenticate to Azure Blob Storage.
///
/// A tagged enum so exactly one credential is representable. The previous flat
/// shape had `account_key` and `connection_string` as independent `Option`s,
/// which made "neither" and "both" expressible and pushed the check into
/// runtime validation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AzureAuth {
    /// Shared account key — the classic Azure Storage credential.
    AccountKey {
        /// Storage account name.
        account_name: String,
        /// Storage account key.
        account_key: String,
    },
    /// Full `DefaultEndpointsProtocol=...;AccountName=...;AccountKey=...`
    /// string. Parsed by the storage backend, not by us.
    ConnectionString {
        /// The connection string verbatim.
        value: String,
    },
    /// A pre-minted Shared Access Signature. First-class rather than something
    /// smuggled through a connection string.
    Sas {
        /// Storage account name.
        account_name: String,
        /// SAS token, with or without a leading `?`.
        token: String,
    },
    /// Local Azurite emulator, using the well-known development credentials.
    ///
    /// Explicit, where it used to be *inferred* from connection-string
    /// contents — which is why the emulator path was historically the least
    /// obvious code in the Azure backend.
    Emulator,
}

/// Azurite's published development connection string.
///
/// These are the emulator's fixed, publicly-documented credentials — they are
/// not a secret and are identical on every Azurite install. Defined once here
/// so the `Emulator` auth variant resolves the same way in every consumer.
pub const AZURITE_DEV_CONNECTION_STRING: &str = "DefaultEndpointsProtocol=http;\
AccountName=devstoreaccount1;\
AccountKey=Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/KBHBeksoGMGw==;\
BlobEndpoint=http://127.0.0.1:10000/devstoreaccount1;";

/// Azure Blob Storage configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AzureStorage {
    /// Container name
    pub container: String,

    /// Blob path prefix
    #[serde(default)]
    pub prefix: String,

    /// Credential. `None` only when reading a pre-v3 config; validation turns
    /// that into an actionable migration error rather than a bare serde
    /// "missing field" message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<AzureAuth>,

    /// Fields from the pre-v3 flat shape, kept solely so a stale config is
    /// *recognised* and reported precisely. Never written back out.
    #[serde(flatten, default)]
    pub legacy: LegacyAzureFields,
}

/// Pre-`config_version` 3 Azure fields. Retained for detection and migration
/// only — see [`AzureAuth`] for the current shape.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LegacyAzureFields {
    /// Old top-level `account_name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_name: Option<String>,
    /// Old top-level `account_key`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_key: Option<String>,
    /// Old top-level `connection_string`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_string: Option<String>,
}

impl LegacyAzureFields {
    /// Whether any pre-v3 field was present in the parsed config.
    pub fn is_present(&self) -> bool {
        self.account_name.is_some()
            || self.account_key.is_some()
            || self.connection_string.is_some()
    }
}

/// Google Cloud Storage configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GCSStorage {
    /// GCS bucket name
    pub bucket: String,

    /// Project ID
    pub project_id: String,

    /// Credentials file path (can be overridden via env)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials_path: Option<String>,

    /// Object prefix
    #[serde(default)]
    pub prefix: String,
}

/// Multi-backend storage configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MultiBackendStorage {
    /// Primary backend name
    pub primary: String,

    /// Replica backends
    #[serde(default)]
    pub replicas: Vec<String>,

    /// Individual backend configurations
    pub backends: HashMap<String, serde_json::Value>,
}

/// Performance tuning configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PerformanceConfig {
    /// Override for client-side parallel chunk uploads. When None, falls back
    /// to MEDIAGIT_UPLOAD_CONCURRENCY env var or the internal default (32).
    #[serde(default)]
    pub upload_concurrency: Option<usize>,

    /// Override for client-side parallel chunk downloads. When None, falls back
    /// to MEDIAGIT_DOWNLOAD_CONCURRENCY env var or the internal default (24).
    #[serde(default)]
    pub download_concurrency: Option<usize>,

    /// Override for server-side concurrent pack-write workers. When None,
    /// falls back to MEDIAGIT_PACK_WORKERS env var or the internal default (8).
    #[serde(default)]
    pub pack_workers: Option<usize>,

    // Accepted, discarded, never written. Same contract as the
    // `deprecated_*` fields on `Config` — see the note there.
    /// Was `CacheConfig`. No cache dispatcher was ever written.
    #[serde(default, rename = "cache", skip_serializing)]
    pub deprecated_cache: Option<serde_json::Value>,

    /// Was `buffer_size`.
    #[serde(default, rename = "buffer_size", skip_serializing)]
    pub deprecated_buffer_size: Option<serde_json::Value>,

    /// Never existed on this schema. `schema.rs`'s own test fixture set it,
    /// and `mediagit-storage`'s real connection-pool knobs are env vars in
    /// `http_pool.rs`, not config.
    #[serde(default, rename = "connection_pool", skip_serializing)]
    pub deprecated_connection_pool: Option<serde_json::Value>,

    /// Never existed on this schema. Same story as `connection_pool`.
    #[serde(default, rename = "timeouts", skip_serializing)]
    pub deprecated_timeouts: Option<serde_json::Value>,
}

/// Remote repository configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RemoteConfig {
    /// Remote URL (e.g., "http://localhost:3000/repo-name")
    pub url: String,

    /// Fetch URL (if different from url)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fetch: Option<String>,

    /// Push URL (if different from url)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub push: Option<String>,

    /// JWT bearer token for this remote (client auth, M2). Lowest-precedence
    /// credential source — `MEDIAGIT_TOKEN`/`MEDIAGIT_API_KEY` env vars and
    /// the OS keychain are checked first (see `resolve_credentials` in
    /// `mediagit-cli/src/repo.rs`). Stored in plaintext in `config.toml`; a
    /// world/group-readable config file triggers a warning when this is read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,

    /// API key for this remote (client auth, M2). Same precedence and
    /// plaintext-storage caveat as `token`; only one of `token`/`api_key`
    /// should be set per remote (`token` wins if both are).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    /// Was `default_fetch`. Accepted, discarded, never written -- the same
    /// contract as the `deprecated_*` fields on `Config`.
    ///
    /// `RemoteConfig::new` set it to `Some(true)` and it was not
    /// `skip_serializing`, so **every config that has ever had a remote
    /// carries `default_fetch = true`** -- and nothing ever read it. Removing
    /// it without this absorber makes `RemoteConfig`'s `deny_unknown_fields`
    /// reject all of them.
    #[serde(default, rename = "default_fetch", skip_serializing)]
    pub deprecated_default_fetch: Option<serde_json::Value>,
}

impl RemoteConfig {
    /// Create a new remote configuration
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            fetch: None,
            push: None,
            token: None,
            api_key: None,
            deprecated_default_fetch: None,
        }
    }

    /// Get the effective fetch URL
    pub fn fetch_url(&self) -> &str {
        self.fetch.as_deref().unwrap_or(&self.url)
    }

    /// Get the effective push URL
    pub fn push_url(&self) -> &str {
        self.push.as_deref().unwrap_or(&self.url)
    }
}

/// Branch tracking configuration (similar to Git's branch.<name>.remote and branch.<name>.merge)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BranchConfig {
    /// The remote to push/pull from by default
    pub remote: String,

    /// The remote branch to merge from (e.g., "refs/heads/main")
    pub merge: String,
}

impl BranchConfig {
    /// Create a new branch tracking config
    pub fn new(remote: impl Into<String>, merge: impl Into<String>) -> Self {
        Self {
            remote: remote.into(),
            merge: merge.into(),
        }
    }
}

/// Branch protection rules
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct BranchProtection {
    /// Prevent force-push to this branch
    #[serde(default = "default_true")]
    pub prevent_force_push: bool,

    /// Prevent deletion of this branch
    #[serde(default = "default_true")]
    pub prevent_deletion: bool,

    /// Require pull request reviews before merge
    #[serde(default)]
    pub require_reviews: bool,

    /// Minimum number of approvals required (if require_reviews is true)
    #[serde(default = "default_min_approvals")]
    pub min_approvals: u32,
}

impl BranchProtection {
    /// Create default protection (prevent force-push and deletion)
    pub fn default_protection() -> Self {
        Self {
            prevent_force_push: true,
            prevent_deletion: true,
            require_reviews: false,
            min_approvals: 1,
        }
    }

    /// Create protection with review requirement
    pub fn with_reviews(min_approvals: u32) -> Self {
        Self {
            prevent_force_push: true,
            prevent_deletion: true,
            require_reviews: true,
            min_approvals,
        }
    }
}

fn default_min_approvals() -> u32 {
    1
}

// Default value functions
fn default_true() -> bool {
    true
}

fn default_file_permissions() -> String {
    "0644".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Config {
            storage: StorageConfig::FileSystem(FileSystemStorage::default()),
            performance: PerformanceConfig::default(),
            author: AuthorConfig::default(),
            remotes: HashMap::new(),
            branches: HashMap::new(),
            protected_branches: HashMap::new(),
            custom: HashMap::new(),
            cdc_seed: 0,
            repo_namespace: None,
            layout_version: default_layout_version(),
            repo_id: None,
            config_version: crate::migration::CONFIG_VERSION,
            deprecated_app: None,
            deprecated_observability: None,
            deprecated_security: None,
            deprecated_compression: None,
        }
    }
}

impl Default for FileSystemStorage {
    fn default() -> Self {
        FileSystemStorage {
            base_path: "./data".to_string(),
            create_dirs: true,
            sync: false,
            file_permissions: "0644".to_string(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert!(matches!(config.storage, StorageConfig::FileSystem(_)));
        assert_eq!(config.layout_version, default_layout_version());
    }

    #[test]
    fn test_serialization_roundtrip() {
        let config = Config::default();
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(config, deserialized);
    }

    #[test]
    fn test_config_without_cdc_seed_defaults_to_zero() {
        // Existing configs written before this field existed must still parse,
        // with cdc_seed defaulting to 0 (legacy/unseeded chunking).
        //
        // This fixture used to carry [app], [compression], [performance.cache],
        // [performance.connection_pool], [performance.timeouts],
        // [observability] and [security]. Four of those sections were dead and
        // are now deleted; three -- [compression], [performance.connection_pool]
        // and [performance.timeouts] -- never existed on this schema at all and
        // were being silently discarded by serde every time this test ran. That
        // is the hole `deny_unknown_fields` closes, and this fixture was the
        // evidence it was open. Reaching a config in that shape is the
        // migration's job, exercised by
        // `strict_parsing_rejects_a_key_the_schema_does_not_define` and the
        // v3 -> v4 load tests.
        let toml_str = r#"
[storage]
backend = "filesystem"
base_path = "./data"
[performance]
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.cdc_seed, 0);
    }

    #[test]
    fn resolve_remote_url_rejects_ssh_scheme() {
        // No transport implements ssh:// — a bare ssh:// URL must fall through
        // to the remote-name lookup and fail there, not be passed through.
        let config = Config::default();
        assert!(config.resolve_remote_url("ssh://user@host/repo").is_err());
    }

    #[test]
    fn resolve_remote_url_passes_through_http_and_https() {
        let config = Config::default();
        assert_eq!(
            config.resolve_remote_url("http://host/repo").unwrap(),
            "http://host/repo"
        );
        assert_eq!(
            config.resolve_remote_url("https://host/repo").unwrap(),
            "https://host/repo"
        );
    }

    #[test]
    fn resolve_remote_url_resolves_remote_name() {
        let mut config = Config::default();
        config.set_remote("origin", "https://host/repo");
        assert_eq!(
            config.resolve_remote_url("origin").unwrap(),
            "https://host/repo"
        );
    }

    /// A config.toml written by v3 of this tool. Every one of them looks like
    /// this: `save()` serialized the dead sections unconditionally, so this is
    /// not a hypothetical shape — it is what is on disk in every repository
    /// created before today.
    const V3_CONFIG_AS_ACTUALLY_WRITTEN: &str = r#"
config_version = 3
layout_version = 2
cdc_seed = 42

[app]
name = "mediagit"
version = "0.3.0"
environment = "development"
port = 8080
host = "127.0.0.1"
debug = false

[storage]
backend = "filesystem"
base_path = "./data"
create_dirs = true
sync = false
file_permissions = "0644"

[performance]
buffer_size = 65536

[performance.cache]
enabled = true
cache_type = "memory"
max_size = 536870912
ttl = 3600
compression = false

[observability]
log_level = "info"
log_format = "json"
tracing_enabled = true
sample_rate = 0.1

[observability.metrics]
enabled = true
port = 9090
endpoint = "/metrics"
interval = 60

[security]
https_enabled = false
auth_enabled = false
cors_origins = ["http://localhost:3000"]

[security.rate_limiting]
enabled = false
requests_per_second = 1000
burst_size = 2000
"#;

    fn write_repo(dir: &std::path::Path, contents: &str) {
        std::fs::create_dir_all(dir.join(".mediagit")).unwrap();
        std::fs::write(dir.join(".mediagit/config.toml"), contents).unwrap();
    }

    /// The whole point of `deny_unknown_fields`. Before it, this key parsed,
    /// validated, reported success and did nothing — which is how a CORS
    /// setting, a TLS certificate path and a closed-registration switch all
    /// came to be configured in a file nothing read.
    #[tokio::test]
    async fn strict_parsing_rejects_a_key_the_schema_does_not_define() {
        let dir = tempfile::tempdir().unwrap();
        write_repo(
            dir.path(),
            r#"
config_version = 4

[storage]
backend = "filesystem"
base_path = "./data"

[performance]
upload_concurency = 8
"#,
        );

        let err = Config::load(dir.path()).await.unwrap_err().to_string();
        assert!(
            err.contains("upload_concurency"),
            "the error must name the offending key, got: {err}"
        );
        assert!(
            err.contains("[custom]"),
            "the error must point at the sanctioned escape hatch, got: {err}"
        );
    }

    /// The other half, and the one that makes the strictness shippable: a
    /// config in the shape this tool has been writing for its whole life must
    /// still open. If this fails, every existing repository is bricked.
    #[tokio::test]
    async fn a_v3_config_still_loads_and_is_migrated_not_rejected() {
        let dir = tempfile::tempdir().unwrap();
        write_repo(dir.path(), V3_CONFIG_AS_ACTUALLY_WRITTEN);

        let config = Config::load(dir.path())
            .await
            .expect("a v3 config must migrate, not fail to parse");

        // Migrated, and the live values carried across untouched.
        assert_eq!(config.config_version, crate::migration::CONFIG_VERSION);
        assert_eq!(config.cdc_seed, 42);
        assert_eq!(config.layout_version, 2);

        // The original is preserved before the rewrite.
        assert!(
            dir.path().join(".mediagit/config.toml.bak").exists(),
            "the pre-migration config must be backed up"
        );

        // The dead sections are gone from the file on disk, not merely ignored
        // in memory — otherwise the next load would reject them.
        let rewritten = std::fs::read_to_string(dir.path().join(".mediagit/config.toml")).unwrap();
        for dead in [
            "[app]",
            "[observability]",
            "[security]",
            "[performance.cache]",
            "buffer_size",
        ] {
            assert!(
                !rewritten.contains(dead),
                "{dead} must not survive the migration, got:\n{rewritten}"
            );
        }
    }

    /// Loading twice must be stable. A migration that leaves behind something
    /// the strict parse rejects would pass the test above and fail on the very
    /// next command — the repo would open exactly once.
    #[tokio::test]
    async fn a_migrated_config_reloads_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        write_repo(dir.path(), V3_CONFIG_AS_ACTUALLY_WRITTEN);

        let first = Config::load(dir.path()).await.unwrap();
        let second = Config::load(dir.path())
            .await
            .expect("the rewritten config must satisfy its own strict parse");
        assert_eq!(first, second);
    }

    /// A broken config must FAIL, not resolve to the default one.
    ///
    /// `mediagit-cli`'s `create_storage_backend` used to do
    /// `Config::load(..).unwrap_or_default()`, and `resolve_repo_id` then
    /// persisted that default over the real file — losing `cdc_seed`,
    /// `repo_namespace` and `layout_version` in one step. `Config::load`
    /// already returns the default for an ABSENT file, so `Err` must mean
    /// "present and unreadable" and nothing may paper over it.
    #[tokio::test]
    async fn an_unreadable_config_is_an_error_not_a_default() {
        let dir = tempfile::tempdir().unwrap();
        write_repo(dir.path(), "this is not valid toml {{{");
        assert!(
            Config::load(dir.path()).await.is_err(),
            "an unparseable config must be an error"
        );

        // And the absent-file case still yields the default, which is what
        // makes the distinction safe to rely on.
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(Config::load(empty.path()).await.unwrap(), Config::default());
    }

    /// `cdc_seed` is a random `u64`, so about half of all real repositories
    /// carry a value above `i64::MAX`. A `toml::Value` cannot hold one, so any
    /// load path that goes through `toml::Value` fails on those repos — and
    /// because the failure lands in `unwrap_or_default()` call sites, it
    /// presents as a repository that has lost its own identity rather than as
    /// a parse error. Fixtures without a seed stay green throughout.
    #[tokio::test]
    async fn a_cdc_seed_above_i64_max_loads_and_round_trips() {
        const SEED: u64 = 17254340638138469876;
        assert!(SEED > i64::MAX as u64, "the fixture must exercise the bug");

        let dir = tempfile::tempdir().unwrap();
        write_repo(
            dir.path(),
            &format!(
                r#"
config_version = 4
cdc_seed = {SEED}
repo_namespace = "r1"

[storage]
backend = "filesystem"
base_path = "./data"

[performance]
"#
            ),
        );

        let config = Config::load(dir.path()).await.unwrap();
        assert_eq!(config.cdc_seed, SEED);
        assert_eq!(config.repo_namespace.as_deref(), Some("r1"));

        config.save(dir.path()).unwrap();
        let reloaded = Config::load(dir.path()).await.unwrap();
        assert_eq!(reloaded.cdc_seed, SEED);
    }

    /// Every config that has ever had a remote carries `default_fetch = true`:
    /// `RemoteConfig::new` set it and it was serialized. The field is deleted,
    /// `RemoteConfig` is `deny_unknown_fields`, and without the absorber every
    /// one of those configs would now be rejected. Same trap as the top-level
    /// sections, one level down, and missed on the first pass because
    /// `RemoteConfig` is a map VALUE rather than a named section.
    #[tokio::test]
    async fn a_remote_carrying_default_fetch_still_loads_and_is_cleaned() {
        let dir = tempfile::tempdir().unwrap();
        write_repo(
            dir.path(),
            r#"
config_version = 4

[storage]
backend = "filesystem"
base_path = "./data"

[performance]

[remotes.origin]
url = "https://example.com/r.git"
default_fetch = true
"#,
        );

        let config = Config::load(dir.path())
            .await
            .expect("a remote with default_fetch must load, not fail to parse");
        assert_eq!(
            config.remotes.get("origin").map(|r| r.url.as_str()),
            Some("https://example.com/r.git")
        );

        config.save(dir.path()).unwrap();
        let rewritten = std::fs::read_to_string(dir.path().join(".mediagit/config.toml")).unwrap();
        assert!(
            !rewritten.contains("default_fetch"),
            "it must be dropped on the next write, got:
{rewritten}"
        );
    }

    /// `custom` is what the rejection message tells users to reach for, so it
    /// has to actually work under strict parsing.
    #[tokio::test]
    async fn custom_is_a_real_escape_hatch_under_strict_parsing() {
        let dir = tempfile::tempdir().unwrap();
        write_repo(
            dir.path(),
            r#"
config_version = 4

[storage]
backend = "filesystem"
base_path = "./data"

[performance]

[custom]
studio_pipeline_id = "vfx-42"
"#,
        );

        let config = Config::load(dir.path()).await.unwrap();
        assert_eq!(
            config
                .custom
                .get("studio_pipeline_id")
                .and_then(|v| v.as_str()),
            Some("vfx-42")
        );
    }
}

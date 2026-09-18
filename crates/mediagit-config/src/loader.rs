// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Multi-format loading for [`Config`].
//!
//! # `warn_unknown_keys` was here, and is gone
//!
//! This module used to carry a `warn_unknown_keys` scanner that LOGGED keys
//! serde had silently discarded. It existed because `Config` deliberately did
//! not set `deny_unknown_fields`, and its own doc comment gave the reason:
//! rejecting would break every deployed config carrying a stray key, and
//! "someone mid-outage should not have their server refuse to boot over a dead
//! key it has been ignoring for a year."
//!
//! That reasoning has been overtaken on all three counts, so `Config` is now
//! `deny_unknown_fields` and the scanner is deleted rather than left as
//! unreachable code:
//!
//! * The one config it named as a blocker —
//!   `dev-tests/qa-suite/config/backends/minio.toml` and its `force_path_style`
//!   — was fixed; that key is gone and documented as never having existed.
//! * `mediagit_server::ServerConfig` already sets `deny_unknown_fields`, so the
//!   server side has been refusing unknown keys for some time. The client being
//!   permissive was the inconsistency, not the strictness.
//! * The `deprecated_*` fields on `Config` and `PerformanceConfig` now accept
//!   and discard every key this tool itself ever wrote, so no config *we*
//!   produced can be rejected, and `Config::warn_about_deprecated_keys` still
//!   names them on load. What remains rejectable is a key a human typed or a
//!   document invented, which is precisely the case a warning was too quiet for.
//!
//! The rule the scanner enforced is unchanged and still tested, both halves;
//! only the consequence moved from a log line to a refusal. See the tests at
//! the bottom of this file.

use crate::error::{ConfigError, ConfigResult};
use crate::schema::Config;
use crate::validation::Validator;
use std::path::Path;
use tokio::fs;
use tracing::debug;

/// Configuration format
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigFormat {
    Toml,
    Yaml,
    Json,
}

impl ConfigFormat {
    /// Detect format from file extension
    pub fn from_path<P: AsRef<Path>>(path: P) -> ConfigResult<Self> {
        let path = path.as_ref();
        match path.extension().and_then(|ext| ext.to_str()) {
            Some("toml") => Ok(ConfigFormat::Toml),
            Some("yaml") | Some("yml") => Ok(ConfigFormat::Yaml),
            Some("json") => Ok(ConfigFormat::Json),
            Some(ext) => Err(ConfigError::UnsupportedFormat(ext.to_string())),
            None => Err(ConfigError::InvalidPath(path.to_path_buf())),
        }
    }

    /// Get format name as string
    pub fn name(&self) -> &'static str {
        match self {
            ConfigFormat::Toml => "TOML",
            ConfigFormat::Yaml => "YAML",
            ConfigFormat::Json => "JSON",
        }
    }
}

/// Configuration loader
pub struct ConfigLoader {
    validate: bool,
}

impl ConfigLoader {
    /// Create a new configuration loader
    pub fn new() -> Self {
        ConfigLoader { validate: true }
    }

    /// Create a loader without validation
    pub fn without_validation() -> Self {
        ConfigLoader { validate: false }
    }

    /// Load configuration from a file
    pub async fn load_file<P: AsRef<Path>>(&self, path: P) -> ConfigResult<Config> {
        let path = path.as_ref();
        debug!("Loading configuration from: {}", path.display());

        if !path.exists() {
            return Err(ConfigError::FileNotFound(path.to_path_buf()));
        }

        let content = fs::read_to_string(path).await?;
        let format = ConfigFormat::from_path(path)?;

        debug!(
            "Loaded {} configuration file: {}",
            format.name(),
            path.display()
        );

        self.load_from_string(&content, format)
    }

    /// Load configuration from a string
    pub fn load_from_string(&self, content: &str, format: ConfigFormat) -> ConfigResult<Config> {
        let config = match format {
            ConfigFormat::Toml => self.parse_toml(content)?,
            ConfigFormat::Yaml => self.parse_yaml(content)?,
            ConfigFormat::Json => self.parse_json(content)?,
        };

        debug!("Configuration loaded from {}", format.name());

        if self.validate {
            config.validate()?;
            debug!("Configuration validated successfully");
        }

        Ok(config)
    }

    /// Merge multiple configuration files
    pub async fn load_and_merge<P: AsRef<Path>>(&self, paths: &[P]) -> ConfigResult<Config> {
        if paths.is_empty() {
            return Err(ConfigError::ValidationError(
                "at least one configuration file must be provided".to_string(),
            ));
        }

        let mut merged = self.load_file(&paths[0]).await?;

        for path in &paths[1..] {
            let config = self.load_file(path).await?;
            self.merge_configs(&mut merged, &config);
        }

        if self.validate {
            merged.validate()?;
        }

        Ok(merged)
    }

    /// Parse TOML configuration
    fn parse_toml(&self, content: &str) -> ConfigResult<Config> {
        let config: Config = toml::from_str(content)?;
        Ok(config)
    }

    /// Parse YAML configuration
    fn parse_yaml(&self, content: &str) -> ConfigResult<Config> {
        let config: Config = serde_yaml::from_str(content)?;
        Ok(config)
    }

    /// Parse JSON configuration
    fn parse_json(&self, content: &str) -> ConfigResult<Config> {
        let config: Config = serde_json::from_str(content)?;
        Ok(config)
    }

    /// Merge second config into first (second takes precedence)
    fn merge_configs(&self, base: &mut Config, overlay: &Config) {
        // Merge performance settings
        base.performance = overlay.performance.clone();

        // Merge custom settings
        for (key, value) in &overlay.custom {
            base.custom.insert(key.clone(), value.clone());
        }
    }
}

impl Default for ConfigLoader {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_format_detection() {
        assert_eq!(
            ConfigFormat::from_path("config.toml").unwrap(),
            ConfigFormat::Toml
        );
        assert_eq!(
            ConfigFormat::from_path("config.yaml").unwrap(),
            ConfigFormat::Yaml
        );
        assert_eq!(
            ConfigFormat::from_path("config.yml").unwrap(),
            ConfigFormat::Yaml
        );
        assert_eq!(
            ConfigFormat::from_path("config.json").unwrap(),
            ConfigFormat::Json
        );
    }

    #[test]
    fn test_format_detection_error() {
        assert!(ConfigFormat::from_path("config.xml").is_err());
        assert!(ConfigFormat::from_path("config").is_err());
    }

    #[test]
    fn test_parse_json() {
        let loader = ConfigLoader::without_validation();
        let json = r#"
        {
            "storage": {
                "backend": "filesystem",
                "base_path": "./objects"
            },
            "performance": { "upload_concurrency": 8 }
        }
        "#;
        let config = loader.load_from_string(json, ConfigFormat::Json);
        assert!(config.is_ok(), "{config:?}");
    }

    #[test]
    fn test_parse_toml() {
        let loader = ConfigLoader::without_validation();
        let toml = r#"
        [storage]
        backend = "filesystem"
        base_path = "./objects"

        [performance]
        upload_concurrency = 8
        "#;
        let config = loader.load_from_string(toml, ConfigFormat::Toml);
        assert!(config.is_ok(), "{config:?}");
    }

    #[test]
    fn test_parse_yaml() {
        let loader = ConfigLoader::without_validation();
        let yaml = r#"storage:
  backend: filesystem
  base_path: ./objects
performance:
  upload_concurrency: 8"#;
        let config = loader.load_from_string(yaml, ConfigFormat::Yaml);
        assert!(config.is_ok(), "{config:?}");
    }

    // ---- strict parsing, both halves ------------------------------------
    //
    // These replace the `warn_unknown_keys` tests. A detector that cannot fire
    // is worse than none, so the rule is unchanged: an unknown key must be
    // named, AND a valid config must be accepted. Only the consequence moved,
    // from a log line to a refusal.

    /// The exact keys the 2026-09-08 docs audit found: one that never existed
    /// on the schema, and a plain typo. Both were silently discarded.
    #[test]
    fn unknown_keys_are_rejected_and_named() {
        let err = ConfigLoader::new()
            .load_from_string(
                r#"
[storage]
backend = "s3"
bucket = "my-media-bucket"
region = "us-east-1"
encryption = true
"#,
                ConfigFormat::Toml,
            )
            .expect_err("an unknown key must be refused, not dropped")
            .to_string();
        assert!(
            err.contains("encryption"),
            "the error must name the offending key, got: {err}"
        );
    }

    /// A valid config must load. `[custom]` is a `HashMap`, so its arbitrary
    /// keys are legitimate and must never be rejected — it is the sanctioned
    /// place for anything the schema does not define.
    #[test]
    fn a_valid_config_including_custom_keys_still_loads() {
        let config = ConfigLoader::new()
            .load_from_string(
                r#"
[storage]
backend = "s3"
bucket = "my-media-bucket"
region = "us-east-1"
access_key_id = "AKIAEXAMPLE"
secret_access_key = "secret"

[custom]
anything_at_all = "is valid here"
"#,
                ConfigFormat::Toml,
            )
            .expect("a valid config must load");
        assert_eq!(
            config
                .custom
                .get("anything_at_all")
                .and_then(|v| v.as_str()),
            Some("is valid here")
        );
    }

    /// REGRESSION, carried over from the `warn_unknown_keys` suite it replaces.
    ///
    /// `cdc_seed` is a random `u64`, so about half of all real repos carry a
    /// value above `i64::MAX`. The first version of the old check re-parsed the
    /// document into a `toml::Value`, whose integers are `i64`, and died at
    /// line 1 on exactly those repos — silently, while its own tests stayed
    /// green because their fixtures had no seed. Any fixture here must carry
    /// one, and `Config::load` normalizes through `serde_json::Value` for the
    /// same reason.
    #[test]
    fn strict_parsing_survives_a_cdc_seed_above_i64_max() {
        let config = ConfigLoader::new()
            .load_from_string(
                r#"
cdc_seed = 13222148509884148795
repo_namespace = "r1"

[storage]
backend = "s3"
bucket = "my-media-bucket"
region = "us-east-1"
"#,
                ConfigFormat::Toml,
            )
            .expect("a u64 cdc_seed must not break parsing");
        assert_eq!(config.cdc_seed, 13222148509884148795);
    }

    /// An unknown TABLE is refused by its own name, not once per key inside it.
    #[test]
    fn an_unknown_table_is_refused_by_name() {
        let err = ConfigLoader::new()
            .load_from_string(
                r#"
[storage]
backend = "filesystem"
base_path = "./objects"

[nonsense]
alpha = 1
beta = 2
"#,
                ConfigFormat::Toml,
            )
            .expect_err("an unknown table must be refused")
            .to_string();
        assert!(err.contains("nonsense"), "got: {err}");
        assert!(
            !err.contains("alpha") && !err.contains("beta"),
            "the table is named once, not its contents: {err}"
        );
    }
}

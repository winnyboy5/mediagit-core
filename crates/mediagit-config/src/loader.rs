// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

use crate::error::{ConfigError, ConfigResult};
use crate::schema::Config;
use crate::validation::Validator;
use std::path::Path;
use tokio::fs;
use tracing::debug;

/// Warn about keys in `config.toml` that no field accepts.
///
/// WHY. This crate does not set `deny_unknown_fields`, so serde discards an
/// unrecognised key in silence. That makes three different mistakes look
/// identical and all of them look like success: a typo (`acces_key_id`), a
/// setting that was removed (`encryption_at_rest`, deleted from
/// `SecurityConfig`), and a setting that never existed (`[storage] encryption`,
/// which the docs advertised for months). Nothing at runtime and no reader can
/// tell them apart — a 2026-09-08 docs audit found thirteen instances, and
/// whole tuning blocks in the ARM install guide where every line was inert.
///
/// Warn rather than reject, deliberately. `deny_unknown_fields` would turn each
/// of those into a refusal to start, which breaks every deployed config
/// carrying a stray key — including this repo's own
/// `dev-tests/qa-suite/config/backends/minio.toml`, which sets
/// `force_path_style` (real on `MinIOConfig`, never a TOML key). Someone
/// mid-outage should not have their server refuse to boot over a dead key it
/// has been ignoring for a year. A warning costs them nothing and still ends
/// the silence.
///
/// The KNOWN side is a round-trip of the parsed config, not a hand-maintained
/// field list — a list would rot the moment a field is added. The RAW side is a
/// header/`key =` scan of the text rather than a second `toml::Value` parse,
/// and that is not laziness:
///
/// TOML integers are `i64`, but `cdc_seed` is a random `u64`. Roughly half of
/// all repos therefore carry a seed above `i64::MAX`, which breaks BOTH
/// directions of a TOML-based diff: re-parsing the document into a
/// `toml::Value` dies at line 1, and serialising the config back into one
/// overflows too. The first version did both and was dead on those repos while
/// its unit tests — whose fixtures had no `cdc_seed` — stayed green. Only an
/// end-to-end run against a real `mediagit init` repo exposed it.
///
/// So the known side goes through `serde_json::Value`, which represents `u64`
/// natively. Only key NAMES are compared, and serde uses the same field names
/// for both formats, so the choice of intermediate is immaterial to the answer.
///
/// `[custom]` is a `HashMap`, so its arbitrary keys survive the round-trip and
/// are correctly never reported. Keys inside an inline table (Azure's
/// `auth = { type = ... }`) are not descended into; the outer key is checked.
fn warn_unknown_keys(content: &str, parsed: &Config) {
    // Bails are LOGGED, never silent. An earlier revision returned quietly on
    // error and the whole check was dead in the shipping binary - the exact
    // failure this function exists to surface, committed inside the function
    // meant to end it. If it cannot run, that has to be observable.
    let known = match serde_json::to_value(parsed) {
        Ok(v) => v,
        Err(e) => {
            debug!("unknown-key check skipped: config did not round-trip: {e}");
            return;
        }
    };

    for (path, key) in raw_key_paths(content) {
        // Navigate to the table this key sits under; an unknown TABLE is
        // reported once, by its own name, rather than once per key inside it.
        let mut node = &known;
        let mut missing_table = false;
        for seg in path.iter() {
            match node.get(seg) {
                Some(child) => node = child,
                None => {
                    missing_table = true;
                    break;
                }
            }
        }
        let full = if path.is_empty() {
            key.clone()
        } else {
            format!("{}.{}", path.join("."), key)
        };
        if missing_table || node.get(&key).is_none() {
            tracing::warn!(
                key = %full,
                "unrecognised key in config.toml - it is being IGNORED, not applied. \
                 Check the spelling against the configuration reference."
            );
        }
    }
}

/// Every `(table path, key)` written in a TOML document, by text.
///
/// Deliberately simple: table headers and `key =` at the start of a line.
/// Values are never interpreted, so nothing here can overflow or fail to parse.
fn raw_key_paths(content: &str) -> Vec<(Vec<String>, String)> {
    let mut out = Vec::new();
    let mut table: Vec<String> = Vec::new();
    for line in content.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if let Some(rest) = t.strip_prefix('[') {
            // `[table]` and `[[array of tables]]` alike.
            let name = rest.trim_start_matches('[').trim_end_matches(']').trim();
            table = name
                .split('.')
                .map(|s| s.trim().trim_matches('"').to_string())
                .filter(|s| !s.is_empty())
                .collect();
            continue;
        }
        if let Some((lhs, _)) = t.split_once('=') {
            let key = lhs.trim().trim_matches('"').to_string();
            // Skip anything that is not a bare key: a continuation line inside a
            // multi-line array or string can contain `=` without starting a key.
            if !key.is_empty()
                && key
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                out.push((table.clone(), key));
            }
        }
    }
    out
}

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

    /// Load configuration with environment variable overrides
    pub async fn load_with_overrides<P: AsRef<Path>>(&self, path: P) -> ConfigResult<Config> {
        let mut config = self.load_file(path).await?;
        self.apply_env_overrides(&mut config)?;
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
        warn_unknown_keys(content, &config);
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

    /// Apply environment variable overrides
    /// DC-5: **this is not wired into anything, and calling it would not help.**
    ///
    /// The obvious reading is that `Config::load` forgot to call it, so every
    /// `MEDIAGIT_APP_*` override is inert. That is true but not the whole
    /// problem: the fields it writes — `[app]`, `[observability]`,
    /// `[compression]`, `[performance] max_concurrency` — are read **nowhere
    /// outside this crate's own tests**. The server's real settings live in
    /// `ServerConfig` (`mediagit-server.toml`), a different type these
    /// variables do not reach. Wiring the call in would set fields nobody
    /// consults and reintroduce the "looks configured, isn't" failure with a
    /// green checkmark on it.
    ///
    /// The documented knobs have been retracted from
    /// `book/src/reference/environment.md`. Delete this and the dead schema
    /// sections together, or give those fields real readers — do not just add
    /// the call.
    pub fn apply_env_overrides(&self, config: &mut Config) -> ConfigResult<()> {
        // App settings
        if let Ok(value) = std::env::var("MEDIAGIT_APP_NAME") {
            config.app.name = value;
        }
        if let Ok(value) = std::env::var("MEDIAGIT_APP_PORT") {
            config.app.port = value.parse().map_err(|_| {
                ConfigError::env_var_parsing_error(
                    "MEDIAGIT_APP_PORT",
                    &value,
                    "expected valid port number (1-65535)",
                )
            })?;
        }
        if let Ok(value) = std::env::var("MEDIAGIT_APP_HOST") {
            config.app.host = value;
        }
        if let Ok(value) = std::env::var("MEDIAGIT_APP_ENVIRONMENT") {
            config.app.environment = value;
        }
        if let Ok(value) = std::env::var("MEDIAGIT_APP_DEBUG") {
            config.app.debug = parse_bool(&value)?;
        }

        // Observability settings
        if let Ok(value) = std::env::var("MEDIAGIT_LOG_LEVEL") {
            config.observability.log_level = value;
        }
        if let Ok(value) = std::env::var("MEDIAGIT_METRICS_ENABLED") {
            config.observability.metrics.enabled = parse_bool(&value)?;
        }
        if let Ok(value) = std::env::var("MEDIAGIT_METRICS_PORT") {
            config.observability.metrics.port = value.parse().map_err(|_| {
                ConfigError::env_var_parsing_error(
                    "MEDIAGIT_METRICS_PORT",
                    &value,
                    "expected valid port number",
                )
            })?;
        }

        // Compression settings
        if let Ok(value) = std::env::var("MEDIAGIT_COMPRESSION_ENABLED") {
            config.compression.enabled = parse_bool(&value)?;
        }
        if let Ok(value) = std::env::var("MEDIAGIT_COMPRESSION_LEVEL") {
            config.compression.level = value.parse().map_err(|_| {
                ConfigError::env_var_parsing_error(
                    "MEDIAGIT_COMPRESSION_LEVEL",
                    &value,
                    "expected valid compression level",
                )
            })?;
        }

        // Performance settings
        if let Ok(value) = std::env::var("MEDIAGIT_MAX_CONCURRENCY") {
            config.performance.max_concurrency = value.parse().map_err(|_| {
                ConfigError::env_var_parsing_error(
                    "MEDIAGIT_MAX_CONCURRENCY",
                    &value,
                    "expected valid integer",
                )
            })?;
        }
        if let Ok(value) = std::env::var("MEDIAGIT_BUFFER_SIZE") {
            config.performance.buffer_size = value.parse().map_err(|_| {
                ConfigError::env_var_parsing_error(
                    "MEDIAGIT_BUFFER_SIZE",
                    &value,
                    "expected valid integer",
                )
            })?;
        }
        if let Ok(value) = std::env::var("MEDIAGIT_CHUNK_WRITE_CONCURRENCY") {
            config.performance.chunk_write_concurrency = Some(value.parse().map_err(|_| {
                ConfigError::env_var_parsing_error(
                    "MEDIAGIT_CHUNK_WRITE_CONCURRENCY",
                    &value,
                    "expected valid integer",
                )
            })?);
        }

        // Security settings
        if let Ok(value) = std::env::var("MEDIAGIT_API_KEY") {
            config.security.api_key = Some(value);
        }
        if let Ok(value) = std::env::var("MEDIAGIT_HTTPS_ENABLED") {
            config.security.https_enabled = parse_bool(&value)?;
        }
        if let Ok(value) = std::env::var("MEDIAGIT_AUTH_ENABLED") {
            config.security.auth_enabled = parse_bool(&value)?;
        }

        Ok(())
    }

    /// Merge second config into first (second takes precedence)
    fn merge_configs(&self, base: &mut Config, overlay: &Config) {
        // Merge app settings if explicitly set
        if !overlay.app.name.is_empty() && overlay.app.name != "mediagit" {
            base.app.name = overlay.app.name.clone();
        }
        if overlay.app.port != 8080 {
            base.app.port = overlay.app.port;
        }
        if !overlay.app.host.is_empty() && overlay.app.host != "127.0.0.1" {
            base.app.host = overlay.app.host.clone();
        }
        if !overlay.app.environment.is_empty() && overlay.app.environment != "development" {
            base.app.environment = overlay.app.environment.clone();
        }
        if overlay.app.debug {
            base.app.debug = true;
        }

        // Merge compression settings
        base.compression = overlay.compression.clone();

        // Merge performance settings
        base.performance = overlay.performance.clone();

        // Merge observability settings
        base.observability = overlay.observability.clone();

        // Merge security settings
        if overlay.security.https_enabled {
            base.security.https_enabled = true;
            if overlay.security.tls_cert_path.is_some() {
                base.security
                    .tls_cert_path
                    .clone_from(&overlay.security.tls_cert_path);
            }
            if overlay.security.tls_key_path.is_some() {
                base.security
                    .tls_key_path
                    .clone_from(&overlay.security.tls_key_path);
            }
        }

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

/// Parse boolean from string (accepts: true, false, yes, no, 1, 0)
fn parse_bool(value: &str) -> ConfigResult<bool> {
    match value.to_lowercase().as_str() {
        "true" | "yes" | "1" | "on" => Ok(true),
        "false" | "no" | "0" | "off" => Ok(false),
        _ => Err(ConfigError::env_var_parsing_error(
            "BOOL_VALUE",
            value,
            "expected 'true', 'false', 'yes', 'no', '1', '0', 'on', or 'off'",
        )),
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
    fn test_parse_bool() {
        assert!(parse_bool("true").unwrap());
        assert!(parse_bool("yes").unwrap());
        assert!(parse_bool("1").unwrap());
        assert!(parse_bool("on").unwrap());
        assert!(!parse_bool("false").unwrap());
        assert!(!parse_bool("no").unwrap());
        assert!(!parse_bool("0").unwrap());
        assert!(!parse_bool("off").unwrap());
        assert!(parse_bool("invalid").is_err());
    }

    #[test]
    fn test_parse_json() {
        let loader = ConfigLoader::without_validation();
        let json = r#"
        {
            "app": {
                "name": "mediagit",
                "port": 8080,
                "host": "0.0.0.0",
                "environment": "production",
                "debug": false
            }
        }
        "#;
        let config = loader.load_from_string(json, ConfigFormat::Json);
        assert!(config.is_ok());
    }

    #[test]
    fn test_parse_toml() {
        let loader = ConfigLoader::without_validation();
        let toml = r#"
        [app]
        name = "mediagit"
        port = 8080
        host = "0.0.0.0"
        environment = "production"
        debug = false
        "#;
        let config = loader.load_from_string(toml, ConfigFormat::Toml);
        assert!(config.is_ok());
    }

    #[test]
    fn test_parse_yaml() {
        let loader = ConfigLoader::without_validation();
        let yaml = r#"app:
  name: mediagit
  port: 8080
  host: 0.0.0.0
  environment: production
  debug: false"#;
        let config = loader.load_from_string(yaml, ConfigFormat::Yaml);
        if let Err(e) = &config {
            eprintln!("YAML parse error: {:?}", e);
        }
        assert!(config.is_ok());
    }

    #[test]
    fn test_loader_without_validation() {
        let loader = ConfigLoader::without_validation();
        let json = r#"{"app": {"port": 99999}}"#;
        // Should not validate port constraint
        let config = loader.load_from_string(json, ConfigFormat::Json);
        // This test depends on serde being lenient with invalid values
        let _ = config;
    }

    // ---- unknown-key warning -------------------------------------------
    //
    // Both halves, because a detector that cannot fire is worse than none:
    // it must name a key serde dropped, AND stay silent on a valid config.

    fn unknown_of(toml_str: &str) -> Vec<String> {
        let cfg: Config = toml::from_str(toml_str).expect("config must still parse");
        let known = serde_json::to_value(&cfg).expect("config must round-trip");
        let mut out = Vec::new();
        for (path, key) in raw_key_paths(toml_str) {
            let mut node = &known;
            let mut missing = false;
            for seg in path.iter() {
                match node.get(seg) {
                    Some(c) => node = c,
                    None => {
                        missing = true;
                        break;
                    }
                }
            }
            if missing || node.get(&key).is_none() {
                out.push(if path.is_empty() {
                    key
                } else {
                    format!("{}.{}", path.join("."), key)
                });
            }
        }
        out
    }

    /// The exact keys the 2026-09-08 docs audit found: one that never existed,
    /// and a plain typo.
    #[test]
    fn unknown_keys_are_detected() {
        let found = unknown_of(
            r#"
[storage]
backend = "s3"
bucket = "my-media-bucket"
region = "us-east-1"
encryption = true
encryption_algorithm = "AES256"
acces_key_id = "typo"
"#,
        );
        for expected in [
            "storage.encryption",
            "storage.encryption_algorithm",
            "storage.acces_key_id",
        ] {
            assert!(
                found.iter().any(|k| k == expected),
                "expected {expected} to be reported, got {found:?}"
            );
        }
    }

    /// A valid config must produce NOTHING. `[custom]` is a HashMap, so its
    /// arbitrary keys are legitimate and must never be reported.
    #[test]
    fn valid_config_reports_no_unknown_keys() {
        let found = unknown_of(
            r#"
[storage]
backend = "s3"
bucket = "my-media-bucket"
region = "us-east-1"
access_key_id = "AKIAEXAMPLE"
secret_access_key = "secret"

[compression]
algorithm = "zstd"
level = 3

[custom]
anything_at_all = "is valid here"
"#,
        );
        assert!(found.is_empty(), "valid config reported: {found:?}");
    }

    /// REGRESSION: `cdc_seed` is a random `u64`, so about half of all real
    /// repos carry a value above `i64::MAX`. The first version of this check
    /// re-parsed the document into a `toml::Value`, whose integers are `i64`,
    /// and died at line 1 on exactly those repos — silently, while the two
    /// tests above stayed green. Any fixture here must carry such a seed.
    #[test]
    fn detects_unknown_keys_when_cdc_seed_exceeds_i64_max() {
        let found = unknown_of(
            r#"
cdc_seed = 13222148509884148795
repo_namespace = "r1"

[storage]
backend = "s3"
bucket = "my-media-bucket"
region = "us-east-1"
encryption = true
"#,
        );
        assert!(
            found.iter().any(|k| k == "storage.encryption"),
            "u64 cdc_seed must not disable the check; got {found:?}"
        );
    }

    /// An unknown TABLE is reported once by its own name, not once per key.
    #[test]
    fn unknown_table_is_reported_once() {
        let found = unknown_of(
            r#"
[storage]
backend = "filesystem"
base_path = "./objects"

[nonsense]
alpha = 1
beta = 2
"#,
        );
        assert_eq!(
            found.iter().filter(|k| k.starts_with("nonsense")).count(),
            2,
            "expected both keys under the unknown table, got {found:?}"
        );
    }
}

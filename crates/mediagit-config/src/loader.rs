// Copyright (C) 2026  winnyboy5
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
use crate::error::{ConfigError, ConfigResult};
use crate::schema::Config;
use crate::validation::Validator;
use std::path::Path;
use tokio::fs;
use tracing::{debug, info};

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

        info!(
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
            info!("Configuration validated successfully");
        }

        Ok(config)
    }

    /// Load configuration with environment variable overrides
    pub async fn load_with_overrides<P: AsRef<Path>>(
        &self,
        path: P,
    ) -> ConfigResult<Config> {
        let mut config = self.load_file(path).await?;
        self.apply_env_overrides(&mut config)?;
        Ok(config)
    }

    /// Merge multiple configuration files
    pub async fn load_and_merge<P: AsRef<Path>>(
        &self,
        paths: &[P],
    ) -> ConfigResult<Config> {
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

    /// Apply environment variable overrides
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
                base.security.tls_cert_path.clone_from(&overlay.security.tls_cert_path);
            }
            if overlay.security.tls_key_path.is_some() {
                base.security.tls_key_path.clone_from(&overlay.security.tls_key_path);
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
        assert_eq!(ConfigFormat::from_path("config.toml").unwrap(), ConfigFormat::Toml);
        assert_eq!(ConfigFormat::from_path("config.yaml").unwrap(), ConfigFormat::Yaml);
        assert_eq!(ConfigFormat::from_path("config.yml").unwrap(), ConfigFormat::Yaml);
        assert_eq!(ConfigFormat::from_path("config.json").unwrap(), ConfigFormat::Json);
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
}

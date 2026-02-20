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
//! Configuration management system for MediaGit Core
//!
//! This crate provides a comprehensive configuration management system with support for
//! multiple formats (TOML, YAML, JSON), environment variable overrides, validation,
//! and migration capabilities.
//!
//! # Features
//!
//! - Multi-format configuration support (TOML, YAML, JSON)
//! - Environment variable overrides with `MEDIAGIT_` prefix
//! - Comprehensive configuration validation with detailed error messages
//! - Configuration migration framework for version upgrades
//! - Support for storage backends (filesystem, S3, Azure, GCS, multi-backend)
//! - Performance tuning options (caching, connection pooling, timeouts)
//! - Security settings (TLS, encryption, rate limiting, CORS)
//! - Observability configuration (logging, metrics, tracing)
//!
//! # Example
//!
//! ```no_run
//! use mediagit_config::ConfigLoader;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let loader = ConfigLoader::new();
//!     let config = loader.load_with_overrides("config.toml").await?;
//!
//!     println!("Loaded configuration for: {}", config.app.name);
//!     println!("Running on: {}:{}", config.app.host, config.app.port);
//!
//!     Ok(())
//! }
//! ```

pub mod error;
pub mod loader;
pub mod migration;
pub mod schema;
pub mod validation;

// Re-export commonly used items
pub use error::{ConfigError, ConfigResult};
pub use loader::{ConfigFormat, ConfigLoader};
pub use migration::{ConfigMigration, MigrationManager, MigrationV0ToV1, CONFIG_VERSION};
pub use schema::*;
pub use validation::Validator;

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_creation() {
        let config = Config::default();
        assert_eq!(config.app.name, "mediagit");
        assert_eq!(config.app.port, 8080);
    }

    #[test]
    fn test_config_serialization() {
        let config = Config::default();
        let json = serde_json::to_string_pretty(&config).unwrap();
        assert!(json.contains("mediagit"));
    }

    #[test]
    fn test_config_validation() {
        let config = Config::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_valid_app_port() {
        let config = Config::default();
        // port is u16, so it defaults to 8080 which is valid
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_invalid_log_level() {
        let mut config = Config::default();
        config.observability.log_level = "invalid_level".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_custom_settings() {
        let mut config = Config::default();
        config.custom.insert(
            "custom_key".to_string(),
            serde_json::json!("custom_value"),
        );

        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("custom_key"));
    }
}

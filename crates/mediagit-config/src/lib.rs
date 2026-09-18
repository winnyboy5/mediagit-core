// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

#![allow(missing_docs)]
//! The client's per-repository `.mediagit/config.toml`.
//!
//! **Scope, stated up front because getting it wrong is what produced four
//! separate dead-config incidents in this crate:** this type is the *client's*
//! per-repo config, read by `mediagit-cli` and, for `performance.pack_workers`,
//! by the server when it opens a repo. It is **not** the server's own
//! configuration — that is `mediagit_server::ServerConfig`, loaded from
//! `mediagit-server.toml`, and nothing in this crate reaches it. A setting that
//! belongs to the server does not work here no matter how plausibly it is
//! named. See the tombstone on [`Config`].
//!
//! # What this crate does
//!
//! - Parses TOML, YAML and JSON into one [`Config`] type
//! - **Rejects unrecognised keys** rather than discarding them silently; put
//!   anything the schema does not define under `[custom]`
//! - Validates storage settings — bucket naming rules, octal file permissions,
//!   Azure credential shape
//! - Migrates older `config_version`s forward, with a backup, on load
//! - Describes storage backends: filesystem, S3, Azure, GCS, multi-backend
//!
//! # What it does not do
//!
//! - **No environment-variable overrides.** This crate reads no environment
//!   variables at all. `MEDIAGIT_UPLOAD_CONCURRENCY` and friends are read by
//!   the consumers of these fields, which treat a `None` here as "fall back to
//!   the env var, then to an internal default".
//! - No caching, connection-pool, timeout, TLS, CORS, rate-limit or logging
//!   settings. Those either live in `ServerConfig`, are env knobs in
//!   `mediagit-storage`, or do not exist.
//!
//! # Example
//!
//! ```no_run
//! use mediagit_config::Config;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Migrates and validates on the way in.
//!     let config = Config::load(".").await?;
//!
//!     println!("storage: {:?}", config.storage);
//!     println!("remotes: {:?}", config.list_remotes());
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
pub use migration::{
    CONFIG_VERSION, ConfigMigration, MigrationManager, MigrationV0ToV1, MigrationV1ToV2,
    MigrationV2ToV3, MigrationV3ToV4,
};
pub use schema::*;
pub use validation::Validator;

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_creation() {
        let config = Config::default();
        assert!(matches!(config.storage, StorageConfig::FileSystem(_)));
        assert_eq!(config.config_version, CONFIG_VERSION);
    }

    #[test]
    fn test_config_serialization() {
        let config = Config::default();
        let json = serde_json::to_string_pretty(&config).unwrap();
        assert!(json.contains("filesystem"));
    }

    #[test]
    fn test_config_validation() {
        let config = Config::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_custom_settings() {
        let mut config = Config::default();
        config
            .custom
            .insert("custom_key".to_string(), serde_json::json!("custom_value"));

        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("custom_key"));
    }
}

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

use crate::error::{ConfigError, ConfigResult};
use crate::schema::*;
use std::path::Path;

/// Validator for configuration settings
pub trait Validator {
    fn validate(&self) -> ConfigResult<()>;
}

impl Validator for Config {
    fn validate(&self) -> ConfigResult<()> {
        self.app.validate()?;
        self.storage.validate()?;
        self.compression.validate()?;
        self.performance.validate()?;
        self.observability.validate()?;
        self.security.validate()?;
        Ok(())
    }
}

impl Validator for AppConfig {
    fn validate(&self) -> ConfigResult<()> {
        if self.name.is_empty() {
            return Err(ConfigError::MissingRequired("app.name".to_string()));
        }

        if self.port == 0 {
            return Err(ConfigError::invalid_value(
                "app.port",
                format!("port must be between 1 and 65535, got {}", self.port),
            ));
        }

        if self.host.is_empty() {
            return Err(ConfigError::MissingRequired("app.host".to_string()));
        }

        let valid_environments = ["development", "staging", "production"];
        if !valid_environments.contains(&self.environment.as_str()) {
            return Err(ConfigError::invalid_value(
                "app.environment",
                format!("must be one of: {}", valid_environments.join(", ")),
            ));
        }

        Ok(())
    }
}

impl Validator for StorageConfig {
    fn validate(&self) -> ConfigResult<()> {
        match self {
            StorageConfig::FileSystem(fs) => fs.validate(),
            StorageConfig::S3(s3) => s3.validate(),
            StorageConfig::Azure(azure) => azure.validate(),
            StorageConfig::GCS(gcs) => gcs.validate(),
            StorageConfig::Multi(_) => Err(ConfigError::invalid_value(
                "storage.backend",
                "storage type 'multi' is not supported",
            )),
        }
    }
}

impl Validator for FileSystemStorage {
    fn validate(&self) -> ConfigResult<()> {
        if self.base_path.is_empty() {
            return Err(ConfigError::MissingRequired(
                "storage.base_path".to_string(),
            ));
        }

        // Validate file permissions format (octal)
        if !is_valid_octal(&self.file_permissions) {
            return Err(ConfigError::invalid_value(
                "storage.file_permissions",
                format!("must be valid octal, got {}", self.file_permissions),
            ));
        }

        Ok(())
    }
}

impl Validator for S3Storage {
    fn validate(&self) -> ConfigResult<()> {
        if self.bucket.is_empty() {
            return Err(ConfigError::MissingRequired("storage.bucket".to_string()));
        }

        if self.region.is_empty() {
            return Err(ConfigError::MissingRequired("storage.region".to_string()));
        }

        // S3 bucket names must be 3-63 characters long
        if self.bucket.len() < 3 || self.bucket.len() > 63 {
            return Err(ConfigError::invalid_value(
                "storage.bucket",
                "bucket name must be 3-63 characters long",
            ));
        }

        // Validate bucket name format
        if !self
            .bucket
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.')
        {
            return Err(ConfigError::invalid_value(
                "storage.bucket",
                "bucket name must contain only lowercase letters, digits, hyphens, and dots",
            ));
        }

        // Validate encryption algorithm (it's a String, not Option<String>)
        if self.encryption_algorithm != "AES256"
            && self.encryption_algorithm != "aws:kms"
            && !self.encryption_algorithm.starts_with("aws:kms:")
        {
            return Err(ConfigError::invalid_value(
                "storage.encryption_algorithm",
                format!("unsupported algorithm: {}", self.encryption_algorithm),
            ));
        }

        Ok(())
    }
}

/// Actionable message for a config still using the pre-v3 flat Azure shape.
///
/// Deliberately shows the replacement block verbatim: a user hitting this is
/// mid-outage with a server that will not start, and "invalid config" without
/// the fix is not help.
fn azure_legacy_shape_error(legacy: &LegacyAzureFields) -> ConfigError {
    let suggested = if legacy.connection_string.is_some() {
        "auth = { type = \"connection_string\", value = \"<your connection string>\" }"
    } else {
        "auth = { type = \"account_key\", account_name = \"<name>\", account_key = \"<key>\" }"
    };
    ConfigError::ValidationError(format!(
        "Azure storage config uses the removed flat format \
         (account_name/account_key/connection_string at the top level).\n\
         Replace those keys with an `auth` block:\n\n    {suggested}\n\n\
         Other variants: {{ type = \"sas\", account_name = \"<name>\", token = \"<sas>\" }} \
         or {{ type = \"emulator\" }} for local Azurite.\n\
         Configs carrying `config_version` are migrated automatically; this error means \
         the version was absent or already current while the keys were still flat."
    ))
}

impl Validator for AzureStorage {
    fn validate(&self) -> ConfigResult<()> {
        // Legacy shape is checked first: it produces a fix, where the generic
        // "missing auth" below would only produce a complaint.
        if self.legacy.is_present() {
            return Err(azure_legacy_shape_error(&self.legacy));
        }

        let Some(auth) = &self.auth else {
            return Err(ConfigError::MissingRequired("storage.auth".to_string()));
        };

        if self.container.is_empty() {
            return Err(ConfigError::MissingRequired(
                "storage.container".to_string(),
            ));
        }

        // Azure container names must be 3-63 characters
        if self.container.len() < 3 || self.container.len() > 63 {
            return Err(ConfigError::invalid_value(
                "storage.container",
                "container name must be 3-63 characters long",
            ));
        }

        // Per-variant emptiness. "Which credential" is now the type system's
        // job — this only catches present-but-blank values.
        match auth {
            AzureAuth::AccountKey {
                account_name,
                account_key,
            } => {
                if account_name.is_empty() {
                    return Err(ConfigError::MissingRequired(
                        "storage.auth.account_name".to_string(),
                    ));
                }
                if account_key.is_empty() {
                    return Err(ConfigError::MissingRequired(
                        "storage.auth.account_key".to_string(),
                    ));
                }
            }
            AzureAuth::ConnectionString { value } => {
                if value.is_empty() {
                    return Err(ConfigError::MissingRequired(
                        "storage.auth.value".to_string(),
                    ));
                }
            }
            AzureAuth::Sas {
                account_name,
                token,
            } => {
                if account_name.is_empty() {
                    return Err(ConfigError::MissingRequired(
                        "storage.auth.account_name".to_string(),
                    ));
                }
                if token.is_empty() {
                    return Err(ConfigError::MissingRequired(
                        "storage.auth.token".to_string(),
                    ));
                }
            }
            // Emulator uses well-known development credentials; nothing to check.
            AzureAuth::Emulator => {}
        }

        Ok(())
    }
}

impl Validator for GCSStorage {
    fn validate(&self) -> ConfigResult<()> {
        if self.bucket.is_empty() {
            return Err(ConfigError::MissingRequired("storage.bucket".to_string()));
        }

        if self.project_id.is_empty() {
            return Err(ConfigError::MissingRequired(
                "storage.project_id".to_string(),
            ));
        }

        Ok(())
    }
}

impl Validator for MultiBackendStorage {
    fn validate(&self) -> ConfigResult<()> {
        if self.primary.is_empty() {
            return Err(ConfigError::MissingRequired("storage.primary".to_string()));
        }

        if self.backends.is_empty() {
            return Err(ConfigError::MissingRequired("storage.backends".to_string()));
        }

        if !self.backends.contains_key(&self.primary) {
            return Err(ConfigError::invalid_value(
                "storage.primary",
                format!("primary backend '{}' not found in backends", self.primary),
            ));
        }

        for replica in &self.replicas {
            if !self.backends.contains_key(replica) {
                return Err(ConfigError::invalid_value(
                    "storage.replicas",
                    format!("replica backend '{}' not found in backends", replica),
                ));
            }
        }

        Ok(())
    }
}

impl Validator for CompressionConfig {
    fn validate(&self) -> ConfigResult<()> {
        if self.enabled {
            // Validate level based on algorithm
            match self.algorithm {
                CompressionAlgorithm::Zstd => {
                    if self.level < 1 || self.level > 22 {
                        return Err(ConfigError::invalid_value(
                            "compression.level",
                            "zstd level must be between 1 and 22",
                        ));
                    }
                }
                CompressionAlgorithm::Brotli => {
                    if self.level > 11 {
                        return Err(ConfigError::invalid_value(
                            "compression.level",
                            "brotli level must be between 0 and 11",
                        ));
                    }
                }
                CompressionAlgorithm::None => {
                    // No validation needed
                }
            }
        }

        // Validate algorithm configs
        for (algo_name, algo_config) in &self.algorithms {
            if let Some(level) = algo_config.level {
                match algo_name.as_str() {
                    "zstd" => {
                        if !(1..=22).contains(&level) {
                            return Err(ConfigError::invalid_value(
                                format!("compression.algorithms.{}.level", algo_name),
                                "zstd level must be between 1 and 22",
                            ));
                        }
                    }
                    "brotli" => {
                        if level > 11 {
                            return Err(ConfigError::invalid_value(
                                format!("compression.algorithms.{}.level", algo_name),
                                "brotli level must be between 0 and 11",
                            ));
                        }
                    }
                    _ => {
                        return Err(ConfigError::invalid_value(
                            "compression.algorithms",
                            format!("unknown algorithm: {}", algo_name),
                        ));
                    }
                }
            }
        }

        Ok(())
    }
}

impl Validator for PerformanceConfig {
    fn validate(&self) -> ConfigResult<()> {
        if self.max_concurrency == 0 {
            return Err(ConfigError::invalid_value(
                "performance.max_concurrency",
                "must be greater than 0",
            ));
        }

        if self.buffer_size == 0 {
            return Err(ConfigError::invalid_value(
                "performance.buffer_size",
                "must be greater than 0",
            ));
        }

        self.cache.validate()?;
        self.connection_pool.validate()?;
        self.timeouts.validate()?;

        Ok(())
    }
}

impl Validator for CacheConfig {
    fn validate(&self) -> ConfigResult<()> {
        if self.enabled {
            let valid_types = ["memory", "disk", "redis"];
            if !valid_types.contains(&self.cache_type.as_str()) {
                return Err(ConfigError::invalid_value(
                    "cache.cache_type",
                    format!("must be one of: {}", valid_types.join(", ")),
                ));
            }

            if self.max_size == 0 {
                return Err(ConfigError::invalid_value(
                    "cache.max_size",
                    "must be greater than 0",
                ));
            }

            if self.ttl == 0 {
                return Err(ConfigError::invalid_value(
                    "cache.ttl",
                    "must be greater than 0",
                ));
            }
        }

        Ok(())
    }
}

impl Validator for ConnectionPoolConfig {
    fn validate(&self) -> ConfigResult<()> {
        if self.max_connections == 0 {
            return Err(ConfigError::invalid_value(
                "connection_pool.max_connections",
                "must be greater than 0",
            ));
        }

        if self.min_connections > self.max_connections {
            return Err(ConfigError::ConflictingValues(
                "min_connections cannot be greater than max_connections".to_string(),
            ));
        }

        if self.timeout == 0 {
            return Err(ConfigError::invalid_value(
                "connection_pool.timeout",
                "must be greater than 0",
            ));
        }

        Ok(())
    }
}

impl Validator for TimeoutConfig {
    fn validate(&self) -> ConfigResult<()> {
        let fields = [
            ("request", self.request),
            ("read", self.read),
            ("write", self.write),
            ("connection", self.connection),
        ];

        for (name, value) in fields.iter() {
            if *value == 0 {
                return Err(ConfigError::invalid_value(
                    format!("timeouts.{}", name),
                    "must be greater than 0",
                ));
            }
        }

        Ok(())
    }
}

impl Validator for ObservabilityConfig {
    fn validate(&self) -> ConfigResult<()> {
        let valid_levels = ["debug", "info", "warn", "error", "trace"];
        if !valid_levels.contains(&self.log_level.as_str()) {
            return Err(ConfigError::invalid_value(
                "observability.log_level",
                format!("must be one of: {}", valid_levels.join(", ")),
            ));
        }

        let valid_formats = ["json", "text"];
        if !valid_formats.contains(&self.log_format.as_str()) {
            return Err(ConfigError::invalid_value(
                "observability.log_format",
                format!("must be one of: {}", valid_formats.join(", ")),
            ));
        }

        if self.sample_rate < 0.0 || self.sample_rate > 1.0 {
            return Err(ConfigError::invalid_value(
                "observability.sample_rate",
                "must be between 0.0 and 1.0",
            ));
        }

        self.metrics.validate()?;

        Ok(())
    }
}

impl Validator for MetricsConfig {
    fn validate(&self) -> ConfigResult<()> {
        if self.enabled {
            if self.port == 0 {
                return Err(ConfigError::invalid_value(
                    "metrics.port",
                    format!("port must be between 1 and 65535, got {}", self.port),
                ));
            }

            if self.endpoint.is_empty() {
                return Err(ConfigError::MissingRequired("metrics.endpoint".to_string()));
            }

            if !self.endpoint.starts_with('/') {
                return Err(ConfigError::invalid_value(
                    "metrics.endpoint",
                    "must start with /",
                ));
            }

            if self.interval == 0 {
                return Err(ConfigError::invalid_value(
                    "metrics.interval",
                    "must be greater than 0",
                ));
            }
        }

        Ok(())
    }
}

impl Validator for SecurityConfig {
    fn validate(&self) -> ConfigResult<()> {
        if self.https_enabled {
            if self.tls_cert_path.is_none() {
                return Err(ConfigError::MissingRequired(
                    "security.tls_cert_path".to_string(),
                ));
            }

            if self.tls_key_path.is_none() {
                return Err(ConfigError::MissingRequired(
                    "security.tls_key_path".to_string(),
                ));
            }

            // Validate paths exist
            if let Some(cert_path) = &self.tls_cert_path
                && !Path::new(cert_path).exists()
            {
                return Err(ConfigError::FileNotFound(cert_path.clone().into()));
            }

            if let Some(key_path) = &self.tls_key_path
                && !Path::new(key_path).exists()
            {
                return Err(ConfigError::FileNotFound(key_path.clone().into()));
            }
        }

        if self.encryption_at_rest {
            if self.encryption_key_path.is_none() {
                return Err(ConfigError::MissingRequired(
                    "security.encryption_key_path".to_string(),
                ));
            }

            if let Some(key_path) = &self.encryption_key_path
                && !Path::new(key_path).exists()
            {
                return Err(ConfigError::FileNotFound(key_path.clone().into()));
            }
        }

        self.rate_limiting.validate()?;

        Ok(())
    }
}

impl Validator for RateLimitConfig {
    fn validate(&self) -> ConfigResult<()> {
        if self.enabled {
            if self.requests_per_second == 0 {
                return Err(ConfigError::invalid_value(
                    "rate_limiting.requests_per_second",
                    "must be greater than 0",
                ));
            }

            if self.burst_size < self.requests_per_second {
                return Err(ConfigError::ConflictingValues(
                    "burst_size must be at least equal to requests_per_second".to_string(),
                ));
            }
        }

        Ok(())
    }
}

/// Helper function to validate octal string format
fn is_valid_octal(s: &str) -> bool {
    if s.starts_with('0') && s.len() == 4 {
        s[1..].chars().all(|c| ('0'..='7').contains(&c))
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_config() {
        let config = Config::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_valid_port() {
        let config = Config::default();
        // Port is u16 and default is 8080, so it's valid
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_invalid_environment() {
        let mut config = Config::default();
        config.app.environment = "invalid".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_invalid_octal_permissions() {
        let mut config = Config::default();
        if let StorageConfig::FileSystem(fs) = &mut config.storage {
            fs.file_permissions = "644".to_string();
        }
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_compression_level_validation() {
        let mut config = Config::default();
        config.compression.level = 30;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_cache_validation() {
        let mut config = Config::default();
        config.performance.cache.cache_type = "invalid".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_log_level_validation() {
        let mut config = Config::default();
        config.observability.log_level = "invalid".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_multi_backend_storage_rejected() {
        let config = Config {
            storage: StorageConfig::Multi(MultiBackendStorage {
                primary: "s3".to_string(),
                replicas: vec![],
                backends: Default::default(),
            }),
            ..Config::default()
        };
        let err = config
            .validate()
            .expect_err("multi backend should be rejected");
        assert!(
            err.to_string()
                .contains("storage type 'multi' is not supported")
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod azure_auth_tests {
    use crate::schema::{AzureAuth, AzureStorage, LegacyAzureFields};
    use crate::validation::Validator;

    fn azure(auth: Option<AzureAuth>) -> AzureStorage {
        AzureStorage {
            container: "media".to_string(),
            prefix: String::new(),
            auth,
            legacy: LegacyAzureFields::default(),
        }
    }

    #[test]
    fn each_auth_variant_validates() {
        for auth in [
            AzureAuth::AccountKey {
                account_name: "acct".into(),
                account_key: "key".into(),
            },
            AzureAuth::ConnectionString {
                value: "AccountName=x;AccountKey=y;".into(),
            },
            AzureAuth::Sas {
                account_name: "acct".into(),
                token: "sv=2022-11-02&sig=x".into(),
            },
            // Emulator carries no credentials by design.
            AzureAuth::Emulator,
        ] {
            assert!(azure(Some(auth.clone())).validate().is_ok(), "{auth:?}");
        }
    }

    #[test]
    fn legacy_flat_shape_reports_the_replacement_block() {
        // A user hitting this has a server that will not start; the message
        // must contain the fix, not just a complaint.
        let mut cfg = azure(None);
        cfg.legacy = LegacyAzureFields {
            account_name: Some("acct".into()),
            account_key: Some("key".into()),
            connection_string: None,
        };
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("removed flat format"), "got: {err}");
        assert!(
            err.contains("auth = { type = \"account_key\""),
            "got: {err}"
        );
    }

    #[test]
    fn legacy_connection_string_suggests_that_variant() {
        let mut cfg = azure(None);
        cfg.legacy = LegacyAzureFields {
            connection_string: Some("AccountName=x;".into()),
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("type = \"connection_string\""), "got: {err}");
    }

    #[test]
    fn missing_auth_is_rejected() {
        assert!(azure(None).validate().is_err());
    }

    #[test]
    fn blank_credential_values_are_rejected() {
        // The enum makes "which credential" unrepresentable; validation only
        // has to catch present-but-empty.
        let cases = [
            AzureAuth::AccountKey {
                account_name: String::new(),
                account_key: "k".into(),
            },
            AzureAuth::AccountKey {
                account_name: "a".into(),
                account_key: String::new(),
            },
            AzureAuth::ConnectionString {
                value: String::new(),
            },
            AzureAuth::Sas {
                account_name: "a".into(),
                token: String::new(),
            },
        ];
        for auth in cases {
            assert!(azure(Some(auth.clone())).validate().is_err(), "{auth:?}");
        }
    }

    #[test]
    fn container_name_rules_still_apply() {
        let mut cfg = azure(Some(AzureAuth::Emulator));
        cfg.container = "ab".into(); // below Azure's 3-char minimum
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn auth_block_deserialises_from_toml() {
        // Proves the tag spelling users will actually type.
        let cfg: AzureStorage = toml::from_str(
            r#"
            container = "media"
            auth = { type = "account_key", account_name = "acct", account_key = "k" }
        "#,
        )
        .unwrap();
        assert!(matches!(cfg.auth, Some(AzureAuth::AccountKey { .. })));
        assert!(!cfg.legacy.is_present());
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn old_toml_is_recognised_as_legacy_not_a_parse_error() {
        let cfg: AzureStorage = toml::from_str(
            r#"
            container = "media"
            account_name = "acct"
            account_key = "k"
        "#,
        )
        .unwrap();
        assert!(
            cfg.legacy.is_present(),
            "flat keys must be captured for diagnosis"
        );
        assert!(cfg.auth.is_none());
        assert!(cfg.validate().is_err());
    }
}

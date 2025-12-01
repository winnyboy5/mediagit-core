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
                format!(
                    "must be one of: {}",
                    valid_environments.join(", ")
                ),
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
            StorageConfig::Multi(multi) => multi.validate(),
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
            return Err(ConfigError::MissingRequired(
                "storage.region".to_string(),
            ));
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

impl Validator for AzureStorage {
    fn validate(&self) -> ConfigResult<()> {
        if self.account_name.is_empty() {
            return Err(ConfigError::MissingRequired(
                "storage.account_name".to_string(),
            ));
        }

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

        // Must have either account_key or connection_string
        if self.account_key.is_none() && self.connection_string.is_none() {
            return Err(ConfigError::ValidationError(
                "Azure storage requires either account_key or connection_string".to_string(),
            ));
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
            return Err(ConfigError::MissingRequired(
                "storage.primary".to_string(),
            ));
        }

        if self.backends.is_empty() {
            return Err(ConfigError::MissingRequired(
                "storage.backends".to_string(),
            ));
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
                        if level < 1 || level > 22 {
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
                return Err(ConfigError::MissingRequired(
                    "metrics.endpoint".to_string(),
                ));
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
            if let Some(cert_path) = &self.tls_cert_path {
                if !Path::new(cert_path).exists() {
                    return Err(ConfigError::FileNotFound(cert_path.clone().into()));
                }
            }

            if let Some(key_path) = &self.tls_key_path {
                if !Path::new(key_path).exists() {
                    return Err(ConfigError::FileNotFound(key_path.clone().into()));
                }
            }
        }

        if self.encryption_at_rest {
            if self.encryption_key_path.is_none() {
                return Err(ConfigError::MissingRequired(
                    "security.encryption_key_path".to_string(),
                ));
            }

            if let Some(key_path) = &self.encryption_key_path {
                if !Path::new(key_path).exists() {
                    return Err(ConfigError::FileNotFound(key_path.clone().into()));
                }
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
        s[1..].chars().all(|c| c >= '0' && c <= '7')
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
        if let StorageConfig::FileSystem(ref mut fs) = &mut config.storage {
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
}

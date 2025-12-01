use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("IO error reading configuration file: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Failed to parse TOML configuration: {0}")]
    TomlParseError(#[from] toml::de::Error),

    #[error("Failed to parse YAML configuration: {0}")]
    YamlParseError(#[from] serde_yaml::Error),

    #[error("Failed to parse JSON configuration: {0}")]
    JsonParseError(#[from] serde_json::error::Error),

    #[error("Failed to serialize configuration: {0}")]
    SerializationError(String),

    #[error("Configuration validation failed: {0}")]
    ValidationError(String),

    #[error("Unsupported configuration format: {0}. Supported formats: toml, yaml, json")]
    UnsupportedFormat(String),

    #[error("Configuration file not found at path: {}", .0.display())]
    FileNotFound(PathBuf),

    #[error("Invalid configuration path: {}", .0.display())]
    InvalidPath(PathBuf),

    #[error("Environment variable parsing error: {variable_name}={value}. {reason}")]
    EnvVarParsingError {
        variable_name: String,
        value: String,
        reason: String,
    },

    #[error("Configuration migration failed: {0}")]
    MigrationError(String),

    #[error("Invalid configuration value for field '{field}': {reason}")]
    InvalidValue { field: String, reason: String },

    #[error("Missing required configuration field: {0}")]
    MissingRequired(String),

    #[error("Conflicting configuration values: {0}")]
    ConflictingValues(String),

    #[error("Environment variable error: {0}")]
    EnvVarError(#[from] std::env::VarError),

    #[error("Unknown error: {0}")]
    Other(String),
}

impl ConfigError {
    pub fn validation_error(message: impl Into<String>) -> Self {
        ConfigError::ValidationError(message.into())
    }

    pub fn env_var_parsing_error(
        variable_name: impl Into<String>,
        value: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        ConfigError::EnvVarParsingError {
            variable_name: variable_name.into(),
            value: value.into(),
            reason: reason.into(),
        }
    }

    pub fn invalid_value(field: impl Into<String>, reason: impl Into<String>) -> Self {
        ConfigError::InvalidValue {
            field: field.into(),
            reason: reason.into(),
        }
    }

    pub fn migration_error(message: impl Into<String>) -> Self {
        ConfigError::MigrationError(message.into())
    }
}

pub type ConfigResult<T> = Result<T, ConfigError>;

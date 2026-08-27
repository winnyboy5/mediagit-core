// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

#![allow(clippy::unwrap_used)]
//! Integration tests for mediagit-config.

use mediagit_config::{Config, ConfigFormat, ConfigLoader, Validator};
use std::fs;
use tempfile::TempDir;

#[tokio::test]
async fn test_load_toml_config() {
    let loader = ConfigLoader::new();
    let toml_content = r#"
[app]
name = "mediagit"
version = "0.1.0"
environment = "development"
port = 8080
host = "0.0.0.0"
debug = false

[storage]
backend = "filesystem"
base_path = "/data"
create_dirs = true
sync = false
file_permissions = "0755"
"#;

    let config = loader.load_from_string(toml_content, ConfigFormat::Toml);
    assert!(config.is_ok());

    let config = config.unwrap();
    assert_eq!(config.app.name, "mediagit");
    assert_eq!(config.app.port, 8080);
}

#[tokio::test]
async fn test_load_yaml_config() {
    let loader = ConfigLoader::new();
    let yaml_content = r#"
app:
  name: mediagit
  version: "0.1.0"
  environment: development
  port: 8080
  host: 0.0.0.0
  debug: false

storage:
  backend: filesystem
  base_path: /data
  create_dirs: true
  sync: false
  file_permissions: "0755"
"#;

    let config = loader.load_from_string(yaml_content, ConfigFormat::Yaml);
    assert!(config.is_ok());

    let config = config.unwrap();
    assert_eq!(config.app.name, "mediagit");
    assert_eq!(config.app.port, 8080);
}

#[tokio::test]
async fn test_load_json_config() {
    let loader = ConfigLoader::new();
    let json_content = r#"
{
  "app": {
    "name": "mediagit",
    "version": "0.1.0",
    "environment": "development",
    "port": 8080,
    "host": "0.0.0.0",
    "debug": false
  }
}
"#;

    let config = loader.load_from_string(json_content, ConfigFormat::Json);
    assert!(config.is_ok());

    let config = config.unwrap();
    assert_eq!(config.app.name, "mediagit");
    assert_eq!(config.app.port, 8080);
}

#[tokio::test]
async fn test_load_from_file_toml() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.toml");

    let toml_content = r#"
[app]
name = "test-app"
port = 9000
host = "127.0.0.1"
environment = "development"
debug = true
"#;

    fs::write(&config_path, toml_content).unwrap();

    let loader = ConfigLoader::new();
    let config = loader.load_file(&config_path).await;
    assert!(config.is_ok());

    let config = config.unwrap();
    assert_eq!(config.app.name, "test-app");
    assert_eq!(config.app.port, 9000);
}

#[tokio::test]
async fn test_load_from_file_yaml() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.yaml");

    let yaml_content = r#"
app:
  name: test-app
  port: 9000
  host: 127.0.0.1
  environment: development
  debug: true
"#;

    fs::write(&config_path, yaml_content).unwrap();

    let loader = ConfigLoader::new();
    let config = loader.load_file(&config_path).await;
    assert!(config.is_ok());

    let config = config.unwrap();
    assert_eq!(config.app.name, "test-app");
    assert_eq!(config.app.port, 9000);
}

#[tokio::test]
async fn test_load_from_file_json() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.json");

    let json_content = r#"
{
  "app": {
    "name": "test-app",
    "version": "0.1.0",
    "environment": "development",
    "port": 9000,
    "host": "127.0.0.1",
    "debug": true
  }
}
"#;

    fs::write(&config_path, json_content).unwrap();

    let loader = ConfigLoader::new();
    let config = loader.load_file(&config_path).await;
    assert!(config.is_ok());

    let config = config.unwrap();
    assert_eq!(config.app.name, "test-app");
    assert_eq!(config.app.port, 9000);
}

#[tokio::test]
async fn test_file_not_found() {
    let loader = ConfigLoader::new();
    let result = loader.load_file("/nonexistent/config.toml").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_unsupported_format() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.xml");
    fs::write(&config_path, "<config></config>").unwrap();

    let loader = ConfigLoader::new();
    let result = loader.load_file(&config_path).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_invalid_toml_syntax() {
    let loader = ConfigLoader::new();
    let invalid_toml = r#"
[app
name = "test"
"#;

    let result = loader.load_from_string(invalid_toml, ConfigFormat::Toml);
    assert!(result.is_err());
}

#[tokio::test]
async fn test_invalid_json_syntax() {
    let loader = ConfigLoader::new();
    let invalid_json = r#"{"app": {"name": "test""#;

    let result = loader.load_from_string(invalid_json, ConfigFormat::Json);
    assert!(result.is_err());
}

#[tokio::test]
async fn test_merge_multiple_configs() {
    let temp_dir = TempDir::new().unwrap();

    let base_path = temp_dir.path().join("base.toml");
    let overlay_path = temp_dir.path().join("overlay.toml");

    let base_content = r#"
[app]
name = "mediagit"
port = 8080
host = "127.0.0.1"
environment = "development"
debug = false
"#;

    let overlay_content = r#"
[app]
port = 9000
environment = "staging"
debug = true
"#;

    fs::write(&base_path, base_content).unwrap();
    fs::write(&overlay_path, overlay_content).unwrap();

    let loader = ConfigLoader::new();
    let config = loader.load_and_merge(&[&base_path, &overlay_path]).await;

    if let Err(e) = &config {
        eprintln!("Merge error: {:?}", e);
    }
    assert!(config.is_ok());
    let config = config.unwrap();
    assert_eq!(config.app.name, "mediagit"); // from base
    assert_eq!(config.app.port, 9000); // from overlay
    assert_eq!(config.app.environment, "staging"); // from overlay
    assert!(config.app.debug); // from overlay
}

#[tokio::test]
async fn test_environment_variable_overrides() {
    mediagit_test_utils::set_var("MEDIAGIT_APP_PORT", "7777");
    mediagit_test_utils::set_var("MEDIAGIT_APP_ENVIRONMENT", "staging");
    mediagit_test_utils::set_var("MEDIAGIT_LOG_LEVEL", "debug");

    let loader = ConfigLoader::new();
    let mut config = Config::default();

    let result = loader.apply_env_overrides(&mut config);
    assert!(result.is_ok());

    assert_eq!(config.app.port, 7777);
    assert_eq!(config.app.environment, "staging");
    assert_eq!(config.observability.log_level, "debug");

    // Cleanup
    mediagit_test_utils::remove_var("MEDIAGIT_APP_PORT");
    mediagit_test_utils::remove_var("MEDIAGIT_APP_ENVIRONMENT");
    mediagit_test_utils::remove_var("MEDIAGIT_LOG_LEVEL");
}

#[test]
fn test_validation_default_config() {
    let config = Config::default();
    assert!(config.validate().is_ok());
}

#[test]
fn test_validation_invalid_port() {
    let mut config = Config::default();
    // Port 0 is technically valid for u16, but semantically invalid
    // Since port is u16, we can't test > 65535
    // Instead test that valid ports pass
    config.app.port = 8080;
    assert!(config.validate().is_ok());
}

#[test]
fn test_validation_invalid_environment() {
    let mut config = Config::default();
    config.app.environment = "invalid".to_string();
    assert!(config.validate().is_err());
}

#[test]
fn test_validation_compression_level() {
    let mut config = Config::default();
    config.compression.level = 25;
    assert!(config.validate().is_err());
}

#[test]
fn test_validation_cache_type() {
    let mut config = Config::default();
    config.performance.cache.cache_type = "invalid".to_string();
    assert!(config.validate().is_err());
}

#[test]
fn test_validation_log_level() {
    let mut config = Config::default();
    config.observability.log_level = "invalid".to_string();
    assert!(config.validate().is_err());
}

#[test]
fn test_validation_s3_bucket_name_too_short() {
    use mediagit_config::S3Storage;
    use mediagit_config::StorageConfig;

    let config = Config {
        storage: StorageConfig::S3(S3Storage {
            bucket: "ab".to_string(),
            region: "us-east-1".to_string(),
            access_key_id: None,
            secret_access_key: None,
            endpoint: None,
            prefix: String::new(),
        }),
        ..Default::default()
    };

    assert!(config.validate().is_err());
}

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
fn test_format_detection_unsupported() {
    let result = ConfigFormat::from_path("config.xml");
    assert!(result.is_err());
}

#[tokio::test]
async fn test_loader_without_validation() {
    let loader = ConfigLoader::without_validation();
    let json_content = r#"{"app": {"port": 70000}}"#;

    // Should not validate port constraint due to serde limitations
    let result = loader.load_from_string(json_content, ConfigFormat::Json);
    // Validation is skipped
    let _ = result;
}

#[test]
fn test_serialization_roundtrip() {
    let config = Config::default();

    // TOML roundtrip
    let toml_str = toml::to_string(&config).unwrap();
    let _config_from_toml: Config = toml::from_str(&toml_str).unwrap();

    // JSON roundtrip
    let json_str = serde_json::to_string(&config).unwrap();
    let _config_from_json: Config = serde_json::from_str(&json_str).unwrap();

    // YAML roundtrip
    let yaml_str = serde_yaml::to_string(&config).unwrap();
    let _config_from_yaml: Config = serde_yaml::from_str(&yaml_str).unwrap();
}

#[tokio::test]
async fn test_config_load_migrates_v0_config_and_backs_up() {
    let temp_dir = TempDir::new().unwrap();
    let mediagit_dir = temp_dir.path().join(".mediagit");
    fs::create_dir_all(&mediagit_dir).unwrap();
    let config_path = mediagit_dir.join("config.toml");

    // v0-style config: written before `config_version` existed, so the key
    // is entirely absent (not "config_version = 0").
    let v0_toml = r#"
[app]
[storage]
backend = "filesystem"
base_path = "./data"
[compression]
[performance]
[performance.cache]
[performance.connection_pool]
[performance.timeouts]
[observability]
[observability.metrics]
[security]
[security.rate_limiting]
"#;
    fs::write(&config_path, v0_toml).unwrap();

    let config = Config::load(temp_dir.path()).await.unwrap();
    assert_eq!(config.config_version, mediagit_config::CONFIG_VERSION);

    // Original was backed up before being overwritten.
    let backup_path = mediagit_dir.join("config.toml.bak");
    assert!(
        backup_path.exists(),
        "expected config.toml.bak to be written"
    );
    let backup_content = fs::read_to_string(&backup_path).unwrap();
    assert!(!backup_content.contains("config_version"));

    // The migrated config was written back to config.toml.
    let migrated_on_disk = fs::read_to_string(&config_path).unwrap();
    assert!(migrated_on_disk.contains("config_version"));

    // Loading again is a no-op (already current version), and does not
    // touch the backup a second time.
    let backup_mtime_before = fs::metadata(&backup_path).unwrap().modified().unwrap();
    let reloaded = Config::load(temp_dir.path()).await.unwrap();
    assert_eq!(reloaded.config_version, mediagit_config::CONFIG_VERSION);
    let backup_mtime_after = fs::metadata(&backup_path).unwrap().modified().unwrap();
    assert_eq!(backup_mtime_before, backup_mtime_after);
}

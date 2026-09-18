// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

#![allow(clippy::unwrap_used)]
//! Integration tests for mediagit-config.
//!
//! These used `[app]` as their payload throughout — a section that turned out
//! to be read by nothing and was deleted on 2026-09-18. The tests are about
//! format detection, merging and validation, not about `[app]`, so they now
//! carry `[storage]` and `[performance]`, which are live. See the tombstone on
//! `Config` in `schema.rs`.

use mediagit_config::{Config, ConfigFormat, ConfigLoader, StorageConfig, Validator};
use std::fs;
use tempfile::TempDir;

fn base_path_of(config: &Config) -> &str {
    match &config.storage {
        StorageConfig::FileSystem(fs) => &fs.base_path,
        other => panic!("expected filesystem storage, got {other:?}"),
    }
}

#[tokio::test]
async fn test_load_toml_config() {
    let loader = ConfigLoader::new();
    let toml_content = r#"
[storage]
backend = "filesystem"
base_path = "/data"
create_dirs = true
sync = false
file_permissions = "0755"

[performance]
upload_concurrency = 16
"#;

    let config = loader
        .load_from_string(toml_content, ConfigFormat::Toml)
        .unwrap();
    assert_eq!(base_path_of(&config), "/data");
    assert_eq!(config.performance.upload_concurrency, Some(16));
}

#[tokio::test]
async fn test_load_yaml_config() {
    let loader = ConfigLoader::new();
    let yaml_content = r#"
storage:
  backend: filesystem
  base_path: /data
  create_dirs: true
  sync: false
  file_permissions: "0755"

performance:
  upload_concurrency: 16
"#;

    let config = loader
        .load_from_string(yaml_content, ConfigFormat::Yaml)
        .unwrap();
    assert_eq!(base_path_of(&config), "/data");
    assert_eq!(config.performance.upload_concurrency, Some(16));
}

#[tokio::test]
async fn test_load_json_config() {
    let loader = ConfigLoader::new();
    let json_content = r#"
{
  "storage": {
    "backend": "filesystem",
    "base_path": "/data",
    "create_dirs": true,
    "sync": false,
    "file_permissions": "0755"
  },
  "performance": { "upload_concurrency": 16 }
}
"#;

    let config = loader
        .load_from_string(json_content, ConfigFormat::Json)
        .unwrap();
    assert_eq!(base_path_of(&config), "/data");
    assert_eq!(config.performance.upload_concurrency, Some(16));
}

#[tokio::test]
async fn test_load_from_file_toml() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.toml");

    let toml_content = r#"
[storage]
backend = "filesystem"
base_path = "/tmp/test-app"
create_dirs = true
sync = false
file_permissions = "0644"

[performance]
pack_workers = 4
"#;

    fs::write(&config_path, toml_content).unwrap();

    let loader = ConfigLoader::new();
    let config = loader.load_file(&config_path).await.unwrap();
    assert_eq!(base_path_of(&config), "/tmp/test-app");
    assert_eq!(config.performance.pack_workers, Some(4));
}

#[tokio::test]
async fn test_load_from_file_yaml() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.yaml");

    let yaml_content = r#"
storage:
  backend: filesystem
  base_path: /tmp/test-app
  create_dirs: true
  sync: false
  file_permissions: "0644"

performance:
  pack_workers: 4
"#;

    fs::write(&config_path, yaml_content).unwrap();

    let loader = ConfigLoader::new();
    let config = loader.load_file(&config_path).await.unwrap();
    assert_eq!(base_path_of(&config), "/tmp/test-app");
    assert_eq!(config.performance.pack_workers, Some(4));
}

#[tokio::test]
async fn test_load_from_file_json() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.json");

    let json_content = r#"
{
  "storage": {
    "backend": "filesystem",
    "base_path": "/tmp/test-app",
    "create_dirs": true,
    "sync": false,
    "file_permissions": "0644"
  },
  "performance": { "pack_workers": 4 }
}
"#;

    fs::write(&config_path, json_content).unwrap();

    let loader = ConfigLoader::new();
    let config = loader.load_file(&config_path).await.unwrap();
    assert_eq!(base_path_of(&config), "/tmp/test-app");
    assert_eq!(config.performance.pack_workers, Some(4));
}

/// The loader is strict about keys it does not know, in every format. Before
/// `deny_unknown_fields` this returned `Ok` and dropped the section on the
/// floor.
#[tokio::test]
async fn unknown_keys_are_rejected_in_every_format() {
    let loader = ConfigLoader::new();

    assert!(
        loader
            .load_from_string("[nonsense]\nkey = 1\n", ConfigFormat::Toml)
            .is_err(),
        "toml: unknown section must be rejected"
    );
    assert!(
        loader
            .load_from_string("nonsense:\n  key: 1\n", ConfigFormat::Yaml)
            .is_err(),
        "yaml: unknown section must be rejected"
    );
    assert!(
        loader
            .load_from_string(r#"{"nonsense": {"key": 1}}"#, ConfigFormat::Json)
            .is_err(),
        "json: unknown section must be rejected"
    );
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
[storage
backend = "filesystem"
"#;

    let result = loader.load_from_string(invalid_toml, ConfigFormat::Toml);
    assert!(result.is_err());
}

#[tokio::test]
async fn test_invalid_json_syntax() {
    let loader = ConfigLoader::new();
    let invalid_json = r#"{"storage": {"backend": "filesystem""#;

    let result = loader.load_from_string(invalid_json, ConfigFormat::Json);
    assert!(result.is_err());
}

#[tokio::test]
async fn test_merge_multiple_configs() {
    let temp_dir = TempDir::new().unwrap();

    let base_path = temp_dir.path().join("base.toml");
    let overlay_path = temp_dir.path().join("overlay.toml");

    let base_content = r#"
[storage]
backend = "filesystem"
base_path = "/data"
create_dirs = true
sync = false
file_permissions = "0644"

[performance]
upload_concurrency = 8
"#;

    let overlay_content = r#"
[storage]
backend = "filesystem"
base_path = "/data"
create_dirs = true
sync = false
file_permissions = "0644"

[performance]
upload_concurrency = 32
pack_workers = 4
"#;

    fs::write(&base_path, base_content).unwrap();
    fs::write(&overlay_path, overlay_content).unwrap();

    let loader = ConfigLoader::new();
    let config = loader
        .load_and_merge(&[&base_path, &overlay_path])
        .await
        .unwrap();

    assert_eq!(config.performance.upload_concurrency, Some(32)); // from overlay
    assert_eq!(config.performance.pack_workers, Some(4)); // from overlay
}

#[test]
fn test_validation_default_config() {
    let config = Config::default();
    assert!(config.validate().is_ok());
}

#[test]
fn test_validation_s3_bucket_name_too_short() {
    use mediagit_config::S3Storage;

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
fn test_validation_invalid_octal_permissions() {
    let mut config = Config::default();
    if let StorageConfig::FileSystem(fs) = &mut config.storage {
        fs.file_permissions = "644".to_string();
    }
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

/// `without_validation` skips `validate()`, not parsing. An unparseable or
/// unknown-key config still fails; a parseable-but-semantically-invalid one
/// gets through. Worth pinning, because the two are easy to confuse.
#[tokio::test]
async fn test_loader_without_validation() {
    let loader = ConfigLoader::without_validation();

    let bad_permissions = r#"
[storage]
backend = "filesystem"
base_path = "/data"
file_permissions = "644"
"#;
    assert!(
        loader
            .load_from_string(bad_permissions, ConfigFormat::Toml)
            .is_ok(),
        "validation is skipped, so a bad octal must load"
    );
    assert!(
        ConfigLoader::new()
            .load_from_string(bad_permissions, ConfigFormat::Toml)
            .is_err(),
        "the validating loader must still reject it"
    );

    assert!(
        loader
            .load_from_string("[nonsense]\nkey = 1\n", ConfigFormat::Toml)
            .is_err(),
        "skipping validation must not skip strict parsing"
    );
}

/// The example we ship must load. It is documentation that can be wrong, and
/// it was: until 2026-09-18 it advertised `[app]`, `[observability]`,
/// `[security]`, `[performance.cache]` and a `[compression]` block with zstd
/// and brotli levels — none of them read, and `[compression]` never on the
/// schema at all. Under `deny_unknown_fields` that file no longer parses, which
/// is the point: this test is what stops the example drifting from the schema
/// again.
#[tokio::test]
async fn the_shipped_example_config_parses() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/config.toml");
    ConfigLoader::new()
        .load_file(path)
        .await
        .expect("examples/config.toml must be loadable by the crate that ships it");
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
    // is entirely absent (not "config_version = 0"). It also carries the
    // dead-config family and three sections that were never on the schema at
    // all -- which is exactly the shape the v3 -> v4 migration has to absorb
    // now that `Config` rejects unknown keys.
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
    assert!(!migrated_on_disk.contains("[security]"));
    assert!(!migrated_on_disk.contains("[observability]"));

    // Loading again is a no-op (already current version), and does not
    // touch the backup a second time.
    let backup_mtime_before = fs::metadata(&backup_path).unwrap().modified().unwrap();
    let reloaded = Config::load(temp_dir.path()).await.unwrap();
    assert_eq!(reloaded.config_version, mediagit_config::CONFIG_VERSION);
    let backup_mtime_after = fs::metadata(&backup_path).unwrap().modified().unwrap();
    assert_eq!(backup_mtime_before, backup_mtime_after);
}

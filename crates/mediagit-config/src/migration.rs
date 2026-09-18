// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

use crate::error::{ConfigError, ConfigResult};
use serde_json::{Value, json};
use std::collections::HashMap;
use tracing::{debug, info};

/// Configuration version
pub const CONFIG_VERSION: u32 = 4;

/// Migration trait for handling config upgrades
pub trait ConfigMigration {
    /// Get the source version this migration handles
    fn source_version(&self) -> u32;

    /// Get the target version after migration
    fn target_version(&self) -> u32;

    /// Execute the migration
    fn migrate(&self, config: Value) -> ConfigResult<Value>;

    /// Get migration description
    fn description(&self) -> &str;
}

/// Migration manager
pub struct MigrationManager {
    migrations: HashMap<(u32, u32), Box<dyn ConfigMigration>>,
}

impl MigrationManager {
    /// Create a new migration manager
    pub fn new() -> Self {
        MigrationManager {
            migrations: HashMap::new(),
        }
    }

    /// Register a migration
    pub fn register(&mut self, migration: Box<dyn ConfigMigration>) {
        let key = (migration.source_version(), migration.target_version());
        self.migrations.insert(key, migration);
    }

    /// Migrate configuration from one version to another
    pub fn migrate(
        &self,
        mut config: Value,
        from_version: u32,
        to_version: u32,
    ) -> ConfigResult<Value> {
        if from_version == to_version {
            return Ok(config);
        }

        if from_version > to_version {
            return Err(ConfigError::migration_error(format!(
                "Cannot migrate from version {} to lower version {}",
                from_version, to_version
            )));
        }

        let mut current_version = from_version;
        while current_version < to_version {
            let next_version = current_version + 1;
            if next_version > to_version {
                break;
            }

            let key = (current_version, next_version);
            match self.migrations.get(&key) {
                Some(migration) => {
                    debug!(
                        "Applying migration from v{} to v{}: {}",
                        current_version,
                        next_version,
                        migration.description()
                    );
                    config = migration.migrate(config)?;
                    info!(
                        "Successfully migrated configuration from v{} to v{}",
                        current_version, next_version
                    );
                    current_version = next_version;
                }
                None => {
                    return Err(ConfigError::migration_error(format!(
                        "No migration found from v{} to v{}",
                        current_version, next_version
                    )));
                }
            }
        }

        Ok(config)
    }

    /// Get all registered migrations
    pub fn list_migrations(&self) -> Vec<String> {
        let mut migrations: Vec<_> = self
            .migrations
            .values()
            .map(|m| {
                format!(
                    "v{} -> v{}: {}",
                    m.source_version(),
                    m.target_version(),
                    m.description()
                )
            })
            .collect();
        migrations.sort();
        migrations
    }
}

impl Default for MigrationManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Example migration from v0 to v1: Add default metrics configuration
pub struct MigrationV0ToV1;

impl ConfigMigration for MigrationV0ToV1 {
    fn source_version(&self) -> u32 {
        0
    }

    fn target_version(&self) -> u32 {
        1
    }

    fn migrate(&self, mut config: Value) -> ConfigResult<Value> {
        // Add metrics configuration if not present
        if !config["observability"]["metrics"].is_object() {
            config["observability"]["metrics"] = json!({
                "enabled": true,
                "port": 9090,
                "endpoint": "/metrics",
                "interval": 60
            });
        }

        Ok(config)
    }

    fn description(&self) -> &str {
        "Add default metrics configuration"
    }
}

/// Migration from v1 to v2: object-store layout v2 (per-repo namespace +
/// true hash fanout). `repo_namespace` and `layout_version` are new fields
/// with `#[serde(default)]` on `Config`, so a config missing them already
/// parses fine without running this migration — it exists for explicitness
/// and so `layout_version` is recorded as `1` (not silently absent) on
/// configs that predate the field, matching the on-disk `LAYOUT` marker
/// semantics (missing marker == v1, never v2).
pub struct MigrationV1ToV2;

impl ConfigMigration for MigrationV1ToV2 {
    fn source_version(&self) -> u32 {
        1
    }

    fn target_version(&self) -> u32 {
        2
    }

    fn migrate(&self, mut config: Value) -> ConfigResult<Value> {
        if config["layout_version"].is_null() {
            config["layout_version"] = json!(1);
        }
        // repo_namespace intentionally left absent (None) rather than
        // invented here — the storage factory computes a default (sanitized
        // repo dir basename) at open time; this migration must not silently
        // rename an existing physical layout.
        Ok(config)
    }

    fn description(&self) -> &str {
        "Record explicit layout_version=1 for pre-layout-v2 configs"
    }
}

/// v2 -> v3: move the flat Azure credential keys into a tagged `auth` block.
///
/// This is a **mechanical relocation**, not an interpretation: the credential
/// value is carried across byte-for-byte, only its position changes. Anything
/// requiring a guess is refused rather than assumed —
///
/// * both `account_key` and `connection_string` present: ambiguous, the user
///   must say which one they meant.
/// * neither present: there is nothing to move, and inventing `emulator`
///   because the endpoint *looks* local is exactly the kind of silent
///   reinterpretation that makes a credential bug hard to see.
///
/// Configs with no Azure storage block pass through untouched.
pub struct MigrationV2ToV3;

impl ConfigMigration for MigrationV2ToV3 {
    fn source_version(&self) -> u32 {
        2
    }

    fn target_version(&self) -> u32 {
        3
    }

    fn migrate(&self, mut config: Value) -> ConfigResult<Value> {
        let Some(storage) = config.get_mut("storage") else {
            return Ok(config);
        };
        // Only the azure variant of the storage enum is affected.
        if storage.get("backend").and_then(Value::as_str) != Some("azure") {
            return Ok(config);
        }
        // Already migrated (or hand-written in the new shape).
        if storage.get("auth").is_some_and(|a| !a.is_null()) {
            return Ok(config);
        }

        let account_name = storage
            .get("account_name")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let account_key = storage
            .get("account_key")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let connection_string = storage
            .get("connection_string")
            .and_then(Value::as_str)
            .map(str::to_owned);

        let auth = match (&account_key, &connection_string) {
            (Some(_), Some(_)) => {
                return Err(ConfigError::ValidationError(
                    "Azure config has both account_key and connection_string; \
                        cannot migrate automatically because only you know which was \
                        in use. Replace them by hand with a single `auth` block: \
                        auth = { type = \"account_key\", account_name = \"...\", account_key = \"...\" } \
                        or auth = { type = \"connection_string\", value = \"...\" }"
                        .to_string(),
                ));
            }
            (Some(key), None) => {
                let Some(name) = account_name.clone() else {
                    return Err(ConfigError::ValidationError(
                        "Azure config has account_key but no account_name; cannot migrate. \
                            Write the `auth` block by hand."
                            .to_string(),
                    ));
                };
                json!({ "type": "account_key", "account_name": name, "account_key": key })
            }
            (None, Some(cs)) => json!({ "type": "connection_string", "value": cs }),
            (None, None) => {
                return Err(ConfigError::ValidationError(
                    "Azure config has neither account_key nor connection_string; \
                        nothing to migrate. Add an `auth` block explicitly — use \
                        { type = \"emulator\" } for local Azurite."
                        .to_string(),
                ));
            }
        };

        if let Some(obj) = storage.as_object_mut() {
            obj.insert("auth".to_string(), auth);
            // Drop the moved keys so the rewritten file has one shape only.
            obj.remove("account_name");
            obj.remove("account_key");
            obj.remove("connection_string");
        }
        info!("Migrated Azure storage config to tagged `auth` block (v2 -> v3)");
        Ok(config)
    }

    fn description(&self) -> &str {
        "Move flat Azure account_name/account_key/connection_string into a tagged auth block"
    }
}

/// v3 -> v4: drop the dead-config family.
///
/// `[app]`, `[observability]` (with its nested `[observability.metrics]`),
/// `[security]` (with `[security.rate_limiting]`), `[performance.cache]` and
/// `performance.buffer_size` were parsed and validated by this crate and read
/// by nothing outside it. They are deleted from the schema; see the tombstone
/// on `Config`.
///
/// **What actually protects existing configs is not this migration**, and the
/// distinction matters. `Config` is `deny_unknown_fields`, so an old file has
/// to survive *parsing* before any migration can run. That job belongs to
/// `DeprecatedSections`/`DeprecatedPerformance`, which accept these keys and
/// discard them, and to `skip_serializing`, which keeps `save()` from writing
/// them back. By the time a value reaches this migration on the load path, the
/// typed parse has usually already dropped them.
///
/// This migration is the `3 -> 4` step the `MigrationManager` requires, and a
/// defensive strip for a value that did not come through that parse. It is
/// tested directly rather than through `Config::load`, because on the load path
/// it has nothing left to do.
///
/// `[compression]`, `[performance.connection_pool]` and `[performance.timeouts]`
/// are handled with the rest. Those never existed on any version of this schema
/// — they were being silently discarded by serde, and one of them is still
/// sitting in a test fixture in this repository.
pub struct MigrationV3ToV4;

/// Top-level sections deleted in v4.
const V4_REMOVED_SECTIONS: &[&str] = &[
    "app",
    "observability",
    "security",
    // Never on the schema; silently dropped until strict parsing arrived.
    "compression",
];

/// Keys deleted from `[performance]` in v4.
const V4_REMOVED_PERFORMANCE_KEYS: &[&str] = &[
    "cache",
    "buffer_size",
    // Never on the schema; see above.
    "connection_pool",
    "timeouts",
];

impl ConfigMigration for MigrationV3ToV4 {
    fn source_version(&self) -> u32 {
        3
    }

    fn target_version(&self) -> u32 {
        4
    }

    fn migrate(&self, mut config: Value) -> ConfigResult<Value> {
        let mut dropped: Vec<&str> = Vec::new();

        if let Some(obj) = config.as_object_mut() {
            for key in V4_REMOVED_SECTIONS {
                if obj.remove(*key).is_some() {
                    dropped.push(key);
                }
            }
        }

        if let Some(perf) = config.get_mut("performance").and_then(Value::as_object_mut) {
            for key in V4_REMOVED_PERFORMANCE_KEYS {
                if perf.remove(*key).is_some() {
                    dropped.push(key);
                }
            }
        }

        if dropped.is_empty() {
            debug!("No dead config sections present (v3 -> v4)");
        } else {
            info!(
                dropped = %dropped.join(", "),
                "Removed config sections that nothing read (v3 -> v4)"
            );
        }
        Ok(config)
    }

    fn description(&self) -> &str {
        "Remove the [app], [observability], [security] and [performance.cache] dead-config family"
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_migration_manager() {
        let mut manager = MigrationManager::new();
        manager.register(Box::new(MigrationV0ToV1));

        let config = json!({
            "app": { "name": "mediagit" }
        });

        let result = manager.migrate(config, 0, 1);
        assert!(result.is_ok());

        let migrated = result.unwrap();
        assert!(
            migrated["observability"]["metrics"]["enabled"]
                .as_bool()
                .unwrap()
        );
    }

    #[test]
    fn test_no_migration_needed() {
        let manager = MigrationManager::new();
        let config = json!({"app": {"name": "mediagit"}});

        let result = manager.migrate(config.clone(), 1, 1);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), config);
    }

    #[test]
    fn test_invalid_downgrade() {
        let manager = MigrationManager::new();
        let config = json!({"app": {"name": "mediagit"}});

        let result = manager.migrate(config, 2, 1);
        assert!(result.is_err());
    }

    #[test]
    fn test_missing_migration_path() {
        let manager = MigrationManager::new();
        let config = json!({"app": {"name": "mediagit"}});

        let result = manager.migrate(config, 0, 2);
        assert!(result.is_err());
    }

    #[test]
    fn test_migration_v0_to_v1() {
        let migration = MigrationV0ToV1;
        let config = json!({
            "app": { "name": "mediagit" },
            "observability": {}
        });

        let result = migration.migrate(config);
        assert!(result.is_ok());

        let migrated = result.unwrap();
        assert_eq!(
            migrated["observability"]["metrics"]["port"].as_u64(),
            Some(9090)
        );
        assert_eq!(
            migrated["observability"]["metrics"]["endpoint"].as_str(),
            Some("/metrics")
        );
    }

    #[test]
    fn test_migration_v1_to_v2_records_layout_version() {
        let migration = MigrationV1ToV2;
        let config = json!({ "app": { "name": "mediagit" } });

        let migrated = migration.migrate(config).unwrap();
        assert_eq!(migrated["layout_version"].as_u64(), Some(1));
        assert!(migrated["repo_namespace"].is_null());
    }

    #[test]
    fn test_list_migrations() {
        let mut manager = MigrationManager::new();
        manager.register(Box::new(MigrationV0ToV1));

        let migrations = manager.list_migrations();
        assert!(!migrations.is_empty());
        assert!(migrations[0].contains("v0 -> v1"));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod azure_v3_tests {
    use super::*;

    fn flat_azure(extra: &Value) -> Value {
        let mut storage = json!({
            "backend": "azure",
            "container": "media",
            "prefix": "repos/",
        });
        if let (Some(obj), Some(ex)) = (storage.as_object_mut(), extra.as_object()) {
            for (k, v) in ex {
                obj.insert(k.clone(), v.clone());
            }
        }
        json!({ "storage": storage })
    }

    #[test]
    fn migrates_account_key_without_altering_the_credential() {
        let cfg = flat_azure(&json!({ "account_name": "acct", "account_key": "SECRET==" }));
        let out = MigrationV2ToV3.migrate(cfg).unwrap();
        let auth = &out["storage"]["auth"];

        assert_eq!(auth["type"], "account_key");
        assert_eq!(auth["account_name"], "acct");
        // The point of a mechanical migration: the secret moves, unchanged.
        assert_eq!(auth["account_key"], "SECRET==");
        // Old keys are removed so the rewritten file has exactly one shape.
        assert!(out["storage"].get("account_key").is_none());
        assert!(out["storage"].get("account_name").is_none());
    }

    #[test]
    fn migrates_connection_string() {
        let cfg = flat_azure(&json!({ "connection_string": "AccountName=x;AccountKey=y;" }));
        let out = MigrationV2ToV3.migrate(cfg).unwrap();
        assert_eq!(out["storage"]["auth"]["type"], "connection_string");
        assert_eq!(
            out["storage"]["auth"]["value"],
            "AccountName=x;AccountKey=y;"
        );
        assert!(out["storage"].get("connection_string").is_none());
    }

    #[test]
    fn refuses_ambiguous_both_credentials() {
        // Only the operator knows which one was actually in use. Picking one
        // would silently change which credential authenticates the backend.
        let cfg = flat_azure(&json!({
            "account_name": "acct",
            "account_key": "K",
            "connection_string": "AccountName=x;AccountKey=y;",
        }));
        let err = MigrationV2ToV3.migrate(cfg).unwrap_err().to_string();
        assert!(
            err.contains("both account_key and connection_string"),
            "error must name the ambiguity, got: {err}"
        );
    }

    #[test]
    fn refuses_when_there_is_nothing_to_migrate() {
        // Inferring `emulator` from a local-looking endpoint is exactly the
        // silent reinterpretation this migration must not do.
        let err = MigrationV2ToV3
            .migrate(flat_azure(&json!({})))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("neither account_key nor connection_string"),
            "got: {err}"
        );
    }

    #[test]
    fn refuses_account_key_without_account_name() {
        let cfg = flat_azure(&json!({ "account_key": "K" }));
        assert!(MigrationV2ToV3.migrate(cfg).is_err());
    }

    #[test]
    fn already_migrated_config_is_untouched() {
        let cfg = json!({ "storage": {
            "backend": "azure",
            "container": "media",
            "auth": { "type": "emulator" },
        }});
        let out = MigrationV2ToV3.migrate(cfg.clone()).unwrap();
        assert_eq!(out, cfg);
    }

    #[test]
    fn non_azure_backends_pass_through() {
        let cfg = json!({ "storage": { "backend": "s3", "bucket": "b", "region": "us-east-1" }});
        let out = MigrationV2ToV3.migrate(cfg.clone()).unwrap();
        assert_eq!(out, cfg);
    }

    #[test]
    fn config_without_storage_passes_through() {
        let cfg = json!({ "app": { "name": "mediagit" } });
        let out = MigrationV2ToV3.migrate(cfg.clone()).unwrap();
        assert_eq!(out, cfg);
    }

    #[test]
    fn v3_to_v4_strips_the_dead_family_and_keeps_everything_live() {
        let cfg = json!({
            "app": { "name": "mediagit", "port": 8080 },
            "observability": { "log_level": "info", "metrics": { "enabled": true } },
            "security": { "cors_origins": ["http://localhost:3000"] },
            "compression": { "level": 9 },
            "storage": { "backend": "filesystem", "base_path": "./data" },
            "performance": {
                "upload_concurrency": 8,
                "buffer_size": 65536,
                "cache": { "cache_type": "memory" },
                "connection_pool": { "max_idle": 4 },
                "timeouts": { "connect": 5 },
            },
            "cdc_seed": 42,
        });

        let out = MigrationV3ToV4.migrate(cfg).unwrap();

        for dead in ["app", "observability", "security", "compression"] {
            assert!(out.get(dead).is_none(), "{dead} must be removed");
        }
        let perf = out.get("performance").unwrap();
        for dead in ["buffer_size", "cache", "connection_pool", "timeouts"] {
            assert!(
                perf.get(dead).is_none(),
                "performance.{dead} must be removed"
            );
        }

        // Everything that has a read site survives untouched.
        assert_eq!(perf.get("upload_concurrency"), Some(&json!(8)));
        assert_eq!(out.get("cdc_seed"), Some(&json!(42)));
        assert_eq!(
            out.get("storage").and_then(|s| s.get("base_path")),
            Some(&json!("./data"))
        );
    }

    #[test]
    fn v3_to_v4_is_a_no_op_on_a_config_that_never_had_the_dead_sections() {
        let cfg = json!({
            "storage": { "backend": "filesystem", "base_path": "./data" },
            "performance": { "pack_workers": 4 },
        });
        let out = MigrationV3ToV4.migrate(cfg.clone()).unwrap();
        assert_eq!(out, cfg);
    }

    #[test]
    fn v3_to_v4_survives_a_config_with_no_performance_table() {
        let cfg = json!({ "storage": { "backend": "filesystem", "base_path": "./data" } });
        let out = MigrationV3ToV4.migrate(cfg.clone()).unwrap();
        assert_eq!(out, cfg);
    }
}

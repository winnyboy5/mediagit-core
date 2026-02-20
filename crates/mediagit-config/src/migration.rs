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
use serde_json::{json, Value};
use std::collections::HashMap;
use tracing::{debug, info};

/// Configuration version
pub const CONFIG_VERSION: u32 = 1;

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
    pub fn migrate(&self, mut config: Value, from_version: u32, to_version: u32) -> ConfigResult<Value> {
        if from_version == to_version {
            return Ok(config);
        }

        if from_version > to_version {
            return Err(ConfigError::migration_error(
                format!("Cannot migrate from version {} to lower version {}", from_version, to_version),
            ));
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
                        current_version, next_version,
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
                    return Err(ConfigError::migration_error(
                        format!("No migration found from v{} to v{}", current_version, next_version),
                    ));
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

        // Ensure compression algorithm is set
        if config["compression"]["algorithm"].is_null() {
            config["compression"]["algorithm"] = json!("zstd");
        }

        Ok(config)
    }

    fn description(&self) -> &str {
        "Add default metrics configuration and compression algorithm"
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
        assert!(migrated["observability"]["metrics"]["enabled"].as_bool().unwrap());
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
        assert_eq!(migrated["observability"]["metrics"]["port"].as_u64(), Some(9090));
        assert_eq!(
            migrated["observability"]["metrics"]["endpoint"].as_str(),
            Some("/metrics")
        );
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

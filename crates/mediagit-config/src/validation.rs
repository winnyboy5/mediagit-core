// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

use crate::error::{ConfigError, ConfigResult};
use crate::schema::*;

/// Validator for configuration settings
pub trait Validator {
    fn validate(&self) -> ConfigResult<()>;
}

impl Validator for Config {
    fn validate(&self) -> ConfigResult<()> {
        self.storage.validate()?;
        self.performance.validate()?;
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

impl Validator for PerformanceConfig {
    fn validate(&self) -> ConfigResult<()> {
        // Every field here is an `Option<usize>` override with an env var and
        // an internal default behind it, so there is nothing to reject. Kept
        // as a named impl so `Config::validate` reads as a complete list of
        // what this type contains rather than a selective one.
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
    fn test_invalid_octal_permissions() {
        let mut config = Config::default();
        if let StorageConfig::FileSystem(fs) = &mut config.storage {
            fs.file_permissions = "644".to_string();
        }
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

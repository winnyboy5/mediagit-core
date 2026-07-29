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

//! API Key authentication implementation
//!
//! Provides secure API key generation and validation using SHA-256 hashing.

use rand::RngExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::sync::RwLock;

use super::{AuthError, AuthResult, persist};

/// API Key structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKey {
    /// Unique key identifier
    pub id: String,

    /// Hashed API key (never store plaintext)
    pub key_hash: String,

    /// User ID associated with this API key
    pub user_id: String,

    /// Key name/description
    pub name: String,

    /// Permissions granted to this API key
    pub permissions: Vec<String>,

    /// Creation timestamp
    pub created_at: i64,

    /// AU-14: when this key was last accepted, or `None` if never.
    ///
    /// Without it a leaked key is indistinguishable from an unused one: an
    /// operator auditing keys could see what exists but not what is live, so
    /// stale keys accumulate and a compromised one leaves no trace.
    ///
    /// Recorded at coarse granularity on purpose — see `validate_key`.
    #[serde(default)]
    pub last_used: Option<i64>,
}

/// How stale `last_used` may be before a validation records a fresh value.
///
/// Recording every use would put a write lock and a disk write on the auth hot
/// path, so this is deliberately coarse: it answers "is this key in use, and
/// roughly when last" — which is the audit question — rather than serving as a
/// request log. Bounded to one write per key per interval.
fn last_used_resolution_secs() -> i64 {
    std::env::var("MEDIAGIT_APIKEY_LAST_USED_RESOLUTION")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|n| *n >= 0)
        .unwrap_or(300)
}

/// API Key authentication handler, backed in-memory with optional JSONL
/// persistence (mirrors [`super::credentials::CredentialsStore`]).
pub struct ApiKeyAuth {
    /// Map of key ID -> API Key
    keys: RwLock<HashMap<String, ApiKey>>,

    /// Path to `api_keys.jsonl` when persistence is enabled; `None` for a
    /// purely in-memory store.
    store_path: Option<PathBuf>,
}

impl ApiKeyAuth {
    /// Create new API key authenticator (in-memory only, no persistence).
    pub fn new() -> Self {
        Self {
            keys: RwLock::new(HashMap::new()),
            store_path: None,
        }
    }

    /// Load API keys persisted at `store_dir/api_keys.jsonl`, or start fresh
    /// if the file doesn't exist yet (first boot). An existing-but-corrupt
    /// file is a hard error. Set `MEDIAGIT_AUTH_PERSIST=0` to force
    /// in-memory behavior even when a directory is given.
    pub fn load_or_new(store_dir: &Path) -> AuthResult<Self> {
        if !persist::persist_enabled() {
            return Ok(Self::new());
        }

        let path = store_dir.join("api_keys.jsonl");
        let records: Vec<ApiKey> = persist::load_jsonl(&path)?;

        let mut keys = HashMap::new();
        for key in records {
            keys.insert(key.id.clone(), key);
        }

        Ok(Self {
            keys: RwLock::new(keys),
            store_path: Some(path),
        })
    }

    /// Persist the current in-memory state to `api_keys.jsonl`. A no-op
    /// when this store was constructed with [`ApiKeyAuth::new`] (no store
    /// path).
    async fn persist(&self) -> AuthResult<()> {
        let Some(path) = &self.store_path else {
            return Ok(());
        };
        let keys = self.keys.read().await;
        let records: Vec<&ApiKey> = keys.values().collect();
        persist::save_jsonl(path, &records).await
    }

    /// Generate new API key
    ///
    /// # Arguments
    /// * `user_id` - User ID to associate with the key
    /// * `name` - Descriptive name for the key
    /// * `permissions` - Permissions granted to this key
    ///
    /// # Returns
    /// Tuple of (plaintext key, API key structure)
    ///
    /// **IMPORTANT**: The plaintext key is only returned once and must be saved by the user.
    pub async fn generate_key(
        &self,
        user_id: String,
        name: String,
        permissions: Vec<String>,
    ) -> AuthResult<(String, ApiKey)> {
        // Generate secure random API key
        let key = self.generate_random_key();

        // Hash the key for storage
        let key_hash = self.hash_key(&key);

        // Create unique ID
        let id = format!("ak_{}", self.generate_random_id());

        let api_key = ApiKey {
            id: id.clone(),
            key_hash,
            user_id,
            name,
            permissions,
            created_at: chrono::Utc::now().timestamp(),
            // Never used yet — distinguishable from "used long ago", which is
            // the distinction an audit actually needs.
            last_used: None,
        };

        // Store the key
        {
            let mut keys = self.keys.write().await;
            keys.insert(id, api_key.clone());
        }
        self.persist().await?;

        Ok((key, api_key))
    }

    /// Validate API key and return associated key information
    ///
    /// # Arguments
    /// * `key` - Plaintext API key to validate
    ///
    /// # Returns
    /// API key structure if valid
    pub async fn validate_key(&self, key: &str) -> AuthResult<ApiKey> {
        let key_hash = self.hash_key(key);

        let mut found = {
            let keys = self.keys.read().await;
            match keys.values().find(|k| k.key_hash == key_hash) {
                Some(k) => k.clone(),
                None => return Err(AuthError::InvalidApiKey),
            }
        };

        // AU-14: record use, coarsely. The read lock is released above before
        // taking the write lock, so validation stays a read for the common
        // case and only pays for a write once per resolution interval.
        let now = chrono::Utc::now().timestamp();
        let due = found
            .last_used
            .is_none_or(|prev| now.saturating_sub(prev) >= last_used_resolution_secs());

        if due {
            {
                let mut keys = self.keys.write().await;
                if let Some(stored) = keys.get_mut(&found.id) {
                    stored.last_used = Some(now);
                }
            }
            found.last_used = Some(now);

            // Best-effort: `last_used` is audit telemetry, and failing an
            // otherwise valid authentication because a disk write failed
            // would turn a monitoring feature into an outage.
            if let Err(e) = self.persist().await {
                tracing::warn!(key_id = %found.id, error = %e, "failed to persist API key last_used");
            }
        }

        Ok(found)
    }

    /// Revoke API key by ID
    pub async fn revoke_key(&self, key_id: &str) -> AuthResult<()> {
        {
            let mut keys = self.keys.write().await;
            keys.remove(key_id)
                .ok_or_else(|| AuthError::UserNotFound(format!("API key not found: {}", key_id)))?;
        }

        self.persist().await
    }

    /// AU-1: revoke every API key belonging to `user_id`, returning the count.
    ///
    /// Deleting a user previously cleared their credentials and grants but left
    /// their API keys intact — and `ApiKey` carries no expiry, so those keys
    /// authenticated indefinitely. An offboarded user, or one removed *because*
    /// their account was compromised, kept working push access forever with no
    /// mechanism that could ever retire it.
    ///
    /// Idempotent: revoking for a user with no keys is a no-op that still
    /// reports success, so callers need not special-case it.
    pub async fn revoke_user_keys(&self, user_id: &str) -> AuthResult<usize> {
        let removed = {
            let mut keys = self.keys.write().await;
            let doomed: Vec<String> = keys
                .values()
                .filter(|k| k.user_id == user_id)
                .map(|k| k.id.clone())
                .collect();
            for id in &doomed {
                keys.remove(id);
            }
            doomed.len()
        };

        if removed > 0 {
            self.persist().await?;
        }
        Ok(removed)
    }

    /// List all API keys for a user
    pub async fn list_user_keys(&self, user_id: &str) -> Vec<ApiKey> {
        let keys = self.keys.read().await;

        keys.values()
            .filter(|k| k.user_id == user_id)
            .cloned()
            .collect()
    }

    /// List every API key across all users (H3 admin surface). Metadata
    /// only — `key_hash` is included on `ApiKey` but the plaintext key was
    /// never stored, so there's nothing secret to leak here.
    pub async fn list_all_keys(&self) -> Vec<ApiKey> {
        let keys = self.keys.read().await;
        keys.values().cloned().collect()
    }

    /// Extract API key from header
    ///
    /// Expects format: "X-API-Key: <key>"
    pub fn extract_from_header(header_value: &str) -> &str {
        header_value.trim()
    }

    // Private helper methods

    fn generate_random_key(&self) -> String {
        let mut rng = rand::rng();
        let bytes: Vec<u8> = (0..32).map(|_| rng.random::<u8>()).collect();
        hex::encode(bytes)
    }

    fn generate_random_id(&self) -> String {
        let mut rng = rand::rng();
        let bytes: Vec<u8> = (0..16).map(|_| rng.random::<u8>()).collect();
        hex::encode(bytes)
    }

    fn hash_key(&self, key: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(key.as_bytes());
        hex::encode(hasher.finalize())
    }
}

impl Default for ApiKeyAuth {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
// Tests hold the process-global env lock across awaits to serialize
// env-var access (see persist::ENV_LOCK).
#[allow(clippy::unwrap_used, clippy::await_holding_lock)]
mod tests {
    use super::*;

    /// AU-14: a leaked key must be distinguishable from an unused one.
    /// Without `last_used`, an operator auditing keys can see what exists but
    /// not what is live, so stale keys accumulate and a compromised key leaves
    /// no trace of having been used.
    #[tokio::test]
    async fn validating_a_key_records_that_it_was_used() {
        let auth = ApiKeyAuth::new();
        let (plaintext, created) = auth
            .generate_key("u1".to_string(), "CI".to_string(), vec![])
            .await
            .unwrap();

        assert_eq!(
            created.last_used, None,
            "a freshly minted key has never been used, which must be              distinguishable from 'used long ago'"
        );

        let validated = auth.validate_key(&plaintext).await.unwrap();
        assert!(validated.last_used.is_some(), "use must be recorded");

        // ...and it is durable, not just returned to this caller.
        let listed = auth.list_user_keys("u1").await;
        assert!(
            listed
                .iter()
                .any(|k| k.id == created.id && k.last_used.is_some()),
            "the recorded use must be visible to an audit, not only to the              request that caused it"
        );
    }

    /// Recording every single use would put a write lock and a disk write on
    /// the auth hot path. With the default resolution, repeated validations
    /// inside the interval must not keep rewriting the timestamp.
    #[tokio::test]
    async fn repeated_validation_does_not_rewrite_within_the_resolution() {
        let auth = ApiKeyAuth::new();
        let (plaintext, _) = auth
            .generate_key("u2".to_string(), "CI".to_string(), vec![])
            .await
            .unwrap();

        let first = auth.validate_key(&plaintext).await.unwrap().last_used;
        assert!(first.is_some());

        for _ in 0..5 {
            let again = auth.validate_key(&plaintext).await.unwrap().last_used;
            assert_eq!(
                again, first,
                "within the resolution interval the timestamp must be left                  alone; rewriting it would mean a lock and a disk write per                  authenticated request"
            );
        }
    }

    /// An invalid key must not be recorded as used, or the audit trail
    /// reports activity for keys that never authenticated.
    #[tokio::test]
    async fn a_rejected_key_records_nothing() {
        let auth = ApiKeyAuth::new();
        let (_plaintext, created) = auth
            .generate_key("u3".to_string(), "CI".to_string(), vec![])
            .await
            .unwrap();

        assert!(auth.validate_key("mg_not_a_real_key").await.is_err());

        let listed = auth.list_user_keys("u3").await;
        let stored = listed.iter().find(|k| k.id == created.id).unwrap();
        assert_eq!(stored.last_used, None);
    }

    #[tokio::test]
    async fn test_generate_and_validate_key() {
        let api_key_auth = ApiKeyAuth::new();

        let permissions = vec!["repo:read".to_string()];
        let (plaintext_key, _api_key) = api_key_auth
            .generate_key(
                "user123".to_string(),
                "Test Key".to_string(),
                permissions.clone(),
            )
            .await
            .unwrap();

        // Validate the key
        let validated = api_key_auth.validate_key(&plaintext_key).await.unwrap();

        assert_eq!(validated.user_id, "user123");
        assert_eq!(validated.name, "Test Key");
        assert_eq!(validated.permissions, permissions);
    }

    #[tokio::test]
    async fn test_invalid_key() {
        let api_key_auth = ApiKeyAuth::new();

        let result = api_key_auth.validate_key("invalid_key").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_revoke_key() {
        let api_key_auth = ApiKeyAuth::new();

        let (plaintext_key, api_key) = api_key_auth
            .generate_key("user123".to_string(), "Test Key".to_string(), vec![])
            .await
            .unwrap();

        // Revoke the key
        api_key_auth.revoke_key(&api_key.id).await.unwrap();

        // Key should no longer be valid
        let result = api_key_auth.validate_key(&plaintext_key).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_list_user_keys() {
        let api_key_auth = ApiKeyAuth::new();

        // Generate multiple keys for same user
        api_key_auth
            .generate_key("user123".to_string(), "Key 1".to_string(), vec![])
            .await
            .unwrap();

        api_key_auth
            .generate_key("user123".to_string(), "Key 2".to_string(), vec![])
            .await
            .unwrap();

        api_key_auth
            .generate_key("user456".to_string(), "Key 3".to_string(), vec![])
            .await
            .unwrap();

        let user_keys = api_key_auth.list_user_keys("user123").await;
        assert_eq!(user_keys.len(), 2);
    }

    #[tokio::test]
    async fn test_list_all_keys() {
        let api_key_auth = ApiKeyAuth::new();

        api_key_auth
            .generate_key("user123".to_string(), "Key 1".to_string(), vec![])
            .await
            .unwrap();
        api_key_auth
            .generate_key("user456".to_string(), "Key 2".to_string(), vec![])
            .await
            .unwrap();

        let all_keys = api_key_auth.list_all_keys().await;
        assert_eq!(all_keys.len(), 2);
    }

    #[test]
    fn test_extract_from_header() {
        let key = "abc123xyz";
        let extracted = ApiKeyAuth::extract_from_header(key);
        assert_eq!(extracted, key);
    }

    // ---- H1: persistence ----

    #[tokio::test]
    async fn persists_and_reloads_across_restart() {
        let _guard = persist::ENV_LOCK.read().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let api_key_auth = ApiKeyAuth::load_or_new(tmp.path()).unwrap();
        let (plaintext_key, _) = api_key_auth
            .generate_key("user123".to_string(), "Test Key".to_string(), vec![])
            .await
            .unwrap();

        assert!(tmp.path().join("api_keys.jsonl").exists());

        // Fresh authenticator from the same dir simulates a server restart.
        let reloaded = ApiKeyAuth::load_or_new(tmp.path()).unwrap();
        let validated = reloaded.validate_key(&plaintext_key).await;
        assert!(validated.is_ok());
        assert_eq!(validated.unwrap().user_id, "user123");
    }

    #[tokio::test]
    async fn corrupt_store_file_hard_errors() {
        let _guard = persist::ENV_LOCK.read().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        tokio::fs::write(
            tmp.path().join("api_keys.jsonl"),
            b"{\"v\":1}\nnot valid json\n",
        )
        .await
        .unwrap();

        let result = ApiKeyAuth::load_or_new(tmp.path());
        assert!(result.is_err(), "corrupt store file must hard-error");
    }

    #[tokio::test]
    async fn missing_store_file_starts_fresh() {
        let _guard = persist::ENV_LOCK.read().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let api_key_auth = ApiKeyAuth::load_or_new(tmp.path()).unwrap();
        assert_eq!(api_key_auth.list_user_keys("anyone").await.len(), 0);
    }

    #[tokio::test]
    async fn persist_disabled_writes_no_files() {
        let _guard = persist::ENV_LOCK.write().unwrap();
        mediagit_test_utils::set_var("MEDIAGIT_AUTH_PERSIST", "0");
        let tmp = tempfile::tempdir().unwrap();
        let api_key_auth = ApiKeyAuth::load_or_new(tmp.path()).unwrap();
        api_key_auth
            .generate_key("user123".to_string(), "Test Key".to_string(), vec![])
            .await
            .unwrap();
        mediagit_test_utils::remove_var("MEDIAGIT_AUTH_PERSIST");

        assert!(!tmp.path().join("api_keys.jsonl").exists());
    }
}

//! API Key authentication implementation
//!
//! Provides secure API key generation and validation using SHA-256 hashing.

use rand::{thread_rng, Rng};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use tokio::sync::RwLock;

use super::{AuthError, AuthResult};

/// API Key structure
#[derive(Debug, Clone)]
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
}

/// API Key authentication handler
pub struct ApiKeyAuth {
    /// Map of key ID -> API Key
    keys: RwLock<HashMap<String, ApiKey>>,
}

impl ApiKeyAuth {
    /// Create new API key authenticator
    pub fn new() -> Self {
        Self {
            keys: RwLock::new(HashMap::new()),
        }
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
        };

        // Store the key
        let mut keys = self.keys.write().await;
        keys.insert(id, api_key.clone());

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

        let keys = self.keys.read().await;

        // Find key by hash
        for api_key in keys.values() {
            if api_key.key_hash == key_hash {
                return Ok(api_key.clone());
            }
        }

        Err(AuthError::InvalidApiKey)
    }

    /// Revoke API key by ID
    pub async fn revoke_key(&self, key_id: &str) -> AuthResult<()> {
        let mut keys = self.keys.write().await;

        keys.remove(key_id)
            .ok_or_else(|| AuthError::UserNotFound(format!("API key not found: {}", key_id)))?;

        Ok(())
    }

    /// List all API keys for a user
    pub async fn list_user_keys(&self, user_id: &str) -> Vec<ApiKey> {
        let keys = self.keys.read().await;

        keys.values()
            .filter(|k| k.user_id == user_id)
            .cloned()
            .collect()
    }

    /// Extract API key from header
    ///
    /// Expects format: "X-API-Key: <key>"
    pub fn extract_from_header(header_value: &str) -> &str {
        header_value.trim()
    }

    // Private helper methods

    fn generate_random_key(&self) -> String {
        let mut rng = thread_rng();
        let bytes: Vec<u8> = (0..32).map(|_| rng.gen()).collect();
        hex::encode(bytes)
    }

    fn generate_random_id(&self) -> String {
        let mut rng = thread_rng();
        let bytes: Vec<u8> = (0..16).map(|_| rng.gen()).collect();
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
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_generate_and_validate_key() {
        let api_key_auth = ApiKeyAuth::new();

        let permissions = vec!["repo:read".to_string()];
        let (plaintext_key, api_key) = api_key_auth
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
            .generate_key(
                "user123".to_string(),
                "Test Key".to_string(),
                vec![],
            )
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

    #[test]
    fn test_extract_from_header() {
        let key = "abc123xyz";
        let extracted = ApiKeyAuth::extract_from_header(key);
        assert_eq!(extracted, key);
    }
}

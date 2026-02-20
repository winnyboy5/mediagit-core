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

//! Argon2 key derivation for password-based encryption
//!
//! This module provides secure key derivation from passwords using Argon2id,
//! the recommended variant for password hashing and key derivation.
//!
//! # Algorithm Parameters
//!
//! Based on OWASP recommendations for 2024:
//! - **Memory**: 64 MB (65536 KB)
//! - **Iterations**: 3
//! - **Parallelism**: 4 threads
//! - **Salt**: 128-bit random salt per password
//! - **Output**: 256-bit key for AES-256
//!
//! # Security Features
//!
//! - **Memory-hard**: Resistant to GPU/ASIC attacks
//! - **Time-cost**: Configurable iteration count
//! - **Random salts**: Each password derivation uses unique salt
//! - **Key caching**: Optional in-memory cache to avoid re-derivation

use crate::encryption::{EncryptionKey, KEY_SIZE};
use argon2::{
    password_hash::SaltString,
    Algorithm, Argon2, ParamsBuilder, Version,
};
use rand::thread_rng;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::{debug, info};
use zeroize::Zeroizing;

/// Salt size in bytes (128 bits)
const SALT_SIZE: usize = 16;

/// Key derivation errors
#[derive(Error, Debug)]
pub enum KdfError {
    #[error("Key derivation failed: {0}")]
    DerivationFailed(String),

    #[error("Invalid password")]
    InvalidPassword,

    #[error("Invalid salt: {0}")]
    InvalidSalt(String),

    #[error("Parameter configuration error: {0}")]
    ParameterError(String),
}

/// Argon2 parameters for key derivation
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Argon2Params {
    /// Memory size in KB (default: 65536 = 64 MB)
    pub memory_kb: u32,

    /// Number of iterations (default: 3)
    pub iterations: u32,

    /// Degree of parallelism (default: 4)
    pub parallelism: u32,
}

impl Default for Argon2Params {
    fn default() -> Self {
        Self {
            memory_kb: 65536, // 64 MB
            iterations: 3,
            parallelism: 4,
        }
    }
}

impl Argon2Params {
    /// Create custom parameters
    ///
    /// # Security Warning
    ///
    /// Only reduce these values for testing or resource-constrained environments.
    /// Production systems should use defaults or higher values.
    pub fn custom(memory_kb: u32, iterations: u32, parallelism: u32) -> Self {
        Self {
            memory_kb,
            iterations,
            parallelism,
        }
    }

    /// Low-security parameters for testing
    ///
    /// Memory: 8 MB, Iterations: 1, Parallelism: 1
    pub fn testing() -> Self {
        Self {
            memory_kb: 8192, // 8 MB
            iterations: 1,
            parallelism: 1,
        }
    }
}

/// Salt for key derivation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Salt {
    #[serde(with = "hex")]
    bytes: Vec<u8>,
}

impl Salt {
    /// Generate a new random salt
    pub fn generate() -> Result<Self, KdfError> {
        let salt_string = SaltString::generate(&mut thread_rng());
        let bytes = salt_string
            .as_str()
            .as_bytes()
            .get(..SALT_SIZE)
            .ok_or_else(|| KdfError::InvalidSalt("Generated salt too short".to_string()))?
            .to_vec();

        Ok(Self { bytes })
    }

    /// Create salt from bytes
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, KdfError> {
        if bytes.len() != SALT_SIZE {
            return Err(KdfError::InvalidSalt(format!(
                "Expected {} bytes, got {}",
                SALT_SIZE,
                bytes.len()
            )));
        }
        Ok(Self { bytes })
    }

    /// Get salt bytes
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Convert to hex string for storage
    pub fn to_hex(&self) -> String {
        hex::encode(&self.bytes)
    }

    /// Parse from hex string
    pub fn from_hex(hex_str: &str) -> Result<Self, KdfError> {
        let bytes =
            hex::decode(hex_str).map_err(|e| KdfError::InvalidSalt(format!("Invalid hex: {}", e)))?;
        Self::from_bytes(bytes)
    }
}

/// Derive encryption key from password using Argon2id
///
/// # Arguments
///
/// * `password` - User password (will be zeroized after use)
/// * `salt` - Salt for derivation (unique per password)
/// * `params` - Argon2 parameters
///
/// # Returns
///
/// 256-bit encryption key suitable for AES-256-GCM
///
/// # Examples
///
/// ```no_run
/// use mediagit_security::kdf::{derive_key, Salt, Argon2Params};
/// use secrecy::SecretString;
///
/// let password = SecretString::new("my-secure-password".to_string());
/// let salt = Salt::generate().unwrap();
/// let params = Argon2Params::default();
///
/// let key = derive_key(&password, &salt, params).unwrap();
/// ```
pub fn derive_key(
    password: &SecretString,
    salt: &Salt,
    params: Argon2Params,
) -> Result<EncryptionKey, KdfError> {
    debug!(
        memory_kb = params.memory_kb,
        iterations = params.iterations,
        parallelism = params.parallelism,
        "Deriving key from password"
    );

    // Build Argon2 parameters
    let argon2_params = ParamsBuilder::new()
        .m_cost(params.memory_kb)
        .t_cost(params.iterations)
        .p_cost(params.parallelism)
        .output_len(KEY_SIZE)
        .build()
        .map_err(|e| KdfError::ParameterError(e.to_string()))?;

    // Create Argon2 context
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, argon2_params);

    // Derive key
    let mut key_bytes = Zeroizing::new(vec![0u8; KEY_SIZE]);
    argon2
        .hash_password_into(
            password.expose_secret().as_bytes(),
            salt.as_bytes(),
            &mut key_bytes,
        )
        .map_err(|e| KdfError::DerivationFailed(e.to_string()))?;

    let key = EncryptionKey::from_bytes(key_bytes.to_vec())
        .map_err(|e| KdfError::DerivationFailed(e.to_string()))?;

    info!("Key derivation complete");
    Ok(key)
}

/// Key cache for avoiding repeated derivations
///
/// Stores derived keys in memory to avoid expensive re-computation.
/// Keys are indexed by (password_hash, salt_hex) for security.
/// 
/// The cache has a configurable maximum size (default 10,000 entries).
/// When full, the oldest entry is evicted to make room for new ones.
#[derive(Clone)]
pub struct KeyCache {
    cache: Arc<RwLock<KeyCacheInner>>,
}

/// Internal cache state with LRU tracking
struct KeyCacheInner {
    /// Key -> (EncryptionKey, insertion_order)
    entries: HashMap<String, (EncryptionKey, u64)>,
    /// Maximum number of entries
    max_entries: usize,
    /// Counter for tracking insertion order (for LRU eviction)
    insertion_counter: u64,
}

impl KeyCache {
    /// Default maximum cache entries
    pub const DEFAULT_MAX_ENTRIES: usize = 10_000;

    /// Create a new key cache with default capacity
    pub fn new() -> Self {
        Self::with_capacity(Self::DEFAULT_MAX_ENTRIES)
    }

    /// Create a new key cache with specified maximum entries
    pub fn with_capacity(max_entries: usize) -> Self {
        Self {
            cache: Arc::new(RwLock::new(KeyCacheInner {
                entries: HashMap::new(),
                max_entries,
                insertion_counter: 0,
            })),
        }
    }

    /// Get a cache key from password and salt
    fn cache_key(password: &SecretString, salt: &Salt) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(password.expose_secret().as_bytes());
        hasher.update(salt.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Get cached key or derive if not present
    ///
    /// # Arguments
    ///
    /// * `password` - User password
    /// * `salt` - Salt for derivation
    /// * `params` - Argon2 parameters
    ///
    /// # Returns
    ///
    /// Encryption key (from cache or freshly derived)
    pub async fn get_or_derive(
        &self,
        password: &SecretString,
        salt: &Salt,
        params: Argon2Params,
    ) -> Result<EncryptionKey, KdfError> {
        let key = Self::cache_key(password, salt);

        // Check cache first
        {
            let cache = self.cache.read().await;
            if let Some((cached_key, _)) = cache.entries.get(&key) {
                debug!("Using cached key");
                return Ok(cached_key.clone());
            }
        }

        // Derive key
        debug!("Cache miss, deriving key");
        let derived_key = derive_key(password, salt, params)?;

        // Store in cache with eviction if needed
        {
            let mut cache = self.cache.write().await;
            
            // Evict oldest entry if at capacity
            if cache.entries.len() >= cache.max_entries {
                // Find the entry with the lowest insertion_counter (oldest)
                if let Some((oldest_key, _)) = cache
                    .entries
                    .iter()
                    .min_by_key(|(_, (_, order))| order)
                    .map(|(k, v)| (k.clone(), v.clone()))
                {
                    cache.entries.remove(&oldest_key);
                    debug!("Evicted oldest key from cache");
                }
            }
            
            cache.insertion_counter += 1;
            let order = cache.insertion_counter;
            cache.entries.insert(key, (derived_key.clone(), order));
        }

        Ok(derived_key)
    }

    /// Clear all cached keys
    pub async fn clear(&self) {
        let mut cache = self.cache.write().await;
        cache.entries.clear();
        cache.insertion_counter = 0;
        info!("Key cache cleared");
    }

    /// Get cache statistics
    pub async fn stats(&self) -> CacheStats {
        let cache = self.cache.read().await;
        CacheStats {
            entries: cache.entries.len(),
            max_entries: cache.max_entries,
        }
    }
}

impl Default for KeyCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Cache statistics
#[derive(Debug, Clone)]
pub struct CacheStats {
    pub entries: usize,
    pub max_entries: usize,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_salt_generation() {
        let salt = Salt::generate().unwrap();
        assert_eq!(salt.as_bytes().len(), SALT_SIZE);
    }

    #[test]
    fn test_salt_hex_roundtrip() {
        let salt = Salt::generate().unwrap();
        let hex = salt.to_hex();
        let parsed = Salt::from_hex(&hex).unwrap();
        assert_eq!(salt.as_bytes(), parsed.as_bytes());
    }

    #[test]
    fn test_derive_key() {
        let password = SecretString::new("test-password".to_string());
        let salt = Salt::generate().unwrap();
        let params = Argon2Params::testing(); // Fast for tests

        let key = derive_key(&password, &salt, params).unwrap();
        assert_eq!(key.expose_key().len(), KEY_SIZE);
    }

    #[test]
    fn test_derive_key_deterministic() {
        let password = SecretString::new("test-password".to_string());
        let salt = Salt::from_bytes(vec![42u8; SALT_SIZE]).unwrap();
        let params = Argon2Params::testing();

        let key1 = derive_key(&password, &salt, params).unwrap();
        let key2 = derive_key(&password, &salt, params).unwrap();

        assert_eq!(key1.expose_key(), key2.expose_key());
    }

    #[test]
    fn test_derive_key_different_salts() {
        let password = SecretString::new("test-password".to_string());
        let salt1 = Salt::generate().unwrap();
        let salt2 = Salt::generate().unwrap();
        let params = Argon2Params::testing();

        let key1 = derive_key(&password, &salt1, params).unwrap();
        let key2 = derive_key(&password, &salt2, params).unwrap();

        assert_ne!(key1.expose_key(), key2.expose_key());
    }

    #[test]
    fn test_derive_key_different_passwords() {
        let password1 = SecretString::new("password1".to_string());
        let password2 = SecretString::new("password2".to_string());
        let salt = Salt::from_bytes(vec![42u8; SALT_SIZE]).unwrap();
        let params = Argon2Params::testing();

        let key1 = derive_key(&password1, &salt, params).unwrap();
        let key2 = derive_key(&password2, &salt, params).unwrap();

        assert_ne!(key1.expose_key(), key2.expose_key());
    }

    #[tokio::test]
    async fn test_key_cache() {
        let cache = KeyCache::new();
        let password = SecretString::new("test-password".to_string());
        let salt = Salt::generate().unwrap();
        let params = Argon2Params::testing();

        // First call should derive
        let key1 = cache.get_or_derive(&password, &salt, params).await.unwrap();

        // Second call should use cache
        let key2 = cache.get_or_derive(&password, &salt, params).await.unwrap();

        assert_eq!(key1.expose_key(), key2.expose_key());

        let stats = cache.stats().await;
        assert_eq!(stats.entries, 1);
    }

    #[tokio::test]
    async fn test_cache_clear() {
        let cache = KeyCache::new();
        let password = SecretString::new("test-password".to_string());
        let salt = Salt::generate().unwrap();
        let params = Argon2Params::testing();

        cache.get_or_derive(&password, &salt, params).await.unwrap();
        assert_eq!(cache.stats().await.entries, 1);

        cache.clear().await;
        assert_eq!(cache.stats().await.entries, 0);
    }

    #[test]
    fn test_params_default() {
        let params = Argon2Params::default();
        assert_eq!(params.memory_kb, 65536);
        assert_eq!(params.iterations, 3);
        assert_eq!(params.parallelism, 4);
    }

    #[test]
    fn test_params_testing() {
        let params = Argon2Params::testing();
        assert!(params.memory_kb < Argon2Params::default().memory_kb);
        assert!(params.iterations < Argon2Params::default().iterations);
    }
}

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

use crate::encryption::{EncryptionKey, KEY_SIZE};
use argon2::{Algorithm, Argon2, ParamsBuilder, Version};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, info};
use zeroize::Zeroizing;

/// Salt size in bytes (128 bits)
const SALT_SIZE: usize = 16;

/// Key derivation errors
#[derive(Error, Debug)]
pub enum KdfError {
    /// Argon2id computation failed (e.g., memory allocation or parameter rejection).
    #[error("Key derivation failed: {0}")]
    DerivationFailed(String),

    /// Password was empty or otherwise invalid for key derivation.
    #[error("Invalid password")]
    InvalidPassword,

    /// Salt bytes were the wrong length or could not be parsed.
    #[error("Invalid salt: {0}")]
    InvalidSalt(String),

    /// Argon2 parameter values (memory, iterations, parallelism) were out of range.
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
    /// Generate a new random salt: `SALT_SIZE` raw bytes straight from the OS
    /// CSPRNG.
    ///
    /// Not `SaltString::generate` + truncation: `SaltString::generate` returns
    /// *base64-encoded* randomness, and slicing its first `SALT_SIZE`
    /// *characters* keeps only `SALT_SIZE * 6` bits of real entropy (base64 is
    /// ~6 bits/char), not the `SALT_SIZE * 8` this module's doc claims and
    /// callers assume.
    pub fn generate() -> Result<Self, KdfError> {
        let mut bytes = vec![0u8; SALT_SIZE];
        getrandom::fill(&mut bytes)
            .map_err(|e| KdfError::InvalidSalt(format!("OS RNG unavailable: {e}")))?;
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
        let bytes = hex::decode(hex_str)
            .map_err(|e| KdfError::InvalidSalt(format!("Invalid hex: {}", e)))?;
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
/// let password = SecretString::from("my-secure-password".to_string());
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
        let password = SecretString::from("test-password".to_string());
        let salt = Salt::generate().unwrap();
        let params = Argon2Params::testing(); // Fast for tests

        let key = derive_key(&password, &salt, params).unwrap();
        assert_eq!(key.expose_key().len(), KEY_SIZE);
    }

    #[test]
    fn test_derive_key_deterministic() {
        let password = SecretString::from("test-password".to_string());
        let salt = Salt::from_bytes(vec![42u8; SALT_SIZE]).unwrap();
        let params = Argon2Params::testing();

        let key1 = derive_key(&password, &salt, params).unwrap();
        let key2 = derive_key(&password, &salt, params).unwrap();

        assert_eq!(key1.expose_key(), key2.expose_key());
    }

    #[test]
    fn test_derive_key_different_salts() {
        let password = SecretString::from("test-password".to_string());
        let salt1 = Salt::generate().unwrap();
        let salt2 = Salt::generate().unwrap();
        let params = Argon2Params::testing();

        let key1 = derive_key(&password, &salt1, params).unwrap();
        let key2 = derive_key(&password, &salt2, params).unwrap();

        assert_ne!(key1.expose_key(), key2.expose_key());
    }

    #[test]
    fn test_derive_key_different_passwords() {
        let password1 = SecretString::from("password1".to_string());
        let password2 = SecretString::from("password2".to_string());
        let salt = Salt::from_bytes(vec![42u8; SALT_SIZE]).unwrap();
        let params = Argon2Params::testing();

        let key1 = derive_key(&password1, &salt, params).unwrap();
        let key2 = derive_key(&password2, &salt, params).unwrap();

        assert_ne!(key1.expose_key(), key2.expose_key());
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

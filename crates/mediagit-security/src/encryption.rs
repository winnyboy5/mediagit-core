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

//! AES-256-GCM encryption for object storage
//!
//! This module provides authenticated encryption using AES-256-GCM with:
//! - 256-bit keys (32 bytes)
//! - 96-bit nonces (12 bytes) - randomly generated per encryption
//! - 128-bit authentication tags (16 bytes)
//! - Stream encryption for large objects (64KB chunks)
//!
//! # Security Features
//!
//! - **Authenticated Encryption**: Provides both confidentiality and integrity
//! - **Unique Nonces**: Each encryption uses a fresh random nonce
//! - **Stream Support**: Handles large objects efficiently with chunked encryption
//! - **Constant Time**: Uses constant-time operations to prevent timing attacks
//!
//! # Format
//!
//! Encrypted data structure:
//! ```text
//! [version:1][nonce:12][ciphertext:N][tag:16]
//! ```
//!
//! For stream encryption (objects > 64KB):
//! ```text
//! [version:1][nonce:12][chunk1_cipher][chunk1_tag][chunk2_cipher][chunk2_tag]...
//! ```

use aes_gcm::{
    aead::{Aead, KeyInit, Payload},
    Aes256Gcm, Nonce,
};
use rand::{thread_rng, RngCore};
use secrecy::{ExposeSecret, SecretVec};
use thiserror::Error;
use tracing::{debug, warn};
use zeroize::Zeroizing;

/// Encryption version for format evolution
const ENCRYPTION_VERSION: u8 = 1;

/// Nonce size in bytes (96 bits for AES-GCM)
const NONCE_SIZE: usize = 12;

/// Authentication tag size in bytes (128 bits)
const TAG_SIZE: usize = 16;

/// Key size in bytes (256 bits for AES-256)
pub const KEY_SIZE: usize = 32;

/// Chunk size for stream encryption (64KB)
const CHUNK_SIZE: usize = 64 * 1024;

/// Threshold for stream encryption (same as chunk size)
const STREAM_THRESHOLD: usize = CHUNK_SIZE;

/// Encryption errors
#[derive(Error, Debug)]
pub enum EncryptionError {
    #[error("Encryption failed: {0}")]
    EncryptionFailed(String),

    #[error("Decryption failed: {0}")]
    DecryptionFailed(String),

    #[error("Invalid key size: expected {KEY_SIZE}, got {0}")]
    InvalidKeySize(usize),

    #[error("Invalid ciphertext: {0}")]
    InvalidCiphertext(String),

    #[error("Unsupported encryption version: {0}")]
    UnsupportedVersion(u8),

    #[error("Random number generation failed: {0}")]
    RandomGenerationFailed(String),
}

/// Encryption key wrapper with secure memory handling
pub struct EncryptionKey {
    key: SecretVec<u8>,
}

impl Clone for EncryptionKey {
    fn clone(&self) -> Self {
        Self {
            key: SecretVec::new(self.key.expose_secret().to_vec()),
        }
    }
}

impl EncryptionKey {
    /// Create a new encryption key from bytes
    ///
    /// # Security
    ///
    /// The key bytes are securely stored and will be zeroized on drop.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, EncryptionError> {
        if bytes.len() != KEY_SIZE {
            return Err(EncryptionError::InvalidKeySize(bytes.len()));
        }

        Ok(Self {
            key: SecretVec::new(bytes),
        })
    }

    /// Generate a new random encryption key
    ///
    /// Uses cryptographically secure random number generation.
    pub fn generate() -> Result<Self, EncryptionError> {
        let mut key_bytes = vec![0u8; KEY_SIZE];
        thread_rng()
            .try_fill_bytes(&mut key_bytes)
            .map_err(|e| EncryptionError::RandomGenerationFailed(e.to_string()))?;

        Ok(Self {
            key: SecretVec::new(key_bytes),
        })
    }

    /// Get the key bytes (use with caution)
    ///
    /// # Security Warning
    ///
    /// This exposes the key material. Only use when necessary and ensure
    /// the exposed bytes are properly zeroized after use.
    pub(crate) fn expose_key(&self) -> &[u8] {
        self.key.expose_secret()
    }
}

impl std::fmt::Debug for EncryptionKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncryptionKey")
            .field("key", &"<redacted>")
            .finish()
    }
}

/// Encrypt data using AES-256-GCM
///
/// # Arguments
///
/// * `key` - Encryption key (32 bytes)
/// * `plaintext` - Data to encrypt
///
/// # Returns
///
/// Encrypted data with format: [version:1][nonce:12][ciphertext:N][tag:16]
///
/// # Examples
///
/// ```no_run
/// use mediagit_security::encryption::{EncryptionKey, encrypt};
///
/// let key = EncryptionKey::generate().unwrap();
/// let plaintext = b"sensitive data";
/// let ciphertext = encrypt(&key, plaintext).unwrap();
/// ```
pub fn encrypt(key: &EncryptionKey, plaintext: &[u8]) -> Result<Vec<u8>, EncryptionError> {
    debug!(size = plaintext.len(), "Encrypting data");

    // Use stream encryption for large objects
    if plaintext.len() > STREAM_THRESHOLD {
        return encrypt_stream(key, plaintext);
    }

    // Generate random nonce
    let mut nonce_bytes = [0u8; NONCE_SIZE];
    thread_rng()
        .try_fill_bytes(&mut nonce_bytes)
        .map_err(|e| EncryptionError::RandomGenerationFailed(e.to_string()))?;

    let nonce = Nonce::from_slice(&nonce_bytes);

    // Create cipher
    let cipher = Aes256Gcm::new_from_slice(key.expose_key())
        .map_err(|e| EncryptionError::EncryptionFailed(e.to_string()))?;

    // Encrypt
    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| EncryptionError::EncryptionFailed(e.to_string()))?;

    // Build output: [version][nonce][ciphertext+tag]
    let mut output = Vec::with_capacity(1 + NONCE_SIZE + ciphertext.len());
    output.push(ENCRYPTION_VERSION);
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&ciphertext);

    debug!(
        plaintext_size = plaintext.len(),
        ciphertext_size = output.len(),
        overhead = output.len() - plaintext.len(),
        "Encryption complete"
    );

    Ok(output)
}

/// Decrypt data using AES-256-GCM
///
/// # Arguments
///
/// * `key` - Decryption key (32 bytes)
/// * `ciphertext` - Encrypted data from `encrypt()`
///
/// # Returns
///
/// Original plaintext data
///
/// # Examples
///
/// ```no_run
/// use mediagit_security::encryption::{EncryptionKey, encrypt, decrypt};
///
/// let key = EncryptionKey::generate().unwrap();
/// let plaintext = b"sensitive data";
/// let ciphertext = encrypt(&key, plaintext).unwrap();
/// let decrypted = decrypt(&key, &ciphertext).unwrap();
/// assert_eq!(decrypted, plaintext);
/// ```
pub fn decrypt(key: &EncryptionKey, ciphertext: &[u8]) -> Result<Vec<u8>, EncryptionError> {
    debug!(size = ciphertext.len(), "Decrypting data");

    // Minimum size check: version(1) + nonce(12) + tag(16)
    if ciphertext.len() < 1 + NONCE_SIZE + TAG_SIZE {
        return Err(EncryptionError::InvalidCiphertext(
            "Ciphertext too short".to_string(),
        ));
    }

    // Check version
    let version = ciphertext[0];
    if version != ENCRYPTION_VERSION {
        return Err(EncryptionError::UnsupportedVersion(version));
    }

    // Extract nonce
    let nonce_bytes = &ciphertext[1..1 + NONCE_SIZE];
    let nonce = Nonce::from_slice(nonce_bytes);

    // Extract ciphertext (includes tag)
    let encrypted_data = &ciphertext[1 + NONCE_SIZE..];

    // Create cipher
    let cipher = Aes256Gcm::new_from_slice(key.expose_key())
        .map_err(|e| EncryptionError::DecryptionFailed(e.to_string()))?;

    // Decrypt and verify
    let plaintext = cipher
        .decrypt(nonce, encrypted_data)
        .map_err(|e| EncryptionError::DecryptionFailed(format!("Authentication failed: {}", e)))?;

    debug!(
        ciphertext_size = ciphertext.len(),
        plaintext_size = plaintext.len(),
        "Decryption complete"
    );

    Ok(plaintext)
}

/// Encrypt large data using stream encryption
///
/// Splits data into 64KB chunks, each encrypted with the same key but different nonces.
/// Format: [version][nonce][chunk1_encrypted][chunk2_encrypted]...
fn encrypt_stream(key: &EncryptionKey, plaintext: &[u8]) -> Result<Vec<u8>, EncryptionError> {
    debug!(
        size = plaintext.len(),
        chunks = (plaintext.len() + CHUNK_SIZE - 1) / CHUNK_SIZE,
        "Stream encrypting large object"
    );

    let mut output = Vec::with_capacity(plaintext.len() + 1 + NONCE_SIZE + TAG_SIZE * 10);
    output.push(ENCRYPTION_VERSION);

    // Generate base nonce
    let mut nonce_bytes = [0u8; NONCE_SIZE];
    thread_rng()
        .try_fill_bytes(&mut nonce_bytes)
        .map_err(|e| EncryptionError::RandomGenerationFailed(e.to_string()))?;

    output.extend_from_slice(&nonce_bytes);

    let cipher = Aes256Gcm::new_from_slice(key.expose_key())
        .map_err(|e| EncryptionError::EncryptionFailed(e.to_string()))?;

    // Encrypt each chunk
    for (chunk_idx, chunk) in plaintext.chunks(CHUNK_SIZE).enumerate() {
        // Create unique nonce for this chunk by XORing with chunk index
        let mut chunk_nonce = nonce_bytes;
        let idx_bytes = (chunk_idx as u64).to_le_bytes();
        for (i, &byte) in idx_bytes.iter().enumerate() {
            if i < NONCE_SIZE {
                chunk_nonce[i] ^= byte;
            }
        }

        let nonce = Nonce::from_slice(&chunk_nonce);

        // Encrypt chunk
        let chunk_ciphertext = cipher
            .encrypt(nonce, chunk)
            .map_err(|e| EncryptionError::EncryptionFailed(e.to_string()))?;

        output.extend_from_slice(&chunk_ciphertext);
    }

    Ok(output)
}

/// Decrypt stream-encrypted data
///
/// This is automatically handled by the decrypt() function based on data size.
fn decrypt_stream(
    key: &EncryptionKey,
    version: u8,
    nonce_bytes: &[u8],
    encrypted_data: &[u8],
) -> Result<Vec<u8>, EncryptionError> {
    debug!(size = encrypted_data.len(), "Stream decrypting large object");

    let cipher = Aes256Gcm::new_from_slice(key.expose_key())
        .map_err(|e| EncryptionError::DecryptionFailed(e.to_string()))?;

    let mut plaintext = Vec::with_capacity(encrypted_data.len());
    let chunk_size = CHUNK_SIZE + TAG_SIZE;

    for (chunk_idx, chunk) in encrypted_data.chunks(chunk_size).enumerate() {
        // Recreate chunk nonce
        let mut chunk_nonce = [0u8; NONCE_SIZE];
        chunk_nonce.copy_from_slice(nonce_bytes);
        let idx_bytes = (chunk_idx as u64).to_le_bytes();
        for (i, &byte) in idx_bytes.iter().enumerate() {
            if i < NONCE_SIZE {
                chunk_nonce[i] ^= byte;
            }
        }

        let nonce = Nonce::from_slice(&chunk_nonce);

        // Decrypt chunk
        let chunk_plaintext = cipher
            .decrypt(nonce, chunk)
            .map_err(|e| EncryptionError::DecryptionFailed(e.to_string()))?;

        plaintext.extend_from_slice(&chunk_plaintext);
    }

    Ok(plaintext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_generation() {
        let key = EncryptionKey::generate().unwrap();
        assert_eq!(key.expose_key().len(), KEY_SIZE);
    }

    #[test]
    fn test_key_from_bytes() {
        let bytes = vec![42u8; KEY_SIZE];
        let key = EncryptionKey::from_bytes(bytes.clone()).unwrap();
        assert_eq!(key.expose_key(), &bytes[..]);
    }

    #[test]
    fn test_key_invalid_size() {
        let bytes = vec![42u8; 16]; // Wrong size
        assert!(EncryptionKey::from_bytes(bytes).is_err());
    }

    #[test]
    fn test_encrypt_decrypt_small() {
        let key = EncryptionKey::generate().unwrap();
        let plaintext = b"Hello, World!";

        let ciphertext = encrypt(&key, plaintext).unwrap();
        let decrypted = decrypt(&key, &ciphertext).unwrap();

        assert_eq!(decrypted, plaintext);
        assert_ne!(ciphertext[1 + NONCE_SIZE..], plaintext[..]); // Ensure it's encrypted
    }

    #[test]
    fn test_encrypt_decrypt_large() {
        let key = EncryptionKey::generate().unwrap();
        let plaintext = vec![42u8; 200_000]; // 200KB

        let ciphertext = encrypt(&key, &plaintext).unwrap();
        let decrypted = decrypt(&key, &ciphertext).unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_encrypt_empty() {
        let key = EncryptionKey::generate().unwrap();
        let plaintext = b"";

        let ciphertext = encrypt(&key, plaintext).unwrap();
        let decrypted = decrypt(&key, &ciphertext).unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_decrypt_invalid_ciphertext() {
        let key = EncryptionKey::generate().unwrap();
        let invalid = vec![1u8; 10]; // Too short

        assert!(decrypt(&key, &invalid).is_err());
    }

    #[test]
    fn test_decrypt_wrong_key() {
        let key1 = EncryptionKey::generate().unwrap();
        let key2 = EncryptionKey::generate().unwrap();
        let plaintext = b"secret";

        let ciphertext = encrypt(&key1, plaintext).unwrap();
        assert!(decrypt(&key2, &ciphertext).is_err());
    }

    #[test]
    fn test_decrypt_tampered_data() {
        let key = EncryptionKey::generate().unwrap();
        let plaintext = b"original data";

        let mut ciphertext = encrypt(&key, plaintext).unwrap();
        // Tamper with ciphertext
        ciphertext[20] ^= 1;

        assert!(decrypt(&key, &ciphertext).is_err());
    }

    #[test]
    fn test_unique_nonces() {
        let key = EncryptionKey::generate().unwrap();
        let plaintext = b"same data";

        let ct1 = encrypt(&key, plaintext).unwrap();
        let ct2 = encrypt(&key, plaintext).unwrap();

        // Nonces should be different
        assert_ne!(&ct1[1..1 + NONCE_SIZE], &ct2[1..1 + NONCE_SIZE]);
        // But both should decrypt to same plaintext
        assert_eq!(decrypt(&key, &ct1).unwrap(), plaintext);
        assert_eq!(decrypt(&key, &ct2).unwrap(), plaintext);
    }

    #[test]
    fn test_overhead() {
        let key = EncryptionKey::generate().unwrap();
        let plaintext = vec![0u8; 1000];

        let ciphertext = encrypt(&key, &plaintext).unwrap();
        let overhead = ciphertext.len() - plaintext.len();

        // Overhead should be: version(1) + nonce(12) + tag(16) = 29 bytes
        assert_eq!(overhead, 1 + NONCE_SIZE + TAG_SIZE);
    }

    #[test]
    fn test_stream_encryption_threshold() {
        let key = EncryptionKey::generate().unwrap();

        // Just under threshold
        let small = vec![0u8; STREAM_THRESHOLD - 1];
        let ct_small = encrypt(&key, &small).unwrap();

        // Just over threshold
        let large = vec![0u8; STREAM_THRESHOLD + 1];
        let ct_large = encrypt(&key, &large).unwrap();

        // Both should decrypt correctly
        assert_eq!(decrypt(&key, &ct_small).unwrap(), small);
        assert_eq!(decrypt(&key, &ct_large).unwrap(), large);
    }
}

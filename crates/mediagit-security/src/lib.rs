// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

#![allow(missing_docs)]
//! Authentication, encryption, and audit trail for MediaGit.
//!
//! Provides security primitives for protecting repository data at rest
//! and in transit, plus authentication for remote operations.
//!
//! # Components
//!
//! - **Encryption**: AES-256-GCM symmetric encryption with Argon2id key derivation
//! - **Authentication**: JWT tokens and API key verification (`auth` feature)
//! - **TLS**: Certificate management for secure transport (`tls` feature)
//! - **Audit**: Structured audit trail for security-sensitive operations
//!
//! # Security Model
//!
//! Keys are never logged or serialized in plaintext. [`encryption::EncryptionKey`]
//! wraps raw key material and only exposes it through `expose_key()` to make
//! accidental leakage difficult.

// Re-export encryption and KDF modules
pub mod encryption;
pub mod envelope;
pub mod kdf;

// Audit logging module
pub mod audit;

// Tag signing (OpenSSH ed25519 key reuse; MediaGit-native signature format)
pub mod sign;

// Authentication module
#[cfg(feature = "auth")]
pub mod auth;

// TLS/Certificate management module
#[cfg(feature = "tls")]
pub mod tls;

/// Re-exported so callers can build the `SecretString` that [`kdf::derive_key`]
/// takes without taking their own `secrecy` dependency (and risking a second,
/// incompatible version of it in the tree).
pub use secrecy::SecretString;

/// Re-exported for the same reason as [`SecretString`]: callers that hold raw
/// key material only long enough to hand it somewhere else need a `Drop`-time
/// wipe, and should get it from the one `zeroize` version this tree agrees on.
pub use zeroize::Zeroizing;

// Re-export commonly used types
pub use audit::{
    AuditEvent, AuditEventType, log_access_denied, log_authentication_failed,
    log_authentication_success, log_invalid_request, log_path_traversal_attempt,
    log_rate_limit_exceeded, log_suspicious_pattern,
};

#[cfg(feature = "auth")]
pub use auth::{
    ApiKey, ApiKeyAuth, AuthError, AuthLayer, AuthResponse, AuthResult, AuthService, AuthUser,
    Claims, CredentialsStore, ErrorResponse, JwtAuth, LoginRequest, RefreshRequest,
    RegisterRequest, TokenPair, User, UserCredentials, UserId, UserInfo, login_handler,
    logout_handler, me_handler, refresh_handler, register_handler, user::Role,
};

#[cfg(feature = "tls")]
pub use tls::{
    Certificate, CertificateBuilder, CertificateError, TlsConfig, TlsConfigBuilder, TlsError,
    TlsResult, config::TlsVersion,
};

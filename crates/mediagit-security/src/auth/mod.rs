// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Authentication module for MediaGit
//!
//! Provides JWT-based and API key authentication for secure access control.
//!
//! # Features
//! - JWT token generation and validation
//! - API key authentication
//! - User management with password hashing
//! - Permission-based access control
//! - HTTP handlers for auth endpoints
//!
//! # Example
//! ```no_run
//! use mediagit_security::auth::{JwtAuth, Claims};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let jwt_auth = JwtAuth::new("secret-key");
//! let token = jwt_auth.generate_token("user@example.com", vec!["repo:read".to_string()])?;
//! let claims = jwt_auth.validate_token(&token)?;
//! # Ok(())
//! # }
//! ```

pub mod apikey;
pub mod credentials;
pub mod grants;
pub mod handlers;
pub mod jwt;
pub mod middleware;
mod persist;
pub mod revocation;
pub mod user;

pub use apikey::{ApiKey, ApiKeyAuth};
pub use credentials::{CredentialsStore, UserCredentials};
pub use grants::{GrantsStore, Level as GrantLevel};
pub use handlers::{
    AuthResponse, AuthService, ErrorResponse, LoginRequest, RefreshRequest, RegisterRequest,
    UserInfo, login_handler, logout_handler, me_handler, refresh_handler, register_handler,
    validate_password_strength, validate_registration_input,
};
pub use jwt::{Claims, JwtAuth, TokenPair};
pub use middleware::{AuthLayer, AuthUser, auth_middleware};
pub use user::{User, UserId};

use thiserror::Error;

/// Authentication errors
#[derive(Debug, Error)]
pub enum AuthError {
    /// Invalid or expired JWT token
    #[error("Invalid or expired token: {0}")]
    InvalidToken(String),

    /// Invalid API key
    #[error("Invalid API key")]
    InvalidApiKey,

    /// User not found
    #[error("User not found: {0}")]
    UserNotFound(String),

    /// Unauthorized access
    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    /// Internal error
    #[error("Authentication error: {0}")]
    Internal(#[from] anyhow::Error),
}

pub type AuthResult<T> = Result<T, AuthError>;

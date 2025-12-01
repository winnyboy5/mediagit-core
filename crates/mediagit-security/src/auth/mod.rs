//! Authentication module for MediaGit
//!
//! Provides JWT-based and API key authentication for secure access control.
//!
//! # Features
//! - JWT token generation and validation
//! - API key authentication
//! - User management
//! - Permission-based access control
//!
//! # Example
//! ```no_run
//! use mediagit_security::auth::{JwtAuth, Claims};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let jwt_auth = JwtAuth::new("secret-key");
//! let token = jwt_auth.generate_token("user@example.com", vec!["repo:read"])?;
//! let claims = jwt_auth.validate_token(&token)?;
//! # Ok(())
//! # }
//! ```

pub mod jwt;
pub mod apikey;
pub mod user;
pub mod middleware;

pub use jwt::{JwtAuth, Claims, TokenPair};
pub use apikey::{ApiKeyAuth, ApiKey};
pub use user::{User, UserId};
pub use middleware::{AuthLayer, AuthUser, auth_middleware};

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

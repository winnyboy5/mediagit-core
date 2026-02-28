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
pub mod handlers;
pub mod jwt;
pub mod middleware;
pub mod user;

pub use apikey::{ApiKey, ApiKeyAuth};
pub use credentials::{CredentialsStore, UserCredentials};
pub use handlers::{
    login_handler, logout_handler, me_handler, refresh_handler, register_handler, AuthResponse,
    AuthService, ErrorResponse, LoginRequest, RefreshRequest, RegisterRequest, UserInfo,
};
pub use jwt::{Claims, JwtAuth, TokenPair};
pub use middleware::{auth_middleware, AuthLayer, AuthUser};
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

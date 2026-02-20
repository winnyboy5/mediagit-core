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

pub mod jwt;
pub mod apikey;
pub mod user;
pub mod middleware;
pub mod credentials;
pub mod handlers;

pub use jwt::{JwtAuth, Claims, TokenPair};
pub use apikey::{ApiKeyAuth, ApiKey};
pub use user::{User, UserId};
pub use middleware::{AuthLayer, AuthUser, auth_middleware};
pub use credentials::{UserCredentials, CredentialsStore};
pub use handlers::{
    AuthService, RegisterRequest, LoginRequest, RefreshRequest,
    AuthResponse, UserInfo, ErrorResponse,
    register_handler, login_handler, refresh_handler, me_handler, logout_handler,
};

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

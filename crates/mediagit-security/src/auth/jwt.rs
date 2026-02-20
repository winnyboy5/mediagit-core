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
//! JWT (JSON Web Token) authentication implementation
//!
//! Provides secure token generation and validation using HMAC-SHA256.

use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

use super::{AuthError, AuthResult};

/// JWT claims containing user identity and permissions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// Subject (user identifier)
    pub sub: String,

    /// Issued at (timestamp)
    pub iat: i64,

    /// Expiration time (timestamp)
    pub exp: i64,

    /// User permissions
    pub permissions: Vec<String>,
}

/// Token pair (access token + refresh token)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
}

/// JWT authentication handler
#[derive(Clone)]
pub struct JwtAuth {
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
    access_token_duration: Duration,
    refresh_token_duration: Duration,
}

impl JwtAuth {
    /// Create new JWT auth with secret key
    ///
    /// # Arguments
    /// * `secret` - Secret key for signing tokens
    ///
    /// # Example
    /// ```
    /// use mediagit_security::auth::JwtAuth;
    ///
    /// let jwt_auth = JwtAuth::new("my-secret-key");
    /// ```
    pub fn new(secret: &str) -> Self {
        Self {
            encoding_key: EncodingKey::from_secret(secret.as_bytes()),
            decoding_key: DecodingKey::from_secret(secret.as_bytes()),
            access_token_duration: Duration::hours(24),
            refresh_token_duration: Duration::days(30),
        }
    }

    /// Create JWT auth with custom token durations
    pub fn with_durations(
        secret: &str,
        access_duration: Duration,
        refresh_duration: Duration,
    ) -> Self {
        Self {
            encoding_key: EncodingKey::from_secret(secret.as_bytes()),
            decoding_key: DecodingKey::from_secret(secret.as_bytes()),
            access_token_duration: access_duration,
            refresh_token_duration: refresh_duration,
        }
    }

    /// Generate access token for user
    ///
    /// # Arguments
    /// * `user_id` - User identifier
    /// * `permissions` - List of user permissions
    ///
    /// # Returns
    /// JWT token string
    pub fn generate_token(
        &self,
        user_id: &str,
        permissions: Vec<String>,
    ) -> AuthResult<String> {
        let now = Utc::now();
        let claims = Claims {
            sub: user_id.to_string(),
            iat: now.timestamp(),
            exp: (now + self.access_token_duration).timestamp(),
            permissions,
        };

        encode(&Header::default(), &claims, &self.encoding_key)
            .map_err(|e| AuthError::Internal(e.into()))
    }

    /// Generate token pair (access + refresh)
    pub fn generate_token_pair(
        &self,
        user_id: &str,
        permissions: Vec<String>,
    ) -> AuthResult<TokenPair> {
        let access_token = self.generate_token(user_id, permissions.clone())?;

        // Refresh token with longer duration
        let now = Utc::now();
        let refresh_claims = Claims {
            sub: user_id.to_string(),
            iat: now.timestamp(),
            exp: (now + self.refresh_token_duration).timestamp(),
            permissions,
        };

        let refresh_token = encode(&Header::default(), &refresh_claims, &self.encoding_key)
            .map_err(|e| AuthError::Internal(e.into()))?;

        Ok(TokenPair {
            access_token,
            refresh_token,
            expires_in: self.access_token_duration.num_seconds(),
        })
    }

    /// Validate and decode JWT token
    ///
    /// # Arguments
    /// * `token` - JWT token string
    ///
    /// # Returns
    /// Decoded claims if token is valid
    ///
    /// # Errors
    /// Returns `AuthError::InvalidToken` if token is invalid or expired
    pub fn validate_token(&self, token: &str) -> AuthResult<Claims> {
        let validation = Validation::default();

        decode::<Claims>(token, &self.decoding_key, &validation)
            .map(|data| data.claims)
            .map_err(|e| AuthError::InvalidToken(e.to_string()))
    }

    /// Refresh access token using refresh token
    pub fn refresh_access_token(&self, refresh_token: &str) -> AuthResult<String> {
        let claims = self.validate_token(refresh_token)?;

        // Generate new access token with same permissions
        self.generate_token(&claims.sub, claims.permissions)
    }

    /// Extract token from Authorization header
    ///
    /// Expects format: "Bearer <token>"
    pub fn extract_from_header(auth_header: &str) -> AuthResult<&str> {
        auth_header
            .strip_prefix("Bearer ")
            .ok_or_else(|| {
                AuthError::InvalidToken("Invalid Authorization header format".to_string())
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_and_validate_token() {
        let jwt_auth = JwtAuth::new("test-secret");
        let permissions = vec!["repo:read".to_string(), "repo:write".to_string()];

        let token = jwt_auth
            .generate_token("user@example.com", permissions.clone())
            .unwrap();

        let claims = jwt_auth.validate_token(&token).unwrap();

        assert_eq!(claims.sub, "user@example.com");
        assert_eq!(claims.permissions, permissions);
    }

    #[test]
    fn test_invalid_token() {
        let jwt_auth = JwtAuth::new("test-secret");

        let result = jwt_auth.validate_token("invalid.token.here");
        assert!(result.is_err());
    }

    #[test]
    fn test_token_pair_generation() {
        let jwt_auth = JwtAuth::new("test-secret");
        let permissions = vec!["repo:read".to_string()];

        let token_pair = jwt_auth
            .generate_token_pair("user@example.com", permissions)
            .unwrap();

        // Both tokens should be valid
        assert!(jwt_auth.validate_token(&token_pair.access_token).is_ok());
        assert!(jwt_auth.validate_token(&token_pair.refresh_token).is_ok());
    }

    #[test]
    fn test_refresh_token() {
        let jwt_auth = JwtAuth::new("test-secret");
        let permissions = vec!["repo:read".to_string()];

        let token_pair = jwt_auth
            .generate_token_pair("user@example.com", permissions)
            .unwrap();

        let new_access_token = jwt_auth
            .refresh_access_token(&token_pair.refresh_token)
            .unwrap();

        let claims = jwt_auth.validate_token(&new_access_token).unwrap();
        assert_eq!(claims.sub, "user@example.com");
    }

    #[test]
    fn test_extract_from_header() {
        let token = "abc123xyz";
        let header = format!("Bearer {}", token);

        let extracted = JwtAuth::extract_from_header(&header).unwrap();
        assert_eq!(extracted, token);
    }

    #[test]
    fn test_extract_from_invalid_header() {
        let result = JwtAuth::extract_from_header("Invalid header");
        assert!(result.is_err());
    }
}

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
//! Axum middleware for authentication
//!
//! Provides HTTP middleware for JWT and API key authentication.

use axum::{
    async_trait,
    extract::{FromRequestParts, Request},
    http::{request::Parts, HeaderMap, StatusCode},
    middleware::Next,
    response::Response,
};
use std::sync::Arc;

use super::{ApiKeyAuth, AuthError, JwtAuth};

/// Authenticated user information extracted from request
#[derive(Debug, Clone)]
pub struct AuthUser {
    /// User ID
    pub user_id: String,

    /// User permissions
    pub permissions: Vec<String>,

    /// Authentication method used
    pub auth_method: AuthMethod,
}

/// Implement FromRequestParts for AuthUser to enable it as an extractor
#[async_trait]
impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
{
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthUser>()
            .cloned()
            .ok_or(StatusCode::UNAUTHORIZED)
    }
}

/// Authentication method
#[derive(Debug, Clone)]
pub enum AuthMethod {
    Jwt,
    ApiKey,
}

impl AuthUser {
    /// Check if user has specific permission
    pub fn has_permission(&self, permission: &str) -> bool {
        self.permissions.contains(&permission.to_string())
    }

    /// Check if user has any of the specified permissions
    pub fn has_any_permission(&self, permissions: &[&str]) -> bool {
        permissions
            .iter()
            .any(|p| self.permissions.contains(&p.to_string()))
    }

    /// Check if user has all specified permissions
    pub fn has_all_permissions(&self, permissions: &[&str]) -> bool {
        permissions
            .iter()
            .all(|p| self.permissions.contains(&p.to_string()))
    }
}

/// Authentication layer for Axum
#[derive(Clone)]
pub struct AuthLayer {
    jwt_auth: Arc<JwtAuth>,
    api_key_auth: Arc<ApiKeyAuth>,
}

impl AuthLayer {
    /// Create new authentication layer
    pub fn new(jwt_auth: Arc<JwtAuth>, api_key_auth: Arc<ApiKeyAuth>) -> Self {
        Self {
            jwt_auth,
            api_key_auth,
        }
    }

    /// Get reference to JWT auth (for testing)
    pub fn jwt_auth(&self) -> &Arc<JwtAuth> {
        &self.jwt_auth
    }

    /// Get reference to API key auth (for testing)
    pub fn api_key_auth(&self) -> &Arc<ApiKeyAuth> {
        &self.api_key_auth
    }

    /// Authenticate request
    ///
    /// Tries JWT first, then API key if JWT fails
    pub async fn authenticate(&self, headers: &HeaderMap) -> Result<AuthUser, AuthError> {
        // Try JWT authentication first
        if let Some(auth_header) = headers.get("authorization") {
            if let Ok(auth_str) = auth_header.to_str() {
                if let Ok(token) = JwtAuth::extract_from_header(auth_str) {
                    if let Ok(claims) = self.jwt_auth.validate_token(token) {
                        return Ok(AuthUser {
                            user_id: claims.sub,
                            permissions: claims.permissions,
                            auth_method: AuthMethod::Jwt,
                        });
                    }
                }
            }
        }

        // Try API key authentication
        if let Some(api_key_header) = headers.get("x-api-key") {
            if let Ok(api_key) = api_key_header.to_str() {
                let key = ApiKeyAuth::extract_from_header(api_key);
                if let Ok(api_key_info) = self.api_key_auth.validate_key(key).await {
                    return Ok(AuthUser {
                        user_id: api_key_info.user_id,
                        permissions: api_key_info.permissions,
                        auth_method: AuthMethod::ApiKey,
                    });
                }
            }
        }

        Err(AuthError::Unauthorized(
            "No valid authentication credentials provided".to_string(),
        ))
    }
}

/// Middleware function for Axum
pub async fn auth_middleware(
    auth_layer: Arc<AuthLayer>,
    mut req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let headers = req.headers().clone();

    match auth_layer.authenticate(&headers).await {
        Ok(auth_user) => {
            // Insert authenticated user into request extensions
            req.extensions_mut().insert(auth_user);

            Ok(next.run(req).await)
        }
        Err(e) => {
            tracing::warn!("Authentication failed: {}", e);
            Err(StatusCode::UNAUTHORIZED)
        }
    }
}

/// Extract authenticated user from request extensions
pub fn get_auth_user(req: &Request) -> Option<&AuthUser> {
    req.extensions().get::<AuthUser>()
}

/// Require specific permission
pub fn require_permission(auth_user: &AuthUser, permission: &str) -> Result<(), StatusCode> {
    if auth_user.has_permission(permission) {
        Ok(())
    } else {
        tracing::warn!(
            "User {} lacks permission: {}",
            auth_user.user_id,
            permission
        );
        Err(StatusCode::FORBIDDEN)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::clone_on_ref_ptr)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};

    #[tokio::test]
    async fn test_jwt_authentication() {
        let jwt_auth = Arc::new(JwtAuth::new("test-secret"));
        let api_key_auth = Arc::new(ApiKeyAuth::new());
        let auth_layer = AuthLayer::new(jwt_auth.clone(), api_key_auth);

        // Generate token
        let permissions = vec!["repo:read".to_string()];
        let token = jwt_auth
            .generate_token("user@example.com", permissions.clone())
            .unwrap();

        // Create headers with JWT
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        );

        // Authenticate
        let auth_user = auth_layer.authenticate(&headers).await.unwrap();

        assert_eq!(auth_user.user_id, "user@example.com");
        assert_eq!(auth_user.permissions, permissions);
        assert!(matches!(auth_user.auth_method, AuthMethod::Jwt));
    }

    #[tokio::test]
    async fn test_api_key_authentication() {
        let jwt_auth = Arc::new(JwtAuth::new("test-secret"));
        let api_key_auth = Arc::new(ApiKeyAuth::new());
        let auth_layer = AuthLayer::new(jwt_auth, api_key_auth.clone());

        // Generate API key
        let permissions = vec!["repo:write".to_string()];
        let (plaintext_key, _) = api_key_auth
            .generate_key("user123".to_string(), "Test Key".to_string(), permissions.clone())
            .await
            .unwrap();

        // Create headers with API key
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-api-key",
            HeaderValue::from_str(&plaintext_key).unwrap(),
        );

        // Authenticate
        let auth_user = auth_layer.authenticate(&headers).await.unwrap();

        assert_eq!(auth_user.user_id, "user123");
        assert_eq!(auth_user.permissions, permissions);
        assert!(matches!(auth_user.auth_method, AuthMethod::ApiKey));
    }

    #[tokio::test]
    async fn test_no_authentication() {
        let jwt_auth = Arc::new(JwtAuth::new("test-secret"));
        let api_key_auth = Arc::new(ApiKeyAuth::new());
        let auth_layer = AuthLayer::new(jwt_auth, api_key_auth);

        let headers = HeaderMap::new();

        let result = auth_layer.authenticate(&headers).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_auth_user_permissions() {
        let auth_user = AuthUser {
            user_id: "test".to_string(),
            permissions: vec!["repo:read".to_string(), "repo:write".to_string()],
            auth_method: AuthMethod::Jwt,
        };

        assert!(auth_user.has_permission("repo:read"));
        assert!(auth_user.has_permission("repo:write"));
        assert!(!auth_user.has_permission("repo:admin"));

        assert!(auth_user.has_any_permission(&["repo:read", "repo:admin"]));
        assert!(auth_user.has_all_permissions(&["repo:read", "repo:write"]));
        assert!(!auth_user.has_all_permissions(&["repo:read", "repo:admin"]));
    }
}

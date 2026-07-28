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

//! HTTP handlers for authentication endpoints
//!
//! Provides Axum handlers for user registration, login, logout, and token refresh.

use axum::{Json, extract::State, http::StatusCode};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use tracing::{info, warn};

use super::{
    AuthError, AuthResult, JwtAuth, TokenPair,
    credentials::CredentialsStore,
    user::{Role, User},
};

/// Shared authentication service state
#[derive(Clone)]
pub struct AuthService {
    pub jwt_auth: Arc<JwtAuth>,
    pub credentials_store: Arc<CredentialsStore>,
    /// Whether `POST /auth/register` is open to anonymous callers. Defaults
    /// to `true` on every constructor below (matches
    /// `ServerConfig::allow_open_registration`'s serde default) so existing
    /// behavior is unchanged unless a caller explicitly opts out.
    pub allow_open_registration: bool,
}

impl AuthService {
    /// Create new authentication service
    pub fn new(jwt_secret: &str) -> Self {
        Self {
            jwt_auth: Arc::new(JwtAuth::new(jwt_secret)),
            credentials_store: Arc::new(CredentialsStore::new()),
            allow_open_registration: true,
        }
    }

    /// Create with custom JWT auth and credentials store
    pub fn with_components(
        jwt_auth: Arc<JwtAuth>,
        credentials_store: Arc<CredentialsStore>,
    ) -> Self {
        Self {
            jwt_auth,
            credentials_store,
            allow_open_registration: true,
        }
    }

    /// Create an authentication service whose credentials are persisted
    /// under `store_dir` (see [`CredentialsStore::load_or_new`]).
    ///
    /// AU-9: this used to also build a `GrantsStore`, which nothing ever read
    /// or wrote. `AppState::grants` is the single instance that
    /// `check_permission` consults and the admin handlers mutate; a second
    /// one here only agreed with it at boot, so any code that reached for it
    /// would have silently no-op'd until the next restart. Removed rather
    /// than documented, so the trap cannot be stepped in.
    pub fn new_with_store_dir(jwt_secret: &str, store_dir: &Path) -> AuthResult<Self> {
        Ok(Self {
            jwt_auth: Arc::new(JwtAuth::new(jwt_secret)),
            credentials_store: Arc::new(CredentialsStore::load_or_new(store_dir)?),
            allow_open_registration: true,
        })
    }
}

// Request/Response types

/// User registration request
#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub email: String,
    pub password: String,
}

/// User login request
#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    /// Email or username
    pub identifier: String,
    pub password: String,
}

/// Token refresh request
#[derive(Debug, Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

/// Authentication response with tokens
#[derive(Debug, Serialize, Deserialize)]
pub struct AuthResponse {
    pub user: UserInfo,
    pub tokens: TokenPair,
}

/// User information (without sensitive data)
#[derive(Debug, Serialize, Deserialize)]
pub struct UserInfo {
    pub id: String,
    pub username: String,
    pub email: String,
    pub role: Role,
    pub permissions: Vec<String>,
}

impl From<User> for UserInfo {
    fn from(user: User) -> Self {
        let permissions = user.permissions();
        Self {
            id: user.id,
            username: user.username,
            email: user.email,
            role: user.role,
            permissions,
        }
    }
}

/// Error response
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

// Shared validation rules

/// Password strength rule shared by registration, self-service password
/// change, and admin password reset, so the minimum-length rule cannot
/// drift between call sites.
pub fn validate_password_strength(password: &str) -> Result<(), String> {
    if password.is_empty() {
        return Err("Password is required".to_string());
    }
    if password.len() < 8 {
        return Err("Password must be at least 8 characters".to_string());
    }
    Ok(())
}

/// Registration input rules shared by `POST /auth/register` and the future
/// server CLI / admin create-user path, so username/email/password rules
/// cannot drift between call sites.
pub fn validate_registration_input(
    username: &str,
    email: &str,
    password: &str,
) -> Result<(), String> {
    let username = username.trim();
    let email = email.trim();

    if username.is_empty() || email.is_empty() || password.is_empty() {
        return Err("Username, email, and password are required".to_string());
    }
    if username.len() < 3 {
        return Err("Username must be at least 3 characters".to_string());
    }
    if !email.contains('@') || !email.contains('.') {
        return Err("Invalid email format".to_string());
    }
    validate_password_strength(password)
}

// Handler functions

/// Register new user
///
/// POST /auth/register
/// Body: RegisterRequest
pub async fn register_handler(
    State(auth_service): State<Arc<AuthService>>,
    Json(req): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<AuthResponse>), (StatusCode, Json<ErrorResponse>)> {
    if !auth_service.allow_open_registration {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ErrorResponse {
                error: "Registration is closed on this server; ask an administrator to create your account".to_string(),
            }),
        ));
    }

    validate_registration_input(&req.username, &req.email, &req.password)
        .map_err(|error| (StatusCode::BAD_REQUEST, Json(ErrorResponse { error })))?;

    // Create user with unique ID.
    //
    // AU-3: self-registration creates a **Read**-role account. It previously
    // granted Write, which composed badly with two other defaults: open
    // registration, and per-repo grant enforcement that only activates once a
    // grant exists (`check_permission`'s `!grants.is_empty()`). On a fresh
    // server with no grants recorded, anyone who could reach this endpoint
    // gained write access to every repository.
    //
    // Read is the least privilege that keeps self-registration useful; an
    // admin promotes the account afterwards via `PUT /auth/users/{id}/role`
    // when write access is actually warranted.
    //
    // There is still no client-controlled `role` field — that would let an
    // unauthenticated caller mint an Admin account via the request body.
    let user_id = uuid::Uuid::new_v4().to_string();
    let user = User::new(user_id, req.username, req.email, Role::Read);

    // Register user
    match auth_service
        .credentials_store
        .register_user(user.clone(), &req.password)
        .await
    {
        Ok(_) => {
            info!("User registered: {} ({})", user.username, user.id);

            // Generate tokens
            let permissions = user.permissions();
            let tokens = auth_service
                .jwt_auth
                .generate_token_pair(&user.id, permissions)
                .map_err(|e| {
                    warn!("Token generation failed: {}", e);
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse {
                            error: "Failed to generate tokens".to_string(),
                        }),
                    )
                })?;

            Ok((
                StatusCode::CREATED,
                Json(AuthResponse {
                    user: user.into(),
                    tokens,
                }),
            ))
        }
        Err(e) => {
            warn!("User registration failed: {}", e);
            Err((
                StatusCode::CONFLICT,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            ))
        }
    }
}

/// Login user
///
/// POST /auth/login
/// Body: LoginRequest
pub async fn login_handler(
    State(auth_service): State<Arc<AuthService>>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<AuthResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Authenticate user
    match auth_service
        .credentials_store
        .authenticate(&req.identifier, &req.password)
        .await
    {
        Ok(user) => {
            info!("User logged in: {} ({})", user.username, user.id);

            // Generate tokens
            let permissions = user.permissions();
            let tokens = auth_service
                .jwt_auth
                .generate_token_pair(&user.id, permissions)
                .map_err(|e| {
                    warn!("Token generation failed: {}", e);
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse {
                            error: "Failed to generate tokens".to_string(),
                        }),
                    )
                })?;

            Ok(Json(AuthResponse {
                user: user.into(),
                tokens,
            }))
        }
        Err(_) => {
            warn!("Login failed for: {}", req.identifier);
            Err((
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponse {
                    error: "Invalid credentials".to_string(),
                }),
            ))
        }
    }
}

/// Refresh access token
///
/// POST /auth/refresh
/// Body: RefreshRequest
pub async fn refresh_handler(
    State(auth_service): State<Arc<AuthService>>,
    Json(req): Json<RefreshRequest>,
) -> Result<Json<TokenPair>, (StatusCode, Json<ErrorResponse>)> {
    match auth_service
        .jwt_auth
        .refresh_access_token(&req.refresh_token)
    {
        Ok(access_token) => {
            // Decode to get expiration
            match auth_service.jwt_auth.validate_token(&access_token) {
                Ok(claims) => {
                    let expires_in = claims.exp - chrono::Utc::now().timestamp();

                    Ok(Json(TokenPair {
                        access_token,
                        refresh_token: req.refresh_token,
                        expires_in,
                    }))
                }
                _ => Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "Token generation failed".to_string(),
                    }),
                )),
            }
        }
        Err(e) => {
            warn!("Token refresh failed: {}", e);
            Err((
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponse {
                    error: "Invalid refresh token".to_string(),
                }),
            ))
        }
    }
}

/// Get current user information
///
/// GET /auth/me
/// Requires: Authorization header with JWT
pub async fn me_handler(
    State(auth_service): State<Arc<AuthService>>,
    auth_user: super::middleware::AuthUser,
) -> Result<Json<UserInfo>, (StatusCode, Json<ErrorResponse>)> {
    match auth_service
        .credentials_store
        .get_user(&auth_user.user_id)
        .await
    {
        Ok(user) => Ok(Json(user.into())),
        Err(e) => {
            warn!("Failed to get user info: {}", e);
            Err((
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "User not found".to_string(),
                }),
            ))
        }
    }
}

/// Logout user (client-side token invalidation)
///
/// POST /auth/logout
/// Note: With JWT, logout is primarily client-side (delete tokens).
/// This endpoint exists for consistency and future token blacklisting.
pub async fn logout_handler() -> StatusCode {
    info!("User logout requested");
    StatusCode::NO_CONTENT
}

// Helper function to convert AuthError to HTTP response
pub fn auth_error_to_response(error: AuthError) -> (StatusCode, Json<ErrorResponse>) {
    let (status, message) = match error {
        AuthError::InvalidToken(_) => (StatusCode::UNAUTHORIZED, "Invalid token".to_string()),
        AuthError::InvalidApiKey => (StatusCode::UNAUTHORIZED, "Invalid API key".to_string()),
        AuthError::UserNotFound(id) => (StatusCode::NOT_FOUND, format!("User not found: {}", id)),
        AuthError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg),
        AuthError::Internal(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };

    (status, Json(ErrorResponse { error: message }))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_register_login_flow() {
        let auth_service = Arc::new(AuthService::new("test-secret"));

        // Register user
        let register_req = RegisterRequest {
            username: "testuser".to_string(),
            email: "test@example.com".to_string(),
            password: "password123".to_string(),
        };

        let result = register_handler(State(Arc::clone(&auth_service)), Json(register_req)).await;

        assert!(result.is_ok());
        let (status, response) = result.unwrap();
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(response.user.username, "testuser");
        assert!(!response.tokens.access_token.is_empty());

        // Login with same credentials
        let login_req = LoginRequest {
            identifier: "test@example.com".to_string(),
            password: "password123".to_string(),
        };

        let result = login_handler(State(Arc::clone(&auth_service)), Json(login_req)).await;

        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.user.username, "testuser");
        assert!(!response.tokens.access_token.is_empty());
    }

    #[tokio::test]
    async fn test_invalid_login() {
        let auth_service = Arc::new(AuthService::new("test-secret"));

        let login_req = LoginRequest {
            identifier: "nonexistent@example.com".to_string(),
            password: "wrongpassword".to_string(),
        };

        let result = login_handler(State(auth_service), Json(login_req)).await;

        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_password_validation() {
        let auth_service = Arc::new(AuthService::new("test-secret"));

        let register_req = RegisterRequest {
            username: "testuser".to_string(),
            email: "test@example.com".to_string(),
            password: "short".to_string(), // Too short
        };

        let result = register_handler(State(auth_service), Json(register_req)).await;

        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_registration_closed_rejected() {
        let mut auth_service = AuthService::new("test-secret");
        auth_service.allow_open_registration = false;
        let auth_service = Arc::new(auth_service);

        let register_req = RegisterRequest {
            username: "testuser".to_string(),
            email: "test@example.com".to_string(),
            password: "password123".to_string(),
        };

        let result = register_handler(State(auth_service), Json(register_req)).await;

        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_registration_open_by_default() {
        // Default AuthService::new must keep behaving exactly as before —
        // allow_open_registration defaults to true.
        let auth_service = Arc::new(AuthService::new("test-secret"));
        assert!(auth_service.allow_open_registration);

        let register_req = RegisterRequest {
            username: "testuser".to_string(),
            email: "test@example.com".to_string(),
            password: "password123".to_string(),
        };

        let result = register_handler(State(auth_service), Json(register_req)).await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_registration_input() {
        assert!(validate_registration_input("ab", "a@b.com", "password123").is_err());
        assert!(validate_registration_input("abc", "not-an-email", "password123").is_err());
        assert!(validate_registration_input("abc", "a@b.com", "short").is_err());
        assert!(validate_registration_input("abc", "a@b.com", "password123").is_ok());
    }

    #[test]
    fn test_validate_password_strength() {
        assert!(validate_password_strength("short").is_err());
        assert!(validate_password_strength("").is_err());
        assert!(validate_password_strength("longenough").is_ok());
    }

    #[tokio::test]
    async fn test_refresh_token() {
        let auth_service = Arc::new(AuthService::new("test-secret"));

        // Register user first
        let register_req = RegisterRequest {
            username: "testuser".to_string(),
            email: "test@example.com".to_string(),
            password: "password123".to_string(),
        };

        let (_, auth_response) =
            register_handler(State(Arc::clone(&auth_service)), Json(register_req))
                .await
                .unwrap();

        // Refresh token
        let refresh_req = RefreshRequest {
            refresh_token: auth_response.tokens.refresh_token.clone(),
        };

        let result = refresh_handler(State(auth_service), Json(refresh_req)).await;

        assert!(result.is_ok());
        let new_tokens = result.unwrap();
        assert!(!new_tokens.access_token.is_empty());
    }
}

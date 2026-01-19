//! HTTP handlers for authentication endpoints
//!
//! Provides Axum handlers for user registration, login, logout, and token refresh.

use axum::{
    extract::State,
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{info, warn};

use super::{
    credentials::CredentialsStore,
    user::{Role, User},
    AuthError, JwtAuth, TokenPair,
};

/// Shared authentication service state
#[derive(Clone)]
pub struct AuthService {
    pub jwt_auth: Arc<JwtAuth>,
    pub credentials_store: Arc<CredentialsStore>,
}

impl AuthService {
    /// Create new authentication service
    pub fn new(jwt_secret: &str) -> Self {
        Self {
            jwt_auth: Arc::new(JwtAuth::new(jwt_secret)),
            credentials_store: Arc::new(CredentialsStore::new()),
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
        }
    }
}

// Request/Response types

/// User registration request
#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub email: String,
    pub password: String,
    #[serde(default = "default_role")]
    pub role: Role,
}

fn default_role() -> Role {
    Role::Write
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

// Handler functions

/// Register new user
///
/// POST /auth/register
/// Body: RegisterRequest
pub async fn register_handler(
    State(auth_service): State<Arc<AuthService>>,
    Json(req): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<AuthResponse>), (StatusCode, Json<ErrorResponse>)> {
    // Validate input - check for empty or whitespace-only strings
    let username = req.username.trim();
    let email = req.email.trim();
    let password = &req.password; // Don't trim password (whitespace can be intentional)

    if username.is_empty() || email.is_empty() || password.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Username, email, and password are required".to_string(),
            }),
        ));
    }

    // Validate username length and characters
    if username.len() < 3 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Username must be at least 3 characters".to_string(),
            }),
        ));
    }

    // Basic email format validation
    if !email.contains('@') || !email.contains('.') {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Invalid email format".to_string(),
            }),
        ));
    }

    if password.len() < 8 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Password must be at least 8 characters".to_string(),
            }),
        ));
    }

    // Create user with unique ID
    let user_id = uuid::Uuid::new_v4().to_string();
    let user = User::new(user_id, req.username, req.email, req.role);

    // Register user
    match auth_service.credentials_store.register_user(user.clone(), &req.password).await {
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
    match auth_service.credentials_store.authenticate(&req.identifier, &req.password).await {
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
    match auth_service.jwt_auth.refresh_access_token(&req.refresh_token) {
        Ok(access_token) => {
            // Decode to get expiration
            if let Ok(claims) = auth_service.jwt_auth.validate_token(&access_token) {
                let expires_in = claims.exp - chrono::Utc::now().timestamp();

                Ok(Json(TokenPair {
                    access_token,
                    refresh_token: req.refresh_token,
                    expires_in,
                }))
            } else {
                Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "Token generation failed".to_string(),
                    }),
                ))
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
    match auth_service.credentials_store.get_user(&auth_user.user_id).await {
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
            role: Role::Write,
        };

        let result = register_handler(
            State(Arc::clone(&auth_service)),
            Json(register_req),
        ).await;

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

        let result = login_handler(
            State(Arc::clone(&auth_service)),
            Json(login_req),
        ).await;

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

        let result = login_handler(
            State(auth_service),
            Json(login_req),
        ).await;

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
            role: Role::Write,
        };

        let result = register_handler(
            State(auth_service),
            Json(register_req),
        ).await;

        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_refresh_token() {
        let auth_service = Arc::new(AuthService::new("test-secret"));

        // Register user first
        let register_req = RegisterRequest {
            username: "testuser".to_string(),
            email: "test@example.com".to_string(),
            password: "password123".to_string(),
            role: Role::Write,
        };

        let (_, auth_response) = register_handler(
            State(Arc::clone(&auth_service)),
            Json(register_req),
        ).await.unwrap();

        // Refresh token
        let refresh_req = RefreshRequest {
            refresh_token: auth_response.tokens.refresh_token.clone(),
        };

        let result = refresh_handler(
            State(auth_service),
            Json(refresh_req),
        ).await;

        assert!(result.is_ok());
        let new_tokens = result.unwrap();
        assert!(!new_tokens.access_token.is_empty());
    }
}

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
    /// AU-16: tokens ended by an explicit logout.
    ///
    /// Owned here and handed to `AuthLayer` by the server, so revoking and
    /// checking use **one** store. Two independent instances that only agreed
    /// at boot is exactly the AU-9 defect: `logout` would record a revocation
    /// the authenticating side never saw, and the token would keep working
    /// while the API reported success.
    pub revoked_tokens: Arc<crate::auth::revocation::RevokedTokens>,
}

impl AuthService {
    /// Create new authentication service
    pub fn new(jwt_secret: &str) -> Self {
        Self {
            jwt_auth: Arc::new(JwtAuth::new(jwt_secret)),
            credentials_store: Arc::new(CredentialsStore::new()),
            allow_open_registration: true,
            revoked_tokens: Arc::new(crate::auth::revocation::RevokedTokens::new()),
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
            revoked_tokens: Arc::new(crate::auth::revocation::RevokedTokens::new()),
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
            revoked_tokens: Arc::new(crate::auth::revocation::RevokedTokens::new()),
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
/// bcrypt hashes at most 72 bytes and **silently discards the rest**.
///
/// Two passphrases sharing a 72-byte prefix are therefore the same password,
/// and a user adopting a long passphrase — the behaviour every modern guide
/// encourages — is protected by only its first 72 bytes without being told.
/// Rejecting is better than truncating: the user picks a different password
/// instead of unknowingly getting a weaker one.
pub const MAX_PASSWORD_BYTES: usize = 72;

/// Passwords common enough that an attacker tries them first, so length alone
/// buys nothing. Deliberately short — a real deployment wants a breach corpus
/// (Have I Been Pwned's k-anonymity API or a local dump); this catches the
/// handful that appear at the top of every list without pretending to be one.
const COMMON_PASSWORDS: &[&str] = &[
    "password",
    "password1",
    "password123",
    "12345678",
    "123456789",
    "1234567890",
    "qwerty",
    "qwertyui",
    "qwerty123",
    "letmein",
    "welcome",
    "welcome1",
    "admin123",
    "iloveyou",
    "sunshine",
    "princess",
    "football",
    "baseball",
    "trustno1",
    "monkey123",
    "dragon123",
    "passw0rd",
    "p@ssword",
    "p@ssw0rd",
    "changeme",
    "abc12345",
    "111111111",
    "000000000",
    "mediagit",
    "mediagit123",
];

/// AU-12.
///
/// Deliberately **no composition rules** (an uppercase, a digit, a symbol).
/// NIST SP 800-63B advises against them: they push people toward predictable
/// shapes like `Password1!`, which satisfy every rule while being among the
/// first an attacker tries, and they block genuinely strong passphrases. What
/// is checked instead is length, the bcrypt ceiling, and whether the password
/// is one an attacker guesses immediately.
///
/// Password **history/reuse prevention is also deliberately absent**. It earns
/// its keep when rotation is forced, and forced rotation is itself advised
/// against — without it, keeping old hashes stores more secrets to defend for
/// very little gain. If scheduled rotation is ever introduced, history should
/// arrive with it.
pub fn validate_password_strength(password: &str) -> Result<(), String> {
    if password.is_empty() {
        return Err("Password is required".to_string());
    }
    if password.len() < 8 {
        return Err("Password must be at least 8 characters".to_string());
    }
    // Bytes, not characters: bcrypt counts bytes, so a 30-character password
    // of multibyte glyphs can exceed the limit.
    if password.len() > MAX_PASSWORD_BYTES {
        return Err(format!(
            "Password must be at most {MAX_PASSWORD_BYTES} bytes; anything longer is              silently truncated by the password hash, so the extra characters would              not protect the account"
        ));
    }

    let lowered = password.to_lowercase();
    if COMMON_PASSWORDS.contains(&lowered.as_str()) {
        return Err(
            "Password is among the most commonly used and would be guessed immediately;              choose something else"
                .to_string(),
        );
    }

    Ok(())
}

/// Is `password` built out of the account's own identifiers?
///
/// `alice`/`alice2024` is guessed on the first attempt by anyone who knows the
/// username, no matter how long it is.
fn password_echoes_identity(password: &str, username: &str, email: &str) -> bool {
    let pw = password.to_lowercase();
    let mut identifiers = vec![username.trim().to_lowercase()];
    let email = email.trim().to_lowercase();
    if let Some((local, _)) = email.split_once('@') {
        identifiers.push(local.to_string());
    }
    identifiers.push(email);

    identifiers
        .iter()
        .filter(|id| id.len() >= 3)
        .any(|id| pw.contains(id.as_str()))
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
    validate_password_strength(password)?;

    // Checked here rather than in `validate_password_strength` because only
    // this path knows who the account belongs to.
    if password_echoes_identity(password, username, email) {
        return Err(
            "Password must not contain your username or email address; those are the              first things an attacker tries"
                .to_string(),
        );
    }

    Ok(())
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
/// AU-16: revoke the presented token, rather than trusting the client to
/// forget it.
///
/// This used to return 204 and do nothing at all: the client discarded its
/// copy while the token kept authenticating for the rest of its life, so a
/// token captured before logout still worked and "signed out" described the
/// client, not the server. Logout is the one moment a user explicitly asks for
/// their access to stop.
///
/// Only the presented token is revoked — logging out of one machine must not
/// sign the user out everywhere else.
///
/// Still 204 when no usable token is presented: logout is idempotent, and
/// failing it would leave a client unable to clear its own state.
pub async fn logout_handler(
    State(auth_service): State<Arc<AuthService>>,
    headers: axum::http::HeaderMap,
) -> StatusCode {
    let revoked = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| crate::auth::JwtAuth::extract_from_header(h).ok())
        .and_then(|token| auth_service.jwt_auth.validate_token(token).ok());

    match revoked {
        Some(claims) => {
            auth_service
                .revoked_tokens
                .revoke(&claims.jti, claims.exp)
                .await;
            info!(user_id = %claims.sub, "User logged out; token revoked");
        }
        None => {
            info!("Logout requested without a valid token; nothing to revoke");
        }
    }

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

    /// bcrypt hashes at most 72 bytes and discards the rest silently, so a
    /// longer passphrase is protected by only its prefix and two passphrases
    /// sharing that prefix are the same password. Rejecting is better than
    /// handing someone a weaker password than they typed.
    #[test]
    fn password_longer_than_the_hash_limit_is_rejected() {
        let at_limit = "a".repeat(MAX_PASSWORD_BYTES);
        assert!(validate_password_strength(&at_limit).is_ok());

        let over = "a".repeat(MAX_PASSWORD_BYTES + 1);
        let err = validate_password_strength(&over).unwrap_err();
        assert!(err.contains("truncated"), "{err}");
    }

    /// The limit is in bytes because bcrypt counts bytes; a password well
    /// under 72 *characters* can still exceed it.
    #[test]
    fn the_limit_is_measured_in_bytes_not_characters() {
        // 4 bytes each in UTF-8.
        let emoji = "🔒".repeat(20); // 80 bytes, 20 chars
        assert!(emoji.chars().count() < MAX_PASSWORD_BYTES);
        assert!(emoji.len() > MAX_PASSWORD_BYTES);
        assert!(
            validate_password_strength(&emoji).is_err(),
            "a 20-character password that occupies 80 bytes must still be refused"
        );
    }

    /// Length alone buys nothing against a password an attacker tries first.
    #[test]
    fn common_passwords_are_rejected_regardless_of_case() {
        for pw in ["password123", "PASSWORD123", "Qwerty123", "letmein"] {
            assert!(
                validate_password_strength(pw).is_err(),
                "{pw:?} should be refused"
            );
        }
        assert!(validate_password_strength("correct horse battery").is_ok());
    }

    /// No composition rules, deliberately (NIST SP 800-63B): a long passphrase
    /// of only lowercase letters and spaces is strong and must be accepted,
    /// while `Password1!` satisfies every classic rule and is not.
    #[test]
    fn passphrases_are_accepted_without_composition_rules() {
        assert!(validate_password_strength("the quiet render farm hums").is_ok());
        assert!(validate_password_strength("aaaaaaaaaaaaaaaaaaaa").is_ok());
    }

    /// A password built from the account's own identifiers is guessed on the
    /// first attempt by anyone who knows the username.
    #[test]
    fn password_containing_identity_is_rejected() {
        assert!(
            validate_registration_input("alice", "alice@example.com", "alice-2024-summer").is_err()
        );
        assert!(
            validate_registration_input("alice", "alice@example.com", "xxalice@example.comxx")
                .is_err()
        );
        assert!(
            validate_registration_input("alice", "alice@example.com", "the quiet render farm")
                .is_ok()
        );
    }

    /// Short identifiers must not make every password unregisterable — a
    /// two-letter username would otherwise be a substring of almost anything.
    #[test]
    fn very_short_identifiers_do_not_block_registration() {
        // Username minimum is 3 characters, so use the shortest legal one.
        assert!(
            validate_registration_input("abc", "abc@example.com", "zz quiet render farm").is_ok()
        );
    }

    #[tokio::test]
    async fn test_register_login_flow() {
        let auth_service = Arc::new(AuthService::new("test-secret"));

        // Register user
        let register_req = RegisterRequest {
            username: "testuser".to_string(),
            email: "test@example.com".to_string(),
            password: "render farm quiet hum".to_string(),
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
            password: "render farm quiet hum".to_string(),
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
            password: "render farm quiet hum".to_string(),
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
            password: "render farm quiet hum".to_string(),
        };

        let result = register_handler(State(auth_service), Json(register_req)).await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_registration_input() {
        assert!(validate_registration_input("ab", "a@b.com", "render farm quiet hum").is_err());
        assert!(
            validate_registration_input("abc", "not-an-email", "render farm quiet hum").is_err()
        );
        assert!(validate_registration_input("abc", "a@b.com", "short").is_err());
        assert!(validate_registration_input("abc", "a@b.com", "render farm quiet hum").is_ok());
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
            password: "render farm quiet hum".to_string(),
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

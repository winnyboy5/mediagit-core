//! Integration tests for authentication system
//!
//! Tests JWT and API key authentication, permission enforcement,
//! and proper error handling for unauthorized/forbidden requests.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use mediagit_security::auth::{ApiKeyAuth, JwtAuth};
use mediagit_server::{create_router, AppState};
use std::sync::Arc;
use tempfile::TempDir;
use tower::util::ServiceExt;

/// Helper to create test app state with authentication enabled
fn create_test_state_with_auth() -> (Arc<AppState>, String, String) {
    let temp_dir = TempDir::new().unwrap();
    let repos_dir = temp_dir.path().to_path_buf();

    // Create API key auth
    let api_key_auth = Arc::new(ApiKeyAuth::new());

    // JWT secret
    let jwt_secret = "test-secret-key-for-integration-tests";

    // Create state with authentication
    let state = Arc::new(AppState::new_with_auth(
        repos_dir,
        jwt_secret,
        api_key_auth.clone(),
    ));

    // Generate a test JWT token for user with read permissions
    let jwt_auth = JwtAuth::new(jwt_secret);
    let read_token = jwt_auth
        .generate_token("test-user-read", vec!["repo:read".to_string()])
        .unwrap();

    // Generate a test JWT token for user with write permissions
    let write_token = jwt_auth
        .generate_token("test-user-write", vec!["repo:read".to_string(), "repo:write".to_string()])
        .unwrap();

    (state, read_token, write_token)
}

/// Helper to create test app state without authentication
fn create_test_state_without_auth() -> Arc<AppState> {
    let temp_dir = TempDir::new().unwrap();
    let repos_dir = temp_dir.path().to_path_buf();
    Arc::new(AppState::new(repos_dir))
}

#[tokio::test]
async fn test_unauthenticated_request_fails_when_auth_enabled() {
    let (state, _read_token, _write_token) = create_test_state_with_auth();
    let app = create_router(state);

    // Request without authentication header
    let request = Request::builder()
        .uri("/test-repo/info/refs")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    // Should return 401 Unauthorized
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_authenticated_request_with_jwt_succeeds() {
    let (state, read_token, _write_token) = create_test_state_with_auth();

    // Create test repository directory
    let repo_path = state.repos_dir.join("test-repo");
    std::fs::create_dir_all(&repo_path).unwrap();

    let app = create_router(state);

    // Request with valid JWT token
    let request = Request::builder()
        .uri("/test-repo/info/refs")
        .header("Authorization", format!("Bearer {}", read_token))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    // Should return 200 OK
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_authenticated_request_with_api_key_succeeds() {
    let (state, _read_token, _write_token) = create_test_state_with_auth();

    // Generate API key
    let api_key_auth = state.auth().unwrap();
    let (api_key, _api_key_info) = api_key_auth
        .api_key_auth()
        .generate_key(
            "test-user".to_string(),
            "test-key".to_string(),
            vec!["repo:read".to_string()],
        )
        .await
        .unwrap();

    // Create test repository directory
    let repo_path = state.repos_dir.join("test-repo");
    std::fs::create_dir_all(&repo_path).unwrap();

    let app = create_router(state);

    // Request with valid API key
    let request = Request::builder()
        .uri("/test-repo/info/refs")
        .header("X-API-Key", api_key)
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    // Should return 200 OK
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_insufficient_permissions_returns_forbidden() {
    let (state, read_token, _write_token) = create_test_state_with_auth();

    // Create test repository directory
    let repo_path = state.repos_dir.join("test-repo");
    std::fs::create_dir_all(&repo_path).unwrap();

    let app = create_router(state);

    // Request requiring write permission with only read permission token
    let request = Request::builder()
        .uri("/test-repo/refs/update")
        .method("POST")
        .header("Authorization", format!("Bearer {}", read_token))
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"updates":[],"force":false}"#))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    // Should return 403 Forbidden
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_write_permission_allows_write_operations() {
    let (state, _read_token, write_token) = create_test_state_with_auth();

    // Create test repository directory
    let repo_path = state.repos_dir.join("test-repo");
    std::fs::create_dir_all(&repo_path).unwrap();

    let app = create_router(state);

    // Request requiring write permission with write permission token
    let request = Request::builder()
        .uri("/test-repo/refs/update")
        .method("POST")
        .header("Authorization", format!("Bearer {}", write_token))
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"updates":[],"force":false}"#))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    // Should return 200 OK (empty updates succeed)
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_no_auth_mode_allows_all_requests() {
    let state = create_test_state_without_auth();

    // Create test repository directory
    let repo_path = state.repos_dir.join("test-repo");
    std::fs::create_dir_all(&repo_path).unwrap();

    let app = create_router(state);

    // Request without authentication header (auth disabled)
    let request = Request::builder()
        .uri("/test-repo/info/refs")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    // Should return 200 OK even without auth
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_invalid_jwt_token_returns_unauthorized() {
    let (state, _read_token, _write_token) = create_test_state_with_auth();

    // Create test repository directory
    let repo_path = state.repos_dir.join("test-repo");
    std::fs::create_dir_all(&repo_path).unwrap();

    let app = create_router(state);

    // Request with invalid JWT token
    let request = Request::builder()
        .uri("/test-repo/info/refs")
        .header("Authorization", "Bearer invalid-token-here")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    // Should return 401 Unauthorized
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_invalid_api_key_returns_unauthorized() {
    let (state, _read_token, _write_token) = create_test_state_with_auth();

    // Create test repository directory
    let repo_path = state.repos_dir.join("test-repo");
    std::fs::create_dir_all(&repo_path).unwrap();

    let app = create_router(state);

    // Request with invalid API key
    let request = Request::builder()
        .uri("/test-repo/info/refs")
        .header("X-API-Key", "invalid-api-key")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    // Should return 401 Unauthorized
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_read_permission_allows_get_refs() {
    let (state, read_token, _write_token) = create_test_state_with_auth();

    // Create test repository directory
    let repo_path = state.repos_dir.join("test-repo");
    std::fs::create_dir_all(&repo_path).unwrap();

    let app = create_router(state);

    // GET /info/refs with read permission
    let request = Request::builder()
        .uri("/test-repo/info/refs")
        .header("Authorization", format!("Bearer {}", read_token))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_read_permission_allows_download_pack() {
    let (state, read_token, _write_token) = create_test_state_with_auth();

    // Create test repository directory
    let repo_path = state.repos_dir.join("test-repo");
    std::fs::create_dir_all(&repo_path).unwrap();

    // Pre-populate want cache for this test
    {
        let mut want_cache = state.want_cache.lock().await;
        want_cache.insert("test-repo".to_string(), vec![]);
    }

    let app = create_router(state);

    // GET /objects/pack with read permission
    let request = Request::builder()
        .uri("/test-repo/objects/pack")
        .header("Authorization", format!("Bearer {}", read_token))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    // Should return 200 OK (empty pack is valid)
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_read_permission_denies_upload_pack() {
    let (state, read_token, _write_token) = create_test_state_with_auth();

    // Create test repository directory
    let repo_path = state.repos_dir.join("test-repo");
    std::fs::create_dir_all(&repo_path).unwrap();

    let app = create_router(state);

    // POST /objects/pack with only read permission
    let request = Request::builder()
        .uri("/test-repo/objects/pack")
        .method("POST")
        .header("Authorization", format!("Bearer {}", read_token))
        .body(Body::from(vec![]))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    // Should return 403 Forbidden
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_write_permission_allows_upload_pack() {
    let (state, _read_token, write_token) = create_test_state_with_auth();

    // Create test repository directory
    let repo_path = state.repos_dir.join("test-repo");
    std::fs::create_dir_all(&repo_path).unwrap();

    let app = create_router(state);

    // POST /objects/pack with write permission
    let request = Request::builder()
        .uri("/test-repo/objects/pack")
        .method("POST")
        .header("Authorization", format!("Bearer {}", write_token))
        .body(Body::from(vec![]))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    // Should return 500 or 400 (invalid pack data, but auth passed)
    // The important part is it's NOT 401 or 403
    assert_ne!(response.status(), StatusCode::UNAUTHORIZED);
    assert_ne!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_mixed_auth_methods_jwt_preferred() {
    let (state, read_token, _write_token) = create_test_state_with_auth();

    // Generate API key with different permissions
    let api_key_auth = state.auth().unwrap();
    let (api_key, _api_key_info) = api_key_auth
        .api_key_auth()
        .generate_key(
            "test-user-2".to_string(),
            "test-key-2".to_string(),
            vec!["repo:admin".to_string()],
        )
        .await
        .unwrap();

    // Create test repository directory
    let repo_path = state.repos_dir.join("test-repo");
    std::fs::create_dir_all(&repo_path).unwrap();

    let app = create_router(state);

    // Request with both JWT and API key (JWT should be checked first)
    let request = Request::builder()
        .uri("/test-repo/info/refs")
        .header("Authorization", format!("Bearer {}", read_token))
        .header("X-API-Key", api_key)
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    // Should succeed (JWT is valid)
    assert_eq!(response.status(), StatusCode::OK);
}

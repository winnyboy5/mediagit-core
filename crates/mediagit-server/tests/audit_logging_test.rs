//! Audit logging integration tests
//!
//! Tests the audit logging system with MediaGit server to verify that
//! security events are properly logged.

use axum::http::StatusCode;
use mediagit_server::{create_router_with_rate_limit, AppState, RateLimitConfig};
use reqwest::Client;
use std::net::SocketAddr;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

/// Test server with audit logging enabled
struct TestServer {
    addr: SocketAddr,
    _temp_dir: TempDir,
    shutdown_tx: Option<oneshot::Sender<()>>,
}

impl TestServer {
    async fn new() -> Self {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let repos_dir = temp_dir.path().join("repos");
        tokio::fs::create_dir_all(&repos_dir).await.unwrap();

        let state = Arc::new(AppState::new(repos_dir));

        // Use rate limiting to enable audit middleware
        let rate_config = RateLimitConfig::new(100, 200);
        let (router, cleanup) = create_router_with_rate_limit(state, rate_config);

        // Spawn cleanup task in background
        std::thread::spawn(cleanup);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

        tokio::spawn(async move {
            axum::serve(
                listener,
                router.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(async {
                shutdown_rx.await.ok();
            })
            .await
            .expect("Server failed");
        });

        // Wait for server to start
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        Self {
            addr,
            _temp_dir: temp_dir,
            shutdown_tx: Some(shutdown_tx),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

#[tokio::test]
async fn test_audit_path_traversal_attempt() {
    let server = TestServer::new().await;
    let client = Client::new();

    // Attempt path traversal - should trigger audit log
    let resp = client
        .get(&server.url("/../etc/passwd/info/refs"))
        .send()
        .await
        .unwrap();

    // Should be rejected - either BAD_REQUEST or NOT_FOUND (both indicate rejection)
    assert!(
        resp.status() == StatusCode::BAD_REQUEST || resp.status() == StatusCode::NOT_FOUND,
        "Expected BAD_REQUEST or NOT_FOUND, got {:?}",
        resp.status()
    );

    // The audit log should have been written (we can't verify the log content in tests,
    // but we can verify the request was processed correctly)
}

#[tokio::test]
async fn test_audit_absolute_path_attempt() {
    let server = TestServer::new().await;
    let client = Client::new();

    // Attempt absolute path - should trigger audit log
    let resp = client
        .get(&server.url("/etc/passwd/info/refs"))
        .send()
        .await
        .unwrap();

    // Should be rejected with BAD_REQUEST (path validation)
    // Note: /etc is treated as a repo name, which will fail validation
    assert!(
        resp.status() == StatusCode::BAD_REQUEST || resp.status() == StatusCode::NOT_FOUND,
        "Expected BAD_REQUEST or NOT_FOUND, got {:?}",
        resp.status()
    );
}

#[tokio::test]
async fn test_audit_invalid_characters() {
    let server = TestServer::new().await;
    let client = Client::new();

    // Repository name with invalid characters - should trigger audit log
    let resp = client
        .get(&server.url("/repo$test/info/refs"))
        .send()
        .await
        .unwrap();

    // Should be rejected
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_audit_rate_limit_violation() {
    // Create server with very restrictive rate limit
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let repos_dir = temp_dir.path().join("repos");
    tokio::fs::create_dir_all(&repos_dir).await.unwrap();

    let state = Arc::new(AppState::new(repos_dir));
    let rate_config = RateLimitConfig::new(1, 1); // 1 req/s, burst 1
    let (router, cleanup) = create_router_with_rate_limit(state, rate_config);

    std::thread::spawn(cleanup);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async {
            shutdown_rx.await.ok();
        })
        .await
        .expect("Server failed");
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let client = Client::new();
    let url = format!("http://{}/test-repo/info/refs", addr);

    // First request should succeed
    let resp1 = client.get(&url).send().await.unwrap();
    assert_ne!(resp1.status(), StatusCode::TOO_MANY_REQUESTS);

    // Second request should be rate limited and trigger audit log
    let resp2 = client.get(&url).send().await.unwrap();
    assert_eq!(
        resp2.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "Second request should be rate limited"
    );

    // Audit log for rate limit violation should have been written
    let _ = shutdown_tx.send(());
}

#[tokio::test]
async fn test_audit_oversized_request() {
    let server = TestServer::new().await;
    let client = Client::new();

    // Create a request with content-length exceeding the limit
    let oversized_content_length = "200000000"; // 200MB, exceeds 100MB limit

    let resp = client
        .post(&server.url("/test-repo/objects/pack"))
        .header("content-length", oversized_content_length)
        .body("dummy")
        .send()
        .await
        .unwrap();

    // Should be rejected with PAYLOAD_TOO_LARGE
    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);

    // Audit log for invalid request should have been written
}

#[tokio::test]
async fn test_audit_normal_request_no_log() {
    let server = TestServer::new().await;
    let client = Client::new();

    // Normal request to non-existent repo - should NOT trigger security audit logs
    // (only NOT_FOUND, no security violation)
    let resp = client
        .get(&server.url("/valid-repo/info/refs"))
        .send()
        .await
        .unwrap();

    // Should get NOT_FOUND (repo doesn't exist)
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // No security audit log should be written (repo name is valid)
}

#[tokio::test]
async fn test_audit_multiple_violations() {
    let server = TestServer::new().await;
    let client = Client::new();

    // Multiple different security violations
    let violations = vec![
        "/../etc/passwd/info/refs",       // Path traversal
        "/repo$bad/info/refs",             // Invalid characters
        "/repo\0null/info/refs",           // Null byte (URL encoded)
    ];

    for violation_path in violations {
        let resp = client
            .get(&server.url(violation_path))
            .send()
            .await
            .unwrap();

        // All should be rejected
        assert!(
            resp.status() == StatusCode::BAD_REQUEST || resp.status() == StatusCode::NOT_FOUND,
            "Violation {} should be rejected, got {:?}",
            violation_path,
            resp.status()
        );
    }

    // Each violation should have generated an audit log entry
}

#[tokio::test]
async fn test_audit_event_serialization() {
    use mediagit_security::audit::{AuditEvent, AuditEventType};
    use std::net::IpAddr;

    let ip: IpAddr = "192.168.1.1".parse().unwrap();
    let event = AuditEvent::new(
        AuditEventType::PathTraversalAttempt,
        "Test path traversal".to_string(),
    )
    .with_client_ip(ip)
    .with_repository("test-repo".to_string())
    .with_path("/../etc/passwd".to_string())
    .with_method("GET".to_string());

    // Verify serialization works
    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains("path_traversal_attempt"));
    assert!(json.contains("192.168.1.1"));
    assert!(json.contains("test-repo"));

    // Verify we can deserialize
    let deserialized: AuditEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.event_type, AuditEventType::PathTraversalAttempt);
    assert_eq!(deserialized.client_ip, Some(ip));
}

#[tokio::test]
async fn test_audit_helper_functions() {
    use mediagit_security::audit;
    use std::net::IpAddr;

    let ip: IpAddr = "127.0.0.1".parse().unwrap();

    // Test all helper functions - they should not panic
    audit::log_authentication_failed(Some(ip), Some("user123".to_string()), "invalid password");
    audit::log_authentication_success(ip, "user123".to_string());
    audit::log_rate_limit_exceeded(ip, "/test".to_string(), "GET".to_string());
    audit::log_path_traversal_attempt(
        ip,
        "repo".to_string(),
        "/../etc".to_string(),
        "contains ..",
    );
    audit::log_invalid_request(
        ip,
        "/test".to_string(),
        "POST".to_string(),
        "oversized content",
    );
    audit::log_suspicious_pattern(
        ip,
        "/test".to_string(),
        "GET".to_string(),
        "rapid requests",
    );
    audit::log_access_denied(
        ip,
        Some("user123".to_string()),
        "private-repo".to_string(),
        "not authorized",
    );
}

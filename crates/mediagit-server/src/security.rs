//! Security middleware and utilities
//!
//! Provides rate limiting, request validation, and security headers.

use axum::{
    extract::{ConnectInfo, Request},
    http::{HeaderValue, StatusCode},
    middleware::Next,
    response::Response,
};
use mediagit_security::audit;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
pub use tower_governor::{
    governor::{GovernorConfig, GovernorConfigBuilder},
    key_extractor::SmartIpKeyExtractor,
    GovernorLayer,
};

/// Rate limiting configuration
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// Requests per second per IP
    pub requests_per_second: u64,
    /// Burst capacity
    pub burst_size: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            requests_per_second: 100, // 100 req/s
            burst_size: 200,           // Allow burst of 200
        }
    }
}

impl RateLimitConfig {
    /// Create new rate limit configuration
    pub fn new(requests_per_second: u64, burst_size: u32) -> Self {
        Self {
            requests_per_second,
            burst_size,
        }
    }

    /// Build GovernorConfig from configuration
    ///
    /// Creates a rate limiting configuration using IP-based rate limiting (SmartIpKeyExtractor)
    /// which checks proxy headers (x-forwarded-for, x-real-ip) before falling back
    /// to peer IP address.
    ///
    /// To use this config, create a layer with `GovernorLayer::new(config)` or use
    /// `build_with_cleanup()` to also get a cleanup task.
    pub fn build_config(&self) -> Arc<impl Send + Sync> {
        Arc::new(
            GovernorConfigBuilder::default()
                .per_second(self.requests_per_second)
                .burst_size(self.burst_size)
                .use_headers() // Include rate limit headers in responses
                .key_extractor(SmartIpKeyExtractor)
                .finish()
                .expect("Failed to build rate limiter config"),
        )
    }

    /// Build configuration with background cleanup task
    ///
    /// Returns the config and a cleanup handle that should be spawned
    /// to periodically remove old rate limit entries.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use mediagit_server::RateLimitConfig;
    /// use tower_governor::GovernorLayer;
    ///
    /// let rate_config = RateLimitConfig::default();
    /// let (config, cleanup) = rate_config.build_with_cleanup();
    ///
    /// // Create the layer
    /// let layer = GovernorLayer::new(config);
    ///
    /// // Spawn cleanup task in background
    /// std::thread::spawn(cleanup);
    /// ```
    pub fn build_with_cleanup(&self) -> (Arc<impl Send + Sync>, impl FnOnce() + Send + 'static) {
        let config = Arc::new(
            GovernorConfigBuilder::default()
                .per_second(self.requests_per_second)
                .burst_size(self.burst_size)
                .use_headers()
                .key_extractor(SmartIpKeyExtractor)
                .finish()
                .expect("Failed to build rate limiter config"),
        );

        // Create cleanup task
        let limiter = config.limiter().clone();
        let cleanup_task = move || {
            let interval = Duration::from_secs(60);
            loop {
                std::thread::sleep(interval);
                let size = limiter.len();
                if size > 0 {
                    tracing::debug!("Rate limiter storage size: {}, cleaning up...", size);
                    limiter.retain_recent();
                }
            }
        };

        (config, cleanup_task)
    }
}

/// Security headers middleware
///
/// Adds security-related HTTP headers to all responses.
pub async fn security_headers_middleware(
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();

    // HSTS: Force HTTPS for 1 year
    headers.insert(
        "Strict-Transport-Security",
        HeaderValue::from_static("max-age=31536000; includeSubDomains"),
    );

    // Content Security Policy: Restrict resource loading
    headers.insert(
        "Content-Security-Policy",
        HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self'"),
    );

    // X-Frame-Options: Prevent clickjacking
    headers.insert(
        "X-Frame-Options",
        HeaderValue::from_static("DENY"),
    );

    // X-Content-Type-Options: Prevent MIME sniffing
    headers.insert(
        "X-Content-Type-Options",
        HeaderValue::from_static("nosniff"),
    );

    // X-XSS-Protection: Enable XSS filtering (legacy browsers)
    headers.insert(
        "X-XSS-Protection",
        HeaderValue::from_static("1; mode=block"),
    );

    // Referrer-Policy: Control referrer information
    headers.insert(
        "Referrer-Policy",
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );

    // Permissions-Policy: Disable unnecessary browser features
    headers.insert(
        "Permissions-Policy",
        HeaderValue::from_static("geolocation=(), microphone=(), camera=()"),
    );

    Ok(response)
}

/// Extract client IP from request
fn extract_client_ip(request: &Request) -> std::net::IpAddr {
    // Try to get from ConnectInfo extension
    if let Some(ConnectInfo(addr)) = request.extensions().get::<ConnectInfo<SocketAddr>>() {
        return addr.ip();
    }

    // Fallback to localhost
    "127.0.0.1".parse().unwrap()
}

/// Request validation middleware
///
/// Validates incoming requests for size limits and content types.
pub async fn request_validation_middleware(
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // Validate content length (max 2GB for large media files)
    const MAX_CONTENT_LENGTH: u64 = 2 * 1024 * 1024 * 1024; // 2GB

    if let Some(content_length) = request.headers().get("content-length") {
        if let Ok(length_str) = content_length.to_str() {
            if let Ok(length) = length_str.parse::<u64>() {
                if length > MAX_CONTENT_LENGTH {
                    let client_ip = extract_client_ip(&request);
                    let path = request.uri().path().to_string();
                    let method = request.method().to_string();

                    tracing::warn!(
                        "Request exceeds maximum content length: {} > {}",
                        length,
                        MAX_CONTENT_LENGTH
                    );

                    audit::log_invalid_request(
                        client_ip,
                        path,
                        method,
                        &format!("Content length {} exceeds maximum {}", length, MAX_CONTENT_LENGTH),
                    );

                    return Err(StatusCode::PAYLOAD_TOO_LARGE);
                }
            }
        }
    }

    // Validate content type for POST/PUT requests
    let method = request.method();
    if method == "POST" || method == "PUT" {
        if let Some(content_type) = request.headers().get("content-type") {
            let content_type_str = content_type.to_str().unwrap_or("");

            // Allow common types for MediaGit
            let allowed_types = [
                "application/octet-stream",
                "application/json",
                "application/x-git-upload-pack-request",
                "application/x-git-receive-pack-request",
                "multipart/form-data",
            ];

            let is_allowed = allowed_types.iter().any(|&allowed| {
                content_type_str.starts_with(allowed)
            });

            if !is_allowed && !content_type_str.is_empty() {
                tracing::warn!("Unsupported content type: {}", content_type_str);
                // Don't reject, just log warning for now
            }
        }
    }

    // Validate critical headers are present for authenticated requests
    // (Authentication will be added in future sprints)

    Ok(next.run(request).await)
}

/// Audit logging middleware for path traversal detection and rate limiting
///
/// This middleware intercepts requests and responses to log security events:
/// - Path traversal attempts in the repository path
/// - Rate limit violations (429 responses)
pub async fn audit_middleware(
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let client_ip = extract_client_ip(&request);
    let path = request.uri().path().to_string();
    let method = request.method().to_string();

    // Extract repository name from path (format: /:repo/...)
    if let Some(repo_start) = path.strip_prefix('/') {
        if let Some(repo_end) = repo_start.find('/') {
            let repo = &repo_start[..repo_end];

            // Check for path traversal attempts
            if let Err(reason) = validate_repo_name(repo) {
                audit::log_path_traversal_attempt(
                    client_ip,
                    repo.to_string(),
                    path.clone(),
                    reason,
                );
            }
        }
    }

    // Process the request
    let response = next.run(request).await;

    // Check if this was a rate limit violation
    if response.status() == StatusCode::TOO_MANY_REQUESTS {
        audit::log_rate_limit_exceeded(client_ip, path, method);
    }

    Ok(response)
}

/// Middleware to validate repository names in paths before routing
///
/// This middleware intercepts requests before they reach the router,
/// ensuring malicious paths return 400 Bad Request instead of 404 Not Found.
pub async fn path_validation_middleware(
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let client_ip = extract_client_ip(&request);
    let path = request.uri().path();
    let method = request.method().to_string();

    // Extract repo name from path (format: /{repo}/...)
    if let Some(repo) = path.strip_prefix('/').and_then(|p| p.split('/').next()) {
        if !repo.is_empty() {
            if let Err(reason) = validate_repo_name(repo) {
                tracing::warn!("Path validation failed for '{}': {}", repo, reason);
                audit::log_path_traversal_attempt(
                    client_ip,
                    path.to_string(),
                    method.clone(),
                    &format!("Rejected malicious repo name '{}': {}", repo, reason),
                );
                return Err(StatusCode::BAD_REQUEST);
            }
        }
    }

    Ok(next.run(request).await)
}

/// Path traversal prevention
///
/// Checks for path traversal attempts in repository names.
pub fn validate_repo_name(repo: &str) -> Result<(), &'static str> {
    // Reject paths containing ..
    if repo.contains("..") {
        return Err("Path traversal detected");
    }

    // Reject absolute paths
    if repo.starts_with('/') || repo.starts_with('\\') {
        return Err("Absolute paths not allowed");
    }

    // Reject paths with null bytes
    if repo.contains('\0') {
        return Err("Null bytes not allowed");
    }

    // Reject Windows drive letters
    if repo.len() >= 2 && repo.as_bytes()[1] == b':' {
        return Err("Drive letters not allowed");
    }

    // Must contain only safe characters
    let is_safe = repo.chars().all(|c| {
        c.is_alphanumeric() || c == '-' || c == '_' || c == '/' || c == '.'
    });

    if !is_safe {
        return Err("Repository name contains invalid characters");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_repo_name_safe() {
        assert!(validate_repo_name("myrepo").is_ok());
        assert!(validate_repo_name("my-repo").is_ok());
        assert!(validate_repo_name("my_repo").is_ok());
        assert!(validate_repo_name("org/repo").is_ok());
        assert!(validate_repo_name("my.repo").is_ok());
    }

    #[test]
    fn test_validate_repo_name_path_traversal() {
        assert!(validate_repo_name("../etc/passwd").is_err());
        assert!(validate_repo_name("repo/../secrets").is_err());
        assert!(validate_repo_name("..").is_err());
    }

    #[test]
    fn test_validate_repo_name_absolute_paths() {
        assert!(validate_repo_name("/etc/passwd").is_err());
        assert!(validate_repo_name("\\windows\\system32").is_err());
    }

    #[test]
    fn test_validate_repo_name_drive_letters() {
        assert!(validate_repo_name("C:\\repos").is_err());
        assert!(validate_repo_name("D:/repos").is_err());
    }

    #[test]
    fn test_validate_repo_name_null_bytes() {
        assert!(validate_repo_name("repo\0").is_err());
    }

    #[test]
    fn test_validate_repo_name_invalid_chars() {
        assert!(validate_repo_name("repo$test").is_err());
        assert!(validate_repo_name("repo@test").is_err());
        assert!(validate_repo_name("repo test").is_err()); // space
    }

    // TODO: Re-enable when rate limiting is completed (Sprint 3)
    // #[test]
    // fn test_rate_limit_config() {
    //     let config = RateLimitConfig::new(50, 100);
    //     assert_eq!(config.requests_per_second, 50);
    //     assert_eq!(config.burst_size, 100);
    //
    //     let default_config = RateLimitConfig::default();
    //     assert_eq!(default_config.requests_per_second, 100);
    //     assert_eq!(default_config.burst_size, 200);
    // }
}

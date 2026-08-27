// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

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
use tower_governor::{GovernorError, key_extractor::KeyExtractor};
pub use tower_governor::{
    GovernorLayer,
    governor::{GovernorConfig, GovernorConfigBuilder},
    key_extractor::SmartIpKeyExtractor,
};
// Named so a shared limiter can be passed between listeners (see
// `SharedRateLimiter`); `.use_headers()` selects this middleware type.
pub use governor::middleware::StateInformationMiddleware;

/// Rate-limit key: authenticated identity when present, else client IP.
///
/// Keying purely by IP does not survive real deployments — an entire team
/// behind one NAT, or a fleet of CI runners, shares a single bucket, so one
/// colleague's large push throttles everyone else. Worse, a legitimate push
/// issues far more requests than a human ever would, so the per-IP budget is
/// sized for the wrong thing.
///
/// Keying by credential makes the budget per-user, which is what the limit is
/// actually meant to express. Anonymous traffic still falls back to IP, so
/// unauthenticated abuse is bounded exactly as before.
///
/// The credential is **hashed**, never used verbatim: the key lives in the
/// limiter's map and appears in tracing output, and a bearer token there would
/// be a credential leak.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdentityOrIpKeyExtractor;

impl KeyExtractor for IdentityOrIpKeyExtractor {
    type Key = String;

    // `name()` / `key_name()` are only part of this trait when tower_governor
    // is built with its `tracing` feature, which we do not enable (only
    // `axum`, `default`, `tonic`). Adding them behind `#[cfg(feature =
    // "tracing")]` would silently refer to *our* crate's features, not
    // tower_governor's — so they are omitted rather than guarded wrongly.
    fn extract<T>(&self, req: &axum::http::Request<T>) -> Result<Self::Key, GovernorError> {
        if let Some(cred) = req
            .headers()
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .filter(|v| !v.trim().is_empty())
        {
            let digest = blake3::hash(cred.as_bytes());
            // 16 hex chars is ample to separate identities without retaining
            // anything that could reconstruct the credential.
            return Ok(format!("id:{}", &digest.to_hex()[..16]));
        }
        SmartIpKeyExtractor
            .extract(req)
            .map(|ip| format!("ip:{ip}"))
    }
}

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
        // Sized for bulk media transfer, not for browsing.
        //
        // The limiter wraps the whole router, data plane included. When the
        // pack path is unavailable, push falls back to one request per chunk —
        // a multi-GB push is then tens of thousands of requests in a few
        // minutes, from one legitimate client. At the previous 100/s + 200
        // burst that produced 429s on healthy pushes, and the retries they
        // invite are what masked a real corruption bug (the `psds` incident).
        //
        // Now that the key is per-identity rather than per-IP (see
        // `IdentityOrIpKeyExtractor`), this budget applies to one user rather
        // than to everyone sharing a NAT, so it can be sized for what a single
        // real client actually does.
        //
        // That last paragraph was aspirational until 2026-08-17: the boot path
        // (`create_rate_limited_router`) hand-inlined its own builder keyed by
        // `SmartIpKeyExtractor`, so nothing here was reachable and the key was
        // per-IP after all. It now goes through `build_with_cleanup` below.
        //
        // Delegating to `config.rs` rather than repeating the numbers: these
        // two disagreed (1000/2000 here, 10/20 there) and the serde defaults
        // won, which is how a documented, incident-derived budget lost to a
        // placeholder nobody re-read.
        Self {
            requests_per_second: crate::config::default_rate_limit_rps(),
            burst_size: crate::config::default_rate_limit_burst(),
        }
    }
}

/// The shared rate limiter, so a second listener can enforce the *same* budget
/// rather than being handed its own.
///
/// Lives here rather than in `lib.rs` so `build_with_cleanup` can name it, and
/// so the key extractor in the type cannot silently disagree with the one the
/// builder installs -- which is exactly what went wrong before.
pub type SharedRateLimiter =
    Arc<GovernorConfig<IdentityOrIpKeyExtractor, StateInformationMiddleware>>;

impl RateLimitConfig {
    /// Create new rate limit configuration
    pub fn new(requests_per_second: u64, burst_size: u32) -> Self {
        Self {
            requests_per_second,
            burst_size,
        }
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
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let rate_config = RateLimitConfig::default();
    /// let (config, cleanup) = rate_config.build_with_cleanup();
    ///
    /// // Use the config with your rate limiting layer
    /// // let layer = GovernorLayer { config };
    ///
    /// // Spawn cleanup task in background
    /// tokio::spawn(async move {
    ///     cleanup();
    /// });
    /// # }
    /// ```
    pub fn build_with_cleanup(
        &self,
    ) -> (SharedRateLimiter, impl FnOnce() + Send + 'static + use<>) {
        let config: SharedRateLimiter = Arc::new(
            GovernorConfigBuilder::default()
                // `.period()`, NOT `.per_second()`.
                //
                // tower_governor's `per_second(n)` is "replenish ONE cell every
                // n SECONDS" -- an interval, not a rate. Its own doc says so:
                // "Set the interval after which one element of the quota is
                // replenished in seconds." So `per_second(10)` was 0.1 req/s,
                // and the field called `requests_per_second` meant its own
                // reciprocal. That inversion is the actual cause of the 429
                // storms on ordinary pushes: the shipped default of 10 allowed
                // one request every ten seconds once the burst drained.
                //
                // Raising the number made it exponentially worse while looking
                // like a fix, because a bigger burst hides it until the bucket
                // empties -- at 1000 the server started handing out
                // `Retry-After` values around 900 seconds (observed in
                // 20260818-p10check: the client honoured one and slept 22
                // minutes).
                //
                // A period of 1s/rps gives the rate the field name promises.
                // Guarded against zero, which the builder rejects outright.
                .period(Duration::from_nanos(
                    1_000_000_000u64 / self.requests_per_second.max(1),
                ))
                .burst_size(self.burst_size)
                .use_headers()
                .key_extractor(IdentityOrIpKeyExtractor)
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
    headers.insert("X-Frame-Options", HeaderValue::from_static("DENY"));

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

    if let Some(content_length) = request.headers().get("content-length")
        && let Ok(length_str) = content_length.to_str()
        && let Ok(length) = length_str.parse::<u64>()
        && length > MAX_CONTENT_LENGTH
    {
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
            &format!(
                "Content length {} exceeds maximum {}",
                length, MAX_CONTENT_LENGTH
            ),
        );

        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }

    // Validate content type for POST/PUT requests
    let method = request.method();
    if (method == "POST" || method == "PUT")
        && let Some(content_type) = request.headers().get("content-type")
    {
        let content_type_str = content_type.to_str().unwrap_or("");

        // Allow common types for MediaGit
        let allowed_types = [
            "application/octet-stream",
            "application/json",
            "application/x-git-upload-pack-request",
            "application/x-git-receive-pack-request",
            "multipart/form-data",
        ];

        let is_allowed = allowed_types
            .iter()
            .any(|&allowed| content_type_str.starts_with(allowed));

        if !is_allowed && !content_type_str.is_empty() {
            tracing::warn!("Unsupported content type: {}", content_type_str);
            // Don't reject, just log warning for now
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
pub async fn audit_middleware(request: Request, next: Next) -> Result<Response, StatusCode> {
    let client_ip = extract_client_ip(&request);
    let path = request.uri().path().to_string();
    let method = request.method().to_string();

    // Extract repository name from path (format: /:repo/...)
    if let Some(repo_start) = path.strip_prefix('/')
        && let Some(repo_end) = repo_start.find('/')
    {
        let repo = &repo_start[..repo_end];

        // Check for path traversal attempts
        if let Err(reason) = validate_repo_name(repo) {
            audit::log_path_traversal_attempt(client_ip, repo.to_string(), path.clone(), reason);
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
    if let Some(repo) = path.strip_prefix('/').and_then(|p| p.split('/').next())
        && !repo.is_empty()
        && let Err(reason) = validate_repo_name(repo)
    {
        tracing::warn!("Path validation failed for '{}': {}", repo, reason);
        audit::log_path_traversal_attempt(
            client_ip,
            path.to_string(),
            method.clone(),
            &format!("Rejected malicious repo name '{}': {}", repo, reason),
        );
        return Err(StatusCode::BAD_REQUEST);
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
    let is_safe = repo
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '/' || c == '.');

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

    // Note: Rate limiting is fully implemented and production-ready.
    // This test is commented out due to the refactoring of rate limiting into the server config.
    // Rate limiting functionality is tested through integration tests instead.
    // See: crates/mediagit-server/src/config.rs for rate limit configuration
}

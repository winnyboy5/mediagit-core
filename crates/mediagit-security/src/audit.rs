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
//! Audit logging for security events
//!
//! Provides structured logging for security-critical events including:
//! - Failed authentication attempts
//! - Rate limit violations
//! - Path traversal attempts
//! - Suspicious request patterns
//!
//! Audit logs are emitted using the `tracing` framework with structured fields
//! that can be consumed by log aggregation systems.

use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::time::SystemTime;

/// Security event types for audit logging
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuditEventType {
    /// Failed authentication attempt
    AuthenticationFailed,
    /// Successful authentication
    AuthenticationSuccess,
    /// Rate limit exceeded
    RateLimitExceeded,
    /// Path traversal attempt detected
    PathTraversalAttempt,
    /// Invalid request (malformed, oversized, etc.)
    InvalidRequest,
    /// Suspicious pattern detected
    SuspiciousPattern,
    /// Access denied (authorization failure)
    AccessDenied,
}

/// Audit event with structured fields
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    /// Event type
    pub event_type: AuditEventType,
    /// Timestamp of the event
    pub timestamp: SystemTime,
    /// IP address of the client
    pub client_ip: Option<IpAddr>,
    /// User identifier (if authenticated)
    pub user_id: Option<String>,
    /// Repository name (if applicable)
    pub repository: Option<String>,
    /// Request path
    pub path: Option<String>,
    /// HTTP method
    pub method: Option<String>,
    /// Additional context or error message
    pub message: String,
    /// Additional metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

impl AuditEvent {
    /// Create a new audit event
    pub fn new(event_type: AuditEventType, message: String) -> Self {
        Self {
            event_type,
            timestamp: SystemTime::now(),
            client_ip: None,
            user_id: None,
            repository: None,
            path: None,
            method: None,
            message,
            metadata: None,
        }
    }

    /// Set client IP address
    pub fn with_client_ip(mut self, ip: IpAddr) -> Self {
        self.client_ip = Some(ip);
        self
    }

    /// Set user ID
    pub fn with_user_id(mut self, user_id: String) -> Self {
        self.user_id = Some(user_id);
        self
    }

    /// Set repository name
    pub fn with_repository(mut self, repository: String) -> Self {
        self.repository = Some(repository);
        self
    }

    /// Set request path
    pub fn with_path(mut self, path: String) -> Self {
        self.path = Some(path);
        self
    }

    /// Set HTTP method
    pub fn with_method(mut self, method: String) -> Self {
        self.method = Some(method);
        self
    }

    /// Set additional metadata
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }

    /// Log this audit event using tracing
    pub fn log(&self) {
        let event_json = serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string());

        match self.event_type {
            AuditEventType::AuthenticationFailed
            | AuditEventType::RateLimitExceeded
            | AuditEventType::PathTraversalAttempt
            | AuditEventType::SuspiciousPattern
            | AuditEventType::AccessDenied => {
                tracing::warn!(
                    target: "mediagit::security::audit",
                    event_type = ?self.event_type,
                    client_ip = ?self.client_ip,
                    user_id = ?self.user_id,
                    repository = ?self.repository,
                    path = ?self.path,
                    method = ?self.method,
                    message = %self.message,
                    audit_event = %event_json,
                    "Security audit event"
                );
            }
            AuditEventType::InvalidRequest => {
                tracing::info!(
                    target: "mediagit::security::audit",
                    event_type = ?self.event_type,
                    client_ip = ?self.client_ip,
                    path = ?self.path,
                    method = ?self.method,
                    message = %self.message,
                    audit_event = %event_json,
                    "Invalid request"
                );
            }
            AuditEventType::AuthenticationSuccess => {
                tracing::info!(
                    target: "mediagit::security::audit",
                    event_type = ?self.event_type,
                    client_ip = ?self.client_ip,
                    user_id = ?self.user_id,
                    message = %self.message,
                    audit_event = %event_json,
                    "Authentication successful"
                );
            }
        }
    }
}

/// Helper functions for common audit events
/// Default IP address for unknown clients
const UNKNOWN_IP: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(0, 0, 0, 0));

/// Log a failed authentication attempt
pub fn log_authentication_failed(client_ip: Option<IpAddr>, user_id: Option<String>, reason: &str) {
    AuditEvent::new(
        AuditEventType::AuthenticationFailed,
        format!("Authentication failed: {}", reason),
    )
    .with_client_ip(client_ip.unwrap_or(UNKNOWN_IP))
    .with_user_id(user_id.unwrap_or_else(|| "unknown".to_string()))
    .log();
}

/// Log a successful authentication
pub fn log_authentication_success(client_ip: IpAddr, user_id: String) {
    AuditEvent::new(
        AuditEventType::AuthenticationSuccess,
        "Authentication successful".to_string(),
    )
    .with_client_ip(client_ip)
    .with_user_id(user_id)
    .log();
}

/// Log a rate limit violation
pub fn log_rate_limit_exceeded(client_ip: IpAddr, path: String, method: String) {
    AuditEvent::new(
        AuditEventType::RateLimitExceeded,
        "Rate limit exceeded".to_string(),
    )
    .with_client_ip(client_ip)
    .with_path(path)
    .with_method(method)
    .log();
}

/// Log a path traversal attempt
pub fn log_path_traversal_attempt(
    client_ip: IpAddr,
    repository: String,
    path: String,
    reason: &str,
) {
    AuditEvent::new(
        AuditEventType::PathTraversalAttempt,
        format!("Path traversal attempt detected: {}", reason),
    )
    .with_client_ip(client_ip)
    .with_repository(repository)
    .with_path(path)
    .log();
}

/// Log an invalid request
pub fn log_invalid_request(client_ip: IpAddr, path: String, method: String, reason: &str) {
    AuditEvent::new(
        AuditEventType::InvalidRequest,
        format!("Invalid request: {}", reason),
    )
    .with_client_ip(client_ip)
    .with_path(path)
    .with_method(method)
    .log();
}

/// Log a suspicious pattern
pub fn log_suspicious_pattern(
    client_ip: IpAddr,
    path: String,
    method: String,
    description: &str,
) {
    AuditEvent::new(
        AuditEventType::SuspiciousPattern,
        format!("Suspicious pattern: {}", description),
    )
    .with_client_ip(client_ip)
    .with_path(path)
    .with_method(method)
    .log();
}

/// Log an access denied event
pub fn log_access_denied(
    client_ip: IpAddr,
    user_id: Option<String>,
    repository: String,
    reason: &str,
) {
    let mut event = AuditEvent::new(
        AuditEventType::AccessDenied,
        format!("Access denied: {}", reason),
    )
    .with_client_ip(client_ip)
    .with_repository(repository);

    if let Some(uid) = user_id {
        event = event.with_user_id(uid);
    }

    event.log();
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_event_creation() {
        let event = AuditEvent::new(
            AuditEventType::AuthenticationFailed,
            "Test failure".to_string(),
        );

        assert_eq!(event.event_type, AuditEventType::AuthenticationFailed);
        assert_eq!(event.message, "Test failure");
        assert!(event.client_ip.is_none());
        assert!(event.user_id.is_none());
    }

    #[test]
    fn test_audit_event_builder() {
        let ip: IpAddr = "192.168.1.1".parse().unwrap();
        let event = AuditEvent::new(
            AuditEventType::RateLimitExceeded,
            "Rate limit exceeded".to_string(),
        )
        .with_client_ip(ip)
        .with_path("/repo/info/refs".to_string())
        .with_method("GET".to_string());

        assert_eq!(event.event_type, AuditEventType::RateLimitExceeded);
        assert_eq!(event.client_ip, Some(ip));
        assert_eq!(event.path, Some("/repo/info/refs".to_string()));
        assert_eq!(event.method, Some("GET".to_string()));
    }

    #[test]
    fn test_audit_event_serialization() {
        let ip: IpAddr = "10.0.0.1".parse().unwrap();
        let event = AuditEvent::new(
            AuditEventType::PathTraversalAttempt,
            "Attempted access to ../etc/passwd".to_string(),
        )
        .with_client_ip(ip)
        .with_repository("test-repo".to_string());

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("path_traversal_attempt"));
        assert!(json.contains("10.0.0.1"));
        assert!(json.contains("test-repo"));
    }

    #[test]
    fn test_event_type_serialization() {
        let event_types = vec![
            AuditEventType::AuthenticationFailed,
            AuditEventType::AuthenticationSuccess,
            AuditEventType::RateLimitExceeded,
            AuditEventType::PathTraversalAttempt,
            AuditEventType::InvalidRequest,
            AuditEventType::SuspiciousPattern,
            AuditEventType::AccessDenied,
        ];

        for event_type in event_types {
            let json = serde_json::to_string(&event_type).unwrap();
            let deserialized: AuditEventType = serde_json::from_str(&json).unwrap();
            assert_eq!(event_type, deserialized);
        }
    }

    #[test]
    fn test_helper_functions() {
        let ip: IpAddr = "127.0.0.1".parse().unwrap();

        // These should not panic
        log_authentication_failed(Some(ip), Some("user123".to_string()), "invalid password");
        log_authentication_success(ip, "user123".to_string());
        log_rate_limit_exceeded(ip, "/test".to_string(), "GET".to_string());
        log_path_traversal_attempt(
            ip,
            "repo".to_string(),
            "/../etc".to_string(),
            "contains ..",
        );
        log_invalid_request(ip, "/test".to_string(), "POST".to_string(), "invalid content");
        log_suspicious_pattern(ip, "/test".to_string(), "GET".to_string(), "rapid requests");
        log_access_denied(ip, Some("user123".to_string()), "private-repo".to_string(), "not authorized");
    }
}

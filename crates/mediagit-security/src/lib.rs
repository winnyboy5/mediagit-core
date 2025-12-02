// Re-export encryption and KDF modules
pub mod encryption;
pub mod kdf;

// Audit logging module
pub mod audit;

// Authentication module
#[cfg(feature = "auth")]
pub mod auth;

// TLS/Certificate management module
#[cfg(feature = "tls")]
pub mod tls;

// Re-export commonly used types
pub use audit::{
    AuditEvent, AuditEventType,
    log_access_denied, log_authentication_failed, log_authentication_success,
    log_invalid_request, log_path_traversal_attempt, log_rate_limit_exceeded,
    log_suspicious_pattern,
};

#[cfg(feature = "auth")]
pub use auth::{
    ApiKey, ApiKeyAuth, AuthError, AuthLayer, AuthResult, AuthUser, Claims, JwtAuth,
    TokenPair, User, UserId,
    user::Role,
};

#[cfg(feature = "tls")]
pub use tls::{
    Certificate, CertificateBuilder, CertificateError,
    TlsConfig, TlsConfigBuilder, TlsError, TlsResult,
    config::TlsVersion,
};

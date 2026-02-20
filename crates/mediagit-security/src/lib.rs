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
    TokenPair, User, UserId, UserCredentials, CredentialsStore,
    AuthService, RegisterRequest, LoginRequest, RefreshRequest, AuthResponse, UserInfo, ErrorResponse,
    register_handler, login_handler, refresh_handler, me_handler, logout_handler,
    user::Role,
};

#[cfg(feature = "tls")]
pub use tls::{
    Certificate, CertificateBuilder, CertificateError,
    TlsConfig, TlsConfigBuilder, TlsError, TlsResult,
    config::TlsVersion,
};

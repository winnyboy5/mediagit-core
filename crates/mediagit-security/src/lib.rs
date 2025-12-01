// Re-export encryption and KDF modules
pub mod encryption;
pub mod kdf;

// Authentication module
#[cfg(feature = "auth")]
pub mod auth;

// Re-export commonly used types
#[cfg(feature = "auth")]
pub use auth::{
    ApiKey, ApiKeyAuth, AuthError, AuthLayer, AuthResult, AuthUser, Claims, JwtAuth,
    TokenPair, User, UserId,
    user::Role,
};

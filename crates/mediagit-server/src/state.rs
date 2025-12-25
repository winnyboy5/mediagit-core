use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

use mediagit_security::auth::{ApiKeyAuth, AuthLayer, AuthService, JwtAuth};

/// Shared application state
pub struct AppState {
    /// Directory containing repositories
    pub repos_dir: PathBuf,

    /// Cache of objects wanted by clients (repo_name -> list of OIDs)
    /// Used to coordinate between POST /objects/want and GET /objects/pack
    pub want_cache: Mutex<HashMap<String, Vec<String>>>,

    /// Authentication layer (optional - can be disabled for development)
    pub auth_layer: Option<Arc<AuthLayer>>,

    /// Authentication service with user management (optional)
    pub auth_service: Option<Arc<AuthService>>,
}

impl AppState {
    /// Create new app state without authentication (for development)
    pub fn new(repos_dir: PathBuf) -> Self {
        Self {
            repos_dir,
            want_cache: Mutex::new(HashMap::new()),
            auth_layer: None,
            auth_service: None,
        }
    }

    /// Create new app state with authentication enabled
    pub fn new_with_auth(
        repos_dir: PathBuf,
        jwt_secret: &str,
        api_key_auth: Arc<ApiKeyAuth>,
    ) -> Self {
        let jwt_auth = Arc::new(JwtAuth::new(jwt_secret));
        let auth_layer = Arc::new(AuthLayer::new(Arc::clone(&jwt_auth), Arc::clone(&api_key_auth)));
        let auth_service = Arc::new(AuthService::new(jwt_secret));

        Self {
            repos_dir,
            want_cache: Mutex::new(HashMap::new()),
            auth_layer: Some(auth_layer),
            auth_service: Some(auth_service),
        }
    }

    /// Create new app state with full authentication (recommended)
    pub fn new_with_full_auth(repos_dir: PathBuf, jwt_secret: &str) -> Self {
        let auth_service = Arc::new(AuthService::new(jwt_secret));
        let api_key_auth = Arc::new(ApiKeyAuth::new());
        let auth_layer = Arc::new(AuthLayer::new(
            Arc::clone(&auth_service.jwt_auth),
            api_key_auth,
        ));

        Self {
            repos_dir,
            want_cache: Mutex::new(HashMap::new()),
            auth_layer: Some(auth_layer),
            auth_service: Some(auth_service),
        }
    }

    /// Check if authentication is enabled
    pub fn is_auth_enabled(&self) -> bool {
        self.auth_layer.is_some()
    }

    /// Get authentication layer (if enabled)
    pub fn auth(&self) -> Option<&Arc<AuthLayer>> {
        self.auth_layer.as_ref()
    }

    /// Get authentication service (if enabled)
    pub fn auth_service(&self) -> Option<&Arc<AuthService>> {
        self.auth_service.as_ref()
    }
}

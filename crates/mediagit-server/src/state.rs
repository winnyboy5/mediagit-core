use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::Mutex;

use mediagit_security::auth::{ApiKeyAuth, AuthLayer, AuthService, JwtAuth};

/// Unique request ID generator
static REQUEST_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Generate a unique request ID for want/pack coordination
pub fn generate_request_id() -> String {
    let id = REQUEST_ID_COUNTER.fetch_add(1, Ordering::SeqCst);
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{}-{}", timestamp, id)
}

/// Shared application state
pub struct AppState {
    /// Directory containing repositories
    pub repos_dir: PathBuf,

    /// Cache of objects wanted by clients (request_id -> (repo_name, list of OIDs))
    /// Uses unique request IDs to prevent race conditions between concurrent clients
    pub want_cache: Mutex<HashMap<String, (String, Vec<String>)>>,

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

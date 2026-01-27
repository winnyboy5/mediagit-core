use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
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

/// Entry in the want cache with timestamp for TTL-based cleanup
#[derive(Debug, Clone)]
pub struct WantEntry {
    pub repo: String,
    pub want_list: Vec<String>,
    pub created_at: Instant,
}

/// Bounded cache for want requests with automatic cleanup
pub struct WantCache {
    entries: HashMap<String, WantEntry>,
    max_entries: usize,
}

impl WantCache {
    /// Default maximum entries
    pub const DEFAULT_MAX_ENTRIES: usize = 10_000;
    
    /// Create a new want cache with default capacity
    pub fn new() -> Self {
        Self::with_capacity(Self::DEFAULT_MAX_ENTRIES)
    }
    
    /// Create a new want cache with specified capacity
    pub fn with_capacity(max_entries: usize) -> Self {
        Self {
            entries: HashMap::new(),
            max_entries,
        }
    }
    
    /// Insert a want entry, evicting oldest if at capacity
    pub fn insert(&mut self, request_id: String, repo: String, want_list: Vec<String>) {
        // Evict oldest entry if at capacity
        if self.entries.len() >= self.max_entries {
            if let Some((oldest_key, _)) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.created_at)
                .map(|(k, e)| (k.clone(), e.clone()))
            {
                self.entries.remove(&oldest_key);
                tracing::debug!("Evicted oldest want entry: {}", oldest_key);
            }
        }
        
        self.entries.insert(request_id, WantEntry {
            repo,
            want_list,
            created_at: Instant::now(),
        });
    }
    
    /// Remove and return a want entry
    pub fn remove(&mut self, request_id: &str) -> Option<WantEntry> {
        self.entries.remove(request_id)
    }
    
    /// Get current entry count
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    
    /// Check if cache is empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for WantCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared application state
pub struct AppState {
    /// Directory containing repositories
    pub repos_dir: PathBuf,

    /// Cache of objects wanted by clients (request_id -> WantEntry)
    /// Uses unique request IDs to prevent race conditions between concurrent clients
    /// Bounded to prevent memory leaks from abandoned requests
    pub want_cache: Mutex<WantCache>,

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
            want_cache: Mutex::new(WantCache::new()),
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
            want_cache: Mutex::new(WantCache::new()),
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
            want_cache: Mutex::new(WantCache::new()),
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


// MediaGit - Git for Media Files
// Copyright (C) 2025 MediaGit Contributors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU Affero General Public License for more details.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{Mutex, RwLock};

use mediagit_security::auth::{ApiKeyAuth, AuthLayer, AuthService, JwtAuth};
use mediagit_storage::StorageBackend;
use mediagit_versioning::ObjectDatabase;

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
    /// Objects the client claims to already have — used to prune the server's
    /// pack walk so fetches only ship the delta.
    pub have_list: Vec<String>,
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

    /// Maximum age for want entries before automatic cleanup (5 minutes)
    pub const ENTRY_TTL: std::time::Duration = std::time::Duration::from_secs(300);

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

    /// Insert a want entry, cleaning up expired entries and evicting oldest if
    /// still at capacity. TTL cleanup piggybacks on insert() to avoid needing
    /// a background task.
    pub fn insert(
        &mut self,
        request_id: String,
        repo: String,
        want_list: Vec<String>,
        have_list: Vec<String>,
    ) {
        // Sweep expired entries first (TTL-based cleanup)
        let now = Instant::now();
        self.entries
            .retain(|_, entry| now.duration_since(entry.created_at) < Self::ENTRY_TTL);

        // Evict oldest entry if still at capacity after TTL sweep
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

        self.entries.insert(
            request_id,
            WantEntry {
                repo,
                want_list,
                have_list,
                created_at: Instant::now(),
            },
        );
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

/// Location of a chunk within a cloud pack object.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PackLoc {
    pub pack_oid: String,
    pub offset: u64,
    pub length: u32,
    /// BLAKE3 of the compressed chunk bytes stored in the pack.
    /// Used for per-slice integrity verify on pull (absent on old manifests).
    #[serde(default)]
    pub compressed_hash: Option<String>,
}

/// Shared application state
pub struct AppState {
    /// Directory containing repositories
    pub repos_dir: PathBuf,

    /// TTL (seconds) for presigned PUT URLs issued to clients.
    pub presigned_url_ttl_secs: u64,

    /// Cache of objects wanted by clients (request_id -> WantEntry)
    /// Uses unique request IDs to prevent race conditions between concurrent clients
    /// Bounded to prevent memory leaks from abandoned requests
    pub want_cache: Mutex<WantCache>,

    /// Per-repo cache of constructed storage backends. Constructing a backend
    /// (especially Azure/S3) is expensive — TLS handshake plus a container/bucket
    /// existence round-trip — and naively rebuilding it on every handler call
    /// turned tiny pushes into multi-minute operations against cloud storage.
    /// Keyed by canonical repo path; a single fast-path read lock covers the
    /// hot path, with a write-locked double-checked init on miss.
    pub storage_backends: RwLock<HashMap<PathBuf, Arc<dyn StorageBackend>>>,

    /// Per-repo cache of ObjectDatabase templates. Handlers clone() from this
    /// so all concurrent writers share the same Arc<delta_written_pairs> HashSet,
    /// which is required for the TOCTOU circular-delta-chain prevention to work.
    /// Without sharing, each handler has its own HashSet → guard is ineffective.
    pub odb_cache: RwLock<HashMap<PathBuf, ObjectDatabase>>,

    /// In-memory pack manifest index: repo -> chunk_oid_hex -> PackLoc.
    /// Populated from local JSONL on first locate hit; updated on complete_pack.
    pub pack_index: RwLock<HashMap<String, HashMap<String, PackLoc>>>,

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
            presigned_url_ttl_secs: 43200,
            want_cache: Mutex::new(WantCache::new()),
            storage_backends: RwLock::new(HashMap::new()),
            odb_cache: RwLock::new(HashMap::new()),
            pack_index: RwLock::new(HashMap::new()),
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
        let auth_layer = Arc::new(AuthLayer::new(
            Arc::clone(&jwt_auth),
            Arc::clone(&api_key_auth),
        ));
        let auth_service = Arc::new(AuthService::new(jwt_secret));

        Self {
            repos_dir,
            presigned_url_ttl_secs: 43200,
            want_cache: Mutex::new(WantCache::new()),
            storage_backends: RwLock::new(HashMap::new()),
            odb_cache: RwLock::new(HashMap::new()),
            pack_index: RwLock::new(HashMap::new()),
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
            presigned_url_ttl_secs: 43200,
            want_cache: Mutex::new(WantCache::new()),
            storage_backends: RwLock::new(HashMap::new()),
            odb_cache: RwLock::new(HashMap::new()),
            pack_index: RwLock::new(HashMap::new()),
            auth_layer: Some(auth_layer),
            auth_service: Some(auth_service),
        }
    }

    /// Override the presigned URL TTL (called from `main` after reading config).
    pub fn with_presigned_ttl(mut self, secs: u64) -> Self {
        self.presigned_url_ttl_secs = secs;
        self
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

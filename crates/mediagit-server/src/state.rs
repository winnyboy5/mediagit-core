// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use tokio::sync::{Mutex, OnceCell, RwLock};

use mediagit_metrics::MetricsRegistry;
use mediagit_metrics::types::{OperationType as MetricOp, StorageBackend as MetricBackend};
use mediagit_security::auth::{ApiKeyAuth, AuthLayer, AuthService, GrantsStore, JwtAuth};
use mediagit_storage::StorageBackend;
use mediagit_versioning::ObjectDatabase;

use crate::locks::LockRecord;

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
        if self.entries.len() >= self.max_entries
            && let Some((oldest_key, _)) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.created_at)
                .map(|(k, e)| (k.clone(), e.clone()))
        {
            self.entries.remove(&oldest_key);
            tracing::debug!("Evicted oldest want entry: {}", oldest_key);
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

/// D3 dedup guard map type for [`AppState::pack_verify_inflight`] — pulled
/// out because the inline form trips clippy's `type_complexity` lint.
type PackVerifyInflight = Mutex<HashMap<(String, String), Arc<OnceCell<bool>>>>;

/// Shared application state
pub struct AppState {
    /// Directory containing repositories
    pub repos_dir: PathBuf,

    /// TTL (seconds) for presigned PUT URLs issued to clients.
    pub presigned_url_ttl_secs: u64,

    /// Whether `complete_chunk_uploads` verifies chunk *content* (decompress
    /// and hash with BLAKE3) rather than mere existence. Server-side knob
    /// (not a client flag — see `ServerConfig::verify_content_on_complete`)
    /// because the presigned path is the one place an untrusted client's
    /// bytes reach storage without the server ever inspecting them.
    ///
    /// Defaults to `true`, matching `ServerConfig::verify_content_on_complete`
    /// — see that field for why it is affordable now (verification moved off
    /// the push critical path; reads are never speculative). The two defaults
    /// are kept in step deliberately: if they diverged, every test built on
    /// `AppState::new()` would exercise a configuration no real server runs,
    /// which is how a green suite stops meaning anything.
    pub verify_chunks_on_complete: bool,

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
    /// so all concurrent writers share the same Arc<delta_written_pairs> DeltaGraph,
    /// which is required for the TOCTOU circular-delta-chain prevention to work.
    /// Without sharing, each handler has its own graph → guard is ineffective.
    pub odb_cache: RwLock<HashMap<PathBuf, ObjectDatabase>>,

    /// In-memory pack manifest index: repo -> chunk_oid_hex -> PackLoc.
    /// Populated from local JSONL on first locate hit; updated on complete_pack.
    pub pack_index: RwLock<HashMap<String, HashMap<String, PackLoc>>>,

    /// Packs registered but not yet content-verified: repo -> pack_oid.
    ///
    /// The durable `.pending` marker on disk (sibling of the pack's `.jsonl`
    /// manifest — see `pending_marker_path` in `handlers/repo.rs`) is the
    /// SOURCE OF TRUTH for "unverified": marker present means unverified,
    /// marker absent means verified. This set is a cache of that fact, kept
    /// in step under the same write lock as `pack_index` at registration
    /// (`complete_pack`) so the two can't drift — see the "F9 concurrency
    /// guard" pattern already used there. Cleared entry-by-entry by the
    /// background verifier once a pack is resolved (clean or quarantined),
    /// and repopulated at boot by the startup sweep for any marker that
    /// survived a crash.
    pub unverified_packs: RwLock<HashMap<String, HashSet<String>>>,

    /// Millis-since-epoch of the last DATA-PLANE request this server saw
    /// (presigned pack URL mint, chunk download, batch-get). Background pack
    /// verification waits for this to go quiet before re-reading a pack out of
    /// the bucket — see `handlers::repo::wait_for_data_plane_quiet`.
    ///
    /// On AppState rather than a process-global static, and that is
    /// load-bearing for tests: as a global it coupled every test in the binary,
    /// because a test exercising `presign_pack_downloads` would stamp it and
    /// four unrelated `complete_pack` verification tests running in parallel
    /// would then see a busy link and defer past their own poll bounds. One
    /// server process has exactly one AppState, so per-state and per-process
    /// mean the same thing in production.
    pub data_plane_activity: std::sync::atomic::AtomicU64,

    /// D3 dedup guard for presign-triggered full-pack verification
    /// (`ensure_pack_verified_for_presign` in `handlers/transfer.rs`).
    /// Minting a presigned URL is irrevocable, so an unverified pack must be
    /// verified in FULL before a URL for it goes out — but concurrent
    /// pullers (or a presign racing the background worker from
    /// `complete_pack`) targeting the SAME pack must not each pull the whole
    /// pack over the WAN. Keyed by (repo, pack_oid); one `OnceCell` per
    /// pending pack means only the first caller runs verification and every
    /// other caller awaits and reuses its result. Entries are removed once
    /// resolved, so this only holds packs currently mid-verification.
    pub pack_verify_inflight: PackVerifyInflight,

    /// Server-enforced file locks (Tracks B1-B3): repo -> path -> LockRecord.
    /// Lazily loaded per repo from `.mediagit/locks.jsonl` on first access,
    /// following the same double-checked pattern as `storage_backends`.
    pub locks: RwLock<HashMap<String, HashMap<String, LockRecord>>>,

    /// Authentication layer (optional - can be disabled for development)
    pub auth_layer: Option<Arc<AuthLayer>>,

    /// Authentication service with user management (optional)
    pub auth_service: Option<Arc<AuthService>>,

    /// Per-repo authorization grants (H2). Empty (and unpersisted) unless
    /// constructed via [`AppState::new_with_full_auth`], in which case it's
    /// loaded from `grants.jsonl` under the auth store dir — see
    /// `mediagit_security::auth::GrantsStore`.
    pub grants: GrantsStore,

    /// M3 (#2b) test/observability counter: incremented once per `have` OID
    /// whose reachability bitmap was used instead of a BFS walk in
    /// `download_pack`. Not used for any correctness decision — purely lets
    /// tests and operators observe the short-circuit firing.
    pub bitmap_hits: AtomicU64,

    /// DC-4: the registry `/metrics` serves, when that endpoint is enabled.
    ///
    /// `None` when `MEDIAGIT_METRICS_ADDR` is unset, which is the default —
    /// recording is then a branch on an `Option` and costs nothing.
    ///
    /// This exists because the endpoint used to serve **permanent zeros**: the
    /// registry was constructed inside `main`'s metrics block and moved
    /// straight into the server, so no handler could reach it and nothing ever
    /// called a `record_*` method. An operator wiring a dashboard got flatlines
    /// that looked like a quiet system.
    ///
    /// One registry, created once and passed in — not constructed here. Two
    /// instances would agree only on their zeroes (the AU-9 shape).
    pub metrics: Option<MetricsRegistry>,

    /// This server's at-rest encryption master key, when `[encryption]` is
    /// enabled. Wraps every repository key the server holds.
    ///
    /// `None` is both the default and the overwhelmingly common case, and it
    /// must stay completely silent: a server with encryption off behaves
    /// exactly as it did before DC-7.
    pub encryption_master: Option<mediagit_security::encryption::EncryptionKey>,
}

impl AppState {
    /// Supply the at-rest encryption master key read from `[encryption]`.
    ///
    /// A builder rather than a constructor parameter for the same reason
    /// [`Self::with_metrics`] is one: three constructors already exist, and
    /// encryption is off in nearly every server that runs.
    pub fn with_encryption_master(
        mut self,
        master: Option<mediagit_security::encryption::EncryptionKey>,
    ) -> Self {
        self.encryption_master = master;
        self
    }

    /// Attach the registry `/metrics` serves so handlers can record into it.
    ///
    /// Takes the registry rather than making one: the same instance must back
    /// both the recording side and the scrape endpoint, or the endpoint reports
    /// a second registry's zeroes while the real counts go nowhere.
    pub fn with_metrics(mut self, registry: MetricsRegistry) -> Self {
        self.metrics = Some(registry);
        self
    }

    /// Create new app state without authentication (for development)
    pub fn new(repos_dir: PathBuf) -> Self {
        Self {
            repos_dir,
            presigned_url_ttl_secs: 43200,
            verify_chunks_on_complete: true,
            want_cache: Mutex::new(WantCache::new()),
            storage_backends: RwLock::new(HashMap::new()),
            odb_cache: RwLock::new(HashMap::new()),
            pack_index: RwLock::new(HashMap::new()),
            unverified_packs: RwLock::new(HashMap::new()),
            data_plane_activity: std::sync::atomic::AtomicU64::new(0),
            pack_verify_inflight: Mutex::new(HashMap::new()),
            locks: RwLock::new(HashMap::new()),
            auth_layer: None,
            auth_service: None,
            grants: GrantsStore::new(),
            bitmap_hits: AtomicU64::new(0),
            metrics: None,
            encryption_master: None,
        }
    }

    /// Create new app state with authentication enabled
    pub fn new_with_auth(
        repos_dir: PathBuf,
        jwt_secret: &str,
        api_key_auth: Arc<ApiKeyAuth>,
    ) -> Self {
        let jwt_auth = Arc::new(JwtAuth::new(jwt_secret));
        let auth_service = Arc::new(AuthService::new(jwt_secret));
        // AU-2: see `new_with_full_auth` — permissions are re-derived per
        // request from this store, not taken from the token.
        let auth_layer = Arc::new(
            AuthLayer::new(Arc::clone(&jwt_auth), Arc::clone(&api_key_auth))
                .with_credentials_store(Arc::clone(&auth_service.credentials_store))
                // AU-16: the *same* store the auth service revokes into.
                // A second instance would only agree at boot — logout would
                // record a revocation this side never sees, and the token
                // would keep working while the API reported success (AU-9).
                .with_revoked_tokens(Arc::clone(&auth_service.revoked_tokens)),
        );

        Self {
            repos_dir,
            presigned_url_ttl_secs: 43200,
            verify_chunks_on_complete: true,
            want_cache: Mutex::new(WantCache::new()),
            storage_backends: RwLock::new(HashMap::new()),
            odb_cache: RwLock::new(HashMap::new()),
            pack_index: RwLock::new(HashMap::new()),
            unverified_packs: RwLock::new(HashMap::new()),
            data_plane_activity: std::sync::atomic::AtomicU64::new(0),
            pack_verify_inflight: Mutex::new(HashMap::new()),
            locks: RwLock::new(HashMap::new()),
            auth_layer: Some(auth_layer),
            auth_service: Some(auth_service),
            grants: GrantsStore::new(),
            bitmap_hits: AtomicU64::new(0),
            metrics: None,
            encryption_master: None,
        }
    }

    /// Create new app state with full authentication (recommended).
    ///
    /// `auth_store_dir` is where users.jsonl and api_keys.jsonl are
    /// persisted (see `ServerConfig::resolved_auth_store_dir`) so accounts
    /// and API keys survive a server restart. Fails hard if a store file
    /// exists but is corrupt/unreadable — see `CredentialsStore::load_or_new`
    /// and `ApiKeyAuth::load_or_new`.
    pub fn new_with_full_auth(
        repos_dir: PathBuf,
        jwt_secret: &str,
        auth_store_dir: &Path,
    ) -> anyhow::Result<Self> {
        let auth_service = Arc::new(AuthService::new_with_store_dir(jwt_secret, auth_store_dir)?);
        let api_key_auth = Arc::new(ApiKeyAuth::load_or_new(auth_store_dir)?);
        // AU-2: give the layer the live user store so every request re-derives
        // permissions instead of trusting the token's frozen snapshot.
        let auth_layer = Arc::new(
            AuthLayer::new(Arc::clone(&auth_service.jwt_auth), api_key_auth)
                .with_credentials_store(Arc::clone(&auth_service.credentials_store))
                // AU-16: the *same* store the auth service revokes into.
                // A second instance would only agree at boot — logout would
                // record a revocation this side never sees, and the token
                // would keep working while the API reported success (AU-9).
                .with_revoked_tokens(Arc::clone(&auth_service.revoked_tokens)),
        );
        let grants = GrantsStore::load_or_new(auth_store_dir)?;

        Ok(Self {
            repos_dir,
            presigned_url_ttl_secs: 43200,
            verify_chunks_on_complete: true,
            want_cache: Mutex::new(WantCache::new()),
            storage_backends: RwLock::new(HashMap::new()),
            odb_cache: RwLock::new(HashMap::new()),
            pack_index: RwLock::new(HashMap::new()),
            unverified_packs: RwLock::new(HashMap::new()),
            data_plane_activity: std::sync::atomic::AtomicU64::new(0),
            pack_verify_inflight: Mutex::new(HashMap::new()),
            locks: RwLock::new(HashMap::new()),
            auth_layer: Some(auth_layer),
            auth_service: Some(auth_service),
            grants,
            bitmap_hits: AtomicU64::new(0),
            metrics: None,
            encryption_master: None,
        })
    }

    /// Override the presigned URL TTL (called from `main` after reading config).
    pub fn with_presigned_ttl(mut self, secs: u64) -> Self {
        self.presigned_url_ttl_secs = secs;
        self
    }

    /// Override whether `complete_chunk_uploads` verifies chunk content
    /// (called from `main` after reading config).
    pub fn with_verify_chunks_on_complete(mut self, on: bool) -> Self {
        self.verify_chunks_on_complete = on;
        self
    }

    /// Record a completed transfer operation, if `/metrics` is enabled.
    ///
    /// One call site shape for every handler so the labels cannot drift: an
    /// `operation_total` sample labelled success/error, plus the duration.
    /// `Filesystem` is the backend label because these are the *server's* own
    /// request-handling metrics — the object store behind them may be S3 or
    /// Azure, and mislabelling server latency as backend latency would make the
    /// backend look slow for time it never spent.
    ///
    /// A no-op when metrics are off, which is the default.
    pub fn record_op(&self, op: MetricOp, started: Instant, success: bool) {
        let Some(m) = self.metrics.as_ref() else {
            return;
        };
        let backend = MetricBackend::Filesystem;
        m.record_operation_complete(op, backend, success);
        m.record_operation_duration(op, backend, started.elapsed().as_secs_f64());
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

#[cfg(test)]
mod tests {
    use super::*;
    use mediagit_security::auth::User;
    use mediagit_security::auth::user::Role;

    #[tokio::test]
    async fn full_auth_persists_users_across_restart() {
        let repos = tempfile::tempdir().unwrap();
        let auth_dir = tempfile::tempdir().unwrap();

        let state = AppState::new_with_full_auth(
            repos.path().to_path_buf(),
            "test-secret",
            auth_dir.path(),
        )
        .unwrap();

        let user = User::new(
            "user1".to_string(),
            "alice".to_string(),
            "alice@example.com".to_string(),
            Role::Write,
        );
        state
            .auth_service()
            .unwrap()
            .credentials_store
            .register_user(user, "render farm quiet hum")
            .await
            .unwrap();

        // Fresh AppState from the same auth dir simulates a server restart.
        let state2 = AppState::new_with_full_auth(
            repos.path().to_path_buf(),
            "test-secret",
            auth_dir.path(),
        )
        .unwrap();
        let login = state2
            .auth_service()
            .unwrap()
            .credentials_store
            .authenticate("alice@example.com", "render farm quiet hum")
            .await;
        assert!(login.is_ok());
    }

    #[tokio::test]
    async fn full_auth_hard_errors_on_corrupt_store() {
        let repos = tempfile::tempdir().unwrap();
        let auth_dir = tempfile::tempdir().unwrap();
        tokio::fs::write(
            auth_dir.path().join("users.jsonl"),
            b"{\"v\":1}\nnot json\n",
        )
        .await
        .unwrap();

        let result = AppState::new_with_full_auth(
            repos.path().to_path_buf(),
            "test-secret",
            auth_dir.path(),
        );
        assert!(result.is_err());
    }
}

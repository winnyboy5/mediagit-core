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

//! Password hashing and credential management
//!
//! Provides secure password hashing using bcrypt and credential storage.

use bcrypt::{DEFAULT_COST, hash, verify};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::sync::RwLock;
use tracing::warn;

use super::{AuthError, AuthResult, User, UserId, persist, user::Role};

/// User credentials with hashed password
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserCredentials {
    /// User information
    pub user: User,

    /// Bcrypt hashed password (never store plaintext!)
    pub password_hash: String,
}

impl UserCredentials {
    /// Create new user credentials with password
    ///
    /// # Arguments
    /// * `user` - User information
    /// * `password` - Plaintext password (will be hashed)
    ///
    /// # Security
    /// The password is immediately hashed using bcrypt with default cost.
    /// The plaintext password is never stored.
    pub fn new(user: User, password: &str) -> AuthResult<Self> {
        let password_hash = hash(password, bcrypt_cost())
            .map_err(|e| AuthError::Internal(anyhow::anyhow!("Password hashing failed: {}", e)))?;

        Ok(Self {
            user,
            password_hash,
        })
    }

    /// Verify password against stored hash
    ///
    /// # Arguments
    /// * `password` - Plaintext password to verify
    ///
    /// # Returns
    /// `true` if password matches, `false` otherwise
    pub fn verify_password(&self, password: &str) -> bool {
        verify(password, &self.password_hash).unwrap_or(false)
    }

    /// Update password (re-hash with new value)
    pub fn update_password(&mut self, new_password: &str) -> AuthResult<()> {
        self.password_hash = hash(new_password, bcrypt_cost())
            .map_err(|e| AuthError::Internal(anyhow::anyhow!("Password hashing failed: {}", e)))?;
        Ok(())
    }
}

/// User credentials store, backed in-memory with optional JSONL persistence.
///
/// When constructed via [`CredentialsStore::load_or_new`], every mutation
/// (register, password update, delete) is persisted to `users.jsonl` under
/// the given store directory before the call returns, so users survive a
/// server restart. [`CredentialsStore::new`] stays purely in-memory (used by
/// tests and any caller that doesn't want disk I/O).
/// AU-7: consecutive failed logins tolerated before an account is locked.
/// Override with `MEDIAGIT_MAX_LOGIN_FAILURES`; `0` disables lockout.
const DEFAULT_MAX_LOGIN_FAILURES: u32 = 5;

/// AU-7: how long a locked account stays locked, in seconds. Override with
/// `MEDIAGIT_LOGIN_LOCKOUT_SECS`.
const DEFAULT_LOGIN_LOCKOUT_SECS: u64 = 900; // 15 minutes

pub(crate) fn max_login_failures() -> u32 {
    std::env::var("MEDIAGIT_MAX_LOGIN_FAILURES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_MAX_LOGIN_FAILURES)
}

fn login_lockout() -> std::time::Duration {
    let secs = std::env::var("MEDIAGIT_LOGIN_LOCKOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_LOGIN_LOCKOUT_SECS);
    std::time::Duration::from_secs(secs)
}

/// AU-7: bcrypt work factor. Hardcoded to the crate default before, so an
/// operator could not raise it as hardware got faster. Clamped to bcrypt's
/// valid 4..=31 range, and never below 10 — a lower cost is worse than the
/// default and is far more likely to be a mistake than a deliberate choice.
fn bcrypt_cost() -> u32 {
    std::env::var("MEDIAGIT_BCRYPT_COST")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .map(|c| c.clamp(10, 31))
        .unwrap_or(DEFAULT_COST)
}

/// AU-7: consecutive failures and when the account was locked.
#[derive(Debug, Default, Clone)]
struct FailureState {
    consecutive: u32,
    locked_at: Option<std::time::Instant>,
}

pub struct CredentialsStore {
    /// Map of user_id -> credentials
    credentials: RwLock<HashMap<UserId, UserCredentials>>,

    /// AU-7: per-user failed-login tracking. In-memory only and reset on
    /// restart — a restart is operator action, not something an attacker can
    /// induce, so persisting it would add I/O to the auth path for no real
    /// gain.
    failures: RwLock<HashMap<UserId, FailureState>>,

    /// Map of email -> user_id for lookup
    email_index: RwLock<HashMap<String, UserId>>,

    /// Map of username -> user_id for lookup
    username_index: RwLock<HashMap<String, UserId>>,

    /// Path to `users.jsonl` when persistence is enabled; `None` for a
    /// purely in-memory store.
    store_path: Option<PathBuf>,
}

impl CredentialsStore {
    /// Create new credentials store (in-memory only, no persistence).
    pub fn new() -> Self {
        Self {
            credentials: RwLock::new(HashMap::new()),
            failures: RwLock::new(HashMap::new()),
            email_index: RwLock::new(HashMap::new()),
            username_index: RwLock::new(HashMap::new()),
            store_path: None,
        }
    }

    /// Load a credentials store persisted at `store_dir/users.jsonl`, or
    /// start fresh if the file doesn't exist yet (first boot). An
    /// existing-but-corrupt file is a hard error — never silently start
    /// with an empty (i.e. "no users registered") store when the file is
    /// unreadable.
    ///
    /// Set `MEDIAGIT_AUTH_PERSIST=0` to force in-memory behavior (no load,
    /// no writes) even when a directory is given.
    pub fn load_or_new(store_dir: &Path) -> AuthResult<Self> {
        if !persist::persist_enabled() {
            return Ok(Self::new());
        }

        let path = store_dir.join("users.jsonl");
        let records: Vec<UserCredentials> = persist::load_jsonl(&path)?;

        let mut credentials = HashMap::new();
        let mut email_index = HashMap::new();
        let mut username_index = HashMap::new();
        for creds in records {
            email_index.insert(creds.user.email.clone(), creds.user.id.clone());
            username_index.insert(creds.user.username.clone(), creds.user.id.clone());
            credentials.insert(creds.user.id.clone(), creds);
        }

        Ok(Self {
            credentials: RwLock::new(credentials),
            failures: RwLock::new(HashMap::new()),
            email_index: RwLock::new(email_index),
            username_index: RwLock::new(username_index),
            store_path: Some(path),
        })
    }

    /// Persist the current in-memory state to `users.jsonl`. A no-op when
    /// this store was constructed with [`CredentialsStore::new`] (no store
    /// path).
    async fn persist(&self) -> AuthResult<()> {
        let Some(path) = &self.store_path else {
            return Ok(());
        };
        let credentials = self.credentials.read().await;
        let records: Vec<&UserCredentials> = credentials.values().collect();
        persist::save_jsonl(path, &records).await
    }

    /// Register new user with credentials
    ///
    /// # Arguments
    /// * `user` - User information
    /// * `password` - Plaintext password
    ///
    /// # Returns
    /// User credentials if registration successful
    ///
    /// # Errors
    /// Returns error if email or username already exists
    pub async fn register_user(&self, user: User, password: &str) -> AuthResult<UserCredentials> {
        // Check if email already exists
        {
            let email_index = self.email_index.read().await;
            if email_index.contains_key(&user.email) {
                return Err(AuthError::Internal(anyhow::anyhow!(
                    "Email already registered: {}",
                    user.email
                )));
            }
        }

        // Check if username already exists
        {
            let username_index = self.username_index.read().await;
            if username_index.contains_key(&user.username) {
                return Err(AuthError::Internal(anyhow::anyhow!(
                    "Username already taken: {}",
                    user.username
                )));
            }
        }

        // Create credentials
        let credentials = UserCredentials::new(user.clone(), password)?;

        // Store credentials and update indices
        {
            let mut creds = self.credentials.write().await;
            let mut email_idx = self.email_index.write().await;
            let mut username_idx = self.username_index.write().await;

            creds.insert(user.id.clone(), credentials.clone());
            email_idx.insert(user.email.clone(), user.id.clone());
            username_idx.insert(user.username.clone(), user.id.clone());
        }

        self.persist().await?;

        Ok(credentials)
    }

    /// Authenticate user with email/username and password
    ///
    /// # Arguments
    /// * `identifier` - Email or username
    /// * `password` - Plaintext password
    ///
    /// # Returns
    /// User information if authentication successful
    pub async fn authenticate(&self, identifier: &str, password: &str) -> AuthResult<User> {
        // Try to find user by email first, then username
        let user_id = {
            let email_index = self.email_index.read().await;
            if let Some(id) = email_index.get(identifier) {
                Some(id.clone())
            } else {
                let username_index = self.username_index.read().await;
                username_index.get(identifier).cloned()
            }
        };

        let user_id =
            user_id.ok_or_else(|| AuthError::Unauthorized("Invalid credentials".to_string()))?;

        // Get credentials and verify password
        let credentials = self.credentials.read().await;
        let creds = credentials
            .get(&user_id)
            .ok_or_else(|| AuthError::UserNotFound(user_id.clone()))?;

        // AU-7: verify the password *first*, then apply the lockout.
        //
        // The obvious ordering — refuse early while locked — forces a choice
        // between two bad outcomes: either report "account locked", which
        // tells an unauthenticated caller the account exists and turns the
        // defence into an enumeration oracle, or report the generic error and
        // leave a legitimate locked-out user with no idea why their correct
        // password is failing.
        //
        // Verifying first dissolves that. Someone who supplies the correct
        // password has already proved they are not enumerating, so telling
        // *them* the account is locked leaks nothing. Everyone else gets the
        // same generic "Invalid credentials" used for an unknown user and a
        // wrong password.
        //
        // A correct password still does not unlock the account — that would
        // make the lockout bound nothing. It only changes what the caller is
        // told. No extra DoS surface either: the bcrypt cost paid here is the
        // same one an attacker could already force against any unlocked
        // account.
        let password_ok = creds.verify_password(password);
        let is_disabled = creds.user.disabled;
        drop(credentials);

        // AU-11: same ordering, same reason. Announcing "account is disabled"
        // before checking the password would tell any anonymous caller that
        // the account exists. Only the password holder is told why they are
        // being refused; everyone else gets the generic error.
        if is_disabled {
            warn!("Login refused: account {} is disabled", user_id);
            return Err(AuthError::Unauthorized(if password_ok {
                "Account is disabled; contact an administrator".to_string()
            } else {
                "Invalid credentials".to_string()
            }));
        }

        if self.is_locked(&user_id).await {
            warn!("Login refused: account {} is locked out", user_id);
            return Err(if password_ok {
                AuthError::Unauthorized(
                    "Account temporarily locked after repeated failed logins; try again later"
                        .to_string(),
                )
            } else {
                AuthError::Unauthorized("Invalid credentials".to_string())
            });
        }

        if !password_ok {
            self.record_failure(&user_id).await;
            return Err(AuthError::Unauthorized("Invalid credentials".to_string()));
        }

        let credentials = self.credentials.read().await;
        let creds = credentials
            .get(&user_id)
            .ok_or_else(|| AuthError::UserNotFound(user_id.clone()))?;

        {
            let mut user = creds.user.clone();
            user.update_last_login();

            // Update last login in storage
            drop(credentials);
            let mut creds_write = self.credentials.write().await;
            if let Some(stored_creds) = creds_write.get_mut(&user_id) {
                stored_creds.user.update_last_login();
            }

            // AU-7: a success clears the counter, so ordinary typos across a
            // long session never accumulate into a lockout.
            self.failures.write().await.remove(&user_id);

            Ok(user)
        }
    }

    /// AU-7: whether `user_id` is currently locked out, expiring the lock if
    /// the cooldown has elapsed.
    async fn is_locked(&self, user_id: &str) -> bool {
        if max_login_failures() == 0 {
            return false;
        }
        let mut failures = self.failures.write().await;
        let Some(state) = failures.get_mut(user_id) else {
            return false;
        };
        let Some(locked_at) = state.locked_at else {
            return false;
        };
        if locked_at.elapsed() >= login_lockout() {
            // Cooldown served — clear it so the next attempt starts fresh.
            *state = FailureState::default();
            return false;
        }
        true
    }

    /// AU-7: count a failed attempt, locking the account once the allowance
    /// is exhausted.
    async fn record_failure(&self, user_id: &str) {
        let limit = max_login_failures();
        if limit == 0 {
            return;
        }
        let mut failures = self.failures.write().await;
        let state = failures.entry(user_id.to_string()).or_default();
        state.consecutive = state.consecutive.saturating_add(1);
        if state.consecutive >= limit && state.locked_at.is_none() {
            state.locked_at = Some(std::time::Instant::now());
            warn!(
                "Account {} locked after {} consecutive failed logins",
                user_id, state.consecutive
            );
        }
    }

    /// Get user by ID
    pub async fn get_user(&self, user_id: &str) -> AuthResult<User> {
        let credentials = self.credentials.read().await;
        credentials
            .get(user_id)
            .map(|c| c.user.clone())
            .ok_or_else(|| AuthError::UserNotFound(user_id.to_string()))
    }

    /// Get user by email
    pub async fn get_user_by_email(&self, email: &str) -> AuthResult<User> {
        let user_id = {
            let email_index = self.email_index.read().await;
            email_index.get(email).cloned()
        };

        match user_id {
            Some(id) => self.get_user(&id).await,
            None => Err(AuthError::UserNotFound(email.to_string())),
        }
    }

    /// Update user password
    pub async fn update_password(&self, user_id: &str, new_password: &str) -> AuthResult<()> {
        {
            let mut credentials = self.credentials.write().await;

            let creds = credentials
                .get_mut(user_id)
                .ok_or_else(|| AuthError::UserNotFound(user_id.to_string()))?;

            creds.update_password(new_password)?;
        }

        self.persist().await
    }

    /// Verify a user's current password by ID, without mutating anything.
    /// Used by the self-service password-change route, which must confirm
    /// the caller knows their current password before calling
    /// [`CredentialsStore::update_password`].
    pub async fn verify_password(&self, user_id: &str, password: &str) -> AuthResult<bool> {
        let credentials = self.credentials.read().await;
        let creds = credentials
            .get(user_id)
            .ok_or_else(|| AuthError::UserNotFound(user_id.to_string()))?;
        Ok(creds.verify_password(password))
    }

    /// Change a user's role, persisting through the same path
    /// [`CredentialsStore::update_password`] uses.
    pub async fn set_role(&self, user_id: &str, role: Role) -> AuthResult<()> {
        {
            let mut credentials = self.credentials.write().await;
            let creds = credentials
                .get_mut(user_id)
                .ok_or_else(|| AuthError::UserNotFound(user_id.to_string()))?;
            creds.user.role = role;
        }

        self.persist().await
    }

    /// AU-11: suspend or restore an account without deleting it.
    ///
    /// Deletion used to be the only way to stop someone signing in, which
    /// forces a choice between leaving access open and destroying the record
    /// of what they did. A disabled account keeps its id, grants and history
    /// and simply cannot authenticate; re-enabling restores it exactly.
    ///
    /// Takes effect on the account's next request, because the auth layer
    /// re-reads the user on every request rather than trusting the token.
    pub async fn set_disabled(&self, user_id: &str, disabled: bool) -> AuthResult<()> {
        {
            let mut credentials = self.credentials.write().await;
            let creds = credentials
                .get_mut(user_id)
                .ok_or_else(|| AuthError::UserNotFound(user_id.to_string()))?;
            creds.user.disabled = disabled;
        }

        self.persist().await
    }

    /// Register a new user with an explicit role (bootstrap / admin
    /// create-user entry point). Reuses [`CredentialsStore::register_user`]
    /// (and, through it, [`UserCredentials::new`]) so bcrypt hashing and the
    /// duplicate-email/username checks stay in exactly one place.
    pub async fn create_user_with_role(
        &self,
        user_id: UserId,
        username: String,
        email: String,
        password: &str,
        role: Role,
    ) -> AuthResult<UserCredentials> {
        let user = User::new(user_id, username, email, role);
        self.register_user(user, password).await
    }

    /// Count registered users with the given role (last-admin protection).
    pub async fn count_by_role(&self, role: Role) -> usize {
        let credentials = self.credentials.read().await;
        credentials.values().filter(|c| c.user.role == role).count()
    }

    /// Delete user
    pub async fn delete_user(&self, user_id: &str) -> AuthResult<()> {
        {
            let mut credentials = self.credentials.write().await;
            let creds = credentials
                .remove(user_id)
                .ok_or_else(|| AuthError::UserNotFound(user_id.to_string()))?;

            // Remove from indices
            let mut email_index = self.email_index.write().await;
            let mut username_index = self.username_index.write().await;

            email_index.remove(&creds.user.email);
            username_index.remove(&creds.user.username);
        }

        self.persist().await
    }

    /// List all users (without passwords)
    pub async fn list_users(&self) -> Vec<User> {
        let credentials = self.credentials.read().await;
        credentials.values().map(|c| c.user.clone()).collect()
    }

    /// Count total users
    pub async fn count_users(&self) -> usize {
        let credentials = self.credentials.read().await;
        credentials.len()
    }
}

impl Default for CredentialsStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
// Tests hold the process-global env lock across awaits to serialize
// env-var access (see persist::ENV_LOCK).
#[allow(clippy::unwrap_used, clippy::await_holding_lock)]
mod tests {
    use super::*;
    use crate::auth::user::Role;

    async fn store_with_user(id: &str) -> CredentialsStore {
        let store = CredentialsStore::new();
        let user = User::new(
            id.to_string(),
            format!("{id}name"),
            format!("{id}@example.com"),
            Role::Write,
        );
        store
            .register_user(user, "render farm quiet hum")
            .await
            .unwrap();
        store
    }

    /// AU-11: suspension must actually stop authentication, and must be
    /// reversible — the whole point is to avoid deleting the account.
    #[tokio::test]
    async fn disabling_blocks_login_and_re_enabling_restores_it() {
        let store = store_with_user("u1").await;

        assert!(
            store
                .authenticate("u1@example.com", "render farm quiet hum")
                .await
                .is_ok(),
            "precondition: the account works before being disabled"
        );

        store.set_disabled("u1", true).await.unwrap();
        assert!(
            store
                .authenticate("u1@example.com", "render farm quiet hum")
                .await
                .is_err(),
            "a disabled account must not authenticate"
        );

        store.set_disabled("u1", false).await.unwrap();
        assert!(
            store
                .authenticate("u1@example.com", "render farm quiet hum")
                .await
                .is_ok(),
            "re-enabling must restore the account exactly; disable is not delete"
        );
    }

    /// The account and its history survive suspension — otherwise this is just
    /// a slower delete and gives operators no reason to prefer it.
    #[tokio::test]
    async fn disabling_preserves_the_account_record() {
        let store = store_with_user("u2").await;
        let before = store.get_user("u2").await.unwrap();

        store.set_disabled("u2", true).await.unwrap();

        let after = store
            .get_user("u2")
            .await
            .expect("account must still exist");
        assert!(after.disabled);
        assert_eq!(after.id, before.id);
        assert_eq!(after.email, before.email);
        assert_eq!(after.created_at, before.created_at);
        assert!(!after.is_active());
    }

    /// Same enumeration-oracle reasoning as the AU-7 lockout: only the holder
    /// of the correct password learns *why* they were refused. Announcing
    /// "disabled" to any caller would confirm the account exists.
    #[tokio::test]
    async fn disabled_reason_is_revealed_only_to_the_password_holder() {
        let store = store_with_user("u3").await;
        store.set_disabled("u3", true).await.unwrap();

        let right = store
            .authenticate("u3@example.com", "render farm quiet hum")
            .await
            .unwrap_err()
            .to_string();
        assert!(right.contains("disabled"), "got: {right}");

        let wrong = store
            .authenticate("u3@example.com", "not-the-password")
            .await
            .unwrap_err()
            .to_string();
        assert!(
            !wrong.contains("disabled"),
            "a wrong password must not reveal that the account exists: {wrong}"
        );
    }

    #[tokio::test]
    async fn test_register_and_authenticate() {
        let store = CredentialsStore::new();
        let user = User::new(
            "user1".to_string(),
            "testuser".to_string(),
            "test@example.com".to_string(),
            Role::Write,
        );

        // Register user
        let result = store
            .register_user(user.clone(), "render farm quiet hum")
            .await;
        assert!(result.is_ok());

        // Authenticate with email
        let auth_user = store
            .authenticate("test@example.com", "render farm quiet hum")
            .await;
        assert!(auth_user.is_ok());
        assert_eq!(auth_user.unwrap().email, "test@example.com");

        // Authenticate with username
        let auth_user = store
            .authenticate("testuser", "render farm quiet hum")
            .await;
        assert!(auth_user.is_ok());
        assert_eq!(auth_user.unwrap().username, "testuser");
    }

    #[tokio::test]
    async fn test_wrong_password() {
        let store = CredentialsStore::new();
        let user = User::new(
            "user1".to_string(),
            "testuser".to_string(),
            "test@example.com".to_string(),
            Role::Write,
        );

        store
            .register_user(user, "render farm quiet hum")
            .await
            .unwrap();

        // Try wrong password
        let result = store
            .authenticate("test@example.com", "wrongpassword")
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_duplicate_email() {
        let store = CredentialsStore::new();
        let user1 = User::new(
            "user1".to_string(),
            "testuser1".to_string(),
            "test@example.com".to_string(),
            Role::Write,
        );
        let user2 = User::new(
            "user2".to_string(),
            "testuser2".to_string(),
            "test@example.com".to_string(),
            Role::Write,
        );

        store
            .register_user(user1, "render farm quiet hum")
            .await
            .unwrap();

        // Try to register with same email
        let result = store.register_user(user2, "password456").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_update_password() {
        let store = CredentialsStore::new();
        let user = User::new(
            "user1".to_string(),
            "testuser".to_string(),
            "test@example.com".to_string(),
            Role::Write,
        );

        store.register_user(user, "oldpassword").await.unwrap();

        // Update password
        store.update_password("user1", "newpassword").await.unwrap();

        // Old password should fail
        let result = store.authenticate("test@example.com", "oldpassword").await;
        assert!(result.is_err());

        // New password should work
        let result = store.authenticate("test@example.com", "newpassword").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_set_role_persists() {
        let _guard = persist::ENV_LOCK.read().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let store = CredentialsStore::load_or_new(tmp.path()).unwrap();
        let user = User::new(
            "user1".to_string(),
            "testuser".to_string(),
            "test@example.com".to_string(),
            Role::Write,
        );
        store
            .register_user(user, "render farm quiet hum")
            .await
            .unwrap();

        store.set_role("user1", Role::Admin).await.unwrap();
        assert_eq!(store.get_user("user1").await.unwrap().role, Role::Admin);

        // Fresh store from the same dir simulates a server restart: the
        // role change must have been persisted, not just held in memory.
        let store2 = CredentialsStore::load_or_new(tmp.path()).unwrap();
        assert_eq!(store2.get_user("user1").await.unwrap().role, Role::Admin);
    }

    #[tokio::test]
    async fn test_set_role_unknown_user_errors() {
        let store = CredentialsStore::new();
        assert!(store.set_role("nobody", Role::Admin).await.is_err());
    }

    #[tokio::test]
    async fn test_count_by_role() {
        let store = CredentialsStore::new();
        store
            .register_user(
                User::new(
                    "a".to_string(),
                    "a".to_string(),
                    "a@example.com".to_string(),
                    Role::Admin,
                ),
                "render farm quiet hum",
            )
            .await
            .unwrap();
        store
            .register_user(
                User::new(
                    "b".to_string(),
                    "b".to_string(),
                    "b@example.com".to_string(),
                    Role::Write,
                ),
                "render farm quiet hum",
            )
            .await
            .unwrap();

        assert_eq!(store.count_by_role(Role::Admin).await, 1);
        assert_eq!(store.count_by_role(Role::Write).await, 1);
        assert_eq!(store.count_by_role(Role::Read).await, 0);
    }

    #[tokio::test]
    async fn test_create_user_with_role() {
        let store = CredentialsStore::new();
        let creds = store
            .create_user_with_role(
                "user1".to_string(),
                "testuser".to_string(),
                "test@example.com".to_string(),
                "render farm quiet hum",
                Role::Admin,
            )
            .await
            .unwrap();
        assert_eq!(creds.user.role, Role::Admin);
        assert_eq!(store.count_by_role(Role::Admin).await, 1);
    }

    #[tokio::test]
    async fn test_verify_password() {
        let store = CredentialsStore::new();
        let user = User::new(
            "user1".to_string(),
            "testuser".to_string(),
            "test@example.com".to_string(),
            Role::Write,
        );
        store
            .register_user(user, "render farm quiet hum")
            .await
            .unwrap();

        assert!(
            store
                .verify_password("user1", "render farm quiet hum")
                .await
                .unwrap()
        );
        assert!(
            !store
                .verify_password("user1", "wrongpassword")
                .await
                .unwrap()
        );
        assert!(
            store
                .verify_password("nobody", "render farm quiet hum")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn test_delete_user() {
        let store = CredentialsStore::new();
        let user = User::new(
            "user1".to_string(),
            "testuser".to_string(),
            "test@example.com".to_string(),
            Role::Write,
        );

        store
            .register_user(user, "render farm quiet hum")
            .await
            .unwrap();
        assert_eq!(store.count_users().await, 1);

        // Delete user
        store.delete_user("user1").await.unwrap();
        assert_eq!(store.count_users().await, 0);

        // Authentication should fail
        let result = store
            .authenticate("test@example.com", "render farm quiet hum")
            .await;
        assert!(result.is_err());
    }

    #[test]
    fn test_password_hashing() {
        let user = User::new(
            "user1".to_string(),
            "test".to_string(),
            "test@example.com".to_string(),
            Role::Read,
        );
        let password = "my_secure_password";

        let creds = UserCredentials::new(user, password).unwrap();

        // Password should not be stored in plaintext
        assert_ne!(creds.password_hash, password);

        // Should verify correctly
        assert!(creds.verify_password(password));
        assert!(!creds.verify_password("wrong_password"));
    }

    // ---- H1: persistence ----

    #[tokio::test]
    async fn persists_and_reloads_across_restart() {
        let _guard = persist::ENV_LOCK.read().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let store = CredentialsStore::load_or_new(tmp.path()).unwrap();
        let user = User::new(
            "user1".to_string(),
            "testuser".to_string(),
            "test@example.com".to_string(),
            Role::Write,
        );
        store
            .register_user(user, "render farm quiet hum")
            .await
            .unwrap();

        assert!(tmp.path().join("users.jsonl").exists());

        // Fresh store from the same dir simulates a server restart.
        let store2 = CredentialsStore::load_or_new(tmp.path()).unwrap();
        let auth_user = store2
            .authenticate("test@example.com", "render farm quiet hum")
            .await;
        assert!(auth_user.is_ok());
        assert_eq!(store2.count_users().await, 1);
    }

    #[tokio::test]
    async fn corrupt_store_file_hard_errors() {
        let _guard = persist::ENV_LOCK.read().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        tokio::fs::write(
            tmp.path().join("users.jsonl"),
            b"{\"v\":1}\nnot valid json\n",
        )
        .await
        .unwrap();

        let result = CredentialsStore::load_or_new(tmp.path());
        assert!(result.is_err(), "corrupt store file must hard-error");
    }

    #[tokio::test]
    async fn missing_store_file_starts_fresh() {
        let _guard = persist::ENV_LOCK.read().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let store = CredentialsStore::load_or_new(tmp.path()).unwrap();
        assert_eq!(store.count_users().await, 0);
    }

    #[tokio::test]
    async fn persist_disabled_writes_no_files() {
        let _guard = persist::ENV_LOCK.write().unwrap();
        mediagit_test_utils::set_var("MEDIAGIT_AUTH_PERSIST", "0");
        let tmp = tempfile::tempdir().unwrap();
        let store = CredentialsStore::load_or_new(tmp.path()).unwrap();
        let user = User::new(
            "user1".to_string(),
            "testuser".to_string(),
            "test@example.com".to_string(),
            Role::Write,
        );
        store
            .register_user(user, "render farm quiet hum")
            .await
            .unwrap();
        mediagit_test_utils::remove_var("MEDIAGIT_AUTH_PERSIST");

        assert!(!tmp.path().join("users.jsonl").exists());
    }

    /// AU-7: repeated failed logins must lock the account.
    ///
    /// `authenticate` had no failure tracking of any kind, so online password
    /// guessing was bounded only by the server's global bulk-transfer rate
    /// limit (1000 req/s) — effectively unbounded for a single account.
    #[tokio::test]
    async fn repeated_failures_lock_the_account() {
        let store = CredentialsStore::new();
        let user = User::new(
            "u1".to_string(),
            "alice".to_string(),
            "alice@example.com".to_string(),
            Role::Read,
        );
        store.register_user(user, "correct-horse").await.unwrap();

        // Burn through the allowance with wrong passwords.
        for _ in 0..max_login_failures() {
            assert!(store.authenticate("alice", "wrong").await.is_err());
        }

        // The account is now locked: even the CORRECT password is refused.
        // That is the point — otherwise the lockout does not bound guessing.
        assert!(
            store.authenticate("alice", "correct-horse").await.is_err(),
            "account should be locked after repeated failures"
        );
    }

    /// A successful login must clear the failure count, so ordinary typos
    /// never accumulate into a lockout across a user's whole session.
    #[tokio::test]
    async fn success_resets_the_failure_count() {
        let store = CredentialsStore::new();
        let user = User::new(
            "u2".to_string(),
            "bob".to_string(),
            "bob@example.com".to_string(),
            Role::Read,
        );
        store.register_user(user, "correct-horse").await.unwrap();

        for _ in 0..(max_login_failures() - 1) {
            assert!(store.authenticate("bob", "wrong").await.is_err());
        }
        // One success resets the counter...
        assert!(store.authenticate("bob", "correct-horse").await.is_ok());
        // ...so the allowance is full again.
        for _ in 0..(max_login_failures() - 1) {
            assert!(store.authenticate("bob", "wrong").await.is_err());
        }
        assert!(
            store.authenticate("bob", "correct-horse").await.is_ok(),
            "a successful login should have reset the failure count"
        );
    }

    /// AU-7: the lockout must be explained to whoever proves they know the
    /// password, and to nobody else.
    ///
    /// Reporting "locked" to any caller would confirm the account exists —
    /// an enumeration oracle. Reporting nothing to anyone leaves a legitimate
    /// user unable to tell a lockout from a wrong password. Verifying the
    /// password before applying the lockout gives both: proof of knowledge
    /// earns the explanation.
    #[tokio::test]
    async fn lockout_is_explained_only_to_the_password_holder() {
        let store = CredentialsStore::new();
        let user = User::new(
            "u3".to_string(),
            "carol".to_string(),
            "carol@example.com".to_string(),
            Role::Read,
        );
        store.register_user(user, "correct-horse").await.unwrap();

        for _ in 0..max_login_failures() {
            assert!(store.authenticate("carol", "wrong").await.is_err());
        }

        // Correct password, locked account: told why, and still refused.
        let err = store
            .authenticate("carol", "correct-horse")
            .await
            .expect_err("a locked account must refuse even a correct password");
        assert!(
            err.to_string().to_lowercase().contains("lock"),
            "the password holder should learn the account is locked, got: {err}"
        );

        // Wrong password, same locked account: no hint that it exists.
        let err = store
            .authenticate("carol", "still-wrong")
            .await
            .expect_err("locked account must refuse");
        assert!(
            !err.to_string().to_lowercase().contains("lock"),
            "lockout leaked to a caller who does not know the password — \
                that confirms the account exists. Got: {err}"
        );
    }
}

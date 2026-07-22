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
        let password_hash = hash(password, DEFAULT_COST)
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
        self.password_hash = hash(new_password, DEFAULT_COST)
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
pub struct CredentialsStore {
    /// Map of user_id -> credentials
    credentials: RwLock<HashMap<UserId, UserCredentials>>,

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

        if creds.verify_password(password) {
            let mut user = creds.user.clone();
            user.update_last_login();

            // Update last login in storage
            drop(credentials);
            let mut creds_write = self.credentials.write().await;
            if let Some(stored_creds) = creds_write.get_mut(&user_id) {
                stored_creds.user.update_last_login();
            }

            Ok(user)
        } else {
            Err(AuthError::Unauthorized("Invalid credentials".to_string()))
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
        let result = store.register_user(user.clone(), "password123").await;
        assert!(result.is_ok());

        // Authenticate with email
        let auth_user = store.authenticate("test@example.com", "password123").await;
        assert!(auth_user.is_ok());
        assert_eq!(auth_user.unwrap().email, "test@example.com");

        // Authenticate with username
        let auth_user = store.authenticate("testuser", "password123").await;
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

        store.register_user(user, "password123").await.unwrap();

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

        store.register_user(user1, "password123").await.unwrap();

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
        store.register_user(user, "password123").await.unwrap();

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
                "password123",
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
                "password123",
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
                "password123",
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
        store.register_user(user, "password123").await.unwrap();

        assert!(store.verify_password("user1", "password123").await.unwrap());
        assert!(
            !store
                .verify_password("user1", "wrongpassword")
                .await
                .unwrap()
        );
        assert!(
            store
                .verify_password("nobody", "password123")
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

        store.register_user(user, "password123").await.unwrap();
        assert_eq!(store.count_users().await, 1);

        // Delete user
        store.delete_user("user1").await.unwrap();
        assert_eq!(store.count_users().await, 0);

        // Authentication should fail
        let result = store.authenticate("test@example.com", "password123").await;
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
        store.register_user(user, "password123").await.unwrap();

        assert!(tmp.path().join("users.jsonl").exists());

        // Fresh store from the same dir simulates a server restart.
        let store2 = CredentialsStore::load_or_new(tmp.path()).unwrap();
        let auth_user = store2.authenticate("test@example.com", "password123").await;
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
        store.register_user(user, "password123").await.unwrap();
        mediagit_test_utils::remove_var("MEDIAGIT_AUTH_PERSIST");

        assert!(!tmp.path().join("users.jsonl").exists());
    }
}

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
//! Password hashing and credential management
//!
//! Provides secure password hashing using bcrypt and credential storage.

use bcrypt::{hash, verify, DEFAULT_COST};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::RwLock;

use super::{AuthError, AuthResult, User, UserId};

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

/// In-memory user credentials store
///
/// This is a simple in-memory implementation. For production use,
/// replace with a persistent database backend.
pub struct CredentialsStore {
    /// Map of user_id -> credentials
    credentials: RwLock<HashMap<UserId, UserCredentials>>,

    /// Map of email -> user_id for lookup
    email_index: RwLock<HashMap<String, UserId>>,

    /// Map of username -> user_id for lookup
    username_index: RwLock<HashMap<String, UserId>>,
}

impl CredentialsStore {
    /// Create new credentials store
    pub fn new() -> Self {
        Self {
            credentials: RwLock::new(HashMap::new()),
            email_index: RwLock::new(HashMap::new()),
            username_index: RwLock::new(HashMap::new()),
        }
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
                    "Email already registered: {}", user.email
                )));
            }
        }

        // Check if username already exists
        {
            let username_index = self.username_index.read().await;
            if username_index.contains_key(&user.username) {
                return Err(AuthError::Internal(anyhow::anyhow!(
                    "Username already taken: {}", user.username
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

        let user_id = user_id.ok_or_else(|| {
            AuthError::Unauthorized("Invalid credentials".to_string())
        })?;

        // Get credentials and verify password
        let credentials = self.credentials.read().await;
        let creds = credentials.get(&user_id).ok_or_else(|| {
            AuthError::UserNotFound(user_id.clone())
        })?;

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
        let mut credentials = self.credentials.write().await;

        let creds = credentials.get_mut(user_id).ok_or_else(|| {
            AuthError::UserNotFound(user_id.to_string())
        })?;

        creds.update_password(new_password)
    }

    /// Delete user
    pub async fn delete_user(&self, user_id: &str) -> AuthResult<()> {
        let mut credentials = self.credentials.write().await;
        let creds = credentials.remove(user_id).ok_or_else(|| {
            AuthError::UserNotFound(user_id.to_string())
        })?;

        // Remove from indices
        let mut email_index = self.email_index.write().await;
        let mut username_index = self.username_index.write().await;

        email_index.remove(&creds.user.email);
        username_index.remove(&creds.user.username);

        Ok(())
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
        let result = store.authenticate("test@example.com", "wrongpassword").await;
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
}

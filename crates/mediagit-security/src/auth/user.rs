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
//! User model and management

use serde::{Deserialize, Serialize};

/// User ID type
pub type UserId = String;

/// User role for access control
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    /// Read-only access
    Read,

    /// Read and write access
    Write,

    /// Full administrative access
    Admin,
}

/// User account
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    /// Unique user identifier
    pub id: UserId,

    /// Username
    pub username: String,

    /// Email address
    pub email: String,

    /// User role
    pub role: Role,

    /// Created timestamp
    pub created_at: i64,

    /// Last login timestamp
    pub last_login: Option<i64>,
}

impl User {
    /// Create new user
    pub fn new(id: UserId, username: String, email: String, role: Role) -> Self {
        Self {
            id,
            username,
            email,
            role,
            created_at: chrono::Utc::now().timestamp(),
            last_login: None,
        }
    }

    /// Get user permissions based on role
    pub fn permissions(&self) -> Vec<String> {
        match self.role {
            Role::Read => vec!["repo:read".to_string()],
            Role::Write => vec!["repo:read".to_string(), "repo:write".to_string()],
            Role::Admin => vec![
                "repo:read".to_string(),
                "repo:write".to_string(),
                "repo:admin".to_string(),
                "user:manage".to_string(),
            ],
        }
    }

    /// Check if user has specific permission
    pub fn has_permission(&self, permission: &str) -> bool {
        self.permissions().contains(&permission.to_string())
    }

    /// Update last login timestamp
    pub fn update_last_login(&mut self) {
        self.last_login = Some(chrono::Utc::now().timestamp());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_permissions() {
        let read_user = User::new(
            "user1".to_string(),
            "reader".to_string(),
            "reader@example.com".to_string(),
            Role::Read,
        );

        assert_eq!(read_user.permissions(), vec!["repo:read"]);
        assert!(read_user.has_permission("repo:read"));
        assert!(!read_user.has_permission("repo:write"));
    }

    #[test]
    fn test_admin_permissions() {
        let admin_user = User::new(
            "admin1".to_string(),
            "admin".to_string(),
            "admin@example.com".to_string(),
            Role::Admin,
        );

        assert!(admin_user.has_permission("repo:read"));
        assert!(admin_user.has_permission("repo:write"));
        assert!(admin_user.has_permission("repo:admin"));
        assert!(admin_user.has_permission("user:manage"));
    }

    #[test]
    fn test_update_last_login() {
        let mut user = User::new(
            "user1".to_string(),
            "test".to_string(),
            "test@example.com".to_string(),
            Role::Write,
        );

        assert!(user.last_login.is_none());

        user.update_last_login();
        assert!(user.last_login.is_some());
    }
}

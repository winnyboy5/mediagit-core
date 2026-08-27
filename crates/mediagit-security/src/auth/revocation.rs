// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! AU-16: revoked access tokens.
//!
//! `logout` used to return 204 and do nothing. The client discarded its copy
//! of the token, but the token stayed valid server-side for the rest of its
//! life — so a token captured beforehand kept working, and "signed out" was a
//! statement about the client rather than the server. That is the one place a
//! user actively asks for their access to stop.

use std::collections::HashMap;
use tokio::sync::RwLock;

/// Token ids that must be refused, each held until the token would have
/// expired anyway.
///
/// Deliberately in-memory. A revocation list that outlived the tokens it
/// describes would grow forever, and every entry becomes meaningless once the
/// token expires on its own. The cost is that a server restart forgets
/// revocations — acceptable because a restart also invalidates nothing else,
/// and the alternative (persisting) buys at most the remaining minutes of a
/// token's life. See `OP-8`-style multi-instance caveats: like the want cache
/// and rate limiter, this is per-process.
#[derive(Debug, Default)]
pub struct RevokedTokens {
    /// jti -> the token's own `exp`, so entries can be dropped when they stop
    /// mattering rather than accumulating.
    entries: RwLock<HashMap<String, i64>>,
}

impl RevokedTokens {
    pub fn new() -> Self {
        Self::default()
    }

    /// Revoke `jti` until `exp`.
    ///
    /// Prunes expired entries on the way in, so the list stays proportional to
    /// tokens revoked *while still valid* rather than to all logouts ever. No
    /// background task to own or leak.
    pub async fn revoke(&self, jti: &str, exp: i64) {
        if jti.is_empty() {
            return;
        }
        let now = chrono::Utc::now().timestamp();
        let mut entries = self.entries.write().await;
        entries.retain(|_, token_exp| *token_exp > now);
        entries.insert(jti.to_string(), exp);
    }

    /// Is this token revoked?
    ///
    /// An empty `jti` is treated as *not* revoked: tokens minted before the
    /// field existed carry none, and refusing them all would sign out every
    /// active session on upgrade.
    pub async fn is_revoked(&self, jti: &str) -> bool {
        if jti.is_empty() {
            return false;
        }
        let now = chrono::Utc::now().timestamp();
        let entries = self.entries.read().await;
        match entries.get(jti) {
            Some(exp) => *exp > now,
            None => false,
        }
    }

    /// Number of live revocations (test/diagnostics).
    pub async fn len(&self) -> usize {
        let now = chrono::Utc::now().timestamp();
        self.entries
            .read()
            .await
            .values()
            .filter(|exp| **exp > now)
            .count()
    }

    pub async fn is_empty(&self) -> bool {
        self.len().await == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn soon() -> i64 {
        chrono::Utc::now().timestamp() + 3600
    }

    #[tokio::test]
    async fn revoked_token_is_refused_and_others_are_not() {
        let store = RevokedTokens::new();
        store.revoke("token-a", soon()).await;

        assert!(store.is_revoked("token-a").await);
        assert!(
            !store.is_revoked("token-b").await,
            "revoking one session must not affect another"
        );
    }

    /// The list must not grow with every logout ever performed; an entry stops
    /// mattering the moment the token would have expired regardless.
    #[tokio::test]
    async fn expired_entries_are_pruned() {
        let store = RevokedTokens::new();
        let already_expired = chrono::Utc::now().timestamp() - 1;

        store.revoke("old", already_expired).await;
        assert!(
            !store.is_revoked("old").await,
            "an entry past its own exp is meaningless and must not be reported"
        );

        // Inserting anything prunes, so the dead entry is actually gone.
        store.revoke("fresh", soon()).await;
        assert_eq!(store.len().await, 1);
    }

    /// Tokens minted before `jti` existed carry none. Refusing them all would
    /// sign out every active session the moment this shipped.
    #[tokio::test]
    async fn tokens_without_an_id_are_not_treated_as_revoked() {
        let store = RevokedTokens::new();
        store.revoke("", soon()).await;
        assert!(!store.is_revoked("").await);
        assert!(store.is_empty().await);
    }
}

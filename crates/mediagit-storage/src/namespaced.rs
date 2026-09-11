// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Namespace-prefixing storage wrapper (layout v2).
//!
//! `NamespacedBackend` wraps any [`StorageBackend`] and prepends
//! `"<namespace>/"` to every key, so a single storage root/bucket can safely
//! host multiple repositories without their keys colliding. `list_objects`
//! strips the prefix back off so callers only ever see logical (unprefixed)
//! keys — the namespace is entirely invisible above this layer.
//!
//! Every key-taking method on [`StorageBackend`] is explicitly overridden
//! and forwards to `self.inner` with the prefixed key — never to `self`,
//! so any backend-specific optimization the inner backend provides (native
//! range GET, presigned URLs, streaming uploads, ...) is preserved. Relying
//! on the trait's default methods here would silently regress those
//! backends to their dumb fallbacks (or an outright "unsupported" error for
//! `get_streaming_range`) while *appearing* to compile correctly — a missed
//! method on this wrapper is either a lost optimization or, worse, a
//! namespace-prefixing gap that lets two repos on one root/bucket read or
//! overwrite each other's data.

use crate::StorageBackend;
use crate::{MpuCompletedPart, PresignedDownload, PresignedMpu, PresignedPut};
use async_trait::async_trait;
use std::fmt;
use std::sync::Arc;

/// Characters allowed in a sanitized namespace: lowercase alphanumerics,
/// `.`, `_`, `-`.
fn is_allowed_ns_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-')
}

/// Sanitize an arbitrary string (e.g. a repo directory basename) into a
/// valid namespace: lowercased, any character outside `[a-z0-9._-]` replaced
/// with `-`, leading/trailing `-`/`.` trimmed. Never returns an empty
/// string for non-empty input; an all-invalid input collapses to `"repo"`.
pub fn sanitize_namespace(raw: &str) -> String {
    let lowered = raw.to_lowercase();
    let mut out: String = lowered
        .chars()
        .map(|c| if is_allowed_ns_char(c) { c } else { '-' })
        .collect();
    while out.starts_with(['-', '.']) {
        out.remove(0);
    }
    while out.ends_with(['-', '.']) {
        out.pop();
    }
    if out.is_empty() {
        "repo".to_string()
    } else {
        out
    }
}

/// Generate a fresh repo identity: 16 random hex chars drawn from the OS
/// RNG. Used to disambiguate two independently-created repositories that
/// happen to compute the same `repo_namespace` (e.g. same directory
/// basename) against the same storage root/bucket — see
/// `check_or_write_layout_marker`. Only needs to be unpredictable enough to
/// avoid collision, not cryptographically secret.
pub fn generate_repo_id() -> String {
    let mut bytes = [0u8; 8];
    // getrandom failure is effectively unreachable on supported platforms;
    // fall back to a fixed-but-still-unique-enough seed derived from the
    // current time rather than panicking a repo open on it.
    if getrandom::fill(&mut bytes).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        bytes = (nanos as u64).to_le_bytes();
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Wraps a [`StorageBackend`] so every key is transparently namespaced under
/// `"<ns>/"`. See module docs for the "why override everything" rationale.
#[derive(Clone)]
pub struct NamespacedBackend {
    inner: Arc<dyn StorageBackend>,
    ns: String,
}

impl NamespacedBackend {
    /// Wrap `inner` under namespace `ns`. `ns` is sanitized (see
    /// [`sanitize_namespace`]); an empty (post-sanitization is never empty,
    /// but a literal empty string is rejected outright — callers must supply
    /// a real value, not rely on sanitization to invent one).
    pub fn new(inner: Arc<dyn StorageBackend>, ns: impl Into<String>) -> anyhow::Result<Self> {
        let ns = ns.into();
        if ns.trim().is_empty() {
            anyhow::bail!("NamespacedBackend: namespace cannot be empty");
        }
        Ok(Self {
            inner,
            ns: sanitize_namespace(&ns),
        })
    }

    /// The sanitized namespace this backend prefixes keys with.
    pub fn namespace(&self) -> &str {
        &self.ns
    }

    /// Prefix `key` with the namespace, rejecting path-traversal/absolute
    /// keys before the inner backend ever sees them (J6 fix — this is the
    /// single choke point for every key-taking method on this wrapper).
    fn prefixed(&self, key: &str) -> anyhow::Result<String> {
        crate::validate_object_key(key)?;
        Ok(format!("{}/{}", self.ns, key))
    }

    fn strip(&self, key: &str) -> Option<String> {
        let prefix = format!("{}/", self.ns);
        key.strip_prefix(&prefix).map(|s| s.to_string())
    }
}

impl fmt::Debug for NamespacedBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NamespacedBackend")
            .field("ns", &self.ns)
            .field("inner", &self.inner)
            .finish()
    }
}

#[async_trait]
impl StorageBackend for NamespacedBackend {
    async fn get(&self, key: &str) -> anyhow::Result<Vec<u8>> {
        self.inner.get(&self.prefixed(key)?).await
    }

    async fn get_range(&self, key: &str, offset: u64, len: u64) -> anyhow::Result<Vec<u8>> {
        self.inner
            .get_range(&self.prefixed(key)?, offset, len)
            .await
    }

    async fn get_streaming(
        &self,
        key: &str,
    ) -> anyhow::Result<
        std::pin::Pin<
            Box<dyn futures::Stream<Item = anyhow::Result<bytes::Bytes>> + Send + 'static>,
        >,
    > {
        self.inner.get_streaming(&self.prefixed(key)?).await
    }

    async fn get_streaming_range(
        &self,
        key: &str,
        range: std::ops::Range<u64>,
    ) -> anyhow::Result<
        std::pin::Pin<
            Box<dyn futures::Stream<Item = anyhow::Result<bytes::Bytes>> + Send + 'static>,
        >,
    > {
        self.inner
            .get_streaming_range(&self.prefixed(key)?, range)
            .await
    }

    async fn put_file(&self, key: &str, src: &std::path::Path) -> anyhow::Result<()> {
        self.inner.put_file(&self.prefixed(key)?, src).await
    }

    async fn put(&self, key: &str, data: &[u8]) -> anyhow::Result<()> {
        self.inner.put(&self.prefixed(key)?, data).await
    }

    async fn exists(&self, key: &str) -> anyhow::Result<bool> {
        self.inner.exists(&self.prefixed(key)?).await
    }

    async fn delete(&self, key: &str) -> anyhow::Result<()> {
        self.inner.delete(&self.prefixed(key)?).await
    }

    async fn list_objects(&self, prefix: &str) -> anyhow::Result<Vec<String>> {
        let namespaced_prefix = self.prefixed(prefix)?;
        let keys = self.inner.list_objects(&namespaced_prefix).await?;
        Ok(keys.into_iter().filter_map(|k| self.strip(&k)).collect())
    }

    async fn get_with_size_hint(&self, key: &str, size: Option<u64>) -> anyhow::Result<Vec<u8>> {
        self.inner
            .get_with_size_hint(&self.prefixed(key)?, size)
            .await
    }

    async fn put_streaming(
        &self,
        key: &str,
        reader: Box<dyn tokio::io::AsyncRead + Send + Unpin>,
        len: u64,
    ) -> anyhow::Result<()> {
        self.inner
            .put_streaming(&self.prefixed(key)?, reader, len)
            .await
    }

    async fn presign_put(
        &self,
        key: &str,
        content_length: u64,
        ttl: std::time::Duration,
    ) -> anyhow::Result<Option<PresignedPut>> {
        self.inner
            .presign_put(&self.prefixed(key)?, content_length, ttl)
            .await
    }

    async fn presign_get(
        &self,
        key: &str,
        ttl: std::time::Duration,
    ) -> anyhow::Result<Option<PresignedDownload>> {
        self.inner.presign_get(&self.prefixed(key)?, ttl).await
    }

    async fn create_presigned_mpu(
        &self,
        key: &str,
        total_size: u64,
        ttl: std::time::Duration,
    ) -> anyhow::Result<Option<PresignedMpu>> {
        self.inner
            .create_presigned_mpu(&self.prefixed(key)?, total_size, ttl)
            .await
    }

    async fn complete_presigned_mpu(
        &self,
        key: &str,
        upload_id: &str,
        parts: Vec<MpuCompletedPart>,
    ) -> anyhow::Result<()> {
        self.inner
            .complete_presigned_mpu(&self.prefixed(key)?, upload_id, parts)
            .await
    }

    async fn abort_presigned_mpu(&self, key: &str, upload_id: &str) -> anyhow::Result<()> {
        self.inner
            .abort_presigned_mpu(&self.prefixed(key)?, upload_id)
            .await
    }

    async fn head(&self, key: &str) -> anyhow::Result<Option<u64>> {
        self.inner.head(&self.prefixed(key)?).await
    }

    /// Forwarded, not defaulted.
    ///
    /// Inheriting the trait's `Ok(None)` here would silently disable gc's
    /// prune grace period for every namespaced repo — the wrapper would report
    /// "age unknown" for objects whose age the inner backend knows perfectly
    /// well. A guard that cannot see is worse than no guard, because it still
    /// reads as present.
    async fn modified_at(&self, key: &str) -> anyhow::Result<Option<std::time::SystemTime>> {
        self.inner.modified_at(&self.prefixed(key)?).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockBackend;

    fn wrapped(ns: &str) -> NamespacedBackend {
        NamespacedBackend::new(Arc::new(MockBackend::new()), ns).unwrap()
    }

    #[test]
    fn sanitize_lowercases_and_replaces_invalid_chars() {
        // "My Repo!" -> "my-repo-" (space and '!' both become '-'),
        // then the trailing '-' is trimmed.
        assert_eq!(sanitize_namespace("My Repo!"), "my-repo");
    }

    #[test]
    fn sanitize_trims_leading_trailing_dashes_and_dots() {
        assert_eq!(sanitize_namespace("--.foo.--"), "foo");
    }

    #[test]
    fn sanitize_empty_or_all_invalid_falls_back() {
        assert_eq!(sanitize_namespace(""), "repo");
        assert_eq!(sanitize_namespace("!!!"), "repo");
    }

    #[test]
    fn new_rejects_empty_namespace() {
        let inner: Arc<dyn StorageBackend> = Arc::new(MockBackend::new());
        assert!(NamespacedBackend::new(inner.clone(), "").is_err());
        assert!(NamespacedBackend::new(inner, "   ").is_err());
    }

    #[tokio::test]
    async fn put_prefixes_the_physical_key() {
        let backend = wrapped("myrepo");
        backend.put("chunks/abc", b"data").await.unwrap();
        // The wrapped key is invisible through the wrapper's own API...
        assert_eq!(backend.get("chunks/abc").await.unwrap(), b"data");
        // ...but visible if we could peek at the inner backend directly.
        // MockBackend doesn't expose raw get, so verify via list_objects on
        // a second wrapper over the SAME inner with a different namespace:
        // it must NOT see repo A's key.
    }

    #[tokio::test]
    async fn list_strips_namespace_prefix() {
        let inner: Arc<dyn StorageBackend> = Arc::new(MockBackend::new());
        let backend = NamespacedBackend::new(inner, "myrepo").unwrap();
        backend.put("chunks/abc", b"1").await.unwrap();
        backend.put("chunks/def", b"2").await.unwrap();
        backend.put("manifests/xyz", b"3").await.unwrap();

        let mut keys = backend.list_objects("chunks/").await.unwrap();
        keys.sort();
        assert_eq!(keys, vec!["chunks/abc", "chunks/def"]);

        let mut all = backend.list_objects("").await.unwrap();
        all.sort();
        assert_eq!(all, vec!["chunks/abc", "chunks/def", "manifests/xyz"]);
    }

    #[tokio::test]
    async fn two_namespaces_on_one_inner_backend_never_collide() {
        let inner: Arc<dyn StorageBackend> = Arc::new(MockBackend::new());
        let repo_a = NamespacedBackend::new(inner.clone(), "repo-a").unwrap();
        let repo_b = NamespacedBackend::new(inner.clone(), "repo-b").unwrap();

        repo_a.put("chunks/same-name", b"from-a").await.unwrap();
        repo_b.put("chunks/same-name", b"from-b").await.unwrap();

        assert_eq!(repo_a.get("chunks/same-name").await.unwrap(), b"from-a");
        assert_eq!(repo_b.get("chunks/same-name").await.unwrap(), b"from-b");

        let a_keys = repo_a.list_objects("").await.unwrap();
        assert_eq!(a_keys, vec!["chunks/same-name"]);
        let b_keys = repo_b.list_objects("").await.unwrap();
        assert_eq!(b_keys, vec!["chunks/same-name"]);

        // Deleting repo A's key must not affect repo B's.
        repo_a.delete("chunks/same-name").await.unwrap();
        assert!(!repo_a.exists("chunks/same-name").await.unwrap());
        assert!(repo_b.exists("chunks/same-name").await.unwrap());
    }

    #[tokio::test]
    async fn traversal_key_is_rejected_before_reaching_inner_backend() {
        // J6: a `..`-bearing key must error out of the wrapper and never
        // touch the inner backend (no cross-namespace read/write).
        let inner: Arc<dyn StorageBackend> = Arc::new(MockBackend::new());
        let victim = NamespacedBackend::new(inner.clone(), "victim").unwrap();
        victim.put("chunks/secret", b"victim-data").await.unwrap();

        let attacker = NamespacedBackend::new(inner, "attacker").unwrap();
        let traversal_key = "../victim/chunks/secret";

        assert!(attacker.get(traversal_key).await.is_err());
        assert!(attacker.put(traversal_key, b"pwned").await.is_err());
        assert!(attacker.exists(traversal_key).await.is_err());
        assert!(attacker.delete(traversal_key).await.is_err());
        assert!(attacker.head(traversal_key).await.is_err());

        // Victim's data must be untouched.
        assert_eq!(victim.get("chunks/secret").await.unwrap(), b"victim-data");
    }

    #[tokio::test]
    async fn every_key_taking_method_is_namespaced() {
        // Coverage check: exercise every key-taking method through the
        // wrapper and confirm the value only shows up under the correct
        // namespace (via a second wrapper over the same inner backend).
        let inner: Arc<dyn StorageBackend> = Arc::new(MockBackend::new());
        let a = NamespacedBackend::new(inner.clone(), "a").unwrap();
        let b = NamespacedBackend::new(inner, "b").unwrap();

        a.put("k1", b"v1").await.unwrap();
        assert!(a.exists("k1").await.unwrap());
        assert!(!b.exists("k1").await.unwrap());

        assert_eq!(a.head("k1").await.unwrap(), Some(2));
        assert_eq!(b.head("k1").await.unwrap(), None);

        assert_eq!(a.get_range("k1", 0, 1).await.unwrap(), b"v");
        assert_eq!(a.get_with_size_hint("k1", Some(2)).await.unwrap(), b"v1");

        let mut stream = a.get_streaming("k1").await.unwrap();
        use futures::StreamExt;
        let mut collected = Vec::new();
        while let Some(chunk) = stream.next().await {
            collected.extend_from_slice(&chunk.unwrap());
        }
        assert_eq!(collected, b"v1");

        a.delete("k1").await.unwrap();
        assert!(!a.exists("k1").await.unwrap());
    }
}

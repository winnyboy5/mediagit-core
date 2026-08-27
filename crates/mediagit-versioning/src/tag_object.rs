// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Tag object: metadata for an annotated tag, stored as a first-class ODB
//! object (replacing the old `{tag_ref}.meta` companion-file hack).
//!
//! Unlike a lightweight tag (a ref pointing directly at a commit), an
//! annotated tag is a real object: `refs/tags/<name>` points at the Tag
//! object's OID, and the Tag object in turn points at its `target`.
//!
//! No leading format-version byte: this matches the existing convention for
//! ODB objects — [`crate::Commit`] and [`crate::Tree`] don't carry one
//! either (only free-standing, non-ODB formats like the reachability bitmap
//! do). Format evolution for ODB objects is a beta-cycle breaking change
//! (no migration), same as Commit/Tree.

use crate::{ObjectType, Oid, Signature};
use serde::{Deserialize, Serialize};
use std::fmt;

/// An annotated tag object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tag {
    /// OID of the object this tag points at (usually a commit).
    pub target: Oid,

    /// Declared type of `target`. Validated against the actual object by
    /// fsck; also used by graph walkers to know how to continue traversal.
    pub target_type: ObjectType,

    /// Tag name (without the `refs/tags/` prefix).
    pub name: String,

    /// Who created the tag and when.
    pub tagger: Signature,

    /// Tag message.
    pub message: String,

    /// MediaGit-native ed25519 signature over [`Tag::signing_payload`],
    /// present only when `MEDIAGIT_SIGN` was enabled at creation time.
    /// `None` means the tag is unsigned. This is **not** git's signature
    /// format — MediaGit is a standalone VCS; the ed25519 key is the user's
    /// existing OpenSSH key, reused only for familiar key-management UX.
    pub signature: Option<Vec<u8>>,
}

/// The exact byte sequence a signature is computed over: every `Tag` field
/// except `signature` itself, in this field order. Kept as a separate type
/// (rather than cloning `Tag` and zeroing `signature`) so the payload
/// definition can never silently drift from what's actually signed.
#[derive(Serialize)]
struct TagSigningPayload<'a> {
    target: &'a Oid,
    target_type: &'a ObjectType,
    name: &'a str,
    tagger: &'a Signature,
    message: &'a str,
}

impl Tag {
    /// Create a new, unsigned annotated tag.
    pub fn new(
        target: Oid,
        target_type: ObjectType,
        name: String,
        tagger: Signature,
        message: String,
    ) -> Self {
        Self {
            target,
            target_type,
            name,
            tagger,
            message,
            signature: None,
        }
    }

    /// The canonical bytes to sign/verify: postcard serialization of every
    /// field except `signature`. See [`TagSigningPayload`].
    pub fn signing_payload(&self) -> anyhow::Result<Vec<u8>> {
        let payload = TagSigningPayload {
            target: &self.target,
            target_type: &self.target_type,
            name: &self.name,
            tagger: &self.tagger,
            message: &self.message,
        };
        crate::format::serialize(&payload)
    }

    /// Serialize tag to bytes
    pub fn serialize(&self) -> anyhow::Result<Vec<u8>> {
        crate::format::serialize(self)
            .map_err(|e| anyhow::anyhow!("Tag serialization failed: {}", e))
    }

    /// Deserialize tag from bytes
    pub fn deserialize(data: &[u8]) -> anyhow::Result<Self> {
        crate::format::deserialize(data)
            .map_err(|e| anyhow::anyhow!("Tag deserialization failed: {}", e))
    }

    /// Write tag to object database and return its OID
    pub async fn write(&self, odb: &crate::ObjectDatabase) -> anyhow::Result<Oid> {
        let data = self.serialize()?;
        odb.write(ObjectType::Tag, &data).await
    }

    /// Read tag from object database by OID
    pub async fn read(odb: &crate::ObjectDatabase, oid: &Oid) -> anyhow::Result<Self> {
        let data = odb.read(oid).await?;
        Self::deserialize(&data)
    }
}

impl fmt::Display for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn tagger() -> Signature {
        Signature::new(
            "Alice".to_string(),
            "alice@example.com".to_string(),
            Utc::now(),
        )
    }

    #[test]
    fn test_tag_serialization_roundtrip_unsigned() {
        let target = Oid::hash(b"commit");
        let tag = Tag::new(
            target,
            ObjectType::Commit,
            "v1.0.0".to_string(),
            tagger(),
            "Release 1.0.0".to_string(),
        );

        let bytes = tag.serialize().unwrap();
        let decoded = Tag::deserialize(&bytes).unwrap();
        assert_eq!(tag, decoded);
        assert!(decoded.signature.is_none());
    }

    #[test]
    fn test_tag_serialization_roundtrip_signed() {
        let target = Oid::hash(b"commit");
        let mut tag = Tag::new(
            target,
            ObjectType::Commit,
            "v2.0.0".to_string(),
            tagger(),
            "Release 2.0.0".to_string(),
        );
        tag.signature = Some(vec![1, 2, 3, 4, 5]);

        let bytes = tag.serialize().unwrap();
        let decoded = Tag::deserialize(&bytes).unwrap();
        assert_eq!(tag, decoded);
        assert_eq!(decoded.signature, Some(vec![1, 2, 3, 4, 5]));
    }

    #[test]
    fn test_signing_payload_excludes_signature() {
        let target = Oid::hash(b"commit");
        let mut unsigned = Tag::new(
            target,
            ObjectType::Commit,
            "v1.0.0".to_string(),
            tagger(),
            "msg".to_string(),
        );
        let payload_before = unsigned.signing_payload().unwrap();

        unsigned.signature = Some(vec![9, 9, 9]);
        let payload_after = unsigned.signing_payload().unwrap();

        assert_eq!(
            payload_before, payload_after,
            "signing payload must be identical regardless of the signature field"
        );
    }

    #[tokio::test]
    async fn test_tag_odb_roundtrip() {
        use mediagit_storage::mock::MockBackend;
        use std::sync::Arc;

        let storage = Arc::new(MockBackend::new());
        let odb = crate::ObjectDatabase::new(storage, 100);

        let target = Oid::hash(b"commit");
        let tag = Tag::new(
            target,
            ObjectType::Commit,
            "v3.0.0".to_string(),
            tagger(),
            "Release 3.0.0".to_string(),
        );

        let tag_oid = tag.write(&odb).await.unwrap();
        let loaded = Tag::read(&odb, &tag_oid).await.unwrap();
        assert_eq!(tag, loaded);
    }
}

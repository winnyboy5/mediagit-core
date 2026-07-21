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

//! Tag signing using the user's existing OpenSSH ed25519 key.
//!
//! This is the `MEDIAGIT_SIGN` feature: annotated tags can be signed with
//! the same `~/.ssh/id_ed25519` key the user already has for SSH auth, for
//! familiar git-*like* key-management UX. The resulting signature is
//! **MediaGit-native** — an OpenSSH-armored [`ssh_key::SshSig`] blob stored
//! inside the `Tag` object — not git's signature format. MediaGit is a
//! standalone VCS; git interop is not a goal.
//!
//! Passphrase-protected keys are detected and rejected with a clear error;
//! passphrase prompting is out of scope for this cycle.

use anyhow::{bail, Context, Result};
use ssh_key::{HashAlg, LineEnding, PrivateKey, PublicKey, SshSig};
use std::path::{Path, PathBuf};

/// Signing namespace embedded in the SSH signature (analogous to git's
/// `git` namespace for `git tag -s` / `ssh-keygen -Y sign`). Verification
/// must use the same namespace a signature was created with.
const SIGN_NAMESPACE: &str = "mediagit-tag";

/// Read the `MEDIAGIT_SIGN` env var / config knob. Off (default) means
/// `tag -a` never signs. Set to `1`/`true`/`on` to opt in.
pub fn sign_enabled() -> bool {
    match std::env::var("MEDIAGIT_SIGN") {
        Ok(v) => matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "on"),
        Err(_) => false,
    }
}

/// Resolve the private key path to sign with: `MEDIAGIT_SIGN_KEY` env
/// override, else `~/.ssh/id_ed25519`.
pub fn default_key_path() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("MEDIAGIT_SIGN_KEY") {
        return Ok(PathBuf::from(p));
    }
    let home = dirs_home().context("Cannot resolve home directory for default SSH key path")?;
    Ok(home.join(".ssh").join("id_ed25519"))
}

/// Minimal home-directory lookup (avoids pulling in the `dirs` crate for a
/// single call): `$HOME` on Unix, `%USERPROFILE%` on Windows.
fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Sign `payload` with the ed25519 private key at `key_path` (OpenSSH
/// format). Returns the OpenSSH-armored signature bytes, suitable for
/// storing verbatim in `Tag::signature` and passing back to
/// [`verify_embedded`].
///
/// Note: no permission check is performed on the key file (ssh/git warn on
/// group/world-readable keys; MediaGit does not yet) — protect the key with
/// filesystem permissions.
pub fn sign(payload: &[u8], key_path: &Path) -> Result<Vec<u8>> {
    let pem = std::fs::read_to_string(key_path)
        .with_context(|| format!("Failed to read SSH key at {}", key_path.display()))?;
    let key = PrivateKey::from_openssh(&pem)
        .with_context(|| format!("Failed to parse SSH key at {}", key_path.display()))?;

    if key.is_encrypted() {
        bail!(
            "SSH key at {} is passphrase-protected; MediaGit does not yet support \
             prompting for a passphrase. Use an unencrypted key, or point \
             MEDIAGIT_SIGN_KEY at one.",
            key_path.display()
        );
    }

    let sig = key
        .sign(SIGN_NAMESPACE, HashAlg::Sha512, payload)
        .context("Failed to sign tag payload")?;
    let armored = sig
        .to_pem(LineEnding::LF)
        .context("Failed to encode signature")?;
    Ok(armored.into_bytes())
}

/// Load the public key matching `key_path` (i.e. the public half of the key
/// [`sign`] would use). Used to self-verify a tag signed with the same key.
pub fn public_key_for(key_path: &Path) -> Result<PublicKey> {
    let pem = std::fs::read_to_string(key_path)
        .with_context(|| format!("Failed to read SSH key at {}", key_path.display()))?;
    let key = PrivateKey::from_openssh(&pem)
        .with_context(|| format!("Failed to parse SSH key at {}", key_path.display()))?;
    Ok(key.public_key().clone())
}

/// Verify `signature` (as produced by [`sign`]) over `payload` against
/// `public_key`. Returns `Ok(true)`/`Ok(false)` for a well-formed signature
/// that verifies/fails to verify; `Err` only for a malformed signature blob
/// (not valid UTF-8 / not a parseable SSH signature).
pub fn verify(payload: &[u8], signature: &[u8], public_key: &PublicKey) -> Result<bool> {
    let sig_str = std::str::from_utf8(signature).context("Signature is not valid UTF-8")?;
    let sig: SshSig = sig_str.parse().context("Failed to parse SSH signature")?;
    Ok(public_key.verify(SIGN_NAMESPACE, payload, &sig).is_ok())
}

/// Verify `signature` over `payload` against the signer's public key
/// **embedded in the SSH signature itself**. This is a TOFU model: a
/// `Some` result proves the tag contents are exactly what the embedded key
/// signed — it does NOT establish who owns that key (trust-anchoring is
/// explicitly out of scope; surface the fingerprint and let the user decide).
/// Requires no local key, so any clone can verify.
///
/// Returns `Ok(Some(fingerprint))` (SHA-256) when the signature verifies,
/// `Ok(None)` when the contents do not match the signature, `Err` only for
/// a malformed signature blob.
pub fn verify_embedded(payload: &[u8], signature: &[u8]) -> Result<Option<String>> {
    let sig_str = std::str::from_utf8(signature).context("Signature is not valid UTF-8")?;
    let sig: SshSig = sig_str.parse().context("Failed to parse SSH signature")?;
    let public_key = PublicKey::from(sig.public_key().clone());
    if public_key.verify(SIGN_NAMESPACE, payload, &sig).is_ok() {
        Ok(Some(public_key.fingerprint(HashAlg::Sha256).to_string()))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssh_key::{rand_core::OsRng, Algorithm};

    fn write_test_key(dir: &Path, name: &str) -> PathBuf {
        let key = PrivateKey::random(&mut OsRng, Algorithm::Ed25519).expect("generate test key");
        let pem = key
            .to_openssh(ssh_key::LineEnding::LF)
            .expect("encode test key");
        let path = dir.join(name);
        std::fs::write(&path, pem.as_bytes()).expect("write test key file");
        path
    }

    #[test]
    fn sign_and_verify_round_trip() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let key_path = write_test_key(tmp.path(), "id_ed25519_a");

        let payload = b"tag payload bytes";
        let signature = sign(payload, &key_path).expect("sign");
        let public_key = public_key_for(&key_path).expect("public key");

        assert!(verify(payload, &signature, &public_key).expect("verify"));
    }

    #[test]
    fn tampered_payload_fails_verification() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let key_path = write_test_key(tmp.path(), "id_ed25519_a");

        let payload = b"tag payload bytes";
        let signature = sign(payload, &key_path).expect("sign");
        let public_key = public_key_for(&key_path).expect("public key");

        assert!(!verify(b"different payload bytes", &signature, &public_key).expect("verify"));
    }

    #[test]
    fn wrong_key_fails_verification() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let key_path = write_test_key(tmp.path(), "id_ed25519_a");
        let other_key_path = write_test_key(tmp.path(), "id_ed25519_b");

        let payload = b"tag payload bytes";
        let signature = sign(payload, &key_path).expect("sign");
        let wrong_public_key = public_key_for(&other_key_path).expect("public key");

        assert!(!verify(payload, &signature, &wrong_public_key).expect("verify"));
    }

    #[test]
    fn verify_embedded_round_trip_reports_signer_fingerprint() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let key_path = write_test_key(tmp.path(), "id_ed25519_a");

        let payload = b"tag payload bytes";
        let signature = sign(payload, &key_path).expect("sign");

        let fingerprint = verify_embedded(payload, &signature)
            .expect("well-formed signature")
            .expect("signature must verify against its embedded key");
        let signer_fp = public_key_for(&key_path)
            .expect("public key")
            .fingerprint(HashAlg::Sha256)
            .to_string();
        assert_eq!(
            fingerprint, signer_fp,
            "fingerprint must identify the signer"
        );
    }

    #[test]
    fn verify_embedded_rejects_tampered_payload() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let key_path = write_test_key(tmp.path(), "id_ed25519_a");

        let signature = sign(b"tag payload bytes", &key_path).expect("sign");

        assert!(
            verify_embedded(b"different payload bytes", &signature)
                .expect("well-formed signature")
                .is_none(),
            "tampered contents must not verify"
        );
    }

    /// Passphrase-protected ed25519 key (passphrase: "testpassphrase"),
    /// generated once with `ssh-keygen -t ed25519 -N testpassphrase` and
    /// embedded so the test needs neither ssh-keygen nor the `encryption`
    /// feature of the ssh-key crate.
    const ENCRYPTED_TEST_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAACmFlczI1Ni1jdHIAAAAGYmNyeXB0AAAAGAAAABDOkw2sMr
FSq4SLV85GrJrFAAAAGAAAAAEAAAAzAAAAC3NzaC1lZDI1NTE5AAAAIHYzX3beMwrdcaSH
sNAot6KurhZ/MhbjXQykFb/iPqljAAAAkHl0tIbgZjvjcQSjI3OBEDIF6VcGIn3drw5wxh
/wT+vwdmRu9pggSrxej1gOwWBAR+MKZ1Q9ghRiqEgeDOj6wsJbdXukzI/bqSXPeaDVvCH4
66ZqeLW8d7Cqg2BDNveE+e+wpr6qKK/ylv709Ma+qR8FDC34qNNDIf9Jb+7gS8+ne5yQLE
XSN9kOZv1lKThz2w==
-----END OPENSSH PRIVATE KEY-----
";

    #[test]
    fn passphrase_protected_key_is_rejected_with_clear_error() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let key_path = tmp.path().join("id_ed25519_encrypted");
        std::fs::write(&key_path, ENCRYPTED_TEST_KEY).expect("write encrypted key");

        let err = sign(b"payload", &key_path).expect_err("encrypted key must be rejected");
        assert!(
            err.to_string().contains("passphrase-protected"),
            "error must name the problem, got: {err}"
        );
    }

    #[test]
    fn sign_enabled_defaults_off_and_respects_knob() {
        // SAFETY: test-only env var scoping.
        std::env::remove_var("MEDIAGIT_SIGN");
        assert!(!sign_enabled(), "default must be OFF (opt-in)");

        std::env::set_var("MEDIAGIT_SIGN", "1");
        assert!(sign_enabled());

        std::env::set_var("MEDIAGIT_SIGN", "true");
        assert!(sign_enabled());

        std::env::set_var("MEDIAGIT_SIGN", "0");
        assert!(!sign_enabled());

        std::env::remove_var("MEDIAGIT_SIGN");
    }
}

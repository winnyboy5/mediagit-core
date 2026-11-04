// SPDX-License-Identifier: BUSL-1.1
//! Owner-only file permissions for key material, on every OS.
//!
//! Unix: mode `0o600`. Windows has no mode bit; a new file inherits its parent
//! directory's ACL, which on most setups grants `Users` or `Authenticated
//! Users` read access. `icacls /inheritance:r /grant:r <owner>:F` removes the
//! inherited entries and leaves only the owner, the posture OpenSSH requires
//! of private keys. The built-in `icacls.exe` is used rather than the raw ACL
//! APIs because the workspace forbids `unsafe_code`.

use std::io;
use std::path::Path;

/// Restrict `path` to its owner. Returns an error when the restriction could
/// not be applied, so callers can warn instead of failing silently.
#[cfg(unix)]
pub fn restrict_to_owner(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

/// See the module docs. The grantee is `DOMAIN\user` so domain and Entra ID
/// accounts resolve; `USERNAME` alone is ambiguous or wrong for them.
#[cfg(windows)]
pub fn restrict_to_owner(path: &Path) -> io::Result<()> {
    let user = std::env::var("USERNAME")
        .map_err(|_| io::Error::new(io::ErrorKind::NotFound, "USERNAME is not set"))?;
    let owner = match std::env::var("USERDOMAIN") {
        Ok(domain) if !domain.is_empty() => format!("{domain}\\{user}"),
        _ => user,
    };
    let out = std::process::Command::new("icacls")
        .arg(path)
        .arg("/inheritance:r")
        .arg("/grant:r")
        .arg(format!("{owner}:F"))
        .output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "icacls failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stdout).trim()
        )))
    }
}

#[cfg(not(any(unix, windows)))]
pub fn restrict_to_owner(_path: &Path) -> io::Result<()> {
    Ok(())
}

/// [`restrict_to_owner`], logging a warning instead of returning the error.
/// For key files whose content is already ciphertext: a permissive ACL weakens
/// a defence but does not expose the key, so writing must not fail over it.
pub fn restrict_to_owner_or_warn(path: &Path) {
    if let Err(e) = restrict_to_owner(path) {
        tracing::warn!(
            path = %path.display(),
            error = %e,
            "could not restrict key file to its owner; it keeps its directory's permissions"
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_is_an_error_not_silent() {
        let d = tempfile::tempdir().unwrap();
        assert!(restrict_to_owner(&d.path().join("absent.key")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn owner_only_mode() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("k");
        std::fs::write(&p, b"x").unwrap();
        restrict_to_owner(&p).unwrap();
        assert_eq!(
            std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[cfg(windows)]
    #[test]
    fn only_the_owner_keeps_access() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("k");
        std::fs::write(&p, b"x").unwrap();
        restrict_to_owner(&p).unwrap();
        let out = std::process::Command::new("icacls")
            .arg(&p)
            .output()
            .unwrap();
        // icacls prints the path before the first ACE; drop it so the user
        // name inside the temp path cannot mask an ACE on that line.
        let acl = String::from_utf8_lossy(&out.stdout)
            .to_lowercase()
            .replace(&p.display().to_string().to_lowercase(), "");
        let acl = acl
            .lines()
            .take_while(|l| !l.contains("successfully processed"))
            .collect::<Vec<_>>()
            .join("\n");
        let user = std::env::var("USERNAME").unwrap().to_lowercase();
        assert!(acl.contains(&user), "owner missing from ACL: {acl}");
        for broad in ["users", "everyone", "authenticated users", "(i)"] {
            let hits = acl
                .lines()
                .filter(|l| l.contains(broad) && !l.contains(&user))
                .count();
            assert_eq!(hits, 0, "{broad} still has access: {acl}");
        }
    }
}

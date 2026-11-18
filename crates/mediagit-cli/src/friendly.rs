// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! One place that turns a failed command into a code, a plain sentence and
//! the raw chain.
//!
//! Typed errors from the protocol client ([`HttpError`], `reqwest::Error`) and
//! common OS errors (`io::Error`) map to a stable `code` plus one short
//! sentence with a next action. Errors that already carry their own code
//! ([`crate::output::CodedError`], needs-input) are left alone. Everything
//! shown goes through [`crate::output::scrub`], so no URL credential, token or
//! query string reaches a person.

use mediagit_protocol::http_error::{HttpError, HttpErrorKind};

/// What `main` prints for a failed command.
#[derive(Debug, PartialEq, Eq)]
pub struct Friendly {
    pub code: &'static str,
    /// Plain sentence plus one next action. Safe to show.
    pub message: String,
    /// The full `{:#}` chain, scrubbed. For a "Details" fold.
    pub details: String,
}

/// OS-level problems with a code of their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Os {
    Refused,
    TimedOut,
    Denied,
    InUse,
    Exists,
    DiskFull,
}

fn os_problem(e: &std::io::Error) -> Option<Os> {
    use std::io::ErrorKind as K;
    // Windows sharing (32) and lock (33) violations have no stable ErrorKind.
    #[cfg(windows)]
    if matches!(e.raw_os_error(), Some(32 | 33)) {
        return Some(Os::InUse);
    }
    match e.kind() {
        K::ConnectionRefused => Some(Os::Refused),
        K::TimedOut => Some(Os::TimedOut),
        K::PermissionDenied => Some(Os::Denied),
        K::ResourceBusy | K::ExecutableFileBusy => Some(Os::InUse),
        K::AlreadyExists => Some(Os::Exists),
        K::StorageFull => Some(Os::DiskFull),
        _ => None,
    }
}

fn os_friendly(p: Os) -> (&'static str, &'static str) {
    match p {
        Os::Refused => (
            "server_unreachable",
            "Cannot reach the server. Check the address and that the server is running, then try again.",
        ),
        Os::TimedOut => (
            "server_timeout",
            "The server did not answer in time. Check your connection and try again.",
        ),
        Os::Denied => (
            "permission_denied",
            "Access to a file or folder was denied. Check its permissions, then try again.",
        ),
        Os::InUse => (
            "file_in_use",
            "A file is in use by another program. Close that program, then try again.",
        ),
        Os::Exists => (
            "already_exists",
            "A file or folder with that name already exists. Choose another name or remove it first.",
        ),
        Os::DiskFull => (
            "disk_full",
            "The disk is full. Free some space, then try again.",
        ),
    }
}

/// Code and sentence for a status the server answered with. `reason` is the
/// server's own text (400/409), used when it sent one.
pub fn http_friendly(h: &HttpError) -> (&'static str, String) {
    let reason = h.reason.as_deref().map(str::trim).filter(|r| !r.is_empty());
    match h.kind {
        HttpErrorKind::Unauthorized => (
            "not_signed_in",
            "You are not signed in, or your sign-in expired. Sign in and try again (mediagit auth login <server>).".into(),
        ),
        HttpErrorKind::Forbidden => (
            "no_access",
            "This account cannot do that here. Ask a server admin to give it access.".into(),
        ),
        HttpErrorKind::NotFound => (
            "remote_not_found",
            "The server does not have that. Check the repository name and the server address.".into(),
        ),
        HttpErrorKind::Conflict => (
            "conflict",
            match reason {
                Some(r) => r.to_owned(),
                None => "That conflicts with what is already on the server. Refresh and try again.".into(),
            },
        ),
        HttpErrorKind::BadRequest => (
            "request_rejected",
            match reason {
                Some(r) => r.to_owned(),
                None => "The server refused the request. Check what you entered and try again.".into(),
            },
        ),
        HttpErrorKind::Server => (
            "server_error",
            match reason {
                Some(r) => format!("The server hit a problem: {r}. Try again in a moment; if it keeps happening, tell the server admin."),
                None => "The server hit a problem. Try again in a moment; if it keeps happening, tell the server admin.".into(),
            },
        ),
        HttpErrorKind::Other => (
            "request_failed",
            "The server could not complete the request. Try again in a moment.".into(),
        ),
    }
}

/// `host:port` of the URL a failed request used, without credentials or query.
fn target(e: &reqwest::Error) -> Option<String> {
    let u = e.url()?;
    let host = u.host_str()?;
    Some(match u.port() {
        Some(p) => format!("{host}:{p}"),
        None => host.to_owned(),
    })
}

/// Map `err` to a friendly code and sentence, or `None` when nothing in the
/// chain is recognised (errors that carry their own code have nothing mappable in them).
pub fn friendly(err: &anyhow::Error) -> Option<Friendly> {
    let (code, message) = err.chain().find_map(|c| {
        if let Some(h) = c.downcast_ref::<HttpError>() {
            return Some(http_friendly(h));
        }
        if let Some(r) = c.downcast_ref::<reqwest::Error>() {
            let at = target(r).map(|t| format!(" at {t}")).unwrap_or_default();
            return match mediagit_protocol::http_error::classify_transport(r) {
                mediagit_protocol::http_error::TransportKind::Unreachable => Some((
                    "server_unreachable",
                    format!("Cannot reach the server{at}. Check the address and your network, then try again."),
                )),
                mediagit_protocol::http_error::TransportKind::TimedOut => Some((
                    "server_timeout",
                    format!("The server{at} did not answer in time. Check your connection and try again."),
                )),
                mediagit_protocol::http_error::TransportKind::Other => None,
            };
        }
        if let Some(io) = c.downcast_ref::<std::io::Error>() {
            return os_problem(io).map(|p| {
                let (code, text) = os_friendly(p);
                (code, text.to_owned())
            });
        }
        None
    })?;
    Some(Friendly {
        code,
        message: crate::output::scrub(&message),
        details: crate::output::scrub(&format!("{err:#}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mediagit_protocol::http_error::http_error;

    fn code_of(e: anyhow::Error) -> Option<&'static str> {
        friendly(&e).map(|f| f.code)
    }

    #[test]
    fn status_maps_to_code() {
        let table = [
            (400, "request_rejected"),
            (401, "not_signed_in"),
            (403, "no_access"),
            (404, "remote_not_found"),
            (409, "conflict"),
            (422, "request_rejected"),
            (500, "server_error"),
            (503, "server_error"),
            (418, "request_failed"),
        ];
        for (status, code) in table {
            let e: anyhow::Error = http_error("GET", "/x", status, "").into();
            assert_eq!(code_of(e.context("outer")), Some(code), "{status}");
        }
    }

    #[test]
    fn server_reason_is_the_sentence_for_400_and_409() {
        let body =
            r#"{"code":"password_too_short","message":"Passwords need at least 8 characters."}"#;
        for status in [400, 409] {
            let e: anyhow::Error = http_error("POST", "/auth/users", status, body).into();
            assert_eq!(
                friendly(&e).unwrap().message,
                "Passwords need at least 8 characters."
            );
        }
    }

    #[cfg(windows)]
    const OS_TABLE: &[(i32, &str)] = &[
        (5, "permission_denied"),
        (32, "file_in_use"),
        (33, "file_in_use"),
        (183, "already_exists"),
        (80, "already_exists"),
        (112, "disk_full"),
        (10061, "server_unreachable"),
        (10060, "server_timeout"),
    ];
    #[cfg(unix)]
    const OS_TABLE: &[(i32, &str)] = &[
        (13, "permission_denied"),
        (1, "permission_denied"),
        (16, "file_in_use"),
        (26, "file_in_use"),
        (17, "already_exists"),
        (28, "disk_full"),
        (111, "server_unreachable"),
        (110, "server_timeout"),
    ];

    #[test]
    fn os_code_maps_to_code_and_sentence() {
        for (raw, code) in OS_TABLE {
            let io = std::io::Error::from_raw_os_error(*raw);
            let e = anyhow::Error::new(io).context("writing the file");
            let f = friendly(&e).unwrap_or_else(|| panic!("os error {raw} not mapped"));
            assert_eq!(f.code, *code, "os error {raw}");
            assert!(
                f.message.ends_with('.') && !f.message.contains("os error"),
                "{}",
                f.message
            );
        }
    }

    #[test]
    fn unknown_and_explicitly_coded_errors_are_left_alone() {
        assert_eq!(code_of(anyhow::anyhow!("something odd")), None);
        let coded = anyhow::Error::new(crate::output::CodedError::new("invalid_args", "bad"));
        assert_eq!(code_of(coded), None);
    }

    #[tokio::test]
    async fn dead_endpoint_is_unreachable_and_leaks_nothing() {
        mediagit_protocol::ensure_crypto_provider();
        let r = reqwest::Client::new()
            .get("http://bob:s3cret@127.0.0.1:1/repo?token=tok123")
            .send()
            .await
            .unwrap_err();
        let f = friendly(&anyhow::Error::new(r).context("Failed to send GET /info/refs")).unwrap();
        assert_eq!(f.code, "server_unreachable");
        assert!(f.message.contains("127.0.0.1:1"), "{}", f.message);
        for text in [&f.message, &f.details] {
            for secret in ["s3cret", "tok123", "bob"] {
                assert!(!text.contains(secret), "{secret} leaked in {text}");
            }
        }
    }

    #[test]
    fn details_keep_the_chain() {
        let e: anyhow::Error = http_error("GET", "/info/refs", 401, "").into();
        let f = friendly(&e.context("fetch failed")).unwrap();
        assert!(f.details.contains("fetch failed") && f.details.contains("401"));
    }
}

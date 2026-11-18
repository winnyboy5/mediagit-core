// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! One typed error for a failed control-plane HTTP call.
//!
//! Callers used to `bail!("<METHOD> <path> failed with status: ..")` and the
//! CLI matched on that text. Now every non-success response goes through
//! [`http_error`], and callers downcast to [`HttpError`] instead.

use std::fmt;

/// What a status means to a person, not the number itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpErrorKind {
    Unauthorized,
    Forbidden,
    NotFound,
    Conflict,
    BadRequest,
    /// The server answered with something the caller can only call a server fault.
    Server,
    Other,
}

impl HttpErrorKind {
    pub fn from_status(status: u16) -> Self {
        match status {
            401 => Self::Unauthorized,
            403 => Self::Forbidden,
            404 => Self::NotFound,
            409 => Self::Conflict,
            400 | 422 => Self::BadRequest,
            500..=599 => Self::Server,
            _ => Self::Other,
        }
    }
}

/// Why a request never got an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportKind {
    /// Nothing is listening, or the name did not resolve.
    Unreachable,
    TimedOut,
    Other,
}

pub fn classify_transport(e: &reqwest::Error) -> TransportKind {
    if e.is_timeout() {
        TransportKind::TimedOut
    } else if e.is_connect() {
        TransportKind::Unreachable
    } else {
        TransportKind::Other
    }
}

/// A non-success HTTP answer. `reason`/`code` come from the server's JSON
/// error body (`{"code": "...", "message": "..."}`, or `{"error": "..."}` from the auth endpoints) when it sent one.
#[derive(Debug, Clone)]
pub struct HttpError {
    pub method: String,
    pub path: String,
    pub status: u16,
    pub kind: HttpErrorKind,
    pub code: Option<String>,
    pub reason: Option<String>,
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = reqwest::StatusCode::from_u16(self.status)
            .map(|s| s.to_string())
            .unwrap_or_else(|_| self.status.to_string());
        write!(
            f,
            "{} {} failed with status: {}",
            self.method, self.path, text
        )?;
        if let Some(r) = &self.reason {
            write!(f, ": {r}")?;
        }
        Ok(())
    }
}

impl std::error::Error for HttpError {}

/// A short plain-text body is the server's reason too (older handlers send one).
fn plain_text(body: &str) -> Option<String> {
    let t = body.trim();
    (!t.is_empty() && t.len() <= 300 && !t.starts_with(['<', '{', '['])).then(|| t.to_owned())
}

/// Build the typed error for a failed response. `body` is the response text
/// (may be empty or not JSON).
pub fn http_error(method: &str, path: &str, status: u16, body: &str) -> HttpError {
    let parsed: Option<serde_json::Value> = serde_json::from_str(body).ok();
    let field = |k: &str| {
        parsed
            .as_ref()
            .and_then(|v| v.get(k))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    HttpError {
        method: method.to_owned(),
        path: path.to_owned(),
        status,
        kind: HttpErrorKind::from_status(status),
        code: field("code"),
        reason: field("message")
            .or_else(|| field("error"))
            .or_else(|| plain_text(body)),
    }
}

/// [`http_error`] for a live response: reads the body, returns an `anyhow`
/// error that downcasts to [`HttpError`].
pub async fn fail(method: &str, path: &str, resp: reqwest::Response) -> anyhow::Error {
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    http_error(method, path, status, &body).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_maps_to_kind() {
        let table = [
            (400, HttpErrorKind::BadRequest),
            (401, HttpErrorKind::Unauthorized),
            (403, HttpErrorKind::Forbidden),
            (404, HttpErrorKind::NotFound),
            (409, HttpErrorKind::Conflict),
            (422, HttpErrorKind::BadRequest),
            (500, HttpErrorKind::Server),
            (503, HttpErrorKind::Server),
            (418, HttpErrorKind::Other),
        ];
        for (status, kind) in table {
            assert_eq!(http_error("GET", "/x", status, "").kind, kind, "{status}");
        }
    }

    #[test]
    fn server_reason_is_kept_and_shown() {
        let e = http_error(
            "POST",
            "/admin/repos",
            400,
            r#"{"code":"repo_name_invalid","message":"Names cannot end in a dot."}"#,
        );
        assert_eq!(e.code.as_deref(), Some("repo_name_invalid"));
        assert_eq!(e.reason.as_deref(), Some("Names cannot end in a dot."));
        assert_eq!(
            e.to_string(),
            "POST /admin/repos failed with status: 400 Bad Request: Names cannot end in a dot."
        );
    }

    #[test]
    fn plain_or_empty_body_has_no_reason() {
        for body in [
            "",
            "<html>oops</html>",
            "{}",
            r#"{"message":""}"#,
            &"x".repeat(400),
        ] {
            let e = http_error("GET", "/info/refs", 401, body);
            assert!(e.reason.is_none() && e.code.is_none(), "{body}");
            assert_eq!(
                e.to_string(),
                "GET /info/refs failed with status: 401 Unauthorized"
            );
        }
    }

    #[test]
    fn reason_is_read_from_every_body_shape() {
        let table = [
            (r#"{"message":"from message"}"#, Some("from message")),
            (r#"{"error":"from error"}"#, Some("from error")),
            (r#"{"message":"m","error":"e"}"#, Some("m")),
            (r#"{"error":""}"#, None),
            ("plain words", Some("plain words")),
        ];
        for (body, want) in table {
            let e = http_error("POST", "/auth/login", 401, body);
            assert_eq!(e.reason.as_deref(), want, "{body}");
        }
    }

    #[test]
    fn short_plain_body_is_the_reason() {
        let e = http_error(
            "GET",
            "/objects/pack",
            500,
            "incomplete closure
",
        );
        assert_eq!(e.reason.as_deref(), Some("incomplete closure"));
    }

    #[test]
    fn survives_anyhow_downcast() {
        let err: anyhow::Error = http_error("GET", "/x", 404, "").into();
        let err = err.context("outer");
        let h = err.chain().find_map(|c| c.downcast_ref::<HttpError>());
        assert_eq!(h.map(|h| h.kind), Some(HttpErrorKind::NotFound));
    }
}

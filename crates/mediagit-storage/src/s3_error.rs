// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! What an S3 SDK failure means. "Not found" must be earned by the response
//! (HTTP 404, or the codes `NoSuchKey` / `NotFound`), never guessed from the
//! error text: the SDK prints every service error, whether 403, 500 or 503, as
//! `service error`, and reading that as "absent" makes a present object look
//! deleted (and a repair believe it).

use aws_sdk_s3::config::http::HttpResponse;
use aws_sdk_s3::error::{ProvideErrorMetadata, SdkError};

/// The decision, over the two facts the response carries.
pub(crate) fn is_not_found_signal(status: Option<u16>, code: Option<&str>) -> bool {
    status == Some(404) || matches!(code, Some("NoSuchKey") | Some("NotFound"))
}

fn facts<E: ProvideErrorMetadata>(e: &SdkError<E, HttpResponse>) -> (Option<u16>, Option<String>) {
    (
        e.raw_response().map(|r| r.status().as_u16()),
        ProvideErrorMetadata::code(e).map(str::to_string),
    )
}

/// Is this SDK error a genuine "object does not exist"?
pub(crate) fn is_not_found<E: ProvideErrorMetadata>(e: &SdkError<E, HttpResponse>) -> bool {
    let (status, code) = facts(e);
    is_not_found_signal(status, code.as_deref())
}

/// On S3 a missing key answers 403, not 404, when the credentials lack
/// `s3:ListBucket`, so a refusal may really be a missing file.
fn refusal_hint(status: Option<u16>, code: Option<&str>) -> Option<&'static str> {
    (status == Some(403) || matches!(code, Some("AccessDenied"))).then_some(
        "the credentials are refused, or they lack list permission on the bucket \
             (s3:ListBucket), which S3 needs to tell a missing file from a forbidden one",
    )
}

/// What the server said, for error text: `status=403 code=AccessDenied`, plus a
/// plain hint on a refusal. Callers (and the retry layer's permanent-error
/// check) see it.
pub(crate) fn describe<E: ProvideErrorMetadata>(e: &SdkError<E, HttpResponse>) -> String {
    let (status, code) = facts(e);
    describe_facts(status, code.as_deref())
}

fn describe_facts(status: Option<u16>, code: Option<&str>) -> String {
    let base = format!(
        "status={} code={}",
        status.map_or("none".to_string(), |s| s.to_string()),
        code.unwrap_or("none")
    );
    match refusal_hint(status, code) {
        Some(h) => format!("{base}; {h}"),
        None => base,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_404_or_a_not_found_code_is_not_found() {
        assert!(is_not_found_signal(Some(404), None));
        assert!(is_not_found_signal(Some(404), Some("Whatever")));
        assert!(is_not_found_signal(None, Some("NoSuchKey")));
        assert!(is_not_found_signal(Some(200), Some("NotFound")));
        assert!(!is_not_found_signal(Some(403), Some("AccessDenied")));
        assert!(!is_not_found_signal(Some(500), Some("InternalError")));
        assert!(!is_not_found_signal(Some(503), Some("SlowDown")));
        assert!(!is_not_found_signal(Some(503), None));
        assert!(!is_not_found_signal(None, None));
    }

    #[test]
    fn a_refusal_says_so_in_plain_words_and_is_never_a_missing_object() {
        let m = describe_facts(Some(403), Some("AccessDenied"));
        assert!(m.contains("s3:ListBucket"), "{m}");
        assert!(m.contains("refused"), "{m}");
        assert!(!m.to_lowercase().contains("object not found"), "{m}");
        // No hint for other failures.
        assert!(!describe_facts(Some(503), Some("SlowDown")).contains("ListBucket"));
        assert_eq!(describe_facts(None, None), "status=none code=none");
    }

    fn sdk_error(
        status: u16,
        code: &str,
    ) -> SdkError<aws_sdk_s3::operation::head_object::HeadObjectError, HttpResponse> {
        use aws_sdk_s3::error::ErrorMetadata;
        use aws_sdk_s3::operation::head_object::HeadObjectError;
        use aws_sdk_s3::primitives::SdkBody;
        let meta = ErrorMetadata::builder().code(code).build();
        SdkError::service_error(
            HeadObjectError::generic(meta),
            HttpResponse::new(status.try_into().unwrap(), SdkBody::empty()),
        )
    }

    #[test]
    fn real_sdk_errors_map_by_status_and_code() {
        assert!(is_not_found(&sdk_error(404, "NotFound")));
        assert!(!is_not_found(&sdk_error(403, "AccessDenied")));
        assert!(!is_not_found(&sdk_error(503, "SlowDown")));
        // The text the old heuristic keyed on is the same for all of them.
        assert_eq!(
            sdk_error(403, "AccessDenied").to_string(),
            sdk_error(404, "NotFound").to_string()
        );
        assert!(
            describe(&sdk_error(403, "AccessDenied")).starts_with("status=403 code=AccessDenied")
        );
    }
}

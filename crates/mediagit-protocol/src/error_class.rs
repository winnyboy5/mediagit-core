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

/// Outcome of classifying a failed direct-transfer HTTP response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferOutcome {
    /// Retry this chunk on a fresh connection with exponential backoff.
    Transient,
    /// Re-request a fresh presigned URL from the server, then retry.
    RefreshUrl,
    /// Fall back to proxy for this chunk only; does not affect siblings.
    PermanentChunk,
    /// Object reported as not-found but may still be replicating (e.g. MPU completion lag).
    /// Caller should sleep for the given duration, then fall back to proxy.
    PermanentChunkAfterDelay(std::time::Duration),
    /// Config-level failure (auth, bucket policy). Increment global counter;
    /// ≥3 of these across the push signals a real misconfiguration.
    PermanentConfig,
}

fn not_found_delay() -> std::time::Duration {
    let ms = std::env::var("MEDIAGIT_404_FALLBACK_DELAY_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(500);
    std::time::Duration::from_millis(ms)
}

/// Extract the error code from an AWS S3 / Azure Blob XML error body.
/// Both clouds use `<Code>SomeCode</Code>`.
pub fn parse_xml_error_code(body: &str) -> &str {
    if let Some(start) = body.find("<Code>") {
        let rest = &body[start + 6..];
        if let Some(end) = rest.find("</Code>") {
            return rest[..end].trim();
        }
    }
    ""
}

/// Extract the error reason from a GCS JSON error body.
/// Looks for `"reason":"<value>"` anywhere in the string.
pub fn parse_gcs_reason(body: &str) -> &str {
    if let Some(pos) = body.find("\"reason\":\"") {
        let rest = &body[pos + 10..];
        if let Some(end) = rest.find('"') {
            return &rest[..end];
        }
    }
    ""
}

/// Classify an S3 / MinIO upload error.
///
/// `header_code` — value of the `x-amz-error-code` response header, if present.
/// `body`        — up to 2 KiB of the response body (AWS XML format).
pub fn classify_s3(status: u16, header_code: &str, body: &str) -> TransferOutcome {
    let xml = parse_xml_error_code(body);
    let code = if !xml.is_empty() { xml } else { header_code };
    match code {
        // Transient 400s — AWS explicitly marks these as retryable
        "RequestTimeout"
        | "RequestTimeoutException"
        | "BadDigest"
        | "IncompleteBody"
        | "InternalError"
        | "RequestAborted"
        | "ServiceUnavailable"
        | "SlowDown" => TransferOutcome::Transient,
        // URL expired or clock skew — re-sign and retry
        "ExpiredToken" | "TokenRefreshRequired" | "RequestTimeTooSkewed" => {
            TransferOutcome::RefreshUrl
        }
        // Config/auth errors
        "AccessDenied"
        | "InvalidAccessKeyId"
        | "SignatureDoesNotMatch"
        | "AuthorizationHeaderMalformed"
        | "InvalidRequest"
        | "InvalidArgument"
        | "EntityTooLarge"
        | "EntityTooSmall"
        | "NoSuchBucket"
        | "NotImplemented" => TransferOutcome::PermanentConfig,
        _ => classify_by_status(status),
    }
}

/// Classify an Azure Blob Storage upload error.
///
/// `ms_error_code` — value of the `x-ms-error-code` response header (preferred).
/// `body`          — up to 2 KiB of the response body (Azure XML format).
pub fn classify_azure(status: u16, ms_error_code: &str, body: &str) -> TransferOutcome {
    let xml = parse_xml_error_code(body);
    let code = if !ms_error_code.is_empty() {
        ms_error_code
    } else {
        xml
    };
    match code {
        "ServerBusy" | "InternalError" | "OperationTimedOut" | "CannotVerifyCopySource" => {
            TransferOutcome::Transient
        }
        // May be clock skew — attempt URL refresh once before giving up
        "AuthenticationFailed" => TransferOutcome::RefreshUrl,
        "AuthorizationFailure"
        | "InsufficientAccountPermissions"
        | "InvalidAuthenticationInfo"
        | "InvalidBlockId"
        | "BlockCountExceedsLimit"
        | "RequestBodyTooLarge"
        | "ConditionNotMet"
        | "BlobAlreadyExists" => TransferOutcome::PermanentConfig,
        _ => classify_by_status(status),
    }
}

/// Classify a GCS upload error from the JSON body.
pub fn classify_gcs(status: u16, body: &str) -> TransferOutcome {
    let reason = parse_gcs_reason(body);
    match reason {
        "rateLimitExceeded" | "userRateLimitExceeded" | "backendError" | "internalError" => {
            TransferOutcome::Transient
        }
        "authError" | "forbidden" => TransferOutcome::PermanentConfig,
        _ => classify_by_status(status),
    }
}

/// Infer the storage backend from the presigned URL.
fn detect_backend(url: &str, content_type: &str) -> &'static str {
    if url.contains(".blob.core.windows.net") {
        "azure"
    } else if url.contains("storage.googleapis.com")
        || url.contains("googleapis.com/storage")
        || (content_type.contains("application/json") && url.contains("googleapis"))
    {
        "gcs"
    } else {
        "s3"
    }
}

/// Classify an upload error, auto-detecting backend from URL.
pub fn classify_auto(
    status: u16,
    url: &str,
    content_type: &str,
    header_code: &str,
    body: &str,
) -> TransferOutcome {
    match detect_backend(url, content_type) {
        "azure" => classify_azure(status, header_code, body),
        "gcs" => classify_gcs(status, body),
        _ => classify_s3(status, header_code, body),
    }
}

/// Classify a presigned-GET download error, auto-detecting backend from URL.
/// Returns `TransferOutcome::PermanentChunk` for "object not found" so the client
/// falls back to the proxy GET path rather than retrying forever.
pub fn classify_auto_get(
    status: u16,
    url: &str,
    content_type: &str,
    header_code: &str,
    body: &str,
) -> TransferOutcome {
    // NoSuchKey on GET → object may not be replicating yet; delay before proxy fallback
    let xml_code = parse_xml_error_code(body);
    if xml_code == "NoSuchKey" || xml_code == "NoSuchObject" {
        return TransferOutcome::PermanentChunkAfterDelay(not_found_delay());
    }
    // 416 Range Not Satisfiable → range overshoot is a real bug, no delay
    if status == 416 {
        return TransferOutcome::PermanentChunk;
    }
    // GCS "notFound" reason → same delayed fallback
    let gcs_reason = parse_gcs_reason(body);
    if gcs_reason == "notFound" {
        return TransferOutcome::PermanentChunkAfterDelay(not_found_delay());
    }
    // Otherwise defer to the same classify_auto logic
    classify_auto(status, url, content_type, header_code, body)
}

fn classify_by_status(status: u16) -> TransferOutcome {
    match status {
        408 | 429 | 500 | 502 | 503 | 504 => TransferOutcome::Transient,
        403 => TransferOutcome::RefreshUrl,
        400..=499 => TransferOutcome::PermanentConfig,
        _ => TransferOutcome::PermanentChunk,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A NOTE ON THE STATUS CODES BELOW, learned from mutation testing.
    //
    // Every one of these tests pairs an error CODE with an HTTP STATUS. When the
    // code is not in the table, classification falls through to
    // classify_by_status. So if a test pairs a code with a status that already
    // yields the same outcome, the assertion holds whether or not the code table
    // exists at all - deleting the entire table still passes.
    //
    // That was literally true here: `classify_azure(503, "ServerBusy", "")` was
    // asserted to be Transient, and 503 is Transient by status anyway. Mutation
    // testing deleted the Azure transient arm and no test noticed.
    //
    // RULE: pair each code with a status whose fallback DIFFERS from the expected
    // outcome, so the assertion can only pass via the code table.
    //   408|429|500|502|503|504 -> Transient
    //   403                     -> RefreshUrl
    //   400..=499               -> PermanentConfig
    //   anything else           -> PermanentChunk
    // In practice: test a Transient code with 400, and a PermanentConfig code
    // with 503.

    #[test]
    fn s3_request_timeout_is_transient() {
        let body = "<Error><Code>RequestTimeout</Code><Message>socket timed out</Message></Error>";
        assert_eq!(classify_s3(400, "", body), TransferOutcome::Transient);
    }

    #[test]
    fn s3_bad_digest_is_transient() {
        assert_eq!(
            classify_s3(400, "", "<Error><Code>BadDigest</Code></Error>"),
            TransferOutcome::Transient
        );
    }

    #[test]
    fn s3_slow_down_is_transient() {
        assert_eq!(
            // 400, not 503: 503 is Transient by status, which would make this
            // pass even with the SlowDown arm deleted.
            classify_s3(400, "", "<Error><Code>SlowDown</Code></Error>"),
            TransferOutcome::Transient
        );
    }

    #[test]
    fn s3_expired_token_triggers_refresh() {
        assert_eq!(
            classify_s3(400, "", "<Error><Code>ExpiredToken</Code></Error>"),
            TransferOutcome::RefreshUrl
        );
    }

    #[test]
    fn s3_access_denied_is_permanent_config() {
        assert_eq!(
            classify_s3(403, "", "<Error><Code>AccessDenied</Code></Error>"),
            TransferOutcome::PermanentConfig
        );
    }

    #[test]
    fn s3_signature_mismatch_is_permanent_config() {
        assert_eq!(
            classify_s3(403, "", "<Error><Code>SignatureDoesNotMatch</Code></Error>"),
            TransferOutcome::PermanentConfig
        );
    }

    #[test]
    fn s3_header_code_takes_priority_over_empty_body() {
        assert_eq!(
            classify_s3(400, "RequestTimeout", ""),
            TransferOutcome::Transient
        );
    }

    #[test]
    fn azure_server_busy_is_transient() {
        // 400 (PermanentConfig by status) so only the code table can produce Transient.
        assert_eq!(
            classify_azure(400, "ServerBusy", ""),
            TransferOutcome::Transient
        );
    }

    #[test]
    fn azure_operation_timed_out_is_transient() {
        assert_eq!(
            classify_azure(400, "", "<Error><Code>OperationTimedOut</Code></Error>"),
            TransferOutcome::Transient
        );
    }

    #[test]
    fn azure_auth_failed_triggers_refresh() {
        // 400, not 403: 403 is RefreshUrl by status and would mask the arm.
        assert_eq!(
            classify_azure(400, "AuthenticationFailed", ""),
            TransferOutcome::RefreshUrl
        );
    }

    #[test]
    fn azure_authorization_failure_is_permanent_config() {
        // 503 is Transient by status, so PermanentConfig can only come from the
        // code table. Without this the whole Azure config arm could be deleted
        // and every test still passed - meaning a bucket-policy failure would be
        // retried as if transient instead of bailing out.
        assert_eq!(
            classify_azure(503, "AuthorizationFailure", ""),
            TransferOutcome::PermanentConfig
        );
    }

    #[test]
    fn azure_header_code_wins_over_body_xml() {
        // x-ms-error-code is documented as authoritative; the body is the
        // fallback. If that priority inverts, this reads AuthorizationFailure
        // and returns PermanentConfig instead of retrying a busy server.
        assert_eq!(
            classify_azure(
                400,
                "ServerBusy",
                "<Error><Code>AuthorizationFailure</Code></Error>"
            ),
            TransferOutcome::Transient
        );
    }

    #[test]
    fn gcs_rate_limit_is_transient() {
        // 400, not 429: 429 is Transient by status and would mask the reason table.
        assert_eq!(
            classify_gcs(
                400,
                r#"{"error":{"errors":[{"reason":"rateLimitExceeded"}]}}"#
            ),
            TransferOutcome::Transient
        );
    }

    #[test]
    fn gcs_backend_error_is_transient() {
        assert_eq!(
            classify_gcs(400, r#"{"error":{"errors":[{"reason":"backendError"}]}}"#),
            TransferOutcome::Transient
        );
    }

    #[test]
    fn gcs_auth_error_is_permanent_config() {
        // 503 is Transient by status; PermanentConfig can only come from the
        // reason table. Otherwise a bad service account is retried forever.
        assert_eq!(
            classify_gcs(503, r#"{"error":{"errors":[{"reason":"authError"}]}}"#),
            TransferOutcome::PermanentConfig
        );
    }

    #[test]
    fn status_429_fallback_is_transient() {
        assert_eq!(classify_s3(429, "", ""), TransferOutcome::Transient);
    }

    #[test]
    fn detect_backend_s3() {
        assert_eq!(
            detect_backend("https://mybucket.s3.amazonaws.com/obj?X-Amz=a", ""),
            "s3"
        );
    }

    #[test]
    fn detect_backend_minio() {
        assert_eq!(
            detect_backend("http://localhost:9000/bucket/key?X-Amz=a", ""),
            "s3"
        );
    }

    #[test]
    fn detect_backend_azure() {
        assert_eq!(
            detect_backend("https://account.blob.core.windows.net/c/b?se=x", ""),
            "azure"
        );
    }

    #[test]
    fn detect_backend_gcs() {
        assert_eq!(
            detect_backend("https://storage.googleapis.com/bucket/obj?X-Goog=x", ""),
            "gcs"
        );
    }

    #[test]
    fn detect_backend_gcs_json_api_url() {
        // The second host form: www.googleapis.com/storage/... rather than
        // storage.googleapis.com. Only this URL shape exercises the second arm
        // of the `||` chain - the first form short-circuits before reaching it.
        assert_eq!(
            detect_backend("https://www.googleapis.com/storage/v1/b/bk/o/obj", ""),
            "gcs"
        );
    }

    #[test]
    fn detect_backend_json_content_type_alone_is_not_gcs() {
        // A JSON content-type is only a GCS hint when the URL is also a Google
        // host. If those two conditions ever become an OR, every S3 endpoint
        // that answers with JSON gets parsed by the GCS classifier and its S3
        // error codes stop being recognised.
        assert_eq!(
            detect_backend("https://mybucket.s3.amazonaws.com/obj", "application/json"),
            "s3"
        );
    }

    #[test]
    fn classify_auto_routes_azure_urls_to_the_azure_table() {
        // CannotVerifyCopySource is Transient only in the Azure table; under the
        // S3 table it is unknown and 400 makes it PermanentConfig. So this can
        // only pass if the azure route is intact.
        assert_eq!(
            classify_auto(
                400,
                "https://account.blob.core.windows.net/c/b?se=x",
                "",
                "CannotVerifyCopySource",
                ""
            ),
            TransferOutcome::Transient
        );
    }

    #[test]
    fn classify_auto_routes_gcs_urls_to_the_gcs_table() {
        // A GCS JSON reason body means nothing to the S3 classifier, which would
        // fall through to status 400 -> PermanentConfig and bail instead of retry.
        assert_eq!(
            classify_auto(
                400,
                "https://storage.googleapis.com/bucket/obj",
                "",
                "",
                r#"{"error":{"errors":[{"reason":"backendError"}]}}"#
            ),
            TransferOutcome::Transient
        );
    }

    // The three tests below close arms that coverage showed had NEVER executed.
    // Mutation testing could not have found them: deleting a catch-all arm makes
    // the match non-exhaustive, so cargo-mutants reports it "unviable" rather
    // than running it. Coverage and mutation are complementary here.

    #[test]
    fn azure_unknown_code_falls_back_to_status() {
        // The code tables cannot enumerate every error a cloud invents, so the
        // UNKNOWN code is the common real-world case, not the exotic one. If this
        // fallback breaks, an unrecognised 503 stops being retried.
        assert_eq!(
            classify_azure(503, "SomeCodeAzureAddedLastTuesday", ""),
            TransferOutcome::Transient
        );
    }

    #[test]
    fn gcs_unknown_reason_falls_back_to_status() {
        assert_eq!(
            classify_gcs(503, r#"{"error":{"errors":[{"reason":"brandNewReason"}]}}"#),
            TransferOutcome::Transient
        );
    }

    #[test]
    fn unexpected_status_is_permanent_chunk() {
        // The catch-all for anything outside 4xx/5xx - e.g. a proxy answering 302.
        // PermanentChunk means "fall back to proxy for this chunk only", which is
        // the right conservative move for a status we do not understand.
        assert_eq!(classify_s3(302, "", ""), TransferOutcome::PermanentChunk);
    }

    #[test]
    fn unknown_403_is_refresh_url_not_permanent() {
        // 403 must be its own arm ahead of the 400..=499 sweep: a presigned URL
        // that has simply expired answers 403, and re-signing recovers it.
        // Folding it into PermanentConfig turns every expiry into a hard failure.
        assert_eq!(classify_s3(403, "", ""), TransferOutcome::RefreshUrl);
    }

    #[test]
    fn unknown_4xx_is_permanent_config_not_permanent_chunk() {
        // The 400..=499 sweep. Without it a 404 falls to the catch-all
        // PermanentChunk, which retries per chunk instead of counting toward the
        // "3 config failures means a real misconfiguration" signal.
        assert_eq!(classify_s3(404, "", ""), TransferOutcome::PermanentConfig);
    }

    #[test]
    fn get_no_such_key_is_delayed_fallback() {
        let body = "<Error><Code>NoSuchKey</Code></Error>";
        let outcome = classify_auto_get(404, "https://bucket.s3.amazonaws.com/key", "", "", body);
        let TransferOutcome::PermanentChunkAfterDelay(delay) = outcome else {
            panic!("expected PermanentChunkAfterDelay, got {outcome:?}");
        };
        // The delay is the whole point of this variant: the object may still be
        // replicating. A zero delay makes the proxy fallback fire immediately and
        // hit the same not-yet-visible object, so assert it is actually a wait.
        assert!(
            delay > std::time::Duration::ZERO,
            "the replication-lag delay must be non-zero, got {delay:?}"
        );
    }

    #[test]
    fn get_gcs_not_found_is_delayed_fallback() {
        let body = r#"{"error":{"errors":[{"reason":"notFound"}]}}"#;
        let outcome = classify_auto_get(
            404,
            "https://storage.googleapis.com/bucket/obj",
            "",
            "",
            body,
        );
        assert!(
            matches!(outcome, TransferOutcome::PermanentChunkAfterDelay(_)),
            "expected PermanentChunkAfterDelay, got {outcome:?}"
        );
    }

    #[test]
    fn get_416_stays_permanent_chunk_no_delay() {
        assert_eq!(
            classify_auto_get(416, "https://bucket.s3.amazonaws.com/key", "", "", ""),
            TransferOutcome::PermanentChunk
        );
    }
}

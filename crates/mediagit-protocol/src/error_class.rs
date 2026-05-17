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
    /// Config-level failure (auth, bucket policy). Increment global counter;
    /// ≥3 of these across the push signals a real misconfiguration.
    PermanentConfig,
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
    // NoSuchKey on GET → object may not be replicated yet → proxy fallback
    let xml_code = parse_xml_error_code(body);
    if xml_code == "NoSuchKey" || xml_code == "NoSuchObject" {
        return TransferOutcome::PermanentChunk;
    }
    // 416 Range Not Satisfiable → treat as permanent (no range headers sent by us)
    if status == 416 {
        return TransferOutcome::PermanentChunk;
    }
    // GCS "notFound" reason
    let gcs_reason = parse_gcs_reason(body);
    if gcs_reason == "notFound" {
        return TransferOutcome::PermanentChunk;
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
            classify_s3(503, "", "<Error><Code>SlowDown</Code></Error>"),
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
        assert_eq!(
            classify_azure(503, "ServerBusy", ""),
            TransferOutcome::Transient
        );
    }

    #[test]
    fn azure_operation_timed_out_is_transient() {
        assert_eq!(
            classify_azure(500, "", "<Error><Code>OperationTimedOut</Code></Error>"),
            TransferOutcome::Transient
        );
    }

    #[test]
    fn azure_auth_failed_triggers_refresh() {
        assert_eq!(
            classify_azure(403, "AuthenticationFailed", ""),
            TransferOutcome::RefreshUrl
        );
    }

    #[test]
    fn gcs_rate_limit_is_transient() {
        assert_eq!(
            classify_gcs(
                429,
                r#"{"error":{"errors":[{"reason":"rateLimitExceeded"}]}}"#
            ),
            TransferOutcome::Transient
        );
    }

    #[test]
    fn gcs_backend_error_is_transient() {
        assert_eq!(
            classify_gcs(500, r#"{"error":{"errors":[{"reason":"backendError"}]}}"#),
            TransferOutcome::Transient
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
    fn get_no_such_key_is_permanent_chunk() {
        let body = "<Error><Code>NoSuchKey</Code></Error>";
        assert_eq!(
            classify_auto_get(404, "https://bucket.s3.amazonaws.com/key", "", "", body),
            TransferOutcome::PermanentChunk
        );
    }

    #[test]
    fn get_416_is_permanent_chunk() {
        assert_eq!(
            classify_auto_get(416, "https://bucket.s3.amazonaws.com/key", "", "", ""),
            TransferOutcome::PermanentChunk
        );
    }
}

use std::sync::OnceLock;
use std::time::Duration;

use aws_sdk_s3::config::SharedHttpClient;
use aws_smithy_http_client::tls::rustls_provider::CryptoMode;
use aws_smithy_http_client::{tls, Builder};

static SHARED_HTTP: OnceLock<SharedHttpClient> = OnceLock::new();

/// Returns a process-wide warm HTTP client shared across all AWS-SDK backends.
///
/// All SDK calls (create_multipart_upload, complete_multipart_upload, head_bucket, …)
/// reuse TCP+TLS sessions from this pool instead of opening new connections on every
/// burst. This eliminates the `dispatch failure` (TCP connect timeout) pattern that
/// appears when the default per-request client opens dozens of fresh connections
/// simultaneously.
///
/// Env knobs (all optional):
///   MEDIAGIT_AWS_POOL_IDLE_SECS — seconds to keep idle connections alive (default: 90)
pub fn shared() -> SharedHttpClient {
    SHARED_HTTP
        .get_or_init(|| {
            let pool_idle_secs: u64 = std::env::var("MEDIAGIT_AWS_POOL_IDLE_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(90);

            Builder::new()
                .pool_idle_timeout(Duration::from_secs(pool_idle_secs))
                .tls_provider(tls::Provider::Rustls(CryptoMode::AwsLc))
                .build_https()
        })
        .clone()
}

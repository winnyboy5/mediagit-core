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

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use mediagit_server::{
    AppState, RateLimitConfig, ServerConfig, create_rate_limited_router, create_router,
    create_router_sharing_rate_limit,
};

mod setup;

/// MediaGit Server - HTTP(S) server for MediaGit repositories
#[derive(Parser, Debug)]
#[command(name = "mediagit-server")]
#[command(about = "MediaGit repository server", long_about = None)]
struct Args {
    /// Setup subcommands (init / admin). Bare `mediagit-server` (no
    /// subcommand) still means "serve", exactly as before.
    #[command(subcommand)]
    command: Option<Cmd>,

    /// Port to listen on (overrides config file)
    #[arg(short, long)]
    port: Option<u16>,

    /// Host address to bind to (overrides config file)
    #[arg(long)]
    host: Option<String>,

    /// Directory for repository storage (overrides config file repos_dir)
    #[arg(long)]
    data_dir: Option<PathBuf>,

    /// Path to config file
    #[arg(short, long, default_value = "mediagit-server.toml")]
    config: String,
}

#[derive(clap::Subcommand, Debug)]
enum Cmd {
    /// Interactive setup wizard: writes mediagit-server.toml and (optionally)
    /// creates the first admin user.
    Init(setup::InitArgs),
    /// Manage users in the auth store without starting the server.
    Admin(setup::AdminArgs),
}

#[tokio::main]
async fn main() -> Result<()> {
    // google-cloud-storage v1 enables aws-lc-rs by default; this crate already
    // depends on rustls with the `ring` feature for inbound TLS. With both
    // providers compiled in, rustls 0.23 refuses to pick automatically and
    // panics on first TLS use ("Could not automatically determine the
    // process-level CryptoProvider"). Install ring explicitly to match the
    // existing inbound TLS path. `_ =` swallows the "already installed" error
    // if some other entry point (e.g. test harness) raced us.
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Parse CLI arguments
    let args = Args::parse();

    // Setup subcommands (init / admin) short-circuit before any of the
    // normal serve-path config loading below. No subcommand -> None -> falls
    // straight through to the existing bare-invocation / --config / --port /
    // --data-dir serve path, unchanged.
    if let Some(cmd) = args.command {
        return match cmd {
            Cmd::Init(init_args) => setup::run_init(&init_args).await,
            Cmd::Admin(admin_args) => setup::run_admin(&admin_args).await,
        };
    }

    // Setup tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "mediagit_server=debug,tower_http=debug,mediagit_storage=warn".into()
            }),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load configuration from file (use path from --config, default is "mediagit-server.toml")
    let mut config = ServerConfig::load(&args.config)?;

    // Override config with CLI arguments if provided
    if let Some(port) = args.port {
        tracing::info!("Overriding port from CLI: {} -> {}", config.port, port);
        config.port = port;
    }
    if let Some(host) = args.host {
        tracing::info!("Overriding host from CLI: {} -> {}", config.host, host);
        config.host = host;
    }
    if let Some(data_dir) = args.data_dir {
        tracing::info!(
            "Overriding repos_dir from CLI: {:?} -> {:?}",
            config.repos_dir,
            data_dir
        );
        config.repos_dir = data_dir;
    }

    tracing::info!("Server configuration: {:?}", config);

    // Startup summary banner — operators should see at a glance what's wired.
    let bind_addr = if config.enable_tls {
        format!("{}:{} (TLS)", config.host, config.tls_port)
    } else {
        format!("{}:{} (HTTP)", config.host, config.port)
    };
    let auth_state = if config.enable_auth { "ON" } else { "OFF" };
    let rl_state = if config.enable_rate_limiting {
        format!(
            "ON ({} rps, burst {})",
            config.rate_limit_rps, config.rate_limit_burst
        )
    } else {
        "OFF".to_string()
    };
    tracing::info!(
        "mediagit-server | listen={} | repos={} | auth={} | rate_limit={}",
        bind_addr,
        config.repos_dir.display(),
        auth_state,
        rl_state,
    );

    // P0-4: refuse to bind a non-loopback address with auth disabled — that's
    // an open server on the network with zero credentials. Loopback-only binds
    // are still allowed (matches today's local-dev default). Escape hatch for
    // operators who really want this: MEDIAGIT_ALLOW_INSECURE_BIND=1.
    if !config.enable_auth
        && !is_loopback_host(&config.host)
        && std::env::var("MEDIAGIT_ALLOW_INSECURE_BIND").as_deref() != Ok("1")
    {
        anyhow::bail!(
            "refusing to start: host '{}' is not loopback-only and `enable_auth` is false \
             in the server config (config keys: enable_auth, host). Either set `enable_auth = true` \
             (and configure `jwt_secret`), bind to 127.0.0.1/localhost, or set \
             MEDIAGIT_ALLOW_INSECURE_BIND=1 to override at your own risk.",
            config.host
        );
    }

    // Create repos directory if it doesn't exist
    std::fs::create_dir_all(&config.repos_dir)?;
    tracing::info!("Repositories directory: {:?}", config.repos_dir);

    // DC-4: build the registry *before* the state, so the same instance backs
    // both the recording side (handlers, via AppState) and the scrape endpoint.
    // It used to be constructed inside the metrics block below and moved
    // straight into the server, unreachable from any handler — which is why
    // `/metrics` served permanent zeros. `None` unless the endpoint is enabled,
    // so the default deployment pays one Option check per request.
    let metrics_registry: Option<mediagit_metrics::MetricsRegistry> =
        if std::env::var("MEDIAGIT_METRICS_ADDR").is_ok() {
            match mediagit_metrics::MetricsRegistry::new() {
                Ok(r) => Some(r),
                Err(e) => {
                    tracing::error!("Failed to create metrics registry: {}", e);
                    None
                }
            }
        } else {
            None
        };

    // Setup shared state with optional authentication
    let state = if config.enable_auth {
        // I2: MEDIAGIT_JWT_SECRET env var wins over the TOML `jwt_secret` key
        // (lets operators keep the secret out of the config file / repo).
        let env_jwt_secret = std::env::var("MEDIAGIT_JWT_SECRET")
            .ok()
            .filter(|s| !s.is_empty());
        if env_jwt_secret.is_some() && config.jwt_secret.is_some() {
            tracing::warn!(
                "Both MEDIAGIT_JWT_SECRET env var and `jwt_secret` in the config file are set; \
                 using the env var"
            );
        }
        let jwt_secret = env_jwt_secret
            .as_deref()
            .or(config.jwt_secret.as_deref())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "JWT secret is required when authentication is enabled \
                     (set `jwt_secret` in the config file or the MEDIAGIT_JWT_SECRET env var)"
                )
            })?;
        tracing::info!("Authentication is ENABLED");
        let auth_store_dir = config.resolved_auth_store_dir();
        let state = Arc::new(attach_metrics(
            AppState::new_with_full_auth(config.repos_dir.clone(), jwt_secret, &auth_store_dir)?
                .with_presigned_ttl(config.presigned_url_ttl_seconds),
            metrics_registry.clone(),
        ));

        // AU-3: warn when the defaults compose into "no tenant isolation".
        //
        // Three separate, individually-defensible backward-compat choices
        // combine badly: registration is open by default, self-registration
        // assigns Role::Write, and per-repo grant enforcement only switches on
        // once at least one grant exists (`check_permission`'s
        // `!grants.is_empty()`). On a freshly provisioned server with no grants
        // recorded, that means anyone who can reach POST /auth/register gets
        // write access to every repository.
        //
        // Not changed silently here: closing registration or defaulting to
        // Role::Read is a breaking change for existing deployments and for the
        // QA drills that self-register. Warning loudly is the honest
        // non-breaking move; the operator can then choose.
        // AU-3: a server with auth on and no admin cannot be administered.
        //
        // Self-registration deliberately creates Read-role accounts and there
        // is no client-controlled `role` field, so an admin can only come from
        // the offline `mediagit-server admin create` bootstrap. Starting
        // without one leaves nobody able to grant access, promote a user, or
        // manage keys — and no in-band way to fix it.
        if let Some(svc) = state.auth_service() {
            let admins = svc
                .credentials_store
                .list_users()
                .await
                .into_iter()
                .filter(|u| u.role == mediagit_security::auth::user::Role::Admin)
                .count();
            if admins == 0 {
                tracing::warn!(
                    "SECURITY: authentication is enabled but NO admin account exists. \
                     Self-registration creates read-only accounts, so nothing can grant \
                     access, promote users or manage API keys. Create one with: \
                     `mediagit-server admin create --username <name> --email <email>`"
                );
            }
        }

        if state.grants.is_empty() && config.allow_open_registration {
            tracing::warn!(
                "SECURITY: no per-repo grants are recorded, so per-repo authorization is \
                 INACTIVE and every authenticated user can read and write EVERY repository. \
                 Registration is also open, and self-registered users receive write \
                 permissions. Record at least one grant to activate per-repo enforcement, \
                 and/or set `allow_open_registration = false` in the server config."
            );
        }

        state
    } else {
        tracing::warn!("Authentication is DISABLED - not suitable for production!");
        Arc::new(attach_metrics(
            AppState::new(config.repos_dir.clone())
                .with_presigned_ttl(config.presigned_url_ttl_seconds),
            metrics_registry.clone(),
        ))
    };

    // I1: startup probe — validate every repo's storage backend construction
    // before we start accepting traffic, so a bad S3/Azure/GCS config surfaces
    // as a boot failure instead of a 500 on a client's first request.
    // MEDIAGIT_STARTUP_PROBE=0 disables it.
    if std::env::var("MEDIAGIT_STARTUP_PROBE").as_deref() != Ok("0") {
        let repo_dirs: Vec<PathBuf> = std::fs::read_dir(&config.repos_dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| p.is_dir())
                    .collect()
            })
            .unwrap_or_default();

        if repo_dirs.is_empty() {
            tracing::debug!(
                "Startup probe: no repos found under {:?}, skipping",
                config.repos_dir
            );
        } else {
            let repo_count = repo_dirs.len();
            tracing::info!(
                "Startup probe: validating storage backend for {} repo(s)...",
                repo_count
            );
            let probe_state = Arc::clone(&state);
            let probe = async move {
                use futures::stream::{self, StreamExt};
                stream::iter(repo_dirs.into_iter().map(|repo_path| {
                    let probe_state = Arc::clone(&probe_state);
                    async move {
                        let result = mediagit_server::handlers::get_or_init_storage(
                            &probe_state,
                            &repo_path,
                        )
                        .await;
                        (repo_path, result)
                    }
                }))
                .buffer_unordered(4)
                .collect::<Vec<_>>()
                .await
            };

            let results = tokio::time::timeout(std::time::Duration::from_secs(30), probe)
                .await
                .map_err(|_| {
                    anyhow::anyhow!(
                        "startup probe timed out after 30s validating {} repo(s); \
                         set MEDIAGIT_STARTUP_PROBE=0 to skip this check",
                        repo_count
                    )
                })?;

            let mut failed = 0;
            for (repo_path, result) in results {
                if let Err(status) = result {
                    failed += 1;
                    tracing::error!(
                        "Startup probe failed for repo {:?}: storage backend init returned {}",
                        repo_path,
                        status
                    );
                }
            }
            if failed > 0 {
                anyhow::bail!(
                    "startup probe failed for {} of {} repo(s); see errors above for details, \
                     or set MEDIAGIT_STARTUP_PROBE=0 to skip this check",
                    failed,
                    repo_count
                );
            }
            tracing::info!("Startup probe passed for {} repo(s)", repo_count);
        }
    }

    // P1-1: optional Prometheus /metrics endpoint on a separate listener, off
    // by default. Set MEDIAGIT_METRICS_ADDR=host:port to enable (e.g.
    // 127.0.0.1:9090). Pure wiring of the existing mediagit-metrics crate —
    // no metric-recording calls are added to request handlers here.
    if let Ok(metrics_addr) = std::env::var("MEDIAGIT_METRICS_ADDR") {
        match metrics_addr.rsplit_once(':') {
            Some((bind_address, port_str)) if port_str.parse::<u16>().is_ok() => {
                let port: u16 = port_str.parse().expect("checked above");
                let bind_address = bind_address.to_string();
                // The *same* registry the handlers record into. Constructing
                // a second one here is what made this endpoint a liar.
                match metrics_registry.clone() {
                    Some(registry) => {
                        let metrics_config = mediagit_metrics::MetricsConfig {
                            port,
                            enabled: true,
                            bind_address,
                        };
                        tracing::info!("Metrics endpoint ENABLED on {}", metrics_addr);
                        let server =
                            mediagit_metrics::MetricsServer::with_config(registry, metrics_config);
                        tokio::spawn(async move {
                            if let Err(e) = server.serve().await {
                                tracing::error!("Metrics server error: {}", e);
                            }
                        });
                    }
                    None => {
                        tracing::error!(
                            "Metrics endpoint requested but the registry failed to build;                              /metrics not started"
                        );
                    }
                }
            }
            _ => {
                tracing::error!(
                    "MEDIAGIT_METRICS_ADDR='{}' is not a valid host:port address; metrics endpoint not started",
                    metrics_addr
                );
            }
        }
    }

    // Build router with optional rate limiting
    let (app, rate_limiter) = if config.enable_rate_limiting {
        tracing::info!(
            "Rate limiting ENABLED: {} req/s, burst {}",
            config.rate_limit_rps,
            config.rate_limit_burst
        );
        let rate_config = RateLimitConfig {
            requests_per_second: config.rate_limit_rps,
            burst_size: config.rate_limit_burst,
        };
        let (router, cleanup, limiter) =
            create_rate_limited_router(Arc::clone(&state), rate_config);

        // Spawn rate limiter cleanup task
        std::thread::spawn(cleanup);

        (router, Some(limiter))
    } else {
        tracing::warn!("Rate limiting is DISABLED - not suitable for production!");
        (create_router(Arc::clone(&state)), None)
    };
    // I4: CORS is off unless `cors_allowed_origins` is set in config.
    let app = mediagit_server::apply_cors_layer(app, config.cors_allowed_origins.as_deref());

    // Start HTTP server (always enabled)
    let http_bind_addr = config.bind_addr();
    tracing::info!("Starting HTTP server on {}", http_bind_addr);

    // If TLS is enabled, start both HTTP and HTTPS servers concurrently.
    // I5: the two `#[cfg(...)]` blocks below (rather than gating the whole
    // if/else on `#[cfg(feature = "tls")]`) exist so a non-tls build with
    // `enable_tls = true` fails loudly at boot instead of silently falling
    // through — previously the entire if/else (HTTP-only branch included)
    // vanished under `#[cfg(feature = "tls")]`, so a non-tls build never
    // bound anything.
    if config.enable_tls {
        #[cfg(not(feature = "tls"))]
        {
            anyhow::bail!(
                "enable_tls=true in configuration but this binary was built without the `tls` \
                 feature; refusing to silently fall back to plain HTTP. Rebuild with \
                 `--features tls` or set `enable_tls = false`."
            );
        }

        #[cfg(feature = "tls")]
        {
            let https_bind_addr = config.tls_bind_addr();
            tracing::info!("Starting HTTPS server on {}", https_bind_addr);

            // Build TLS configuration
            let tls_config = config.build_tls_config()?;
            let certificate = tls_config.load_certificate()?;

            // Build axum-server RustlsConfig from certificate
            let rustls_config = build_axum_rustls_config(&certificate, tls_config.min_tls_version)?;

            // SV-1: the HTTPS listener must carry the same rate limiter as
            // HTTP. It used to call `create_router` unconditionally, so
            // enabling TLS silently disabled rate limiting on the port most
            // likely to face the internet — while startup still logged
            // "Rate limiting ENABLED". The limiter is *shared*, not rebuilt:
            // two independent governors would give an attacker twice the
            // budget simply for splitting traffic across the two ports.
            let https_app = match &rate_limiter {
                Some(limiter) => {
                    create_router_sharing_rate_limit(Arc::clone(&state), Arc::clone(limiter))
                }
                None => create_router(Arc::clone(&state)),
            };
            let https_app = mediagit_server::apply_cors_layer(
                https_app,
                config.cors_allowed_origins.as_deref(),
            );

            // Run both servers concurrently
            tracing::info!(
                "MediaGit server listening on HTTP: {} and HTTPS: {}",
                http_bind_addr,
                https_bind_addr
            );
            tracing::info!("Press Ctrl+C to stop");

            // Spawn HTTP server task
            let http_server = tokio::spawn(async move {
                let listener = tokio::net::TcpListener::bind(&http_bind_addr).await?;
                // ConnectInfo must be supplied or SmartIpKeyExtractor (rate limiting)
                // 500s with "Unable to extract key!" on every request.
                axum::serve(
                    listener,
                    app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
                )
                .with_graceful_shutdown(shutdown_signal())
                .await
            });

            // Spawn HTTPS server task
            let https_handle = axum_server::Handle::new();
            let https_shutdown_handle = https_handle.clone();
            tokio::spawn(async move {
                shutdown_signal().await;
                https_shutdown_handle.graceful_shutdown(Some(std::time::Duration::from_secs(30)));
            });
            let https_server = tokio::spawn(async move {
                let addr: std::net::SocketAddr = https_bind_addr
                    .parse()
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
                axum_server::bind_rustls(addr, rustls_config)
                    .handle(https_handle)
                    .serve(https_app.into_make_service())
                    .await
            });

            // Wait for both servers (or either to fail)
            tokio::select! {
                result = http_server => {
                    result??;
                }
                result = https_server => {
                    result??;
                }
            }
        }
    } else {
        // HTTP only mode
        tracing::info!("MediaGit server listening on {}", http_bind_addr);
        tracing::info!("Press Ctrl+C to stop");

        let listener = tokio::net::TcpListener::bind(&http_bind_addr).await?;
        // ConnectInfo must be supplied or SmartIpKeyExtractor (rate limiting)
        // 500s with "Unable to extract key!" on every request.
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    }

    Ok(())
}

/// Check whether `host` only ever resolves to the local machine (loopback).
/// Used to gate the P0-4 insecure-bind refusal: a loopback bind with auth
/// disabled is still local-only and safe for dev; anything else (0.0.0.0, a
/// real interface IP, or a hostname) with auth off is an open server.
pub(crate) fn is_loopback_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

/// Resolves when the process receives Ctrl+C (all platforms) or SIGTERM
/// (unix only). Used to drive `.with_graceful_shutdown(...)` so in-flight
/// requests (e.g. a multi-GB chunk upload) finish instead of being hard-cut.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("Received Ctrl+C, starting graceful shutdown"),
        _ = terminate => tracing::info!("Received SIGTERM, starting graceful shutdown"),
    }
}

/// Build axum-server RustlsConfig from Certificate
#[cfg(feature = "tls")]
fn build_axum_rustls_config(
    certificate: &mediagit_security::Certificate,
    min_version: mediagit_security::TlsVersion,
) -> Result<axum_server::tls_rustls::RustlsConfig> {
    use axum_server::tls_rustls::RustlsConfig;
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};

    // Parse certificate PEM
    let cert_pem = certificate.cert_pem.as_bytes();
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(cert_pem)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| anyhow::anyhow!("Failed to parse certificate: {}", e))?;

    // Parse private key PEM (handles PKCS#8, RSA PKCS#1, and EC SEC1 automatically)
    let key_pem = certificate.key_pem.as_bytes();
    let private_key: PrivateKeyDer<'static> = PrivateKeyDer::from_pem_slice(key_pem)
        .map_err(|e| anyhow::anyhow!("Failed to parse private key: {}", e))?;

    // DC-9: honour `min_tls_version` instead of taking rustls's defaults.
    //
    // `TlsConfig::min_tls_version` defaults to 1.3 and four docs promised "TLS
    // 1.3 for all network operations", but this function built a fresh
    // `ServerConfig::builder()` and never consulted the setting — and rustls's
    // default accepts **1.2 as well**. An operator reading the config believed
    // 1.3-only while the server happily negotiated 1.2: a silent downgrade,
    // which is worse than an honest 1.2 because nobody goes looking.
    let versions: &[&rustls::SupportedProtocolVersion] = match min_version {
        mediagit_security::TlsVersion::V1_3 => &[&rustls::version::TLS13],
        mediagit_security::TlsVersion::V1_2 => &[&rustls::version::TLS12, &rustls::version::TLS13],
    };
    tracing::info!("TLS minimum version: {:?}", min_version);

    // Build rustls ServerConfig with ALPN to enable HTTP/2 negotiation.
    //
    // `with_no_client_auth` is deliberate and load-bearing: mTLS
    // (`TlsConfig::client_ca_path` / `require_client_cert`) is **not wired**,
    // and there is no operator-facing knob for it in `mediagit-server.toml`.
    // Reading those fields here without a way to set them would be theatre.
    let mut rustls_config = rustls::ServerConfig::builder_with_protocol_versions(versions)
        .with_no_client_auth()
        .with_single_cert(certs, private_key)
        .map_err(|e| anyhow::anyhow!("Failed to build TLS config: {}", e))?;
    rustls_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

    // Convert to axum-server RustlsConfig
    Ok(RustlsConfig::from_config(Arc::new(rustls_config)))
}

/// Attach the metrics registry to state when `/metrics` is enabled.
///
/// A free function rather than inlining `.with_metrics()` at both construction
/// sites: `with_metrics` takes a registry, not an `Option`, so each call site
/// would otherwise need its own `if let` around an already-long expression.
fn attach_metrics(
    state: mediagit_server::AppState,
    registry: Option<mediagit_metrics::MetricsRegistry>,
) -> mediagit_server::AppState {
    match registry {
        Some(r) => state.with_metrics(r),
        None => state,
    }
}

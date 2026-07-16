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
    create_router, create_router_with_rate_limit, AppState, RateLimitConfig, ServerConfig,
};

/// MediaGit Server - HTTP(S) server for MediaGit repositories
#[derive(Parser, Debug)]
#[command(name = "mediagit-server")]
#[command(about = "MediaGit repository server", long_about = None)]
struct Args {
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

    // Setup shared state with optional authentication
    let state = if config.enable_auth {
        let jwt_secret = config.jwt_secret.as_deref().ok_or_else(|| {
            anyhow::anyhow!("JWT secret is required when authentication is enabled")
        })?;
        tracing::info!("Authentication is ENABLED");
        Arc::new(
            AppState::new_with_full_auth(config.repos_dir.clone(), jwt_secret)
                .with_presigned_ttl(config.presigned_url_ttl_seconds),
        )
    } else {
        tracing::warn!("Authentication is DISABLED - not suitable for production!");
        Arc::new(
            AppState::new(config.repos_dir.clone())
                .with_presigned_ttl(config.presigned_url_ttl_seconds),
        )
    };

    // P1-1: optional Prometheus /metrics endpoint on a separate listener, off
    // by default. Set MEDIAGIT_METRICS_ADDR=host:port to enable (e.g.
    // 127.0.0.1:9090). Pure wiring of the existing mediagit-metrics crate —
    // no metric-recording calls are added to request handlers here.
    if let Ok(metrics_addr) = std::env::var("MEDIAGIT_METRICS_ADDR") {
        match metrics_addr.rsplit_once(':') {
            Some((bind_address, port_str)) if port_str.parse::<u16>().is_ok() => {
                let port: u16 = port_str.parse().expect("checked above");
                let bind_address = bind_address.to_string();
                match mediagit_metrics::MetricsRegistry::new() {
                    Ok(registry) => {
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
                    Err(e) => {
                        tracing::error!("Failed to create metrics registry: {}", e);
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
    let (app, _cleanup_task) = if config.enable_rate_limiting {
        tracing::info!(
            "Rate limiting ENABLED: {} req/s, burst {}",
            config.rate_limit_rps,
            config.rate_limit_burst
        );
        let rate_config = RateLimitConfig {
            requests_per_second: config.rate_limit_rps,
            burst_size: config.rate_limit_burst,
        };
        let (router, cleanup) = create_router_with_rate_limit(Arc::clone(&state), rate_config);

        // Spawn rate limiter cleanup task
        std::thread::spawn(cleanup);

        (router, true)
    } else {
        tracing::warn!("Rate limiting is DISABLED - not suitable for production!");
        (create_router(Arc::clone(&state)), false)
    };

    // Start HTTP server (always enabled)
    let http_bind_addr = config.bind_addr();
    tracing::info!("Starting HTTP server on {}", http_bind_addr);

    // If TLS is enabled, start both HTTP and HTTPS servers concurrently
    #[cfg(feature = "tls")]
    if config.enable_tls {
        let https_bind_addr = config.tls_bind_addr();
        tracing::info!("Starting HTTPS server on {}", https_bind_addr);

        // Build TLS configuration
        let tls_config = config.build_tls_config()?;
        let certificate = tls_config.load_certificate()?;

        // Build axum-server RustlsConfig from certificate
        let rustls_config = build_axum_rustls_config(&certificate)?;

        // Create HTTPS app (clone of router)
        let https_app = create_router(Arc::clone(&state));

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
            axum::serve(listener, app)
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
    } else {
        // HTTP only mode
        tracing::info!("MediaGit server listening on {}", http_bind_addr);
        tracing::info!("Press Ctrl+C to stop");

        let listener = tokio::net::TcpListener::bind(&http_bind_addr).await?;
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal())
            .await?;
    }

    Ok(())
}

/// Check whether `host` only ever resolves to the local machine (loopback).
/// Used to gate the P0-4 insecure-bind refusal: a loopback bind with auth
/// disabled is still local-only and safe for dev; anything else (0.0.0.0, a
/// real interface IP, or a hostname) with auth off is an open server.
fn is_loopback_host(host: &str) -> bool {
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

    // Build rustls ServerConfig with ALPN to enable HTTP/2 negotiation
    let mut rustls_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, private_key)
        .map_err(|e| anyhow::anyhow!("Failed to build TLS config: {}", e))?;
    rustls_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

    // Convert to axum-server RustlsConfig
    Ok(RustlsConfig::from_config(Arc::new(rustls_config)))
}

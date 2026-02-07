use anyhow::Result;
use clap::Parser;
use std::sync::Arc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use mediagit_server::{create_router, create_router_with_rate_limit, AppState, RateLimitConfig, ServerConfig};

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

    /// Path to config file
    #[arg(short, long, default_value = "mediagit-server.toml")]
    config: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse CLI arguments
    let args = Args::parse();

    // Setup tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "mediagit_server=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load configuration from file
    let mut config = ServerConfig::load()?;
    
    // Override config with CLI arguments if provided
    if let Some(port) = args.port {
        tracing::info!("Overriding port from CLI: {} -> {}", config.port, port);
        config.port = port;
    }
    if let Some(host) = args.host {
        tracing::info!("Overriding host from CLI: {} -> {}", config.host, host);
        config.host = host;
    }
    
    tracing::info!("Server configuration: {:?}", config);

    // Create repos directory if it doesn't exist
    std::fs::create_dir_all(&config.repos_dir)?;
    tracing::info!("Repositories directory: {:?}", config.repos_dir);

    // Setup shared state with optional authentication
    let state = if config.enable_auth {
        let jwt_secret = config.jwt_secret.as_deref()
            .ok_or_else(|| anyhow::anyhow!("JWT secret is required when authentication is enabled"))?;
        tracing::info!("Authentication is ENABLED");
        Arc::new(AppState::new_with_full_auth(config.repos_dir.clone(), jwt_secret))
    } else {
        tracing::warn!("Authentication is DISABLED - not suitable for production!");
        Arc::new(AppState::new(config.repos_dir.clone()))
    };

    // Build router with optional rate limiting
    let (app, _cleanup_task) = if config.enable_rate_limiting {
        tracing::info!("Rate limiting ENABLED: {} req/s, burst {}",
                      config.rate_limit_rps, config.rate_limit_burst);
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
        tracing::info!("MediaGit server listening on HTTP: {} and HTTPS: {}",
                      http_bind_addr, https_bind_addr);
        tracing::info!("Press Ctrl+C to stop");

        // Spawn HTTP server task
        let http_server = tokio::spawn(async move {
            let listener = tokio::net::TcpListener::bind(&http_bind_addr).await?;
            axum::serve(listener, app).await
        });

        // Spawn HTTPS server task
        let https_server = tokio::spawn(async move {
            let addr = https_bind_addr.parse()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
            axum_server::bind_rustls(addr, rustls_config)
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
        axum::serve(listener, app).await?;
    }

    Ok(())
}

/// Build axum-server RustlsConfig from Certificate
#[cfg(feature = "tls")]
fn build_axum_rustls_config(
    certificate: &mediagit_security::Certificate,
) -> Result<axum_server::tls_rustls::RustlsConfig> {
    use axum_server::tls_rustls::RustlsConfig;

    // axum-server 0.7 requires rustls 0.23
    // Build rustls ServerConfig first
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};

    // Parse certificate PEM
    let cert_pem = certificate.cert_pem.as_bytes();
    let certs: Vec<CertificateDer> = rustls_pemfile::certs(&mut &cert_pem[..])
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| anyhow::anyhow!("Failed to parse certificate: {}", e))?;

    // Parse private key PEM
    let key_pem = certificate.key_pem.as_bytes();
    let mut key_reader = &key_pem[..];

    // Try to read as PKCS#8 first, then RSA
    let private_key = if let Ok(key) = rustls_pemfile::pkcs8_private_keys(&mut key_reader)
        .collect::<Result<Vec<_>, _>>()
    {
        if let Some(key) = key.into_iter().next() {
            PrivateKeyDer::Pkcs8(key)
        } else {
            // Reset reader and try RSA
            key_reader = &key_pem[..];
            let rsa_keys = rustls_pemfile::rsa_private_keys(&mut key_reader)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| anyhow::anyhow!("Failed to parse private key: {}", e))?;

            PrivateKeyDer::Pkcs1(
                rsa_keys.into_iter().next()
                    .ok_or_else(|| anyhow::anyhow!("No private key found in PEM"))?
            )
        }
    } else {
        anyhow::bail!("Failed to parse private key");
    };

    // Build rustls ServerConfig
    let rustls_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, private_key)
        .map_err(|e| anyhow::anyhow!("Failed to build TLS config: {}", e))?;

    // Convert to axum-server RustlsConfig
    Ok(RustlsConfig::from_config(Arc::new(rustls_config)))
}

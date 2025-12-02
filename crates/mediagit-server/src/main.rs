use anyhow::Result;
use std::sync::Arc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use mediagit_server::{create_router, AppState, ServerConfig};

#[tokio::main]
async fn main() -> Result<()> {
    // Setup tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "mediagit_server=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load configuration
    let config = ServerConfig::load()?;
    tracing::info!("Server configuration: {:?}", config);

    // Create repos directory if it doesn't exist
    std::fs::create_dir_all(&config.repos_dir)?;
    tracing::info!("Repositories directory: {:?}", config.repos_dir);

    // Setup shared state
    let state = Arc::new(AppState::new(config.repos_dir.clone()));

    // Build router using library function
    let app = create_router(Arc::clone(&state));

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

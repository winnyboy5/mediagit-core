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
    let app = create_router(state);

    // Start server
    let bind_addr = config.bind_addr();
    tracing::info!("MediaGit server listening on {}", bind_addr);
    tracing::info!("Press Ctrl+C to stop");

    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

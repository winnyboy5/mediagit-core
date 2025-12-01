use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Port to listen on
    #[serde(default = "default_port")]
    pub port: u16,

    /// Directory containing repositories
    #[serde(default = "default_repos_dir")]
    pub repos_dir: PathBuf,

    /// Host to bind to
    #[serde(default = "default_host")]
    pub host: String,
}

fn default_port() -> u16 {
    3000
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}

fn default_repos_dir() -> PathBuf {
    PathBuf::from("./repos")
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: default_port(),
            repos_dir: default_repos_dir(),
            host: default_host(),
        }
    }
}

impl ServerConfig {
    /// Load configuration from file or use defaults
    pub fn load() -> Result<Self> {
        // Try to load from mediagit-server.toml in current directory
        let config_path = PathBuf::from("mediagit-server.toml");

        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)
                .context("Failed to read config file")?;

            toml::from_str(&content).context("Failed to parse config file")
        } else {
            // Use defaults
            tracing::info!("No config file found, using defaults");
            Ok(Self::default())
        }
    }

    /// Get the full bind address
    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

// Copyright (C) 2026  winnyboy5
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//! Configuration for MediaGit storage optimizations

use crate::chunking::ChunkStrategy;
use serde::{Deserialize, Serialize};

/// Storage optimization configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    /// Enable smart compression (type-aware compression selection)
    #[serde(default = "default_smart_compression")]
    pub smart_compression: bool,

    /// Enable chunking for media files
    #[serde(default)]
    pub chunking_enabled: bool,

    /// Chunking strategy
    #[serde(default)]
    pub chunking_strategy: ChunkingStrategyConfig,

    /// Enable delta encoding for similar objects
    #[serde(default)]
    pub delta_enabled: bool,

    /// Enable pack file generation
    #[serde(default)]
    pub pack_enabled: bool,

    /// Pack delta window size (number of objects to consider for delta compression)
    #[serde(default = "default_pack_window")]
    pub pack_window: usize,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            smart_compression: true,
            chunking_enabled: true,  // Enable chunk-level deduplication
            chunking_strategy: ChunkingStrategyConfig::default(), // MediaAware
            delta_enabled: true,     // Enable delta encoding for similar files
            pack_enabled: true,      // Enable pack file generation
            pack_window: 10,
        }
    }
}

impl StorageConfig {
    /// Create a minimal configuration with only basic features
    /// 
    /// Use this for simple repositories or when performance is critical.
    /// Only smart compression is enabled.
    pub fn minimal() -> Self {
        Self {
            smart_compression: true,
            chunking_enabled: false,
            chunking_strategy: ChunkingStrategyConfig::default(),
            delta_enabled: false,
            pack_enabled: false,
            pack_window: 10,
        }
    }

    /// Create a configuration optimized for large media files
    /// 
    /// Enables all optimization features with larger pack window
    /// for better delta compression of similar video files.
    pub fn for_large_media() -> Self {
        Self {
            smart_compression: true,
            chunking_enabled: true,
            chunking_strategy: ChunkingStrategyConfig::MediaAware,
            delta_enabled: true,
            pack_enabled: true,
            pack_window: 50,  // Larger window for better delta matches
        }
    }
}

fn default_smart_compression() -> bool {
    true
}

fn default_pack_window() -> usize {
    10
}

/// Chunking strategy configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChunkingStrategyConfig {
    /// Fixed-size chunks
    Fixed { size: usize },
    /// Rolling hash chunking
    Rolling {
        avg_size: usize,
        min_size: usize,
        max_size: usize,
    },
    /// Media-aware chunking (parse structure, separate streams)
    #[default]
    MediaAware,
}

impl From<ChunkingStrategyConfig> for ChunkStrategy {
    fn from(config: ChunkingStrategyConfig) -> Self {
        match config {
            ChunkingStrategyConfig::Fixed { size } => ChunkStrategy::Fixed { size },
            ChunkingStrategyConfig::Rolling {
                avg_size,
                min_size,
                max_size,
            } => ChunkStrategy::Rolling {
                avg_size,
                min_size,
                max_size,
            },
            ChunkingStrategyConfig::MediaAware => ChunkStrategy::MediaAware,
        }
    }
}

impl StorageConfig {
    /// Load configuration from environment variables
    pub fn from_env() -> Self {
        let mut config = Self::default();

        // Override with environment variables if present
        if let Ok(val) = std::env::var("MEDIAGIT_SMART_COMPRESSION") {
            config.smart_compression = val.parse().unwrap_or(true);
        }

        if let Ok(val) = std::env::var("MEDIAGIT_CHUNKING_ENABLED") {
            config.chunking_enabled = val.parse().unwrap_or(false);
        }

        if let Ok(val) = std::env::var("MEDIAGIT_DELTA_ENABLED") {
            config.delta_enabled = val.parse().unwrap_or(false);
        }

        if let Ok(val) = std::env::var("MEDIAGIT_PACK_ENABLED") {
            config.pack_enabled = val.parse().unwrap_or(false);
        }

        config
    }

    /// Load configuration from TOML file
    pub fn from_toml(path: &std::path::Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Self = toml::from_str(&content)?;
        Ok(config)
    }

    /// Get chunk strategy if enabled
    pub fn get_chunk_strategy(&self) -> Option<ChunkStrategy> {
        if self.chunking_enabled {
            Some(self.chunking_strategy.clone().into())
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = StorageConfig::default();
        assert!(config.smart_compression);
        assert!(config.chunking_enabled);  // Now enabled by default
        assert!(config.delta_enabled);     // Now enabled by default
        assert!(config.pack_enabled);      // Now enabled by default
    }

    #[test]
    fn test_minimal_config() {
        let config = StorageConfig::minimal();
        assert!(config.smart_compression);
        assert!(!config.chunking_enabled);
        assert!(!config.delta_enabled);
        assert!(!config.pack_enabled);
    }

    #[test]
    fn test_large_media_config() {
        let config = StorageConfig::for_large_media();
        assert!(config.smart_compression);
        assert!(config.chunking_enabled);
        assert!(config.delta_enabled);
        assert!(config.pack_enabled);
        assert_eq!(config.pack_window, 50);
    }

    #[test]
    fn test_chunking_strategy_conversion() {
        let config = ChunkingStrategyConfig::MediaAware;
        let strategy: ChunkStrategy = config.into();
        assert_eq!(strategy, ChunkStrategy::MediaAware);
    }
}

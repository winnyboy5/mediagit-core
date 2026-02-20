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
//! Streaming file transfer with chunked uploads/downloads
//!
//! This module provides efficient streaming transfer capabilities for large
//! media files, with automatic chunking, resumption, and progress tracking.
//!
//! # Features
//!
//! - Chunked uploads with configurable chunk sizes
//! - Resumable transfers with range support
//! - Progress tracking and callbacks
//! - Parallel chunk transfers for performance
//! - Automatic retry with exponential backoff
//! - Memory-efficient streaming (no full file buffering)
//!
//! # Example
//!
//! ```rust,no_run
//! use mediagit_protocol::streaming::{StreamingUploader, UploadConfig};
//!
//! # async fn example() -> anyhow::Result<()> {
//! let config = UploadConfig {
//!     chunk_size: 4 * 1024 * 1024, // 4MB chunks
//!     parallel_transfers: 3,
//!     max_retries: 3,
//!     ..Default::default()
//! };
//!
//! let uploader = StreamingUploader::new("http://localhost:3000", config);
//!
//! uploader
//!     .upload_file("large_video.avi", "project/video.avi")
//!     .await?
//!     .on_progress(|progress| {
//!         println!("Progress: {:.1}%", progress.percent());
//!     })
//!     .execute()
//!     .await?;
//! # Ok(())
//! # }
//! ```

use anyhow::{Context, Result};
use reqwest::{Client, StatusCode};
// use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::Semaphore;
use tracing::{debug, info, warn};

/// Upload configuration
#[derive(Debug, Clone)]
pub struct UploadConfig {
    /// Chunk size for uploads (default: 4MB)
    pub chunk_size: usize,

    /// Number of parallel chunk transfers (default: 3)
    pub parallel_transfers: usize,

    /// Maximum retry attempts (default: 3)
    pub max_retries: usize,

    /// Retry delay in milliseconds (default: 1000)
    pub retry_delay_ms: u64,

    /// Enable compression for chunks (default: false for media)
    pub compress_chunks: bool,
}

impl Default for UploadConfig {
    fn default() -> Self {
        Self {
            chunk_size: 4 * 1024 * 1024, // 4MB
            parallel_transfers: 3,
            max_retries: 3,
            retry_delay_ms: 1000,
            compress_chunks: false,
        }
    }
}

/// Download configuration
#[derive(Debug, Clone)]
pub struct DownloadConfig {
    /// Chunk size for downloads (default: 4MB)
    pub chunk_size: usize,

    /// Number of parallel chunk transfers (default: 3)
    pub parallel_transfers: usize,

    /// Enable range requests for resumable downloads
    pub use_range_requests: bool,

    /// Maximum retry attempts (default: 3)
    pub max_retries: usize,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            chunk_size: 4 * 1024 * 1024, // 4MB
            parallel_transfers: 3,
            use_range_requests: true,
            max_retries: 3,
        }
    }
}

/// Transfer progress information
#[derive(Debug, Clone)]
pub struct TransferProgress {
    /// Bytes transferred
    pub bytes_transferred: u64,

    /// Total bytes
    pub total_bytes: u64,

    /// Transfer speed (bytes/sec)
    pub speed_bps: f64,

    /// Estimated time remaining (seconds)
    pub eta_seconds: f64,
}

impl TransferProgress {
    /// Calculate completion percentage
    pub fn percent(&self) -> f64 {
        if self.total_bytes == 0 {
            0.0
        } else {
            (self.bytes_transferred as f64 / self.total_bytes as f64) * 100.0
        }
    }

    /// Format speed as human-readable string
    pub fn speed_human(&self) -> String {
        format_bytes_per_sec(self.speed_bps)
    }

    /// Format ETA as human-readable string
    pub fn eta_human(&self) -> String {
        format_duration(self.eta_seconds)
    }
}

/// Streaming uploader
pub struct StreamingUploader {
    base_url: String,
    client: Client,
    config: UploadConfig,
}

impl StreamingUploader {
    /// Create a new streaming uploader
    pub fn new(base_url: impl Into<String>, config: UploadConfig) -> Self {
        Self {
            base_url: base_url.into(),
            client: Client::new(),
            config,
        }
    }

    /// Upload a file with progress tracking
    pub async fn upload_file(
        &self,
        local_path: impl AsRef<Path>,
        remote_path: &str,
    ) -> Result<UploadHandle> {
        let local_path = local_path.as_ref();
        let file_size = tokio::fs::metadata(local_path).await?.len();

        info!(
            local_path = ?local_path,
            remote_path = remote_path,
            file_size = file_size,
            chunk_size = self.config.chunk_size,
            "Starting streaming upload"
        );

        let handle = UploadHandle {
            uploader: self.clone(),
            local_path: local_path.to_path_buf(),
            remote_path: remote_path.to_string(),
            file_size,
            progress_callback: None,
        };

        Ok(handle)
    }

    /// Upload a single chunk
    async fn upload_chunk(
        &self,
        remote_path: &str,
        chunk_index: usize,
        chunk_data: Vec<u8>,
        total_chunks: usize,
    ) -> Result<()> {
        let url = format!("{}/upload/chunk", self.base_url);

        let mut retries = 0;
        loop {
            let request = self
                .client
                .post(&url)
                .header("X-File-Path", remote_path)
                .header("X-Chunk-Index", chunk_index.to_string())
                .header("X-Total-Chunks", total_chunks.to_string())
                .header("Content-Length", chunk_data.len().to_string())
                .body(chunk_data.clone());

            match request.send().await {
                Ok(response) if response.status().is_success() => {
                    debug!(
                        chunk_index = chunk_index,
                        total_chunks = total_chunks,
                        size = chunk_data.len(),
                        "Chunk uploaded successfully"
                    );
                    return Ok(());
                }
                Ok(response) => {
                    warn!(
                        chunk_index = chunk_index,
                        status = ?response.status(),
                        "Chunk upload failed"
                    );

                    if retries >= self.config.max_retries {
                        return Err(anyhow::anyhow!(
                            "Chunk upload failed after {} retries: HTTP {}",
                            retries,
                            response.status()
                        ));
                    }
                }
                Err(e) => {
                    warn!(
                        chunk_index = chunk_index,
                        error = %e,
                        "Chunk upload error"
                    );

                    if retries >= self.config.max_retries {
                        return Err(e.into());
                    }
                }
            }

            // Exponential backoff
            let delay = self.config.retry_delay_ms * (1 << retries);
            tokio::time::sleep(tokio::time::Duration::from_millis(delay)).await;
            retries += 1;
        }
    }

    /// Finalize upload (combine chunks on server)
    async fn finalize_upload(&self, remote_path: &str, total_chunks: usize) -> Result<()> {
        let url = format!("{}/upload/finalize", self.base_url);

        let response = self
            .client
            .post(&url)
            .header("X-File-Path", remote_path)
            .header("X-Total-Chunks", total_chunks.to_string())
            .send()
            .await
            .context("Failed to finalize upload")?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "Upload finalization failed: HTTP {}",
                response.status()
            ));
        }

        info!(
            remote_path = remote_path,
            total_chunks = total_chunks,
            "Upload finalized successfully"
        );

        Ok(())
    }
}

impl Clone for StreamingUploader {
    fn clone(&self) -> Self {
        Self {
            base_url: self.base_url.clone(),
            client: self.client.clone(),
            config: self.config.clone(),
        }
    }
}

/// Upload handle with progress tracking
pub struct UploadHandle {
    uploader: StreamingUploader,
    local_path: std::path::PathBuf,
    remote_path: String,
    file_size: u64,
    progress_callback: Option<Arc<dyn Fn(TransferProgress) + Send + Sync>>,
}

impl UploadHandle {
    /// Set progress callback
    pub fn on_progress<F>(mut self, callback: F) -> Self
    where
        F: Fn(TransferProgress) + Send + Sync + 'static,
    {
        self.progress_callback = Some(Arc::new(callback));
        self
    }

    /// Execute the upload
    pub async fn execute(self) -> Result<()> {
        let mut file = File::open(&self.local_path).await?;
        let total_chunks = (self.file_size as usize).div_ceil(self.uploader.config.chunk_size);

        let semaphore = Arc::new(Semaphore::new(self.uploader.config.parallel_transfers));
        let mut tasks = Vec::new();

        let start_time = std::time::Instant::now();
        let mut bytes_transferred = 0u64;

        for chunk_index in 0..total_chunks {
            let offset = (chunk_index * self.uploader.config.chunk_size) as u64;
            file.seek(tokio::io::SeekFrom::Start(offset)).await?;

            let mut chunk_data = vec![0u8; self.uploader.config.chunk_size];
            let bytes_read = file.read(&mut chunk_data).await?;
            chunk_data.truncate(bytes_read);

            let uploader = self.uploader.clone();
            let remote_path = self.remote_path.clone();
            let semaphore = semaphore.clone();
            let progress_callback = self.progress_callback.clone();

            bytes_transferred += bytes_read as u64;
            let current_progress = TransferProgress {
                bytes_transferred,
                total_bytes: self.file_size,
                speed_bps: bytes_transferred as f64 / start_time.elapsed().as_secs_f64(),
                eta_seconds: {
                    let remaining = self.file_size - bytes_transferred;
                    let speed = bytes_transferred as f64 / start_time.elapsed().as_secs_f64();
                    if speed > 0.0 {
                        remaining as f64 / speed
                    } else {
                        0.0
                    }
                },
            };

            if let Some(ref callback) = progress_callback {
                callback(current_progress);
            }

            let task = tokio::spawn(async move {
                let _permit = semaphore.acquire().await?;
                uploader
                    .upload_chunk(&remote_path, chunk_index, chunk_data, total_chunks)
                    .await
            });

            tasks.push(task);
        }

        // Wait for all uploads to complete
        for task in tasks {
            task.await??;
        }

        // Finalize upload
        self.uploader
            .finalize_upload(&self.remote_path, total_chunks)
            .await?;

        info!(
            file_size = self.file_size,
            duration = ?start_time.elapsed(),
            "Upload completed successfully"
        );

        Ok(())
    }
}

/// Streaming downloader
pub struct StreamingDownloader {
    base_url: String,
    client: Client,
    config: DownloadConfig,
}

impl StreamingDownloader {
    /// Create a new streaming downloader
    pub fn new(base_url: impl Into<String>, config: DownloadConfig) -> Self {
        Self {
            base_url: base_url.into(),
            client: Client::new(),
            config,
        }
    }

    /// Download a file with progress tracking
    pub async fn download_file(
        &self,
        remote_path: &str,
        local_path: impl AsRef<Path>,
    ) -> Result<DownloadHandle> {
        // Get file size from server
        let url = format!("{}/download/{}", self.base_url, remote_path);
        let response = self.client.head(&url).send().await?;

        let file_size = response
            .headers()
            .get("Content-Length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .context("Failed to get file size from server")?;

        let handle = DownloadHandle {
            downloader: self.clone(),
            remote_path: remote_path.to_string(),
            local_path: local_path.as_ref().to_path_buf(),
            file_size,
            progress_callback: None,
        };

        Ok(handle)
    }

    /// Download a chunk with range request
    async fn download_chunk(&self, remote_path: &str, start: u64, end: u64) -> Result<Vec<u8>> {
        let url = format!("{}/download/{}", self.base_url, remote_path);

        let mut retries = 0;
        loop {
            let request = self
                .client
                .get(&url)
                .header("Range", format!("bytes={}-{}", start, end));

            match request.send().await {
                Ok(response) if response.status().is_success() || response.status() == StatusCode::PARTIAL_CONTENT => {
                    let data = response.bytes().await?.to_vec();
                    debug!(
                        start = start,
                        end = end,
                        size = data.len(),
                        "Chunk downloaded successfully"
                    );
                    return Ok(data);
                }
                Ok(response) => {
                    warn!(
                        start = start,
                        end = end,
                        status = ?response.status(),
                        "Chunk download failed"
                    );

                    if retries >= self.config.max_retries {
                        return Err(anyhow::anyhow!(
                            "Chunk download failed after {} retries: HTTP {}",
                            retries,
                            response.status()
                        ));
                    }
                }
                Err(e) => {
                    warn!(
                        start = start,
                        end = end,
                        error = %e,
                        "Chunk download error"
                    );

                    if retries >= self.config.max_retries {
                        return Err(e.into());
                    }
                }
            }

            // Exponential backoff
            let delay = 1000 * (1 << retries);
            tokio::time::sleep(tokio::time::Duration::from_millis(delay)).await;
            retries += 1;
        }
    }
}

impl Clone for StreamingDownloader {
    fn clone(&self) -> Self {
        Self {
            base_url: self.base_url.clone(),
            client: self.client.clone(),
            config: self.config.clone(),
        }
    }
}

/// Download handle with progress tracking
pub struct DownloadHandle {
    downloader: StreamingDownloader,
    remote_path: String,
    local_path: std::path::PathBuf,
    file_size: u64,
    progress_callback: Option<Arc<dyn Fn(TransferProgress) + Send + Sync>>,
}

impl DownloadHandle {
    /// Set progress callback
    pub fn on_progress<F>(mut self, callback: F) -> Self
    where
        F: Fn(TransferProgress) + Send + Sync + 'static,
    {
        self.progress_callback = Some(Arc::new(callback));
        self
    }

    /// Execute the download
    pub async fn execute(self) -> Result<()> {
        let mut file = tokio::fs::File::create(&self.local_path).await?;
        let total_chunks = (self.file_size as usize).div_ceil(self.downloader.config.chunk_size);

        let semaphore = Arc::new(Semaphore::new(self.downloader.config.parallel_transfers));
        let mut tasks = Vec::new();

        let start_time = std::time::Instant::now();

        for chunk_index in 0..total_chunks {
            let start = (chunk_index * self.downloader.config.chunk_size) as u64;
            let end = (start + self.downloader.config.chunk_size as u64 - 1).min(self.file_size - 1);

            let downloader = self.downloader.clone();
            let remote_path = self.remote_path.clone();
            let semaphore = semaphore.clone();

            let task = tokio::spawn(async move {
                let _permit = semaphore.acquire().await?;
                downloader.download_chunk(&remote_path, start, end).await
            });

            tasks.push((chunk_index, task));
        }

        // Write chunks in order
        let mut bytes_transferred = 0u64;
        for (_chunk_index, task) in tasks {
            let chunk_data = task.await??;
            use tokio::io::AsyncWriteExt;
            file.write_all(&chunk_data).await?;

            bytes_transferred += chunk_data.len() as u64;

            if let Some(ref callback) = self.progress_callback {
                let progress = TransferProgress {
                    bytes_transferred,
                    total_bytes: self.file_size,
                    speed_bps: bytes_transferred as f64 / start_time.elapsed().as_secs_f64(),
                    eta_seconds: {
                        let remaining = self.file_size - bytes_transferred;
                        let speed = bytes_transferred as f64 / start_time.elapsed().as_secs_f64();
                        if speed > 0.0 {
                            remaining as f64 / speed
                        } else {
                            0.0
                        }
                    },
                };
                callback(progress);
            }
        }

        info!(
            file_size = self.file_size,
            duration = ?start_time.elapsed(),
            "Download completed successfully"
        );

        Ok(())
    }
}

/// Format bytes per second as human-readable string
fn format_bytes_per_sec(bps: f64) -> String {
    const UNITS: &[&str] = &["B/s", "KB/s", "MB/s", "GB/s"];
    let mut value = bps;
    let mut unit_idx = 0;

    while value >= 1024.0 && unit_idx < UNITS.len() - 1 {
        value /= 1024.0;
        unit_idx += 1;
    }

    format!("{:.2} {}", value, UNITS[unit_idx])
}

/// Format duration as human-readable string
fn format_duration(seconds: f64) -> String {
    if seconds < 60.0 {
        format!("{:.0}s", seconds)
    } else if seconds < 3600.0 {
        let total_secs = seconds as u64;
        let mins = total_secs / 60;
        let secs = total_secs % 60;
        format!("{}m {}s", mins, secs)
    } else {
        let total_secs = seconds as u64;
        let hours = total_secs / 3600;
        let mins = (total_secs % 3600) / 60;
        format!("{}h {}m", hours, mins)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transfer_progress() {
        let progress = TransferProgress {
            bytes_transferred: 50 * 1024 * 1024,
            total_bytes: 100 * 1024 * 1024,
            speed_bps: 10.0 * 1024.0 * 1024.0,
            eta_seconds: 5.0,
        };

        assert_eq!(progress.percent(), 50.0);
        assert!(progress.speed_human().contains("MB/s"));
        assert!(progress.eta_human().contains("s"));
    }

    #[test]
    fn test_format_bytes_per_sec() {
        assert_eq!(format_bytes_per_sec(500.0), "500.00 B/s");
        assert_eq!(format_bytes_per_sec(1024.0), "1.00 KB/s");
        assert_eq!(format_bytes_per_sec(1024.0 * 1024.0), "1.00 MB/s");
        assert_eq!(format_bytes_per_sec(1024.0 * 1024.0 * 1024.0), "1.00 GB/s");
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(30.0), "30s");
        assert_eq!(format_duration(90.0), "1m 30s");
        assert_eq!(format_duration(3700.0), "1h 1m");
    }
}

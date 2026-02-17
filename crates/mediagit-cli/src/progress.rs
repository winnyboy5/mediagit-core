use chrono::{DateTime, Utc};
use indicatif::{
    HumanBytes, HumanCount, HumanDuration, MultiProgress, ProgressBar, ProgressDrawTarget,
    ProgressFinish, ProgressStyle,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

/// Progress tracker for Git operations
pub struct ProgressTracker {
    multi: Arc<MultiProgress>,
    quiet: bool,
}

impl ProgressTracker {
    /// Create new progress tracker
    /// Uses stderr for progress output to keep stdout clean for piping
    pub fn new(quiet: bool) -> Self {
        Self {
            multi: Arc::new(if quiet {
                MultiProgress::with_draw_target(ProgressDrawTarget::hidden())
            } else {
                MultiProgress::with_draw_target(ProgressDrawTarget::stderr())
            }),
            quiet,
        }
    }

    /// Create progress bar for download operations
    pub fn download_bar(&self, msg: &str) -> ProgressBar {
        if self.quiet {
            return ProgressBar::hidden();
        }

        let pb = self.multi.add(ProgressBar::new(0));
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.cyan} {msg} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})")
                .unwrap()
                .progress_chars("█▓░"),
        );
        pb.set_message(msg.to_string());
        pb.enable_steady_tick(Duration::from_millis(100));
        pb
    }

    /// Create progress bar for upload operations
    #[allow(dead_code)]
    pub fn upload_bar(&self, msg: &str) -> ProgressBar {
        if self.quiet {
            return ProgressBar::hidden();
        }

        let pb = self.multi.add(ProgressBar::new(0));
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} {msg} [{bar:40.green/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})")
                .unwrap()
                .progress_chars("█▓░"),
        );
        pb.set_message(msg.to_string());
        pb.enable_steady_tick(Duration::from_millis(100));
        pb
    }

    /// Create progress bar for object processing
    pub fn object_bar(&self, msg: &str, total: u64) -> ProgressBar {
        if self.quiet {
            return ProgressBar::hidden();
        }

        let pb = self.multi.add(ProgressBar::new(total));
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.yellow} {msg} [{bar:40.yellow/blue}] {pos}/{len} ({percent}%)")
                .unwrap()
                .progress_chars("█▓░"),
        );
        pb.set_message(msg.to_string());
        pb.enable_steady_tick(Duration::from_millis(100));
        pb
    }

    /// Create progress bar for file operations
    pub fn file_bar(&self, msg: &str, total: u64) -> ProgressBar {
        if self.quiet {
            return ProgressBar::hidden();
        }

        let pb = self.multi.add(ProgressBar::new(total));
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.magenta} {msg} [{bar:40.magenta/blue}] {pos}/{len} files ({percent}%)")
                .unwrap()
                .progress_chars("█▓░"),
        );
        pb.set_message(msg.to_string());
        pb.enable_steady_tick(Duration::from_millis(100));
        pb
    }

    /// Create progress bar for verification operations (fsck, verify)
    #[allow(dead_code)]
    pub fn verify_bar(&self, msg: &str, total: u64) -> ProgressBar {
        if self.quiet {
            return ProgressBar::hidden();
        }

        let pb = self.multi.add(
            ProgressBar::new(total).with_finish(ProgressFinish::WithMessage("✓ verified".into())),
        );
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.blue} {msg} [{bar:40.blue/cyan}] {pos}/{len} ({percent}%, {per_sec})")
                .unwrap()
                .progress_chars("█▓░"),
        );
        pb.set_message(msg.to_string());
        pb.enable_steady_tick(Duration::from_millis(100));
        pb
    }

    /// Create progress bar for merge/rebase operations
    #[allow(dead_code)]
    pub fn merge_bar(&self, msg: &str, total: u64) -> ProgressBar {
        if self.quiet {
            return ProgressBar::hidden();
        }

        let pb = self.multi.add(
            ProgressBar::new(total).with_finish(ProgressFinish::WithMessage("✓ complete".into())),
        );
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.magenta} {msg} [{bar:40.magenta/blue}] {pos}/{len}")
                .unwrap()
                .progress_chars("█▓░"),
        );
        pb.set_message(msg.to_string());
        pb.enable_steady_tick(Duration::from_millis(100));
        pb
    }

    /// Create progress bar for streaming I/O operations
    /// Suitable for use with wrap_read()/wrap_write()
    #[allow(dead_code)]
    pub fn io_bar(&self, msg: &str, total_bytes: u64) -> ProgressBar {
        if self.quiet {
            return ProgressBar::hidden();
        }

        let pb = self.multi.add(
            ProgressBar::new(total_bytes)
                .with_finish(ProgressFinish::WithMessage("✓ complete".into())),
        );
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.cyan} {msg} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec})")
                .unwrap()
                .progress_chars("█▓░"),
        );
        pb.set_message(msg.to_string());
        pb.enable_steady_tick(Duration::from_millis(100));
        pb
    }

    /// Create spinner for indeterminate operations
    /// Auto-clears on completion for clean output
    pub fn spinner(&self, msg: &str) -> ProgressBar {
        if self.quiet {
            return ProgressBar::hidden();
        }

        let pb = self.multi.add(
            ProgressBar::new_spinner().with_finish(ProgressFinish::AndClear),
        );
        pb.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.cyan} {msg}")
                .unwrap(),
        );
        pb.set_message(msg.to_string());
        pb.enable_steady_tick(Duration::from_millis(100));
        pb
    }

    /// Check if progress bars are hidden (quiet mode)
    #[allow(dead_code)]
    pub fn is_quiet(&self) -> bool {
        self.quiet
    }

    /// Format a count with thousands separators
    #[allow(dead_code)]
    pub fn format_count(count: u64) -> String {
        format!("{}", HumanCount(count))
    }
}

/// Statistics for Git operations
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct OperationStats {
    pub operation_name: String,
    #[serde(default = "default_timestamp")]
    pub timestamp: DateTime<Utc>,
    pub bytes_downloaded: u64,
    pub bytes_uploaded: u64,
    pub objects_received: u64,
    pub objects_sent: u64,
    pub files_updated: u64,
    pub duration_ms: u64,
}

fn default_timestamp() -> DateTime<Utc> {
    Utc::now()
}

impl OperationStats {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            timestamp: Utc::now(),
            ..Default::default()
        }
    }

    /// Create stats for a specific operation type
    pub fn for_operation(operation_name: &str) -> Self {
        Self {
            operation_name: operation_name.to_string(),
            timestamp: Utc::now(),
            ..Default::default()
        }
    }

    /// Format stats as human-readable string
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();

        if self.bytes_downloaded > 0 {
            parts.push(format!("↓ {}", Self::format_bytes(self.bytes_downloaded)));
        }
        if self.bytes_uploaded > 0 {
            parts.push(format!("↑ {}", Self::format_bytes(self.bytes_uploaded)));
        }
        if self.objects_received > 0 {
            parts.push(format!("{} objects received", self.objects_received));
        }
        if self.objects_sent > 0 {
            parts.push(format!("{} objects sent", self.objects_sent));
        }
        if self.files_updated > 0 {
            parts.push(format!("{} files updated", self.files_updated));
        }
        if self.duration_ms > 0 {
            let duration = Duration::from_millis(self.duration_ms);
            parts.push(format!("in {}", HumanDuration(duration)));
        }

        if parts.is_empty() {
            "No data".to_string()
        } else {
            parts.join(", ")
        }
    }

    fn format_bytes(bytes: u64) -> String {
        format!("{}", HumanBytes(bytes))
    }

    /// Save stats to .mediagit/stats/ directory
    pub fn save(&self, storage_path: &Path) -> anyhow::Result<()> {
        let stats_dir = storage_path.join("stats");
        std::fs::create_dir_all(&stats_dir)?;

        // Create filename with timestamp and operation name
        let filename = format!(
            "{}_{}.json",
            self.timestamp.format("%Y%m%d_%H%M%S"),
            self.operation_name
        );
        let file_path = stats_dir.join(&filename);

        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&file_path, json)?;

        // Keep only the last 100 stats files to prevent unbounded growth
        Self::cleanup_old_stats(&stats_dir, 100)?;

        Ok(())
    }

    /// Load recent stats from .mediagit/stats/ directory
    pub fn load_recent(storage_path: &Path, limit: usize) -> anyhow::Result<Vec<OperationStats>> {
        let stats_dir = storage_path.join("stats");
        if !stats_dir.exists() {
            return Ok(Vec::new());
        }

        let mut entries: Vec<_> = std::fs::read_dir(&stats_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path().extension().map(|ext| ext == "json").unwrap_or(false)
            })
            .collect();

        // Sort by filename (which includes timestamp) in descending order
        entries.sort_by(|a, b| b.path().cmp(&a.path()));

        let mut stats = Vec::new();
        for entry in entries.into_iter().take(limit) {
            if let Ok(content) = std::fs::read_to_string(entry.path()) {
                if let Ok(stat) = serde_json::from_str::<OperationStats>(&content) {
                    stats.push(stat);
                }
            }
        }

        Ok(stats)
    }

    /// Load the most recent stats for a specific operation type
    pub fn load_last_by_type(storage_path: &Path, operation_name: &str) -> anyhow::Result<Option<OperationStats>> {
        let all_stats = Self::load_recent(storage_path, 50)?;
        Ok(all_stats.into_iter().find(|s| s.operation_name == operation_name))
    }

    /// Cleanup old stats files, keeping only the most recent `keep_count`
    fn cleanup_old_stats(stats_dir: &Path, keep_count: usize) -> anyhow::Result<()> {
        let mut entries: Vec<_> = std::fs::read_dir(stats_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path().extension().map(|ext| ext == "json").unwrap_or(false)
            })
            .collect();

        if entries.len() <= keep_count {
            return Ok(());
        }

        // Sort by filename (newest first)
        entries.sort_by(|a, b| b.path().cmp(&a.path()));

        // Remove oldest files
        for entry in entries.into_iter().skip(keep_count) {
            let _ = std::fs::remove_file(entry.path());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_operation_stats_format_bytes() {
        // Print actual values for debugging
        println!("500 B = '{}'", OperationStats::format_bytes(500));
        println!("1 KiB = '{}'", OperationStats::format_bytes(1024));
        println!("1 MiB = '{}'", OperationStats::format_bytes(1024 * 1024));
        println!("1 GiB = '{}'", OperationStats::format_bytes(1024 * 1024 * 1024));
        
        // HumanBytes uses "B", "KiB", "MiB", "GiB" format
        assert!(OperationStats::format_bytes(500).contains("B"));
        assert!(OperationStats::format_bytes(1024).contains("KiB") || OperationStats::format_bytes(1024).contains("KB"));
        assert!(OperationStats::format_bytes(1024 * 1024).contains("MiB") || OperationStats::format_bytes(1024 * 1024).contains("MB"));
        assert!(OperationStats::format_bytes(1024 * 1024 * 1024).contains("GiB") || OperationStats::format_bytes(1024 * 1024 * 1024).contains("GB"));
    }

    #[test]
    fn test_operation_stats_summary() {
        let mut stats = OperationStats::new();
        stats.bytes_downloaded = 1024 * 1024;
        stats.objects_received = 42;
        stats.duration_ms = 1500;

        let summary = stats.summary();
        println!("DEBUG summary: '{}'", summary);
        // HumanBytes uses "MiB" format
        assert!(summary.contains("MiB"), "Expected MiB, got: {}", summary);
        assert!(summary.contains("42 objects"), "Expected 42 objects, got: {}", summary);
        // HumanDuration formats durations in human readable format
        assert!(summary.contains("in "), "Expected 'in ', got: {}", summary);
    }
}

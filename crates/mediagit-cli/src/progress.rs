use indicatif::{HumanBytes, HumanDuration, MultiProgress, ProgressBar, ProgressStyle};
use std::sync::Arc;
use std::time::Duration;

/// Progress tracker for Git operations
pub struct ProgressTracker {
    multi: Arc<MultiProgress>,
    quiet: bool,
}

impl ProgressTracker {
    /// Create new progress tracker
    pub fn new(quiet: bool) -> Self {
        Self {
            multi: Arc::new(MultiProgress::new()),
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
    #[allow(dead_code)]
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

    /// Create spinner for indeterminate operations
    pub fn spinner(&self, msg: &str) -> ProgressBar {
        if self.quiet {
            return ProgressBar::hidden();
        }

        let pb = self.multi.add(ProgressBar::new_spinner());
        pb.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.cyan} {msg}")
                .unwrap(),
        );
        pb.set_message(msg.to_string());
        pb.enable_steady_tick(Duration::from_millis(100));
        pb
    }

    /// Finish all progress bars
    #[allow(dead_code)]
    pub fn finish_all(&self) {
        // All progress bars finish when dropped
    }
}

/// Statistics for Git operations
#[derive(Debug, Default, Clone)]
pub struct OperationStats {
    pub bytes_downloaded: u64,
    pub bytes_uploaded: u64,
    pub objects_received: u64,
    pub objects_sent: u64,
    pub files_updated: u64,
    pub duration_ms: u64,
}

impl OperationStats {
    pub fn new() -> Self {
        Self::default()
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

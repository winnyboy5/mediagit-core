// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

use chrono::{DateTime, Utc};
use indicatif::{
    HumanBytes, HumanDuration, MultiProgress, ProgressBar, ProgressDrawTarget, ProgressFinish,
    ProgressStyle,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

/// An ETA this large is not an estimate, it is an artefact — a stall, or a
/// rate sampled before enough progress landed to mean anything. Reporting
/// "eta 231y" is worse than admitting the number is unknown.
const MAX_MEANINGFUL_ETA: Duration = Duration::from_secs(48 * 60 * 60);

/// RP-6: `--` until an ETA can honestly be computed.
///
/// Nothing has been transferred at position 0, so any ETA is invented; the
/// familiar symptom is "eta 0s" displayed at 0%, which reads as "about to
/// finish" at the exact moment nothing has happened.
fn format_eta(state: &indicatif::ProgressState) -> String {
    eta_display(state.pos(), state.len(), state.eta())
}

/// Split from `format_eta` so the rules are testable: `ProgressState` has no
/// public constructor, so a test cannot reach them through the closure.
fn eta_display(pos: u64, len: Option<u64>, eta: Duration) -> String {
    if pos == 0 || len.is_none_or(|len| len == 0) || eta > MAX_MEANINGFUL_ETA {
        return "--".to_string();
    }
    format!("{}", HumanDuration(eta))
}

/// RP-4: `--` until a rate can honestly be computed.
///
/// Averaged over the whole operation rather than a decaying window: transfer
/// is credited in pack-sized steps, and a decaying window samples the gaps
/// between steps as though they were idle, which is what produced "33 B/s"
/// during an otherwise healthy transfer.
fn format_rate(state: &indicatif::ProgressState) -> String {
    rate_display(state.pos(), state.elapsed())
}

/// See `eta_display` for why this is split out.
fn rate_display(pos: u64, elapsed: Duration) -> String {
    let secs = elapsed.as_secs_f64();
    if pos == 0 || secs < 0.5 {
        return "--".to_string();
    }
    format!("{}/s", HumanBytes((pos as f64 / secs) as u64))
}

/// Standard progress bar templates used across all CLI commands.
/// All bars use 40-char width, "█▓░" characters, 100ms tick, stderr output.
mod templates {
    /// Bytes-based progress for staging (`add`) operations.
    pub const ADD: &str = "{spinner:.green} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, eta {eta}) {msg}";

    /// Item-count progress for object processing (pack, delta, chunk transfer).
    pub const OBJECTS: &str = "{spinner:.yellow} {msg} [{bar:40.yellow/blue}] {pos}/{len} chunks ({percent}%, {elapsed}) eta {eta}";

    /// Bytes-based progress for push uploads with throughput and ETA.
    pub const PUSH: &str = "{spinner:.cyan} [{bar:40.cyan/blue}] {bytes}/{total_bytes} @ {bytes_per_sec} (elapsed {elapsed}, eta {eta})";

    /// Bytes-based progress for chunk downloads — mirrors PUSH format.
    pub const DOWNLOAD_BYTES: &str = "{spinner:.cyan} [{bar:40.cyan/blue}] {bytes}/{total_bytes} @ {bytes_per_sec} (elapsed {elapsed}, eta {eta}) {msg}";

    /// Indeterminate spinner for operations without a known total.
    pub const SPINNER: &str = "{spinner:.cyan} {msg} [{elapsed}]";
}

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

    /// Shared implementation for determinate progress bars.
    ///
    /// RP-4/RP-6: `eta` and `bytes_per_sec` render `--` when they are not yet
    /// knowable, instead of printing a confident wrong number.
    ///
    /// Transfer progress is credited in lumps — a cloud pack's bytes land only
    /// when its upload is confirmed, because that is the first moment the
    /// bytes are known to have arrived. Crediting earlier would mean inventing
    /// progress, and crediting incrementally would require a streaming request
    /// body, which cannot be replayed on retry. So the estimators must cope
    /// with a stream of large steps rather than the lumps being smoothed away:
    /// before the first step lands there is genuinely no rate to report, and
    /// "eta 0s" at 0% or "33 B/s" mid-transfer are both that absence rendered
    /// as fact.
    fn make_bar_impl(&self, total: u64, msg: &str, template: &str) -> ProgressBar {
        let pb = self.multi.add(ProgressBar::new(total));
        pb.set_style(
            ProgressStyle::default_bar()
                .template(template)
                .expect("valid progress template")
                .with_key(
                    "eta",
                    |state: &indicatif::ProgressState, w: &mut dyn std::fmt::Write| {
                        let _ = write!(w, "{}", format_eta(state));
                    },
                )
                .with_key(
                    "bytes_per_sec",
                    |state: &indicatif::ProgressState, w: &mut dyn std::fmt::Write| {
                        let _ = write!(w, "{}", format_rate(state));
                    },
                )
                .progress_chars("█▓░"),
        );
        pb.set_message(msg.to_string());
        pb.enable_steady_tick(Duration::from_millis(100));
        pb
    }

    /// Create progress bar for staging (`add`) operations.
    ///
    /// The total byte count is known upfront and set as the bar's length,
    /// allowing percentage display. The returned bar is suitable for wrapping in `Arc`.
    pub fn add_bar(&self, msg: &str, total_bytes: u64) -> ProgressBar {
        if self.quiet {
            return ProgressBar::hidden();
        }
        self.make_bar_impl(total_bytes, msg, templates::ADD)
    }

    /// Create progress bar for object processing
    pub fn object_bar(&self, msg: &str, total: u64) -> ProgressBar {
        if self.quiet {
            return ProgressBar::hidden();
        }
        self.make_bar_impl(total, msg, templates::OBJECTS)
    }

    /// Create bytes progress bar for push uploads (real throughput + ETA).
    /// `total_bytes` may be 0 initially and updated via `set_length` as chunks are discovered.
    pub fn push_bar(&self, total_bytes: u64) -> ProgressBar {
        if self.quiet {
            return ProgressBar::hidden();
        }
        self.make_bar_impl(total_bytes, "Pushing", templates::PUSH)
    }

    /// Create bytes progress bar for chunk downloads — mirrors push bar format.
    /// `total_bytes` is seeded from manifest sizes in Phase 1; updated via `set_length`.
    pub fn download_bar(&self, msg: &str, total_bytes: u64) -> ProgressBar {
        if self.quiet {
            return ProgressBar::hidden();
        }
        self.make_bar_impl(total_bytes, msg, templates::DOWNLOAD_BYTES)
    }

    /// Create spinner for indeterminate operations
    /// Auto-clears on completion for clean output
    pub fn spinner(&self, msg: &str) -> ProgressBar {
        if self.quiet {
            return ProgressBar::hidden();
        }

        let pb = self
            .multi
            .add(ProgressBar::new_spinner().with_finish(ProgressFinish::AndClear));
        pb.set_style(
            ProgressStyle::default_spinner()
                .template(templates::SPINNER)
                .expect("valid spinner template"),
        );
        pb.set_message(msg.to_string());
        pb.enable_steady_tick(Duration::from_millis(100));
        pb
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
    #[allow(dead_code)] // used in tests only
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
                e.path()
                    .extension()
                    .map(|ext| ext == "json")
                    .unwrap_or(false)
            })
            .collect();

        // Sort by filename (which includes timestamp) in descending order
        entries.sort_by_key(|b| std::cmp::Reverse(b.path()));

        let mut stats = Vec::new();
        for entry in entries.into_iter().take(limit) {
            if let Ok(content) = std::fs::read_to_string(entry.path())
                && let Ok(stat) = serde_json::from_str::<OperationStats>(&content)
            {
                stats.push(stat);
            }
        }

        Ok(stats)
    }

    /// Load the most recent stats for a specific operation type
    pub fn load_last_by_type(
        storage_path: &Path,
        operation_name: &str,
    ) -> anyhow::Result<Option<OperationStats>> {
        let all_stats = Self::load_recent(storage_path, 50)?;
        Ok(all_stats
            .into_iter()
            .find(|s| s.operation_name == operation_name))
    }

    /// Cleanup old stats files, keeping only the most recent `keep_count`
    fn cleanup_old_stats(stats_dir: &Path, keep_count: usize) -> anyhow::Result<()> {
        let mut entries: Vec<_> = std::fs::read_dir(stats_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "json")
                    .unwrap_or(false)
            })
            .collect();

        if entries.len() <= keep_count {
            return Ok(());
        }

        // Sort by filename (newest first)
        entries.sort_by_key(|b| std::cmp::Reverse(b.path()));

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

    /// RP-6: "eta 0s" at 0% told operators a transfer was about to finish at
    /// the exact moment nothing had happened.
    #[test]
    fn eta_is_unknown_until_there_is_progress() {
        assert_eq!(eta_display(0, Some(1000), Duration::from_secs(0)), "--");
        assert_eq!(eta_display(0, Some(1000), Duration::from_secs(5)), "--");
        assert_eq!(eta_display(500, None, Duration::from_secs(5)), "--");
        assert_eq!(eta_display(500, Some(0), Duration::from_secs(5)), "--");
    }

    /// An ETA of years is an artefact of a stall, not an estimate.
    #[test]
    fn implausible_eta_renders_as_unknown() {
        let years = Duration::from_secs(231 * 365 * 24 * 60 * 60);
        assert_eq!(eta_display(500, Some(1000), years), "--");
        assert_ne!(
            eta_display(500, Some(1000), Duration::from_secs(90)),
            "--",
            "a plausible ETA must still be shown"
        );
    }

    /// RP-4: no rate exists before the first credit lands, and transfer is
    /// credited in pack-sized steps — so an early sample is noise, not speed.
    #[test]
    fn rate_is_unknown_until_measurable() {
        assert_eq!(rate_display(0, Duration::from_secs(10)), "--");
        assert_eq!(rate_display(1024, Duration::from_millis(100)), "--");
    }

    /// The rate is a whole-run average, so a 64 MiB pack credited in one step
    /// cannot report more than the link actually carried.
    #[test]
    fn rate_averages_over_the_run_not_the_last_step() {
        // 64 MiB credited at once, 8 s into the transfer => 8 MiB/s, not the
        // "instant" rate of a 64 MiB jump over ~0 s that read as 747 MiB/s.
        let rendered = rate_display(64 * 1024 * 1024, Duration::from_secs(8));
        assert_eq!(rendered, "8.00 MiB/s", "got {rendered}");
    }

    #[test]
    fn test_operation_stats_format_bytes() {
        // Print actual values for debugging
        println!("500 B = '{}'", OperationStats::format_bytes(500));
        println!("1 KiB = '{}'", OperationStats::format_bytes(1024));
        println!("1 MiB = '{}'", OperationStats::format_bytes(1024 * 1024));
        println!(
            "1 GiB = '{}'",
            OperationStats::format_bytes(1024 * 1024 * 1024)
        );

        // HumanBytes uses "B", "KiB", "MiB", "GiB" format
        assert!(OperationStats::format_bytes(500).contains("B"));
        assert!(
            OperationStats::format_bytes(1024).contains("KiB")
                || OperationStats::format_bytes(1024).contains("KB")
        );
        assert!(
            OperationStats::format_bytes(1024 * 1024).contains("MiB")
                || OperationStats::format_bytes(1024 * 1024).contains("MB")
        );
        assert!(
            OperationStats::format_bytes(1024 * 1024 * 1024).contains("GiB")
                || OperationStats::format_bytes(1024 * 1024 * 1024).contains("GB")
        );
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
        assert!(
            summary.contains("42 objects"),
            "Expected 42 objects, got: {}",
            summary
        );
        // HumanDuration formats durations in human readable format
        assert!(summary.contains("in "), "Expected 'in ', got: {}", summary);
    }
}

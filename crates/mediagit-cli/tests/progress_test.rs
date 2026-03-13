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

//! Tests for progress tracking and operation stats.

use mediagit_cli::progress::{OperationStats, ProgressTracker};

#[test]
fn test_operation_stats_creation() {
    let stats = OperationStats::new();
    assert_eq!(stats.bytes_downloaded, 0);
    assert_eq!(stats.bytes_uploaded, 0);
    assert_eq!(stats.objects_received, 0);
    assert_eq!(stats.objects_sent, 0);
    assert_eq!(stats.files_updated, 0);
}

#[test]
fn test_operation_stats_summary() {
    let mut stats = OperationStats::new();
    stats.bytes_downloaded = 1024 * 1024; // 1 MiB - use binary size for predictable output
    stats.objects_received = 42;
    stats.duration_ms = 1500;

    let summary = stats.summary();
    // HumanBytes uses MiB (binary) format, not MB (decimal)
    assert!(summary.contains("MiB"), "Expected MiB, got: {}", summary);
    assert!(
        summary.contains("42 objects"),
        "Expected 42 objects, got: {}",
        summary
    );
    // HumanDuration formats durations in human readable format
    assert!(summary.contains("in "), "Expected 'in ', got: {}", summary);
}

#[test]
fn test_progress_tracker_quiet_mode() {
    let tracker = ProgressTracker::new(true);
    let pb = tracker.spinner("Test");
    assert!(pb.is_hidden());
}

#[test]
fn test_progress_tracker_creates_bars() {
    let tracker = ProgressTracker::new(false);

    let object_pb = tracker.object_bar("Test objects", 100);
    let spinner = tracker.spinner("Test spinner");

    drop(object_pb);
    drop(spinner);
}

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
    assert!(summary.contains("42 objects"), "Expected 42 objects, got: {}", summary);
    // HumanDuration formats durations in human readable format
    assert!(summary.contains("in "), "Expected 'in ', got: {}", summary);
}

#[test]
fn test_progress_tracker_quiet_mode() {
    let tracker = ProgressTracker::new(true);
    let pb = tracker.download_bar("Test");
    assert!(pb.is_hidden());
    
    // All bar types should be hidden in quiet mode
    let verify_pb = tracker.verify_bar("Test", 100);
    assert!(verify_pb.is_hidden());
    
    let merge_pb = tracker.merge_bar("Test", 100);
    assert!(merge_pb.is_hidden());
    
    let io_pb = tracker.io_bar("Test", 1024);
    assert!(io_pb.is_hidden());
}

#[test]
fn test_progress_tracker_creates_bars() {
    let tracker = ProgressTracker::new(false);

    // Test different bar types can be created
    let download_pb = tracker.download_bar("Test download");
    let upload_pb = tracker.upload_bar("Test upload");
    let object_pb = tracker.object_bar("Test objects", 100);
    let file_pb = tracker.file_bar("Test files", 50);
    let spinner = tracker.spinner("Test spinner");

    // Verify they're created (not necessarily visible in test environment)
    drop(download_pb);
    drop(upload_pb);
    drop(object_pb);
    drop(file_pb);
    drop(spinner);
}

#[test]
fn test_new_bar_types() {
    let tracker = ProgressTracker::new(false);
    
    // Test new bar types from Phase 0b
    let verify_pb = tracker.verify_bar("Testing verification", 100);
    let merge_pb = tracker.merge_bar("Testing merge", 50);
    let io_pb = tracker.io_bar("Testing I/O", 1024 * 1024);
    
    // Verify they're created successfully
    drop(verify_pb);
    drop(merge_pb);
    drop(io_pb);
}

#[test]
fn test_is_quiet_method() {
    let quiet_tracker = ProgressTracker::new(true);
    assert!(quiet_tracker.is_quiet());
    
    let normal_tracker = ProgressTracker::new(false);
    assert!(!normal_tracker.is_quiet());
}

#[test]
fn test_format_count() {
    // Test HumanCount formatting
    let formatted = ProgressTracker::format_count(1234567);
    // HumanCount adds thousands separators
    assert!(!formatted.is_empty());
}


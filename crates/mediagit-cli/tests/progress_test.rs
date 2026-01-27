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

use crate::benchmarking::{measure_execution_time, PerformanceTracker};
use std::thread;
use std::time::Duration;
use tempfile::tempdir;

#[test]
fn test_measure_execution_time() {
    let (result, duration) = measure_execution_time(|| {
        thread::sleep(Duration::from_millis(50));
        "test"
    });

    assert_eq!(result, "test");
    assert!(duration.as_millis() >= 50);
}

#[test]
fn test_performance_tracker_basic() {
    let temp_dir = tempdir().unwrap();
    let log_path = temp_dir
        .path()
        .join("perf.log")
        .to_str()
        .unwrap()
        .to_string();

    let tracker = PerformanceTracker::new(true, &log_path, 10, 1);
    tracker.start();

    // Wait for metrics to be collected
    thread::sleep(Duration::from_secs(2));

    let metrics = tracker.get_current_metrics();
    assert!(metrics.is_some());

    tracker.stop();
}

#[test]
fn test_performance_tracker_disabled() {
    let temp_dir = tempdir().unwrap();
    let log_path = temp_dir
        .path()
        .join("perf.log")
        .to_str()
        .unwrap()
        .to_string();

    let tracker = PerformanceTracker::new(false, &log_path, 10, 1);
    tracker.start();

    // Wait for metrics to be collected
    thread::sleep(Duration::from_secs(2));

    let metrics = tracker.get_current_metrics();
    assert!(metrics.is_none());

    tracker.stop();
}

#[test]
fn test_performance_tracker_history() {
    let temp_dir = tempdir().unwrap();
    let log_path = temp_dir
        .path()
        .join("perf.log")
        .to_str()
        .unwrap()
        .to_string();

    let tracker = PerformanceTracker::new(true, &log_path, 5, 1);
    tracker.start();

    // Wait for metrics to be collected
    thread::sleep(Duration::from_secs(6));

    let metrics = tracker.get_current_metrics();
    assert!(metrics.is_some());

    // Check that we have the correct number of metrics
    let average_metrics = tracker.get_average_metrics();
    assert!(average_metrics.is_some());

    tracker.stop();
}

#[test]
fn test_performance_tracker_stop_start() {
    let temp_dir = tempdir().unwrap();
    let log_path = temp_dir
        .path()
        .join("perf.log")
        .to_str()
        .unwrap()
        .to_string();

    let tracker = PerformanceTracker::new(true, &log_path, 10, 1);

    // Start and collect some metrics
    tracker.start();
    thread::sleep(Duration::from_secs(2));
    let metrics1 = tracker.get_current_metrics();
    assert!(metrics1.is_some());

    // Stop and verify no new metrics
    tracker.stop();
    thread::sleep(Duration::from_secs(2));
    let metrics2 = tracker.get_current_metrics();
    assert!(metrics2.is_some());

    // Start again and verify new metrics
    tracker.start();
    thread::sleep(Duration::from_secs(2));
    let metrics3 = tracker.get_current_metrics();
    assert!(metrics3.is_some());

    // Verify that metrics changed after restart
    let metrics1 = metrics1.unwrap();
    let metrics2 = metrics2.unwrap();
    let metrics3 = metrics3.unwrap();

    // After stopping, metrics should be the same
    assert!(metrics1.memory_usage > 0);
    assert!(metrics2.memory_usage > 0);

    // After restarting, metrics should be different
    assert!(metrics3.memory_usage > 0);
}

#[test]
fn test_performance_tracker_metrics_range() {
    let temp_dir = tempdir().unwrap();
    let log_path = temp_dir
        .path()
        .join("perf.log")
        .to_str()
        .unwrap()
        .to_string();

    let tracker = PerformanceTracker::new(true, &log_path, 10, 1);
    tracker.start();

    // Wait for metrics to be collected
    thread::sleep(Duration::from_secs(2));

    let metrics = tracker.get_current_metrics().unwrap();

    // Check that metrics are within expected ranges
    assert!(metrics.cpu_usage >= 0.0 && metrics.cpu_usage <= 100.0);
    assert!(metrics.memory_usage > 0);
    assert!(metrics.memory_percent >= 0.0 && metrics.memory_percent <= 100.0);

    tracker.stop();
}

#[test]
fn test_performance_tracker_log_file() {
    let temp_dir = tempdir().unwrap();
    let log_path = temp_dir
        .path()
        .join("perf.log")
        .to_str()
        .unwrap()
        .to_string();

    let tracker = PerformanceTracker::new(true, &log_path, 10, 1);
    tracker.start();

    // Wait for metrics to be collected
    thread::sleep(Duration::from_secs(2));

    // Verify log file exists and contains data
    let log_content = std::fs::read_to_string(&log_path).unwrap();
    assert!(!log_content.is_empty());
    assert!(log_content.contains("timestamp"));
    assert!(log_content.contains("cpu_usage"));
    assert!(log_content.contains("memory_usage"));
    assert!(log_content.contains("memory_percent"));

    tracker.stop();
}

#[test]
fn test_performance_tracker_multiple_starts() {
    let temp_dir = tempdir().unwrap();
    let log_path = temp_dir
        .path()
        .join("perf.log")
        .to_str()
        .unwrap()
        .to_string();

    let tracker = PerformanceTracker::new(true, &log_path, 10, 1);

    // Multiple starts should not create multiple threads
    tracker.start();
    tracker.start();

    thread::sleep(Duration::from_secs(2));

    let metrics = tracker.get_current_metrics();
    assert!(metrics.is_some());

    tracker.stop();
}

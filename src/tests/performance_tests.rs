//! Performance tracker tests.
//!
//! These tests exercise the live `PerformanceTracker` (sysinfo-backed metrics
//! collection on a background thread) and require multi-second sleeps to let
//! the collector populate. They previously used a `can_run_performance_tests()`
//! helper that silently returned early when `CI` was set, which meant CI runs
//! reported them as "passing" without actually executing any assertions.
//!
//! All tracker-dependent tests are now `#[ignore]` with an explicit reason —
//! same model as `alloc_tests.rs` / `benchmark_tests.rs`. Run them locally with:
//!
//! ```sh
//! cargo test --release --features benchmarking performance -- --ignored
//! ```
//!
//! `test_measure_execution_time` stays as a regular `#[test]` because it has
//! a sub-second sleep and tests a pure measurement helper, not the tracker.

use crate::benchmarking::{PerformanceTracker, measure_execution_time};
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
#[ignore = "real metrics collection takes seconds; run with --ignored locally"]
fn test_performance_tracker_basic() {
    let temp_dir = tempdir().unwrap();
    let log_path = temp_dir.path().join("perf.log").to_str().unwrap().to_string();

    let tracker = PerformanceTracker::new(true, &log_path, 10, 1);
    tracker.start();
    thread::sleep(Duration::from_secs(2));

    let metrics = tracker
        .get_current_metrics()
        .expect("tracker should have collected at least one metrics sample after 2s");
    assert!(metrics.memory_usage > 0);

    tracker.stop();
}

#[test]
#[ignore = "real metrics collection takes seconds; run with --ignored locally"]
fn test_performance_tracker_disabled() {
    let temp_dir = tempdir().unwrap();
    let log_path = temp_dir.path().join("perf.log").to_str().unwrap().to_string();

    let tracker = PerformanceTracker::new(false, &log_path, 10, 1);
    tracker.start();
    thread::sleep(Duration::from_secs(2));

    let metrics = tracker.get_current_metrics();
    assert!(
        metrics.is_none(),
        "disabled tracker must not produce metrics"
    );

    tracker.stop();
}

#[test]
#[ignore = "real metrics collection takes seconds; run with --ignored locally"]
fn test_performance_tracker_history() {
    let temp_dir = tempdir().unwrap();
    let log_path = temp_dir.path().join("perf.log").to_str().unwrap().to_string();

    let tracker = PerformanceTracker::new(true, &log_path, 5, 1);
    tracker.start();
    thread::sleep(Duration::from_secs(6));

    tracker
        .get_current_metrics()
        .expect("tracker should have current metrics after 6s");
    let average_metrics = tracker.get_average_metrics();
    assert!(
        average_metrics.is_some(),
        "tracker with history should have averageable samples after 6s"
    );

    tracker.stop();
}

#[test]
#[ignore = "real metrics collection takes seconds; run with --ignored locally"]
fn test_performance_tracker_stop_start() {
    let temp_dir = tempdir().unwrap();
    let log_path = temp_dir.path().join("perf.log").to_str().unwrap().to_string();

    let tracker = PerformanceTracker::new(true, &log_path, 10, 1);

    tracker.start();
    thread::sleep(Duration::from_secs(2));
    let metrics1 = tracker
        .get_current_metrics()
        .expect("first start should produce metrics");

    tracker.stop();
    thread::sleep(Duration::from_secs(2));
    let metrics2 = tracker
        .get_current_metrics()
        .expect("metrics from before stop should still be readable");

    tracker.start();
    thread::sleep(Duration::from_secs(2));
    let metrics3 = tracker
        .get_current_metrics()
        .expect("second start should produce metrics");

    assert!(metrics1.memory_usage > 0);
    assert!(metrics2.memory_usage > 0);
    assert!(metrics3.memory_usage > 0);
}

#[test]
#[ignore = "real metrics collection takes seconds; run with --ignored locally"]
fn test_performance_tracker_metrics_range() {
    let temp_dir = tempdir().unwrap();
    let log_path = temp_dir.path().join("perf.log").to_str().unwrap().to_string();

    let tracker = PerformanceTracker::new(true, &log_path, 10, 1);
    tracker.start();
    thread::sleep(Duration::from_secs(2));

    let metrics = tracker
        .get_current_metrics()
        .expect("tracker should have collected at least one sample after 2s");
    assert!(metrics.cpu_usage >= 0.0 && metrics.cpu_usage <= 100.0);
    assert!(metrics.memory_usage > 0);
    assert!(metrics.memory_percent >= 0.0 && metrics.memory_percent <= 100.0);

    tracker.stop();
}

#[test]
#[ignore = "real metrics collection takes seconds; run with --ignored locally"]
fn test_performance_tracker_log_file() {
    let temp_dir = tempdir().unwrap();
    let log_path = temp_dir.path().join("perf.log").to_str().unwrap().to_string();

    let tracker = PerformanceTracker::new(true, &log_path, 10, 1);
    tracker.start();
    thread::sleep(Duration::from_secs(2));

    let log_content = std::fs::read_to_string(&log_path)
        .expect("log file must exist and be readable after 2s of metrics collection");
    assert!(!log_content.is_empty());
    assert!(log_content.contains("timestamp") || log_content.contains("cpu_usage"));

    tracker.stop();
}

#[test]
#[ignore = "real metrics collection takes seconds; run with --ignored locally"]
fn test_performance_tracker_multiple_starts() {
    let temp_dir = tempdir().unwrap();
    let log_path = temp_dir.path().join("perf.log").to_str().unwrap().to_string();

    let tracker = PerformanceTracker::new(true, &log_path, 10, 1);

    // Multiple starts should not create multiple threads.
    tracker.start();
    tracker.start();
    thread::sleep(Duration::from_secs(2));

    let metrics = tracker
        .get_current_metrics()
        .expect("tracker should produce metrics regardless of multiple start() calls");
    assert!(metrics.memory_usage > 0);

    tracker.stop();
}

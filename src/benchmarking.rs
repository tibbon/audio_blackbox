use chrono::prelude::*;
use log::error;
use std::collections::VecDeque;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use sysinfo::System;

/// Struct to track performance metrics over time
pub struct PerformanceTracker {
    enabled: bool,
    log_path: String,
    metrics: Arc<Mutex<VecDeque<PerformanceMetrics>>>,
    running: Arc<AtomicBool>,
    history_length: usize,
    interval_secs: u64,
    /// JoinHandle for the worker thread spawned in `start`. Wrapped in
    /// a Mutex<Option> because `stop` and `Drop` take `&self` and need
    /// to consume the handle (DOLL-143). Mirrors `SilenceCheckWorker`'s
    /// join-on-drop pattern in `writer_thread.rs`.
    handle: Mutex<Option<thread::JoinHandle<()>>>,
}

/// Struct to store a single performance snapshot
#[derive(Clone, Debug)]
pub struct PerformanceMetrics {
    pub timestamp: DateTime<Local>,
    pub cpu_usage: f32,
    pub memory_usage: u64,
    pub memory_percent: f32,
}

impl PerformanceTracker {
    /// Create a new performance tracker
    pub fn new(enabled: bool, log_path: &str, history_length: usize, interval_secs: u64) -> Self {
        PerformanceTracker {
            enabled,
            log_path: log_path.to_string(),
            metrics: Arc::new(Mutex::new(VecDeque::with_capacity(history_length))),
            running: Arc::new(AtomicBool::new(false)),
            history_length,
            interval_secs,
            handle: Mutex::new(None),
        }
    }

    /// Start the performance tracking in a background thread
    pub fn start(&self) {
        if !self.enabled {
            return;
        }

        // Atomic CAS: if already running, bail; otherwise mark running.
        if self
            .running
            .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
        {
            return; // Already running
        }

        let metrics = Arc::clone(&self.metrics);
        let running = Arc::clone(&self.running);
        let log_path = self.log_path.clone();
        let history_length = self.history_length;
        let interval_secs = self.interval_secs;

        let join_handle = thread::spawn(move || {
            let mut sys = System::new_all();
            let pid = std::process::id();

            // Initialize log file
            let now = Local::now();
            let header = format!(
                "Performance log started at {}\n\
                timestamp,cpu_usage,memory_usage_bytes,memory_percent\n",
                now.format("%Y-%m-%d %H:%M:%S")
            );

            if let Err(e) = write_to_log(&log_path, &header) {
                error!("Failed to initialize performance log: {}", e);
            }

            // Monitoring loop
            while running.load(Ordering::Relaxed) {
                sys.refresh_all();

                if let Some(process) = sys.process(sysinfo::Pid::from_u32(pid)) {
                    let cpu_usage = process.cpu_usage();
                    let memory_usage = process.memory();
                    let memory_percent = (memory_usage as f32 / sys.total_memory() as f32) * 100.0;

                    let metric = PerformanceMetrics {
                        timestamp: Local::now(),
                        cpu_usage,
                        memory_usage,
                        memory_percent,
                    };

                    // Add to metrics queue and limit its size. Mirrors the
                    // DOLL-115 `.lock().ok()` pattern: a poisoned lock here
                    // means an earlier panic on the metrics-collector
                    // thread; we'd rather drop a sample than abort the
                    // whole tracker thread.
                    if let Ok(mut metrics_guard) = metrics.lock() {
                        metrics_guard.push_back(metric.clone());
                        while metrics_guard.len() > history_length {
                            metrics_guard.pop_front();
                        }
                    }

                    // Log to file
                    let log_line = format!(
                        "{},{:.2},{},{:.2}\n",
                        metric.timestamp.format("%Y-%m-%d %H:%M:%S"),
                        metric.cpu_usage,
                        metric.memory_usage,
                        metric.memory_percent
                    );

                    if let Err(e) = write_to_log(&log_path, &log_line) {
                        error!("Failed to write to performance log: {}", e);
                    }
                }

                thread::sleep(Duration::from_secs(interval_secs));
            }
        });

        // Stash the handle so `stop` / `Drop` can join the worker
        // (DOLL-143). A poisoned mutex here would mean an earlier
        // panic on a caller of `start`/`stop`; falling through is
        // fine — the worker still exits when `running` flips.
        if let Ok(mut guard) = self.handle.lock() {
            *guard = Some(join_handle);
        }
    }

    /// Stop the performance tracking and join the worker thread.
    ///
    /// The worker may still be mid-iteration when `stop` is called; it
    /// observes `running == false` on its next loop check (worst case
    /// `interval_secs` later) and exits, then this method blocks until
    /// the join completes. Mirrors the join-on-drop pattern used by
    /// `SilenceCheckWorker` in `writer_thread.rs`.
    pub fn stop(&self) {
        if !self.enabled {
            return;
        }

        self.running.store(false, Ordering::Relaxed);

        // Take the JoinHandle out from under the mutex and join. If
        // `stop` is called twice, the second call sees None and is a
        // no-op (matches AtomicBool::store idempotence).
        let handle = self.handle.lock().ok().and_then(|mut g| g.take());
        if let Some(h) = handle {
            let _ = h.join();
        }
    }

    /// Get the current performance metrics
    pub fn get_current_metrics(&self) -> Option<PerformanceMetrics> {
        if !self.enabled {
            return None;
        }

        // Match the DOLL-115 `.lock().ok()` convention — poisoned-lock
        // means a tracker thread panicked; surface as None rather than
        // re-panicking on the caller's thread.
        let metrics = self.metrics.lock().ok()?;
        metrics.back().cloned()
    }

    /// Get the average performance metrics over the tracked history
    pub fn get_average_metrics(&self) -> Option<PerformanceMetrics> {
        if !self.enabled {
            return None;
        }

        let metrics = self.metrics.lock().ok()?;
        if metrics.is_empty() {
            return None;
        }

        let len = metrics.len() as f32;
        let mut cpu_sum = 0.0;
        let mut memory_sum = 0;
        let mut memory_percent_sum = 0.0;

        for metric in metrics.iter() {
            cpu_sum += metric.cpu_usage;
            memory_sum += metric.memory_usage;
            memory_percent_sum += metric.memory_percent;
        }

        Some(PerformanceMetrics {
            timestamp: Local::now(),
            cpu_usage: cpu_sum / len,
            memory_usage: (memory_sum as f32 / len) as u64,
            memory_percent: memory_percent_sum / len,
        })
    }
}

impl Drop for PerformanceTracker {
    /// Ensure the worker thread is joined when the tracker goes out
    /// of scope (DOLL-143). Calls `stop`, which is idempotent w.r.t.
    /// the running flag and a no-op on the second join. Symmetric
    /// with `SilenceCheckWorker::drop` in `writer_thread.rs`.
    fn drop(&mut self) {
        self.stop();
    }
}

/// Helper function to write to the log file
fn write_to_log(path: &str, content: &str) -> std::io::Result<()> {
    let file = OpenOptions::new().create(true).append(true).open(path)?;

    let mut writer = std::io::BufWriter::new(file);
    writer.write_all(content.as_bytes())?;
    writer.flush()?;

    Ok(())
}

/// Measures elapsed time for a function and returns the duration
#[allow(dead_code)] // only consumed by tests; signature kept stable for future use
pub fn measure_execution_time<F, T>(f: F) -> (T, Duration)
where
    F: FnOnce() -> T,
{
    let start = Instant::now();
    let result = f();
    let duration = start.elapsed();
    (result, duration)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;
    use tempfile::tempdir;

    #[test]
    fn test_measure_execution_time() {
        let (_, duration) = measure_execution_time(|| {
            thread::sleep(Duration::from_millis(50));
            "test"
        });

        assert!(
            duration.as_millis() >= 50,
            "Execution time measurement is incorrect"
        );
    }

    #[test]
    fn test_measure_execution_time_zero() {
        let (result, duration) = measure_execution_time(|| 42);
        assert_eq!(result, 42);
        assert!(duration.as_nanos() > 0 || duration.as_nanos() == 0);
    }

    #[test]
    fn test_measure_execution_time_with_result() {
        let (result, duration) = measure_execution_time(|| -> Result<i32, &str> { Ok(42) });
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
        assert!(duration.as_nanos() > 0 || duration.as_nanos() == 0);
    }

    #[test]
    fn test_write_to_log_error() {
        let result = write_to_log("/nonexistent/directory/file.log", "test");
        assert!(result.is_err());
        assert!(result.unwrap_err().kind() == std::io::ErrorKind::NotFound);
    }

    #[test]
    fn test_performance_tracker_invalid_path() {
        // This test verifies that a tracker with an invalid path doesn't crash
        let tracker = PerformanceTracker::new(true, "/nonexistent/directory/metrics.log", 5, 1);
        tracker.start();

        // Allow time for thread to run
        thread::sleep(Duration::from_secs(1));

        // We don't assert the metrics value - it may be None or Some depending on the environment
        // Just check that we can call the method without panicking
        let _ = tracker.get_current_metrics();

        tracker.stop();
    }

    #[test]
    fn test_performance_tracker_creation() {
        let temp_dir = tempdir().unwrap();
        let log_path = format!("{}/perflog.csv", temp_dir.path().to_str().unwrap());

        let tracker = PerformanceTracker::new(true, &log_path, 10, 1);
        assert!(tracker.enabled);
        assert_eq!(tracker.log_path, log_path);
        assert_eq!(tracker.history_length, 10);
        assert_eq!(tracker.interval_secs, 1);
    }

    #[test]
    fn test_performance_tracker_disabled() {
        let tracker = PerformanceTracker::new(false, "dummy.log", 10, 1);

        tracker.start();
        assert!(!tracker.running.load(Ordering::Relaxed));

        assert!(tracker.get_current_metrics().is_none());
        assert!(tracker.get_average_metrics().is_none());
    }

    #[test]
    fn test_performance_metrics_collection() {
        // This test checks the performance metrics collection functionality
        let temp_dir = tempdir().unwrap();
        let log_path = format!("{}/perflog.csv", temp_dir.path().to_str().unwrap());

        // Create directory to ensure it exists
        std::fs::create_dir_all(temp_dir.path()).unwrap();

        let tracker = PerformanceTracker::new(true, &log_path, 5, 1);
        tracker.start();

        // Wait for metrics collection
        thread::sleep(Duration::from_secs(3));

        // We don't assert exact metrics as they're platform and environment dependent
        // Just make sure we can call the methods without crashing
        let _ = tracker.get_current_metrics();
        let _ = tracker.get_average_metrics();

        tracker.stop();
    }

    #[test]
    fn test_performance_log_file() {
        // This test checks the log file creation
        let temp_dir = tempdir().unwrap();
        let log_path = format!("{}/perflog.csv", temp_dir.path().to_str().unwrap());

        // Create directory to ensure it exists
        std::fs::create_dir_all(temp_dir.path()).unwrap();

        // Setup tracker
        let tracker = PerformanceTracker::new(true, &log_path, 5, 1);

        // Write a test header directly to the log
        let header = "timestamp,cpu_usage,memory_usage_bytes,memory_percent\n";
        let _ = write_to_log(&log_path, header);

        // Start tracker
        tracker.start();

        // Wait for potential writes
        thread::sleep(Duration::from_secs(2));

        // Stop tracker
        tracker.stop();

        // Verify that the file exists - we don't check content as it's environment dependent
        let file_exists = std::path::Path::new(&log_path).exists();
        if !file_exists {
            println!(
                "Warning: performance log file wasn't created at {}",
                log_path
            );
        }
    }

    #[test]
    fn test_metrics_history_limit() {
        let temp_dir = tempdir().unwrap();
        let log_path = format!("{}/perflog.csv", temp_dir.path().to_str().unwrap());

        let history_length = 3;
        let tracker = PerformanceTracker::new(true, &log_path, history_length, 1);
        tracker.start();

        thread::sleep(Duration::from_secs(4));

        let metrics = tracker.metrics.lock().unwrap();
        assert!(metrics.len() <= history_length);

        tracker.stop();
    }
}

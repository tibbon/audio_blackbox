use chrono::prelude::*;
use std::collections::VecDeque;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use sysinfo::{ProcessExt, System, SystemExt};

/// Struct to track performance metrics over time
pub struct PerformanceTracker {
    enabled: bool,
    log_path: String,
    metrics: Arc<Mutex<VecDeque<PerformanceMetrics>>>,
    running: Arc<Mutex<bool>>,
    history_length: usize,
    interval_secs: u64,
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
            running: Arc::new(Mutex::new(false)),
            history_length,
            interval_secs,
        }
    }

    /// Start the performance tracking in a background thread
    pub fn start(&self) {
        if !self.enabled {
            return;
        }

        let mut running = self.running.lock().unwrap();
        if *running {
            return; // Already running
        }
        *running = true;
        drop(running);

        let metrics = Arc::clone(&self.metrics);
        let running = Arc::clone(&self.running);
        let log_path = self.log_path.clone();
        let history_length = self.history_length;
        let interval_secs = self.interval_secs;

        thread::spawn(move || {
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
                eprintln!("Failed to initialize performance log: {}", e);
            }

            // Monitoring loop
            while *running.lock().unwrap() {
                sys.refresh_all();

                if let Some(process) = sys.process(sysinfo::Pid::from(pid as usize)) {
                    let cpu_usage = process.cpu_usage();
                    let memory_usage = process.memory();
                    let memory_percent = (memory_usage as f32 / sys.total_memory() as f32) * 100.0;

                    let metric = PerformanceMetrics {
                        timestamp: Local::now(),
                        cpu_usage,
                        memory_usage,
                        memory_percent,
                    };

                    // Add to metrics queue and limit its size
                    let mut metrics_guard = metrics.lock().unwrap();
                    metrics_guard.push_back(metric.clone());
                    while metrics_guard.len() > history_length {
                        metrics_guard.pop_front();
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
                        eprintln!("Failed to write to performance log: {}", e);
                    }
                }

                thread::sleep(Duration::from_secs(interval_secs));
            }
        });
    }

    /// Stop the performance tracking
    pub fn stop(&self) {
        if !self.enabled {
            return;
        }

        let mut running = self.running.lock().unwrap();
        *running = false;
    }

    /// Get the current performance metrics
    pub fn get_current_metrics(&self) -> Option<PerformanceMetrics> {
        if !self.enabled {
            return None;
        }

        let metrics = self.metrics.lock().unwrap();
        metrics.back().cloned()
    }

    /// Get the average performance metrics over the tracked history
    pub fn get_average_metrics(&self) -> Option<PerformanceMetrics> {
        if !self.enabled {
            return None;
        }

        let metrics = self.metrics.lock().unwrap();
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

/// Helper function to write to the log file
fn write_to_log(path: &str, content: &str) -> std::io::Result<()> {
    let file = OpenOptions::new().create(true).append(true).open(path)?;

    let mut writer = std::io::BufWriter::new(file);
    writer.write_all(content.as_bytes())?;
    writer.flush()?;

    Ok(())
}

/// Measures elapsed time for a function and returns the duration
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
    use std::fs;
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
        let (result, duration) = measure_execution_time(|| -> Result<i32, &str> {
            Ok(42)
        });
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
        let tracker = PerformanceTracker::new(true, "/nonexistent/directory/metrics.log", 5, 1);
        tracker.start();
        
        thread::sleep(Duration::from_secs(1));
        assert!(tracker.get_current_metrics().is_some());
        
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
        assert!(!*tracker.running.lock().unwrap());
        
        assert!(tracker.get_current_metrics().is_none());
        assert!(tracker.get_average_metrics().is_none());
    }

    #[test]
    fn test_performance_metrics_collection() {
        let temp_dir = tempdir().unwrap();
        let log_path = format!("{}/perflog.csv", temp_dir.path().to_str().unwrap());
        
        let tracker = PerformanceTracker::new(true, &log_path, 5, 1);
        tracker.start();
        
        thread::sleep(Duration::from_secs(2));
        
        let current = tracker.get_current_metrics();
        assert!(current.is_some());
        let metrics = current.unwrap();
        assert!(metrics.cpu_usage >= 0.0);
        assert!(metrics.memory_usage > 0);
        assert!(metrics.memory_percent >= 0.0);
        
        let average = tracker.get_average_metrics();
        assert!(average.is_some());
        
        tracker.stop();
        assert!(!*tracker.running.lock().unwrap());
    }

    #[test]
    fn test_performance_log_file() {
        let temp_dir = tempdir().unwrap();
        let log_path = format!("{}/perflog.csv", temp_dir.path().to_str().unwrap());
        
        let tracker = PerformanceTracker::new(true, &log_path, 5, 1);
        tracker.start();
        
        thread::sleep(Duration::from_secs(2));
        tracker.stop();
        
        assert!(fs::metadata(&log_path).is_ok());
        let content = fs::read_to_string(&log_path).unwrap();
        assert!(content.contains("timestamp,cpu_usage,memory_usage_bytes,memory_percent"));
        assert!(content.lines().count() > 2);
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

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use blackbox::AppConfig;
use blackbox::AudioRecorder;
use blackbox::CpalAudioProcessor;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
use crate::macos::MenuBarApp;

fn main() {
    // Check if we should run the macOS menu bar app
    #[cfg(target_os = "macos")]
    {
        if let Some(arg) = env::args().nth(1) {
            if arg == "--menu-bar" {
                let menu_app = MenuBarApp::new();
                menu_app.run();
                return;
            }
        }
    }

    // Check for configuration file
    let config_path = Path::new("blackbox.toml");
    if !config_path.exists() {
        println!("Configuration file not found, creating default at blackbox.toml");
        let default_config = AppConfig::default();
        if let Err(e) = default_config.create_config_file("blackbox.toml") {
            eprintln!("Failed to create configuration file: {}", e);
            return;
        }
    }

    // Load configuration
    let config = AppConfig::load();

    // Create output directory if it doesn't exist
    let output_dir = config.get_output_dir();
    if !Path::new(&output_dir).exists() {
        if let Err(e) = fs::create_dir_all(&output_dir) {
            eprintln!("Failed to create output directory: {}", e);
            return;
        }
        println!("Created output directory: {}", output_dir);
    }

    // Set up performance monitoring if enabled
    let mut perf_monitor = None;
    if config.get_performance_logging() {
        println!("Performance monitoring enabled");
        perf_monitor = Some(PerformanceMonitor::new());
    }

    // Set up signal handling for clean shutdown
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ::ctrlc::set_handler(move || {
        println!("Shutting down...");
        r.store(false, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");

    // Create processor and recorder
    let processor = match CpalAudioProcessor::new() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Failed to create audio processor: {}", e);
            return;
        }
    };

    let mut recorder = AudioRecorder::with_config(processor, config.clone());

    // Create macOS menu bar app if we're on macOS
    #[cfg(target_os = "macos")]
    let menu_app = MenuBarApp::new();

    // Continuous recording mode
    if config.get_continuous_mode() {
        println!("Starting in continuous recording mode");

        #[cfg(target_os = "macos")]
        menu_app.update_status(true);

        match recorder.start_recording() {
            Ok(_) => println!("Recording started!"),
            Err(e) => {
                eprintln!("Failed to start recording: {}", e);
                return;
            }
        }

        // Main loop
        println!("Press Ctrl+C to stop recording");
        while running.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_secs(1));

            // Check system resources if performance monitoring is enabled
            if let Some(ref mut monitor) = perf_monitor {
                let metrics = monitor.get_metrics();
                if metrics.cpu_usage > 80.0 {
                    eprintln!("Warning: High CPU usage: {:.1}%", metrics.cpu_usage);
                }
                if metrics.memory_usage > 80.0 {
                    eprintln!("Warning: High memory usage: {:.1}%", metrics.memory_usage);
                }
            }
        }

        // Stop recording
        println!("Stopping recording...");

        #[cfg(target_os = "macos")]
        menu_app.update_status(false);
    } else {
        // Normal recording mode
        println!("Starting single recording");

        #[cfg(target_os = "macos")]
        menu_app.update_status(true);

        match recorder.start_recording() {
            Ok(_) => println!("Recording started!"),
            Err(e) => {
                eprintln!("Failed to start recording: {}", e);
                return;
            }
        }

        // Wait for the recording duration
        let duration_secs = config.get_duration();
        println!("Recording for {} seconds...", duration_secs);

        let mut remaining = duration_secs;
        while remaining > 0 && running.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_secs(1));
            remaining -= 1;

            // Check system resources if performance monitoring is enabled
            if let Some(ref mut monitor) = perf_monitor {
                let metrics = monitor.get_metrics();
                if metrics.cpu_usage > 80.0 {
                    eprintln!("Warning: High CPU usage: {:.1}%", metrics.cpu_usage);
                }
                if metrics.memory_usage > 80.0 {
                    eprintln!("Warning: High memory usage: {:.1}%", metrics.memory_usage);
                }
            }

            if remaining % 5 == 0 && remaining > 0 {
                println!("{} seconds remaining...", remaining);
            }
        }

        // Stop recording
        println!("Stopping recording...");

        #[cfg(target_os = "macos")]
        menu_app.update_status(false);
    }

    println!("Recording finished!");
}

// A simple performance monitor
struct PerformanceMonitor {
    metrics: HashMap<String, f32>,
}

struct PerformanceMetrics {
    cpu_usage: f32,
    memory_usage: f32,
}

impl PerformanceMonitor {
    fn new() -> Self {
        PerformanceMonitor {
            metrics: HashMap::new(),
        }
    }

    fn get_metrics(&mut self) -> PerformanceMetrics {
        // In a real implementation, this would measure actual system metrics
        // For now, we just return simulated values

        // Simulate CPU usage between 10-90%
        let cpu = 10.0
            + (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                % 80) as f32;

        // Simulate memory usage between 20-70%
        let mem = 20.0
            + (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                % 50) as f32;

        self.metrics.insert("cpu".to_string(), cpu);
        self.metrics.insert("memory".to_string(), mem);

        PerformanceMetrics {
            cpu_usage: cpu,
            memory_usage: mem,
        }
    }
}

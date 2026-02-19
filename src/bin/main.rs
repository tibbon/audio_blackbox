#![allow(clippy::too_many_lines)]
#![allow(clippy::redundant_clone)]
#![allow(clippy::useless_let_if_seq)]
#![allow(clippy::needless_collect)]
#![allow(clippy::branches_sharing_code)]
#![allow(clippy::use_self)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::needless_pass_by_ref_mut)]

use std::env;
use std::fs;
use std::path::Path;
#[cfg(not(target_os = "macos"))]
use std::process;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use blackbox::AppConfig;
use blackbox::AudioProcessor;
use blackbox::AudioRecorder;
use blackbox::CpalAudioProcessor;
use blackbox::PerformanceTracker;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
use crate::macos::MenuBarApp;

fn main() {
    // Check if we should run the macOS menu bar app
    let args: Vec<String> = env::args().collect();
    if args.contains(&"--menu-bar".to_string()) {
        println!("Menu bar flag detected, starting in macOS menu bar mode");

        #[cfg(target_os = "macos")]
        {
            println!("Creating MenuBarApp instance...");
            let mut menu_app = MenuBarApp::new();
            println!("Menu bar app created successfully");
            println!("Running MenuBarApp...");
            menu_app.run();
        }

        #[cfg(not(target_os = "macos"))]
        {
            eprintln!("Menu bar mode is only available on macOS");
            process::exit(1);
        }

        return;
    }

    // Check for configuration file
    let config_path = Path::new("blackbox.toml");
    if !config_path.exists() {
        println!("Configuration file not found, creating default at blackbox.toml");
        let default_config = AppConfig::default();
        if let Err(e) = default_config.create_config_file("blackbox.toml") {
            eprintln!("Failed to create configuration file: {e}");
            return;
        }
    }

    // Load configuration once at startup
    let config = AppConfig::load();
    println!("Loaded configuration from {}", config_path.display());

    // Create output directory if it doesn't exist
    let output_dir = config.get_output_dir();
    if !Path::new(&output_dir).exists() {
        if let Err(e) = fs::create_dir_all(&output_dir) {
            eprintln!("Failed to create output directory: {e}");
            return;
        }
        println!("Created output directory: {output_dir}");
    }

    // Set up performance monitoring using the real PerformanceTracker
    let perf_tracker = if config.get_performance_logging() {
        println!("Performance monitoring enabled");
        let log_path = format!("{output_dir}/performance.log");
        let tracker = PerformanceTracker::new(true, &log_path, 60, 5);
        tracker.start();
        Some(tracker)
    } else {
        None
    };

    // Set up signal handling for clean shutdown
    let running = Arc::new(AtomicBool::new(true));
    let shutdown_in_progress = Arc::new(AtomicBool::new(false));
    let r = running.clone();
    let s = shutdown_in_progress.clone();
    ::ctrlc::set_handler(move || {
        if !s.load(Ordering::SeqCst) {
            println!("Shutting down...");
            s.store(true, Ordering::SeqCst);
            r.store(false, Ordering::SeqCst);
        }
    })
    .expect("Error setting Ctrl-C handler");

    // Create processor and recorder
    let processor = match CpalAudioProcessor::new() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Failed to create audio processor: {e}");
            return;
        }
    };

    let mut recorder = AudioRecorder::with_config(processor, config.clone());

    // Create macOS menu bar app if we're on macOS
    #[cfg(target_os = "macos")]
    #[cfg(feature = "menu-bar")]
    let mut menu_app = MenuBarApp::new();

    // Start recording
    let mode_label = if config.get_continuous_mode() {
        "continuous"
    } else {
        "single"
    };
    println!("Starting {mode_label} recording");

    #[cfg(target_os = "macos")]
    #[cfg(feature = "menu-bar")]
    menu_app.update_status(true);

    match recorder.start_recording() {
        Ok(_) => println!("Recording started!"),
        Err(e) => {
            eprintln!("Failed to start recording: {e}");
            return;
        }
    }

    // Main recording loop
    println!("Press Ctrl+C to stop recording");
    let duration_secs = if config.get_continuous_mode() {
        0 // 0 means unlimited
    } else {
        config.get_duration()
    };

    if duration_secs > 0 {
        println!("Recording for {duration_secs} seconds...");
    }

    let mut elapsed: u64 = 0;
    while running.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_secs(1));
        elapsed += 1;

        // Check system resources if performance monitoring is enabled
        if let Some(ref tracker) = perf_tracker
            && let Some(metrics) = tracker.get_current_metrics()
        {
            if metrics.cpu_usage > 80.0 {
                eprintln!("Warning: High CPU usage: {:.1}%", metrics.cpu_usage);
            }
            if metrics.memory_percent > 80.0 {
                eprintln!("Warning: High memory usage: {:.1}%", metrics.memory_percent);
            }
        }

        // For fixed-duration mode, check if time is up and print remaining
        if duration_secs > 0 {
            if elapsed >= duration_secs {
                break;
            }
            let remaining = duration_secs - elapsed;
            if remaining > 0 && remaining.is_multiple_of(5) {
                println!("{remaining} seconds remaining...");
            }
        }
    }

    // Stop recording
    println!("Stopping recording...");

    #[cfg(target_os = "macos")]
    #[cfg(feature = "menu-bar")]
    menu_app.update_status(false);

    // Finalize the recording
    if let Err(e) = recorder.processor.finalize() {
        eprintln!("Error finalizing recording: {e}");
    }

    // Stop performance tracking
    if let Some(ref tracker) = perf_tracker {
        tracker.stop();
    }

    println!("Recording finished!");
    std::process::exit(0);
}

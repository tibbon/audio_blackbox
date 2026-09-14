//! `blackbox` — CLI entry point.
//!
//! Distinct from the `SwiftUI` app (which calls Rust via FFI). The CLI:
//! 1. Loads `blackbox.toml` (creates a default if missing), applies
//!    `BLACKBOX_*` env overrides on top.
//! 2. Installs a Ctrl-C handler with a `shutdown_in_progress` debounce
//!    so double-tap doesn't fan out work.
//! 3. Creates a `CpalAudioProcessor`, wraps it in an `AudioRecorder`,
//!    and runs until duration expires or Ctrl-C fires.
//! 4. Finalizes the recording explicitly before exit.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use log::{error, info, warn};

use blackbox::AppConfig;
use blackbox::AudioProcessor;
use blackbox::AudioRecorder;
use blackbox::CpalAudioProcessor;
#[cfg(feature = "benchmarking")]
use blackbox::PerformanceTracker;

fn main() {
    env_logger::init();

    let Some(config) = load_config() else {
        return;
    };

    // Set up performance monitoring using the real PerformanceTracker
    #[cfg(feature = "benchmarking")]
    let perf_tracker = config.get_performance_logging().then(|| {
        info!("Performance monitoring enabled");
        let log_path = format!("{}/performance.log", config.get_output_dir());
        let tracker = PerformanceTracker::new(true, &log_path, 60, 5);
        tracker.start();
        tracker
    });

    let running = install_shutdown_handler();

    // Create processor and recorder
    let processor = match CpalAudioProcessor::new() {
        Ok(p) => p,
        Err(e) => {
            error!("Failed to create audio processor: {e}");
            return;
        }
    };

    let mut recorder = AudioRecorder::with_config(processor, config.clone());

    // Start recording
    let mode_label = if config.get_continuous_mode() {
        "continuous"
    } else {
        "single"
    };
    info!("Starting {mode_label} recording");

    match recorder.start_recording() {
        Ok(_) => info!("Recording started!"),
        Err(e) => {
            error!("Failed to start recording: {e}");
            return;
        }
    }

    // Main recording loop
    info!("Press Ctrl+C to stop recording");
    let duration_secs = if config.get_continuous_mode() {
        0 // 0 means unlimited
    } else {
        config.get_duration()
    };

    if duration_secs > 0 {
        info!("Recording for {duration_secs} seconds...");
    }

    run_until_stopped(&running, duration_secs, || {
        // Check system resources if performance monitoring is enabled
        #[cfg(feature = "benchmarking")]
        if let Some(tracker) = &perf_tracker {
            warn_on_high_usage(tracker);
        }
    });

    // Stop recording
    info!("Stopping recording...");

    // Finalize the recording
    if let Err(e) = recorder.processor_mut().finalize() {
        error!("Error finalizing recording: {e}");
    }

    // Stop performance tracking
    #[cfg(feature = "benchmarking")]
    if let Some(ref tracker) = perf_tracker {
        tracker.stop();
    }

    info!("Recording finished!");
    // DOLL-205: return normally instead of `std::process::exit(0)`.
    // `exit` skips destructors on the stack-rooted `recorder` —
    // notably `SilenceCheckWorker::Drop`, which closes the send side
    // of the silence-check channel and joins the worker. Without that
    // join, a Ctrl-C during recording with `silence_threshold > 0` and
    // pending silence checks on rotated files would cut those checks
    // short, leaving silent files on disk that should have been
    // auto-deleted. Returning from `main` runs all destructors in
    // reverse declaration order.
}

/// Create `blackbox.toml` if it's missing, load the configuration, and create
/// its output directory. Logs and returns `None` if either file step fails.
fn load_config() -> Option<AppConfig> {
    let config_path = Path::new("blackbox.toml");
    if !config_path.exists() {
        info!("Configuration file not found, creating default at blackbox.toml");
        let default_config = AppConfig::default();
        if let Err(e) = default_config.create_config_file("blackbox.toml") {
            error!("Failed to create configuration file: {e}");
            return None;
        }
    }

    // Load configuration once at startup
    let config = AppConfig::load();
    info!("Loaded configuration from {}", config_path.display());

    // Create output directory if it doesn't exist
    let output_dir = config.get_output_dir();
    if !Path::new(&output_dir).exists() {
        if let Err(e) = fs::create_dir_all(&output_dir) {
            error!("Failed to create output directory: {e}");
            return None;
        }
        info!("Created output directory: {output_dir}");
    }

    Some(config)
}

/// Install the Ctrl-C handler and return the flag it clears.
fn install_shutdown_handler() -> Arc<AtomicBool> {
    let running = Arc::new(AtomicBool::new(true));
    let r = Arc::clone(&running);
    let s = Arc::new(AtomicBool::new(false));
    if let Err(e) = ::ctrlc::set_handler(move || {
        // Status flags only — single-bit signal, no synchronizes-with payload.
        if !s.load(Ordering::Relaxed) {
            info!("Shutting down...");
            s.store(true, Ordering::Relaxed);
            r.store(false, Ordering::Relaxed);
        }
    }) {
        // ctrlc::set_handler errors when called twice in a process or when
        // the underlying signal install fails. Log and continue: the CLI
        // still runs to completion via the duration timer; only graceful
        // Ctrl-C shutdown is degraded (DOLL-115).
        warn!("Failed to install Ctrl-C handler ({e}); shutdown will rely on the duration timer.");
    }
    running
}

/// Tick once a second until Ctrl-C clears `running` or `duration_secs`
/// elapse (0 = unlimited), calling `on_tick` every second and logging the
/// time left every 5 seconds.
fn run_until_stopped(running: &AtomicBool, duration_secs: u64, mut on_tick: impl FnMut()) {
    let mut elapsed: u64 = 0;
    while running.load(Ordering::Relaxed) {
        #[expect(
            clippy::disallowed_methods,
            reason = "CLI status loop ticks once per second; there is nothing to wait on but the clock"
        )]
        thread::sleep(Duration::from_secs(1));
        elapsed += 1;

        on_tick();

        // For fixed-duration mode, check if time is up and print remaining
        if duration_secs > 0 {
            if elapsed >= duration_secs {
                break;
            }
            let remaining = duration_secs - elapsed;
            if remaining > 0 && remaining.is_multiple_of(5) {
                info!("{remaining} seconds remaining...");
            }
        }
    }
}

/// Warn when the process is using more than 80% of CPU or memory.
#[cfg(feature = "benchmarking")]
fn warn_on_high_usage(tracker: &PerformanceTracker) {
    if let Some(metrics) = tracker.get_current_metrics() {
        if metrics.cpu_usage > 80.0 {
            warn!("High CPU usage: {:.1}%", metrics.cpu_usage);
        }
        if metrics.memory_percent > 80.0 {
            warn!("High memory usage: {:.1}%", metrics.memory_percent);
        }
    }
}

//! `blackbox` — CLI entry point.
//!
//! Distinct from the `SwiftUI` app (which calls Rust via FFI). The CLI:
//! 1. Loads the configuration file (`BLACKBOX_CONFIG`, else the search order
//!    in `AppConfig::find_config_file`; creates a default if there is none),
//!    applies `BLACKBOX_*` env overrides on top.
//! 2. Installs a Ctrl-C / SIGTERM / SIGHUP handler with a debounce
//!    so double-tap doesn't fan out work.
//! 3. Recovers `.recording.wav` files a crash left in the output directory
//!    (`blackbox::recover_recordings`).
//! 4. Creates a `CpalAudioProcessor`, wraps it in an `AudioRecorder`,
//!    and runs until duration expires, a signal arrives, or the engine
//!    reports a failure (disk full, write failure, stream error).
//! 5. Finalizes the recording explicitly before exit.
//!
//! Exits with status 0 only when the recording ran and finalized cleanly;
//! any failure exits non-zero so scripts and service managers can tell.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
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

fn main() -> ExitCode {
    env_logger::init();

    let Some(config) = load_config() else {
        return ExitCode::FAILURE;
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

    // Repair and rename takes a crash left under `.recording.wav` names.
    // Nothing is recording into the directory yet, which recovery requires.
    let output_dir = config.get_output_dir();
    match blackbox::recover_recordings(&output_dir) {
        Ok(0) => {}
        Ok(n) => info!("Recovered {n} interrupted recording(s) in {output_dir}"),
        Err(e) => warn!("Could not scan {output_dir} for interrupted recordings: {e}"),
    }

    let Some(mut recorder) = start_recorder(&config) else {
        return ExitCode::FAILURE;
    };

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

    let loop_failure = run_until_stopped(&running, duration_secs, || {
        // Check system resources if performance monitoring is enabled
        #[cfg(feature = "benchmarking")]
        if let Some(tracker) = &perf_tracker {
            warn_on_high_usage(tracker);
        }
        engine_failure(recorder.get_processor())
    });

    // Stop recording
    info!("Stopping recording...");

    // Finalize the recording
    let finalized = match recorder.processor_mut().finalize() {
        Ok(()) => true,
        Err(e) => {
            error!("Error finalizing recording: {e}");
            false
        }
    };
    // A flag can trip between the last check and the stop.
    let failure = loop_failure.or_else(|| engine_failure(recorder.get_processor()));
    if let Some(reason) = failure {
        error!("Recording stopped early: {reason}");
    }

    // Stopping doesn't wait for silence checks (the app must not block its
    // main thread on them), so the CLI waits here: exiting now would keep
    // silent files that should have been deleted.
    if !blackbox::wait_for_silence_checks(SILENCE_CHECK_WAIT) {
        warn!(
            "Silence checks still running after {SILENCE_CHECK_WAIT:?}; unchecked files are kept"
        );
    }

    // Stop performance tracking
    #[cfg(feature = "benchmarking")]
    if let Some(ref tracker) = perf_tracker {
        tracker.stop();
    }

    // DOLL-205: return normally instead of `std::process::exit`, so
    // destructors on the stack-rooted `recorder` run.
    if finalized && failure.is_none() {
        info!("Recording finished!");
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Create the processor and recorder and start recording. Logs and returns
/// `None` on failure.
fn start_recorder(config: &AppConfig) -> Option<AudioRecorder<CpalAudioProcessor>> {
    let processor = match CpalAudioProcessor::new() {
        Ok(p) => p,
        Err(e) => {
            error!("Failed to create audio processor: {e}");
            return None;
        }
    };

    let mut recorder = AudioRecorder::with_config(processor, config.clone());

    let mode_label = if config.get_continuous_mode() {
        "continuous"
    } else {
        "single"
    };
    info!("Starting {mode_label} recording");

    match recorder.start_recording() {
        Ok(_) => {
            info!("Recording started!");
            Some(recorder)
        }
        Err(e) => {
            error!("Failed to start recording: {e}");
            None
        }
    }
}

/// How long the CLI waits on exit for queued silence checks. A check
/// decodes a silent file to its end, so a long silent take needs a while.
const SILENCE_CHECK_WAIT: Duration = Duration::from_secs(600);

/// Load the configuration, creating a default file first if none exists, and
/// create its output directory. Logs and returns `None` if either file step
/// fails.
///
/// The default goes where `BLACKBOX_CONFIG` points, or `./blackbox.toml`
/// when it is unset. A config found through `BLACKBOX_CONFIG` or the search
/// order is never shadowed by a new `./blackbox.toml`.
fn load_config() -> Option<AppConfig> {
    let requested = std::env::var_os("BLACKBOX_CONFIG").map(PathBuf::from);
    if AppConfig::find_config_file().is_none() {
        let path = requested.unwrap_or_else(|| PathBuf::from("blackbox.toml"));
        info!(
            "Configuration file not found, creating default at {}",
            path.display()
        );
        let created = path.to_str().map_or_else(
            || Err(format!("{} is not valid UTF-8", path.display())),
            |p| {
                AppConfig::default()
                    .create_config_file(p)
                    .map_err(|e| e.to_string())
            },
        );
        if let Err(e) = created {
            error!("Failed to create configuration file: {e}");
            return None;
        }
    } else if let Some(path) = requested.filter(|p| !p.exists()) {
        warn!(
            "BLACKBOX_CONFIG={} does not exist; using the next configuration file found",
            path.display()
        );
    }

    // Load configuration once at startup; `load` logs which file it read.
    let config = AppConfig::load();

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

/// Install the handler for Ctrl-C (SIGINT), SIGTERM and SIGHUP (ctrlc's
/// `termination` feature) and return the flag it clears, so each of them
/// stops and finalizes the recording.
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

/// Why the engine can no longer record, if it can't: the flags the app's
/// status poll turns into a stop and a notification.
fn engine_failure(processor: &impl AudioProcessor) -> Option<&'static str> {
    if processor.write_failed() {
        Some("writing to disk kept failing (disk full or output directory unwritable)")
    } else if processor.disk_space_low() {
        Some("free disk space fell below min_disk_space_mb")
    } else if processor.stream_error() {
        Some("the audio stream reported an error (device disconnected?)")
    } else {
        None
    }
}

/// Tick once a second until a signal clears `running`, `duration_secs`
/// elapse (0 = unlimited), or `check` reports a failure, which is returned.
/// `check` runs before every tick; the time left is logged every 5 seconds.
fn run_until_stopped(
    running: &AtomicBool,
    duration_secs: u64,
    mut check: impl FnMut() -> Option<&'static str>,
) -> Option<&'static str> {
    let mut elapsed: u64 = 0;
    while running.load(Ordering::Relaxed) {
        if let Some(reason) = check() {
            return Some(reason);
        }
        if duration_secs > 0 && elapsed >= duration_secs {
            break;
        }
        #[expect(
            clippy::disallowed_methods,
            reason = "CLI status loop ticks once per second; there is nothing to wait on but the clock"
        )]
        thread::sleep(Duration::from_secs(1));
        elapsed += 1;

        let remaining = duration_secs.saturating_sub(elapsed);
        if remaining > 0 && remaining.is_multiple_of(5) {
            info!("{remaining} seconds remaining...");
        }
    }
    None
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

#[cfg(test)]
mod tests {
    use super::{engine_failure, run_until_stopped};
    use blackbox::{AppConfig, AudioProcessor, BlackboxError, OutputMode};
    use std::sync::atomic::AtomicBool;

    /// A processor that only reports status flags.
    #[derive(Default)]
    struct Flags {
        write_failed: bool,
        disk_space_low: bool,
        stream_error: bool,
    }

    impl AudioProcessor for Flags {
        fn process_audio(
            &mut self,
            _: &[usize],
            _: OutputMode,
            _: bool,
            _: &AppConfig,
        ) -> Result<(), BlackboxError> {
            Ok(())
        }
        fn finalize(&mut self) -> Result<(), BlackboxError> {
            Ok(())
        }
        fn start_recording(&mut self, _: &AppConfig) -> Result<(), BlackboxError> {
            Ok(())
        }
        fn stop_recording(&mut self) -> Result<(), BlackboxError> {
            Ok(())
        }
        fn is_recording(&self) -> bool {
            true
        }
        fn write_failed(&self) -> bool {
            self.write_failed
        }
        fn disk_space_low(&self) -> bool {
            self.disk_space_low
        }
        fn stream_error(&self) -> bool {
            self.stream_error
        }
    }

    /// Each flag the app treats as "recording stopped" is a CLI failure.
    #[test]
    fn engine_failure_reports_each_flag() {
        assert_eq!(engine_failure(&Flags::default()), None);
        for flags in [
            Flags {
                write_failed: true,
                ..Flags::default()
            },
            Flags {
                disk_space_low: true,
                ..Flags::default()
            },
            Flags {
                stream_error: true,
                ..Flags::default()
            },
        ] {
            assert!(engine_failure(&flags).is_some());
        }
    }

    /// A failure ends the loop at once (before the first one-second tick)
    /// and is returned so `main` exits non-zero. Previously the loop only
    /// watched the signal flag and the timer.
    #[test]
    fn run_until_stopped_returns_the_failure() {
        let running = AtomicBool::new(true);
        let reason = run_until_stopped(&running, 0, || Some("disk full"));
        assert_eq!(reason, Some("disk full"));
    }

    /// A signal (flag cleared) is a clean stop.
    #[test]
    fn run_until_stopped_is_clean_after_a_signal() {
        let running = AtomicBool::new(false);
        assert_eq!(run_until_stopped(&running, 0, || None), None);
    }
}

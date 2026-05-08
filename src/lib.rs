// BlackBox Audio Recorder — Copyright (C) 2023-2026, David Fisher
// Licensed under the Business Source License 1.1 (BUSL-1.1). See LICENSE.

// Lint configuration: keep pedantic/nursery suppressions that match codebase patterns.
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_lossless)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::uninlined_format_args)]
#![allow(clippy::cognitive_complexity)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::significant_drop_in_scrutinee)]
#![allow(clippy::needless_pass_by_value)]
#![allow(clippy::use_self)]
#![allow(clippy::redundant_else)]

mod audio_processor;
mod audio_recorder;
#[cfg(feature = "benchmarking")]
mod benchmarking;
mod config;
mod constants;
mod cpal_processor;
pub mod error;
#[cfg(feature = "ffi")]
pub mod ffi;
#[cfg(target_os = "macos")]
mod macos_sample_rate_listener;
#[cfg(test)]
mod mock_processor;
mod raw_wav_writer;
mod silence_check_worker;
mod utils;
mod writer_thread;

// Only include test_utils in test builds
#[cfg(test)]
pub mod test_utils;

// ----------------------------------------------------------------------------
// Public API re-exports
// ----------------------------------------------------------------------------
// These are consumed by the binaries (src/bin/) and external Rust crates.
// Items used only inside this crate use pub(crate) re-exports below.
pub use audio_processor::AudioProcessor;
pub use audio_recorder::AudioRecorder;
#[cfg(feature = "benchmarking")]
pub use benchmarking::PerformanceTracker;
pub use config::AppConfig;
pub use constants::{OutputMode, RING_BUFFER_SECONDS};
pub use cpal_processor::CpalAudioProcessor;
pub use error::BlackboxError;

// ----------------------------------------------------------------------------
// Crate-internal re-exports
// ----------------------------------------------------------------------------
// Used inside this crate (tests bring them in via `use super::*`; submodules
// reference them through the lib root). Demoted from pub to pub(crate) — pub
// is a SemVer contract, and these are impl details no external caller had a
// reason to touch.
#[cfg(feature = "benchmarking")]
#[allow(unused_imports)]
pub(crate) use benchmarking::{PerformanceMetrics, measure_execution_time};
#[allow(unused_imports)]
pub(crate) use constants::{
    CacheAlignedPeak, DEFAULT_BITS_PER_SAMPLE, DEFAULT_CHANNELS, DEFAULT_CONTINUOUS_MODE,
    DEFAULT_DEBUG, DEFAULT_DURATION, DEFAULT_MIN_DISK_SPACE_MB, DEFAULT_OUTPUT_DIR,
    DEFAULT_OUTPUT_MODE, DEFAULT_PERFORMANCE_LOGGING, DEFAULT_RECORDING_CADENCE,
    DEFAULT_SILENCE_GATE_ENABLED, DEFAULT_SILENCE_GATE_TIMEOUT_SECS, DEFAULT_SILENCE_THRESHOLD,
    MAX_CHANNELS, WRITER_THREAD_READ_CHUNK,
};
#[allow(unused_imports)]
pub(crate) use utils::{
    available_disk_space_mb, check_alsa_availability, is_silent, parse_channel_string,
};

// Expose test utilities
#[cfg(test)]
pub use mock_processor::MockAudioProcessor;

// ---------------------------------------------------------------------------
// Test-only allocation counter — wraps the system allocator with atomic
// counters so we can prove the hot path does zero heap allocations.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod alloc_counter {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::atomic::{AtomicU64, Ordering};

    static ALLOC_COUNT: AtomicU64 = AtomicU64::new(0);

    pub struct CountingAllocator;

    // SAFETY: `CountingAllocator` is a transparent wrapper around `System`.
    // Each method delegates directly with the same `Layout`/`ptr`
    // arguments, preserving every invariant `GlobalAlloc` requires.
    // The added `fetch_add` only writes to a `AtomicU64`, which has no
    // safety implications for the allocator contract.
    unsafe impl GlobalAlloc for CountingAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
            // SAFETY: forwards `layout` unmodified to `System::alloc`,
            // whose contract we satisfy by virtue of being called from
            // a `GlobalAlloc::alloc` impl.
            unsafe { System.alloc(layout) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            // SAFETY: caller's `GlobalAlloc::dealloc` contract pinned `ptr`
            // to a previous `alloc` of the same `layout`; we forward both
            // unmodified.
            unsafe { System.dealloc(ptr, layout) }
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
            // SAFETY: same as `dealloc` plus `new_size` validity — we
            // forward all three args unmodified to `System::realloc`.
            unsafe { System.realloc(ptr, layout, new_size) }
        }
    }

    /// Snapshot the current global allocation count.
    pub fn snapshot() -> u64 {
        ALLOC_COUNT.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
#[global_allocator]
static COUNTING_ALLOCATOR: alloc_counter::CountingAllocator = alloc_counter::CountingAllocator;

#[cfg(test)]
mod tests {
    use super::*;
    use mock_processor::MockAudioProcessor;
    use std::env;
    use std::path::Path;
    use tempfile::tempdir;

    mod alloc_tests;
    mod benchmark_tests;
    mod channel_tests;
    mod config_tests;
    mod cpal_integration_tests;
    mod error_tests;
    #[cfg(feature = "ffi")]
    mod ffi_tests;
    #[cfg(feature = "benchmarking")]
    mod performance_tests;
    mod recorder_tests;
    mod ring_buffer_tests;
    mod shutdown_tests;
    mod silence_gate_tests;
    mod silence_tests;

    // Check if we're running in CI
    fn is_ci() -> bool {
        env::var("CI").is_ok() || env::var("GITHUB_ACTIONS").is_ok()
    }

    /// Re-export the consolidated helper (DOLL-118) for inline tests below.
    use crate::test_utils::default_test_env;

    // Test environment variable handling
    #[test]
    fn test_environment_variable_handling() {
        temp_env::with_vars(default_test_env(), || {
            // Test channels parsing
            assert_eq!(parse_channel_string("0,1").unwrap(), vec![0, 1]);
            assert_eq!(parse_channel_string("0-3").unwrap(), vec![0, 1, 2, 3]);
            assert_eq!(
                parse_channel_string("0,2-4,7").unwrap(),
                vec![0, 2, 3, 4, 7]
            );

            // Test bool parsing
            assert!("true".parse::<bool>().unwrap_or(false));
            assert!(!"false".parse::<bool>().unwrap_or(false));

            // Test duration parsing
            assert_eq!("20".parse::<u64>().unwrap_or(DEFAULT_DURATION), 20);
        });
    }

    #[test]
    fn test_recorder_basic_functionality() {
        if is_ci() {
            println!("Skipping audio test in CI environment");
            return;
        }

        temp_env::with_vars(default_test_env(), || {
            let temp_dir = tempdir().unwrap();
            let temp_path = temp_dir.path().to_str().unwrap();
            let file_name = format!("{}/test.wav", temp_path);

            let processor = MockAudioProcessor::new(&file_name);
            let mut recorder = AudioRecorder::new(processor);

            recorder.start_recording().expect("start_recording");

            // Real post-condition: a valid WAV exists on disk and the
            // recorder is configured for the default output mode. The
            // mock's `audio_processed` boolean is set unconditionally and
            // would not catch a recorder that short-circuited.
            let r = hound::WavReader::open(&file_name)
                .expect("recorder should have produced a readable WAV");
            assert_eq!(r.spec().sample_rate, 44100);
            assert!(r.len() > 0, "WAV should contain samples");
            assert_eq!(recorder.get_processor().output_mode, OutputMode::default());
        });
    }

    #[test]
    fn test_channel_parsing() {
        // Channel parsing is pure logic — no env vars needed
        assert_eq!(parse_channel_string("0,1,2").unwrap(), vec![0, 1, 2]);
        assert_eq!(parse_channel_string("0-3").unwrap(), vec![0, 1, 2, 3]);
        assert_eq!(
            parse_channel_string("0,2-4,6").unwrap(),
            vec![0, 2, 3, 4, 6]
        );
        assert_eq!(parse_channel_string("0,0,1,1").unwrap(), vec![0, 1]);
        assert!(parse_channel_string("invalid").is_err());

        let too_many = (0..=MAX_CHANNELS)
            .collect::<Vec<_>>()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        assert!(parse_channel_string(&too_many).is_err());
    }

    #[test]
    fn test_silence_detection() {
        let mut env = default_test_env();
        env.retain(|&(k, _)| k != "SILENCE_THRESHOLD");
        env.push(("SILENCE_THRESHOLD", Some("10")));

        temp_env::with_vars(env, || {
            let temp_dir = tempdir().unwrap();
            let temp_path = temp_dir.path().to_str().unwrap();

            let file_name = format!("{}/silent-test.wav", temp_path);
            let mut processor = MockAudioProcessor::new(&file_name);
            processor.create_silent_file = true;

            let mut recorder = AudioRecorder::new(processor);
            let result = recorder.start_recording();
            assert!(result.is_ok());

            let path = Path::new(&file_name);
            assert!(path.exists(), "Test file should have been created");

            let _ = recorder.processor_mut().finalize();
            assert!(!path.exists(), "Silent file should have been deleted");
        });
    }

    #[test]
    fn test_silence_deletion() {
        let mut env = default_test_env();
        env.retain(|&(k, _)| k != "SILENCE_THRESHOLD");
        env.push(("SILENCE_THRESHOLD", Some("10")));

        temp_env::with_vars(env, || {
            let temp_dir = tempdir().unwrap();
            let temp_path = temp_dir.path().to_str().unwrap();

            let file_name = format!("{}/silent-test.wav", temp_path);
            let mut processor = MockAudioProcessor::new(&file_name);
            processor.create_silent_file = true;

            let mut recorder = AudioRecorder::new(processor);
            let result = recorder.start_recording();
            assert!(result.is_ok());

            let _ = recorder.processor_mut().finalize();

            let path = Path::new(&file_name);
            assert!(!path.exists(), "Silent file should have been deleted");
        });
    }

    #[test]
    fn test_normal_file_not_deleted() {
        let mut env = default_test_env();
        env.retain(|&(k, _)| k != "SILENCE_THRESHOLD");
        env.push(("SILENCE_THRESHOLD", Some("10")));

        temp_env::with_vars(env, || {
            let temp_dir = tempdir().unwrap();
            let temp_path = temp_dir.path().to_str().unwrap();

            let file_name = format!("{}/normal-test.wav", temp_path);
            let mut processor = MockAudioProcessor::new(&file_name);
            processor.create_silent_file = false;

            let mut recorder = AudioRecorder::new(processor);
            let result = recorder.start_recording();
            assert!(result.is_ok());

            let path = Path::new(&file_name);
            assert!(path.exists(), "File should have been created");

            let _ = recorder.processor_mut().finalize();
            assert!(
                path.exists(),
                "Non-silent file should not have been deleted"
            );
        });
    }
}

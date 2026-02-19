// audio_recorder: A cross-platform audio recording library in Rust
// Copyright (C) 2023, David Fisher
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

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

// Modular organization of code
mod audio_processor;
mod audio_recorder;
mod benchmarking;
mod config;
mod constants;
mod cpal_processor;
#[cfg(test)]
mod mock_processor;
mod utils;

// Only include test_utils in test builds
#[cfg(test)]
pub mod test_utils;

// Re-exports for public API
pub use audio_processor::AudioProcessor;
pub use audio_recorder::AudioRecorder;
pub use benchmarking::{measure_execution_time, PerformanceMetrics, PerformanceTracker};
pub use config::AppConfig;
pub use constants::*;
pub use cpal_processor::CpalAudioProcessor;
pub use utils::*;

// Expose test utilities
#[cfg(test)]
pub use mock_processor::MockAudioProcessor;

#[cfg(test)]
mod tests {
    use super::*;
    use mock_processor::MockAudioProcessor;
    use std::env;
    use std::path::Path;
    use tempfile::tempdir;

    // Include shutdown tests
    mod channel_tests;
    mod config_tests;
    mod performance_tests;
    mod shutdown_tests;
    mod silence_tests;

    // Check if we're running in CI
    fn is_ci() -> bool {
        env::var("CI").is_ok() || env::var("GITHUB_ACTIONS").is_ok()
    }

    /// Standard set of env var overrides to isolate tests from the host environment.
    /// Clears all BLACKBOX_* prefixed vars and sets unprefixed vars to defaults.
    fn default_test_env() -> Vec<(&'static str, Option<&'static str>)> {
        vec![
            ("AUDIO_CHANNELS", Some(DEFAULT_CHANNELS)),
            ("DEBUG", Some("false")),
            ("RECORD_DURATION", Some("30")),
            ("OUTPUT_MODE", Some(DEFAULT_OUTPUT_MODE)),
            ("SILENCE_THRESHOLD", Some("0.01")),
            ("CONTINUOUS_MODE", Some("false")),
            ("RECORDING_CADENCE", Some("300")),
            ("OUTPUT_DIR", Some(DEFAULT_OUTPUT_DIR)),
            ("PERFORMANCE_LOGGING", Some("false")),
            ("BLACKBOX_AUDIO_CHANNELS", None),
            ("BLACKBOX_DEBUG", None),
            ("BLACKBOX_DURATION", None),
            ("BLACKBOX_OUTPUT_MODE", None),
            ("BLACKBOX_SILENCE_THRESHOLD", None),
            ("BLACKBOX_CONTINUOUS_MODE", None),
            ("BLACKBOX_RECORDING_CADENCE", None),
            ("BLACKBOX_OUTPUT_DIR", None),
            ("BLACKBOX_PERFORMANCE_LOGGING", None),
            ("BLACKBOX_CONFIG", None),
        ]
    }

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

            // Create a MockAudioProcessor
            let processor = MockAudioProcessor::new(&file_name);

            // Create recorder with custom processor
            let mut recorder = AudioRecorder::new(processor);

            // Start recording with default parameters
            let record_result = recorder.start_recording();
            assert!(record_result.is_ok());

            // Check that our processor got called correctly
            let processor = recorder.get_processor();
            assert!(processor.audio_processed);
            assert_eq!(processor.output_mode, DEFAULT_OUTPUT_MODE);
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

            let _ = recorder.processor.finalize();
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

            let _ = recorder.processor.finalize();

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

            let _ = recorder.processor.finalize();
            assert!(
                path.exists(),
                "Non-silent file should not have been deleted"
            );
        });
    }
}

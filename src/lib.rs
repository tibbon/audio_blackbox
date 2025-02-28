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

// Modular organization of code
mod audio_processor;
mod audio_recorder;
mod benchmarking;
mod config;
mod constants;
mod cpal_processor;
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
    use lazy_static::lazy_static;
    use mock_processor::MockAudioProcessor;
    use std::env;
    use std::path::Path;
    use std::sync::Mutex;
    use tempfile::tempdir;

    // Check if we're running in CI
    fn is_ci() -> bool {
        env::var("CI").is_ok() || env::var("GITHUB_ACTIONS").is_ok()
    }

    // Use a mutex to serialize test executions
    lazy_static! {
        static ref TEST_MUTEX: Mutex<()> = Mutex::new(());
    }

    fn reset_test_env() {
        // Remove environment variables that might affect tests
        env::remove_var("AUDIO_CHANNELS");
        env::remove_var("DEBUG");
        env::remove_var("RECORD_DURATION");
        env::remove_var("OUTPUT_MODE");
        env::remove_var("SILENCE_THRESHOLD");
        env::remove_var("CONTINUOUS_MODE");
        env::remove_var("RECORDING_CADENCE");
        env::remove_var("OUTPUT_DIR");
        env::remove_var("PERFORMANCE_LOGGING");

        // Set environment variables to override any config file settings
        // This ensures tests run with predictable values regardless of config file
        env::set_var("AUDIO_CHANNELS", DEFAULT_CHANNELS);
        env::set_var("DEBUG", DEFAULT_DEBUG.to_string());
        env::set_var("RECORD_DURATION", DEFAULT_DURATION.to_string());
        env::set_var("OUTPUT_MODE", DEFAULT_OUTPUT_MODE);
        env::set_var("SILENCE_THRESHOLD", DEFAULT_SILENCE_THRESHOLD.to_string());
        env::set_var("CONTINUOUS_MODE", DEFAULT_CONTINUOUS_MODE.to_string());
        env::set_var("RECORDING_CADENCE", DEFAULT_RECORDING_CADENCE.to_string());
        env::set_var("OUTPUT_DIR", DEFAULT_OUTPUT_DIR);
        env::set_var(
            "PERFORMANCE_LOGGING",
            DEFAULT_PERFORMANCE_LOGGING.to_string(),
        );
    }

    // Test environment variable handling
    #[test]
    fn test_environment_variable_handling() {
        let lock = TEST_MUTEX.lock();
        if lock.is_err() {
            println!("Mutex was poisoned, creating a new test environment");
            // Continue with the test even if the mutex was poisoned
        }
        reset_test_env();

        // Test channels parsing
        assert_eq!(parse_channel_string("0,1").unwrap(), vec![0, 1]);
        assert_eq!(parse_channel_string("0-3").unwrap(), vec![0, 1, 2, 3]);
        assert_eq!(
            parse_channel_string("0,2-4,7").unwrap(),
            vec![0, 2, 3, 4, 7]
        );

        // Test bool parsing
        assert_eq!("true".parse::<bool>().unwrap_or_else(|_| false), true);
        assert_eq!("false".parse::<bool>().unwrap_or_else(|_| false), false);

        // Test duration parsing
        assert_eq!("20".parse::<u64>().unwrap_or(DEFAULT_DURATION), 20);

        reset_test_env();
    }

    #[test]
    fn test_recorder_basic_functionality() {
        if is_ci() {
            println!("Skipping audio test in CI environment");
            return;
        }

        let lock = TEST_MUTEX.lock();
        if lock.is_err() {
            println!("Mutex was poisoned, creating a new test environment");
            // Continue with the test even if the mutex was poisoned
        }
        reset_test_env();

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

        // Can't really test actual audio recording without hardware,
        // but we can make sure no errors were thrown

        reset_test_env();
    }

    #[test]
    fn test_channel_parsing() {
        let lock = TEST_MUTEX.lock();
        if lock.is_err() {
            println!("Mutex was poisoned, creating a new test environment");
            // Continue with the test even if the mutex was poisoned
        }
        reset_test_env();

        // Test basic channel list
        assert_eq!(parse_channel_string("0,1,2").unwrap(), vec![0, 1, 2]);

        // Test range parsing
        assert_eq!(parse_channel_string("0-3").unwrap(), vec![0, 1, 2, 3]);

        // Test mixed format
        assert_eq!(
            parse_channel_string("0,2-4,6").unwrap(),
            vec![0, 2, 3, 4, 6]
        );

        // Test deduplication
        assert_eq!(parse_channel_string("0,0,1,1").unwrap(), vec![0, 1]);

        // Test error on invalid format
        assert!(parse_channel_string("invalid").is_err());

        // Test error on too many channels
        let too_many = (0..=MAX_CHANNELS)
            .collect::<Vec<_>>()
            .iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(",");
        assert!(parse_channel_string(&too_many).is_err());

        reset_test_env();
    }

    #[test]
    fn test_silence_detection() {
        let lock = TEST_MUTEX.lock();
        if lock.is_err() {
            println!("Mutex was poisoned, creating a new test environment");
            // Continue with the test even if the mutex was poisoned
        }
        reset_test_env();

        // This test creates a silent WAV file and checks if it's detected as silent
        let temp_dir = tempdir().unwrap();
        let temp_path = temp_dir.path().to_str().unwrap();

        // First, create a silent file
        let file_name = format!("{}/silent-test.wav", temp_path);
        let mut processor = MockAudioProcessor::new(&file_name);

        // Configure the mock to create a silent file
        processor.create_silent_file = true;

        // Set the silence threshold to detect silence
        env::set_var("SILENCE_THRESHOLD", "10");

        // Create the recorder with our mock
        let mut recorder = AudioRecorder::new(processor);

        // Start recording
        let result = recorder.start_recording();
        assert!(result.is_ok());

        // Make sure the file was created
        let path = Path::new(&file_name);

        // The file should exist immediately after recording
        assert!(path.exists(), "Test file should have been created");

        // Now manually finalize to trigger silence detection
        let _ = recorder.processor.finalize();

        // The file should now be deleted since it was silent and threshold is set
        assert!(!path.exists(), "Silent file should have been deleted");

        reset_test_env();
    }

    #[test]
    fn test_silence_deletion() {
        let lock = TEST_MUTEX.lock();
        if lock.is_err() {
            println!("Mutex was poisoned, creating a new test environment");
            // Continue with the test even if the mutex was poisoned
        }
        reset_test_env();

        // Set threshold for this test
        env::set_var("SILENCE_THRESHOLD", "10");

        let temp_dir = tempdir().unwrap();
        let temp_path = temp_dir.path().to_str().unwrap();

        // Create a silent file
        let file_name = format!("{}/silent-test.wav", temp_path);
        let mut processor = MockAudioProcessor::new(&file_name);

        // Configure the mock to create a silent file
        processor.create_silent_file = true;

        // Create the recorder with our mock
        let mut recorder = AudioRecorder::new(processor);

        // Start recording
        let result = recorder.start_recording();
        assert!(result.is_ok());

        // Manually finalize the recording
        let _ = recorder.processor.finalize();

        // The file should now be deleted since it was silent and threshold is set
        let path = Path::new(&file_name);
        assert!(!path.exists(), "Silent file should have been deleted");

        reset_test_env();
    }

    #[test]
    fn test_normal_file_not_deleted() {
        let lock = TEST_MUTEX.lock();
        if lock.is_err() {
            println!("Mutex was poisoned, creating a new test environment");
            // Continue with the test even if the mutex was poisoned
        }
        reset_test_env();

        // Set threshold for this test using environment variable
        // This will be picked up by AppConfig
        env::set_var("SILENCE_THRESHOLD", "10");

        let temp_dir = tempdir().unwrap();
        let temp_path = temp_dir.path().to_str().unwrap();

        let file_name = format!("{}/normal-test.wav", temp_path);
        let processor = MockAudioProcessor::new(&file_name);

        // Configure the mock to create a normal (non-silent) file
        let mut processor = processor;
        processor.create_silent_file = false;

        // Create the recorder with our mock
        let mut recorder = AudioRecorder::new(processor);

        // Start recording
        let result = recorder.start_recording();
        assert!(result.is_ok());

        // Verify the file exists
        let path = Path::new(&file_name);
        assert!(path.exists(), "File should have been created");

        // Manually finalize the recording
        // We need to access the processor directly since we need mutable access
        let _ = recorder.processor.finalize();

        // The file should still exist since it's not silent
        assert!(
            path.exists(),
            "Non-silent file should not have been deleted"
        );

        // Clean up environment after test
        reset_test_env();
    }
}

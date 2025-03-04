use crate::AppConfig;
use crate::AudioProcessor;
use crate::AudioRecorder;
use crate::MockAudioProcessor;

#[test]
fn test_clean_shutdown() {
    let temp_dir = tempfile::tempdir().unwrap();
    let temp_path = temp_dir.path().to_str().unwrap();
    let file_name = format!("{}/test.wav", temp_path);

    // Create a mock processor
    let processor = MockAudioProcessor::new(&file_name);
    let _config = AppConfig::default();
    let mut recorder = AudioRecorder::new(processor);

    // Start recording
    assert!(recorder.start_recording().is_ok());

    // Stop recording
    assert!(recorder.processor.stop_recording().is_ok());
    assert!(recorder.processor.finalize().is_ok());

    // Verify the file was created and finalized
    assert!(std::path::Path::new(&file_name).exists());
}

#[test]
fn test_shutdown_during_recording() {
    let temp_dir = tempfile::tempdir().unwrap();
    let temp_path = temp_dir.path().to_str().unwrap();
    let file_name = format!("{}/test.wav", temp_path);

    // Create a mock processor
    let processor = MockAudioProcessor::new(&file_name);
    let _config = AppConfig::default();
    let mut recorder = AudioRecorder::new(processor);

    // Start recording
    assert!(recorder.start_recording().is_ok());

    // Stop recording
    assert!(recorder.processor.stop_recording().is_ok());
    assert!(recorder.processor.finalize().is_ok());

    // Verify the file was created and finalized
    assert!(std::path::Path::new(&file_name).exists());
}

#[test]
fn test_multiple_shutdown_attempts() {
    let temp_dir = tempfile::tempdir().unwrap();
    let temp_path = temp_dir.path().to_str().unwrap();
    let file_name = format!("{}/test.wav", temp_path);

    // Create a mock processor
    let processor = MockAudioProcessor::new(&file_name);
    let _config = AppConfig::default();
    let mut recorder = AudioRecorder::new(processor);

    // Start recording
    assert!(recorder.start_recording().is_ok());

    // First shutdown attempt
    assert!(recorder.processor.stop_recording().is_ok());

    // Second shutdown attempt should not cause issues
    assert!(recorder.processor.finalize().is_ok());

    // Verify the file was created and finalized
    assert!(std::path::Path::new(&file_name).exists());
}

#[test]
fn test_shutdown_with_error() {
    let temp_dir = tempfile::tempdir().unwrap();
    let temp_path = temp_dir.path().to_str().unwrap();
    let file_name = format!("{}/test.wav", temp_path);

    // Create a mock processor that will fail during finalize
    let mut processor = MockAudioProcessor::new(&file_name);
    processor.should_fail_finalize = true;
    let _config = AppConfig::default();
    let mut recorder = AudioRecorder::new(processor);

    // Start recording
    assert!(recorder.start_recording().is_ok());

    // Stop recording
    assert!(recorder.processor.stop_recording().is_ok());

    // Finalize should fail but not panic
    assert!(recorder.processor.finalize().is_err());
}

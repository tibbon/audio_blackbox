use crate::AppConfig;
use crate::AudioProcessor;
use crate::AudioRecorder;
use crate::MockAudioProcessor;

/// Happy-path smoke test: start → stop → finalize, file exists,
/// state transitions cleanly through the AudioProcessor lifecycle.
#[test]
fn test_clean_shutdown() {
    let temp_dir = tempfile::tempdir().unwrap();
    let temp_path = temp_dir.path().to_str().unwrap();
    let file_name = format!("{}/test.wav", temp_path);

    let processor = MockAudioProcessor::new(&file_name);
    let mut recorder = AudioRecorder::new(processor);

    // Before start: not recording.
    assert!(!recorder.get_processor().is_recording());

    recorder.start_recording().unwrap();

    // After start: the mock has processed audio (audio_processed=true,
    // finalized=false), so is_recording() reports true.
    assert!(
        recorder.get_processor().is_recording(),
        "Recorder should report is_recording=true after start_recording()"
    );

    recorder.processor_mut().stop_recording().unwrap();

    // After stop_recording the mock clears audio_processed; is_recording=false.
    assert!(
        !recorder.get_processor().is_recording(),
        "Recorder should report is_recording=false after stop_recording()"
    );

    recorder.processor_mut().finalize().unwrap();

    // Finalize must mark finalized=true.
    assert!(
        recorder.get_processor().finalized,
        "MockAudioProcessor.finalized must be true after finalize()"
    );

    // The WAV file the mock writes during process_audio should exist.
    assert!(std::path::Path::new(&file_name).exists());
    let metadata = std::fs::metadata(&file_name).unwrap();
    assert!(
        metadata.len() > 0,
        "Mock WAV file should be non-empty (the mock writes 1000 samples)"
    );
}

/// Stop while is_recording=true: must transition cleanly without leaving
/// the processor in a half-shut-down state.
#[test]
fn test_shutdown_during_recording() {
    let temp_dir = tempfile::tempdir().unwrap();
    let temp_path = temp_dir.path().to_str().unwrap();
    let file_name = format!("{}/recording.wav", temp_path);

    let processor = MockAudioProcessor::new(&file_name);
    let mut recorder = AudioRecorder::new(processor);

    recorder.start_recording().unwrap();

    // Precondition: recording is in progress.
    assert!(recorder.get_processor().is_recording());
    assert!(recorder.get_processor().audio_processed);
    assert!(!recorder.get_processor().finalized);
    assert!(
        !recorder.get_processor().created_files.is_empty(),
        "Mock should have populated created_files during start_recording"
    );

    // Stop while recording. Returns Ok and clears the recording flag.
    let stop_result = recorder.processor_mut().stop_recording();
    assert!(
        stop_result.is_ok(),
        "stop_recording during active recording must return Ok, got {:?}",
        stop_result
    );
    assert!(!recorder.get_processor().is_recording());

    // Finalize after a mid-recording stop: file exists and has data.
    recorder.processor_mut().finalize().unwrap();
    assert!(recorder.get_processor().finalized);
    assert!(std::path::Path::new(&file_name).exists());
    let metadata = std::fs::metadata(&file_name).unwrap();
    assert!(metadata.len() > 0);
}

/// Calling stop_recording and finalize multiple times must be idempotent —
/// no panic, no error, state does not regress.
#[test]
fn test_multiple_shutdown_attempts() {
    let temp_dir = tempfile::tempdir().unwrap();
    let temp_path = temp_dir.path().to_str().unwrap();
    let file_name = format!("{}/idempotent.wav", temp_path);

    let processor = MockAudioProcessor::new(&file_name);
    let mut recorder = AudioRecorder::new(processor);

    recorder.start_recording().unwrap();

    // First stop: succeeds, clears the recording flag.
    let first_stop = recorder.processor_mut().stop_recording();
    assert!(first_stop.is_ok(), "first stop_recording: {:?}", first_stop);
    assert!(!recorder.get_processor().is_recording());

    // Second stop on an already-stopped recorder: also Ok, no panic.
    let second_stop = recorder.processor_mut().stop_recording();
    assert!(
        second_stop.is_ok(),
        "second stop_recording must be idempotent, got {:?}",
        second_stop
    );
    assert!(!recorder.get_processor().is_recording());

    // First finalize: succeeds, sets finalized.
    let first_finalize = recorder.processor_mut().finalize();
    assert!(first_finalize.is_ok(), "first finalize: {:?}", first_finalize);
    assert!(recorder.get_processor().finalized);

    // Second finalize: also Ok (mock does not error on double-finalize unless
    // should_fail_finalize is set). State must not regress.
    let second_finalize = recorder.processor_mut().finalize();
    assert!(
        second_finalize.is_ok(),
        "second finalize must be idempotent on the mock, got {:?}",
        second_finalize
    );
    assert!(recorder.get_processor().finalized);

    assert!(std::path::Path::new(&file_name).exists());
}

/// finalize() returning Err must surface to the caller, not panic.
#[test]
fn test_shutdown_with_error() {
    let temp_dir = tempfile::tempdir().unwrap();
    let temp_path = temp_dir.path().to_str().unwrap();
    let file_name = format!("{}/test.wav", temp_path);

    let mut processor = MockAudioProcessor::new(&file_name);
    processor.should_fail_finalize = true;
    let _config = AppConfig::default();
    let mut recorder = AudioRecorder::new(processor);

    recorder.start_recording().unwrap();
    recorder.processor_mut().stop_recording().unwrap();

    // Finalize should fail but not panic.
    let result = recorder.processor_mut().finalize();
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Simulated finalize failure"),
        "Expected the simulated failure message, got: {}",
        err_msg
    );
}

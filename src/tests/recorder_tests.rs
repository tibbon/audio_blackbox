use crate::AudioRecorder;
use crate::audio_processor::AudioProcessor;
use crate::config::AppConfig;
use crate::constants::*;
use crate::error::BlackboxError;
use crate::mock_processor::MockAudioProcessor;

use std::path::Path;
use tempfile::tempdir;

/// Helper to create a standard set of env var overrides for test isolation.
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

#[test]
fn test_recorder_with_config() {
    temp_env::with_vars(default_test_env(), || {
        let temp_dir = tempdir().unwrap();
        let file_name = format!("{}/test.wav", temp_dir.path().to_str().unwrap());

        let processor = MockAudioProcessor::new(&file_name);
        let config = AppConfig::default();
        let mut recorder = AudioRecorder::with_config(processor, config);

        recorder.start_recording().expect("start_recording");
        // Real post-condition: a valid WAV with the mock's expected sample count
        // exists on disk. `audio_processed` is set unconditionally by the mock
        // and would also fire if the recorder short-circuited.
        let reader = hound::WavReader::open(&file_name)
            .expect("recorder should have produced a readable WAV");
        assert_eq!(reader.spec().sample_rate, 44100);
        assert!(reader.len() > 0, "WAV should contain samples");
    });
}

#[test]
fn test_recorder_reload_config() {
    temp_env::with_vars(default_test_env(), || {
        let temp_dir = tempdir().unwrap();
        let file_name = format!("{}/test.wav", temp_dir.path().to_str().unwrap());

        let processor = MockAudioProcessor::new(&file_name);
        let mut recorder = AudioRecorder::new(processor);

        // Record initial config state
        let initial_debug = recorder.config().get_debug();

        // Reload config — should not panic or change behavior in isolated env
        recorder.reload_config();

        assert_eq!(recorder.config().get_debug(), initial_debug);
    });
}

#[test]
fn test_recorder_start_recording_invalid_channels() {
    temp_env::with_vars(
        vec![
            ("AUDIO_CHANNELS", Some("invalid")),
            ("BLACKBOX_AUDIO_CHANNELS", None),
            ("BLACKBOX_CONFIG", None),
        ],
        || {
            let temp_dir = tempdir().unwrap();
            let file_name = format!("{}/test.wav", temp_dir.path().to_str().unwrap());

            let processor = MockAudioProcessor::new(&file_name);
            let mut recorder = AudioRecorder::new(processor);

            // Pattern-match the variant — substring-checking the Display
            // output would silently pass on any other ChannelParse-shaped
            // string error.
            let err = recorder
                .start_recording()
                .expect_err("invalid channels must error");
            assert!(
                matches!(err, BlackboxError::ChannelParse(_)),
                "expected ChannelParse, got {err:?}"
            );
        },
    );
}

#[test]
fn test_recorder_create_default_config() {
    let temp_dir = tempdir().unwrap();
    let config_path = temp_dir.path().join("subdir/blackbox.toml");
    let file_name = format!("{}/test.wav", temp_dir.path().to_str().unwrap());

    let processor = MockAudioProcessor::new(&file_name);
    let recorder = AudioRecorder::new(processor);

    // Should create parent directories and write config
    let result = recorder.create_default_config(config_path.to_str().unwrap());
    assert!(result.is_ok());
    assert!(config_path.exists());

    // Verify the generated file is valid TOML with expected keys
    let content = std::fs::read_to_string(&config_path).unwrap();
    assert!(content.contains("audio_channels"));
    assert!(content.contains("output_mode"));
    assert!(content.contains("silence_threshold"));
}

#[test]
fn test_recorder_split_mode() {
    temp_env::with_vars(
        vec![
            ("AUDIO_CHANNELS", Some("0,1")),
            ("OUTPUT_MODE", Some("split")),
            ("BLACKBOX_AUDIO_CHANNELS", None),
            ("BLACKBOX_OUTPUT_MODE", None),
            ("BLACKBOX_CONFIG", None),
            ("SILENCE_THRESHOLD", Some("0")),
            ("BLACKBOX_SILENCE_THRESHOLD", None),
        ],
        || {
            let temp_dir = tempdir().unwrap();
            let file_name = format!("{}/test.wav", temp_dir.path().to_str().unwrap());

            let processor = MockAudioProcessor::new(&file_name);
            let mut recorder = AudioRecorder::new(processor);

            recorder.start_recording().expect("start_recording");
            // Both the in-mock bookkeeping AND the on-disk artefacts must
            // reflect Split mode: one file per channel, all readable.
            assert_eq!(recorder.get_processor().output_mode, OutputMode::Split);
            let created = &recorder.get_processor().created_files;
            assert_eq!(
                created.len(),
                3,
                "Split mode with channels=0,1 should produce the main file plus one per channel"
            );
            for path in created {
                assert!(Path::new(path).exists(), "created file missing: {path}");
                let r = hound::WavReader::open(path).expect("readable WAV");
                assert!(r.len() > 0, "split file {path} contained no samples");
            }
        },
    );
}

#[test]
fn test_recorder_multichannel_mode() {
    // "single" with >2 channels is the multichannel path (interleaved single file).
    temp_env::with_vars(
        vec![
            ("AUDIO_CHANNELS", Some("0,1,2")),
            ("OUTPUT_MODE", Some("single")),
            ("BLACKBOX_AUDIO_CHANNELS", None),
            ("BLACKBOX_OUTPUT_MODE", None),
            ("BLACKBOX_CONFIG", None),
            ("SILENCE_THRESHOLD", Some("0")),
            ("BLACKBOX_SILENCE_THRESHOLD", None),
        ],
        || {
            let temp_dir = tempdir().unwrap();
            let file_name = format!("{}/test.wav", temp_dir.path().to_str().unwrap());

            let processor = MockAudioProcessor::new(&file_name);
            let mut recorder = AudioRecorder::new(processor);

            recorder.start_recording().expect("start_recording");
            // Single mode: one interleaved file. The mock writes to the spec
            // (channels=2 in the !Split branch); verify that on disk rather
            // than re-reading the field the mock unconditionally set.
            assert_eq!(recorder.get_processor().output_mode, OutputMode::Single);
            let r = hound::WavReader::open(&file_name).expect("readable WAV");
            assert_eq!(r.spec().channels, 2, "single mode mock writes 2 channels");
            assert!(r.len() > 0, "WAV should contain samples");
        },
    );
}

#[test]
fn test_recorder_finalize_error_propagation() {
    temp_env::with_vars(default_test_env(), || {
        let temp_dir = tempdir().unwrap();
        let file_name = format!("{}/test.wav", temp_dir.path().to_str().unwrap());

        let mut processor = MockAudioProcessor::new(&file_name);
        processor.should_fail_finalize = true;

        let mut recorder = AudioRecorder::new(processor);
        recorder.start_recording().expect("start_recording");
        let err = recorder
            .processor_mut()
            .finalize()
            .expect_err("finalize must propagate the simulated failure");
        assert!(
            matches!(&err, BlackboxError::Wav(msg) if msg.contains("Simulated finalize failure")),
            "expected Wav(\"Simulated finalize failure\"), got {err:?}"
        );
    });
}

#[test]
fn test_recorder_start_stop_is_recording() {
    temp_env::with_vars(default_test_env(), || {
        let temp_dir = tempdir().unwrap();
        let file_name = format!("{}/test.wav", temp_dir.path().to_str().unwrap());

        let processor = MockAudioProcessor::new(&file_name);
        let mut recorder = AudioRecorder::new(processor);

        // Before recording
        assert!(!recorder.get_processor().is_recording());

        // Start
        assert!(recorder.start_recording().is_ok());
        assert!(recorder.get_processor().is_recording());

        // Stop
        assert!(recorder.processor_mut().stop_recording().is_ok());
        assert!(!recorder.get_processor().is_recording());
    });
}

#[test]
fn test_recorder_wav_file_valid() {
    temp_env::with_vars(default_test_env(), || {
        let temp_dir = tempdir().unwrap();
        let file_name = format!("{}/test.wav", temp_dir.path().to_str().unwrap());

        let processor = MockAudioProcessor::new(&file_name);
        let mut recorder = AudioRecorder::new(processor);

        assert!(recorder.start_recording().is_ok());
        assert!(recorder.processor_mut().finalize().is_ok());

        // Verify the WAV file is actually readable
        let reader = hound::WavReader::open(&file_name);
        assert!(reader.is_ok(), "WAV file should be readable by hound");

        let reader = reader.unwrap();
        let spec = reader.spec();
        assert_eq!(spec.channels, 2);
        assert_eq!(spec.sample_rate, 44100);
        assert_eq!(spec.bits_per_sample, 24);

        // Verify it has actual samples
        let sample_count = reader.len();
        assert!(sample_count > 0, "WAV file should contain samples");
    });
}

#[test]
fn test_recorder_split_mode_wav_files_valid() {
    temp_env::with_vars(
        vec![
            ("AUDIO_CHANNELS", Some("0,1")),
            ("OUTPUT_MODE", Some("split")),
            ("BLACKBOX_AUDIO_CHANNELS", None),
            ("BLACKBOX_OUTPUT_MODE", None),
            ("BLACKBOX_CONFIG", None),
            ("SILENCE_THRESHOLD", Some("0")),
            ("BLACKBOX_SILENCE_THRESHOLD", None),
        ],
        || {
            let temp_dir = tempdir().unwrap();
            let file_name = format!("{}/test.wav", temp_dir.path().to_str().unwrap());

            let processor = MockAudioProcessor::new(&file_name);
            let mut recorder = AudioRecorder::new(processor);

            assert!(recorder.start_recording().is_ok());
            assert!(recorder.processor_mut().finalize().is_ok());

            // Verify each created file is a valid WAV
            for path in &recorder.get_processor().created_files {
                assert!(
                    Path::new(path).exists(),
                    "Created file should exist: {}",
                    path
                );
                let reader = hound::WavReader::open(path);
                assert!(
                    reader.is_ok(),
                    "Split WAV file should be readable: {}",
                    path
                );
                let reader = reader.unwrap();
                assert_eq!(reader.spec().channels, 1, "Split files should be mono");
                assert!(reader.len() > 0, "Split file should contain samples");
            }
        },
    );
}

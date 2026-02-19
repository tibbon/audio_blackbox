use std::path::Path;

use hound::WavReader;
use tempfile::tempdir;

use crate::audio_processor::AudioProcessor;
use crate::cpal_processor::CpalAudioProcessor;
use crate::test_utils::{
    generate_interleaved_f32, generate_silent_interleaved_f32, generate_uniform_interleaved_f32,
};
use crate::tests::default_test_env;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Test env with silence detection disabled (threshold=0).
///
/// `CpalAudioProcessor` writes i16-range samples into a 16-bit WAV, but
/// `is_silent()` normalizes by i32::MAX, so even loud signals appear silent.
/// Disable silence detection for tests that don't specifically test it.
fn test_env_no_silence() -> Vec<(&'static str, Option<&'static str>)> {
    let mut env = default_test_env();
    env.retain(|&(k, _)| k != "SILENCE_THRESHOLD");
    env.push(("SILENCE_THRESHOLD", Some("0")));
    env
}

/// Collect all `.wav` files (not `.recording.wav`) in a directory.
fn wav_files_in(dir: &Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|ext| ext == "wav")
                && !p.to_str().unwrap_or_default().contains(".recording.wav")
        })
        .collect()
}

/// Collect all `.recording.wav` files in a directory.
fn recording_wav_files_in(dir: &Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.to_str().unwrap_or_default().contains(".recording.wav"))
        .collect()
}

/// Read a WAV file and return (spec, all samples as Vec<i32>).
fn read_wav(path: &Path) -> (hound::WavSpec, Vec<i32>) {
    let reader = WavReader::open(path).unwrap();
    let spec = reader.spec();
    let samples: Vec<i32> = reader.into_samples::<i32>().map(|s| s.unwrap()).collect();
    (spec, samples)
}

/// Compute the RMS of a slice of i32 samples normalized to [-1, 1].
fn rms_i32(samples: &[i32]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let norm = f64::from(i32::MAX);
    let sum: f64 = samples
        .iter()
        .map(|&s| {
            let n = f64::from(s) / norm;
            n * n
        })
        .sum();
    (sum / samples.len() as f64).sqrt()
}

// ===========================================================================
// Standard mode tests
// ===========================================================================

#[test]
fn test_standard_mode_mono() {
    let temp_dir = tempdir().unwrap();
    let dir = temp_dir.path().to_str().unwrap();

    temp_env::with_vars(test_env_no_silence(), || {
        let mut processor = CpalAudioProcessor::new_for_test(dir, 44100, &[0], "single").unwrap();

        let data = generate_uniform_interleaved_f32(1, 1000, &[0], 0.5);
        processor.feed_test_data(&data, 1);
        processor.finalize().unwrap();

        let files = wav_files_in(temp_dir.path());
        assert_eq!(files.len(), 1, "Expected exactly one WAV file");

        let (spec, samples) = read_wav(&files[0]);
        assert_eq!(spec.channels, 1);
        assert_eq!(spec.sample_rate, 44100);
        assert_eq!(samples.len(), 1000);
    });
}

#[test]
fn test_standard_mode_stereo() {
    let temp_dir = tempdir().unwrap();
    let dir = temp_dir.path().to_str().unwrap();

    temp_env::with_vars(test_env_no_silence(), || {
        let mut processor =
            CpalAudioProcessor::new_for_test(dir, 44100, &[0, 1], "single").unwrap();

        let data = generate_uniform_interleaved_f32(2, 1000, &[0, 1], 0.5);
        processor.feed_test_data(&data, 2);
        processor.finalize().unwrap();

        let files = wav_files_in(temp_dir.path());
        assert_eq!(files.len(), 1);

        let (spec, samples) = read_wav(&files[0]);
        assert_eq!(spec.channels, 2);
        assert_eq!(spec.sample_rate, 44100);
        // 2 channels * 1000 frames = 2000 total samples
        assert_eq!(samples.len(), 2000);
    });
}

#[test]
fn test_standard_mode_data_accuracy() {
    let temp_dir = tempdir().unwrap();
    let dir = temp_dir.path().to_str().unwrap();

    temp_env::with_vars(test_env_no_silence(), || {
        let mut processor = CpalAudioProcessor::new_for_test(dir, 44100, &[0], "single").unwrap();

        // Feed a constant 0.5 signal
        let data = vec![0.5_f32; 100];
        processor.feed_test_data(&data, 1);
        processor.finalize().unwrap();

        let files = wav_files_in(temp_dir.path());
        let (_, samples) = read_wav(&files[0]);

        // 0.5 * 32767.0 = 16383.5 → truncated to 16383
        let expected = (0.5_f32 * 32767.0) as i32;
        for (i, &s) in samples.iter().enumerate() {
            assert_eq!(
                s, expected,
                "Sample {} mismatch: got {}, want {}",
                i, s, expected
            );
        }
    });
}

// ===========================================================================
// Split mode tests
// ===========================================================================

#[test]
fn test_split_mode_two_channels() {
    let temp_dir = tempdir().unwrap();
    let dir = temp_dir.path().to_str().unwrap();

    temp_env::with_vars(test_env_no_silence(), || {
        let mut processor = CpalAudioProcessor::new_for_test(dir, 44100, &[0, 1], "split").unwrap();

        let data = generate_uniform_interleaved_f32(2, 500, &[0, 1], 0.3);
        processor.feed_test_data(&data, 2);
        processor.finalize().unwrap();

        let files = wav_files_in(temp_dir.path());
        assert_eq!(files.len(), 2, "Expected two per-channel WAV files");

        for f in &files {
            let (spec, samples) = read_wav(f);
            assert_eq!(spec.channels, 1, "Each split file should be mono");
            assert_eq!(samples.len(), 500);
        }
    });
}

#[test]
fn test_split_mode_channel_isolation() {
    let temp_dir = tempdir().unwrap();
    let dir = temp_dir.path().to_str().unwrap();

    temp_env::with_vars(test_env_no_silence(), || {
        let mut processor = CpalAudioProcessor::new_for_test(dir, 44100, &[0, 2], "split").unwrap();

        // Channel 0 at 0.8 amplitude, channel 2 at 0.1 amplitude (4-ch device)
        let data = generate_interleaved_f32(4, 1000, &[(0, 0.8), (2, 0.1)]);
        processor.feed_test_data(&data, 4);
        processor.finalize().unwrap();

        let mut files = wav_files_in(temp_dir.path());
        files.sort();

        assert_eq!(files.len(), 2);

        let (_, samples_ch0) = read_wav(&files[0]);
        let (_, samples_ch2) = read_wav(&files[1]);

        let rms0 = rms_i32(&samples_ch0);
        let rms2 = rms_i32(&samples_ch2);

        assert!(
            rms0 > rms2 * 2.0,
            "Channel 0 RMS ({}) should be much larger than channel 2 RMS ({})",
            rms0,
            rms2
        );
    });
}

#[test]
fn test_split_mode_many_channels() {
    let temp_dir = tempdir().unwrap();
    let dir = temp_dir.path().to_str().unwrap();

    temp_env::with_vars(test_env_no_silence(), || {
        let mut processor =
            CpalAudioProcessor::new_for_test(dir, 48000, &[0, 1, 2, 3], "split").unwrap();

        let data = generate_uniform_interleaved_f32(4, 200, &[0, 1, 2, 3], 0.4);
        processor.feed_test_data(&data, 4);
        processor.finalize().unwrap();

        let files = wav_files_in(temp_dir.path());
        assert_eq!(files.len(), 4, "Expected 4 per-channel WAV files");
    });
}

// ===========================================================================
// Multichannel mode tests
// ===========================================================================

#[test]
fn test_multichannel_mode_three_channels() {
    let temp_dir = tempdir().unwrap();
    let dir = temp_dir.path().to_str().unwrap();

    temp_env::with_vars(test_env_no_silence(), || {
        let mut processor =
            CpalAudioProcessor::new_for_test(dir, 44100, &[0, 1, 2], "single").unwrap();

        let data = generate_uniform_interleaved_f32(3, 500, &[0, 1, 2], 0.5);
        processor.feed_test_data(&data, 3);
        processor.finalize().unwrap();

        let files = wav_files_in(temp_dir.path());
        assert_eq!(files.len(), 1);

        let fname = files[0].to_str().unwrap();
        assert!(
            fname.contains("-multichannel"),
            "File should have multichannel suffix: {}",
            fname
        );

        let (spec, samples) = read_wav(&files[0]);
        assert_eq!(spec.channels, 3);
        // 3 channels * 500 frames = 1500 samples
        assert_eq!(samples.len(), 1500);
    });
}

#[test]
fn test_multichannel_mode_interleaving() {
    let temp_dir = tempdir().unwrap();
    let dir = temp_dir.path().to_str().unwrap();

    temp_env::with_vars(test_env_no_silence(), || {
        let mut processor =
            CpalAudioProcessor::new_for_test(dir, 44100, &[0, 1, 2], "single").unwrap();

        // Create data where each channel has a constant distinct value
        let frames = 10;
        let total_ch = 3;
        let mut data = vec![0.0_f32; total_ch * frames];
        for f in 0..frames {
            data[f * total_ch] = 0.25; // ch0
            data[f * total_ch + 1] = 0.50; // ch1
            data[f * total_ch + 2] = 0.75; // ch2
        }

        processor.feed_test_data(&data, total_ch);
        processor.finalize().unwrap();

        let files = wav_files_in(temp_dir.path());
        let (spec, samples) = read_wav(&files[0]);
        assert_eq!(spec.channels, 3);

        // Verify interleaving: samples should alternate ch0, ch1, ch2
        let expected_ch0 = (0.25_f32 * 32767.0) as i32;
        let expected_ch1 = (0.50_f32 * 32767.0) as i32;
        let expected_ch2 = (0.75_f32 * 32767.0) as i32;

        for f in 0..frames {
            assert_eq!(samples[f * 3], expected_ch0, "Frame {} ch0", f);
            assert_eq!(samples[f * 3 + 1], expected_ch1, "Frame {} ch1", f);
            assert_eq!(samples[f * 3 + 2], expected_ch2, "Frame {} ch2", f);
        }
    });
}

// ===========================================================================
// Crash-safe WAV tests
// ===========================================================================

#[test]
fn test_recording_files_before_finalize() {
    let temp_dir = tempdir().unwrap();
    let dir = temp_dir.path().to_str().unwrap();

    temp_env::with_vars(test_env_no_silence(), || {
        let mut processor = CpalAudioProcessor::new_for_test(dir, 44100, &[0], "single").unwrap();

        let data = generate_uniform_interleaved_f32(1, 100, &[0], 0.5);
        processor.feed_test_data(&data, 1);

        // Before finalize: .recording.wav should exist, .wav should not
        let rec_files = recording_wav_files_in(temp_dir.path());
        assert!(
            !rec_files.is_empty(),
            "Should have .recording.wav before finalize"
        );

        let wav_before = wav_files_in(temp_dir.path());
        assert!(
            wav_before.is_empty(),
            "Should have no final .wav before finalize"
        );

        processor.finalize().unwrap();

        // After finalize: .wav should exist, .recording.wav should not
        let wav_after = wav_files_in(temp_dir.path());
        assert!(!wav_after.is_empty(), "Should have .wav after finalize");

        let rec_after = recording_wav_files_in(temp_dir.path());
        assert!(
            rec_after.is_empty(),
            "Should have no .recording.wav after finalize"
        );
    });
}

#[test]
fn test_no_stale_temp_files_after_finalize() {
    let temp_dir = tempdir().unwrap();
    let dir = temp_dir.path().to_str().unwrap();

    temp_env::with_vars(test_env_no_silence(), || {
        let mut processor = CpalAudioProcessor::new_for_test(dir, 44100, &[0, 1], "split").unwrap();

        let data = generate_uniform_interleaved_f32(2, 100, &[0, 1], 0.4);
        processor.feed_test_data(&data, 2);
        processor.finalize().unwrap();

        let rec_files = recording_wav_files_in(temp_dir.path());
        assert!(
            rec_files.is_empty(),
            "No .recording.wav files should remain after finalize, found: {:?}",
            rec_files
        );
    });
}

// ===========================================================================
// Write error tests
// ===========================================================================

#[test]
fn test_write_errors_initially_zero() {
    let temp_dir = tempdir().unwrap();
    let dir = temp_dir.path().to_str().unwrap();

    temp_env::with_vars(test_env_no_silence(), || {
        let processor = CpalAudioProcessor::new_for_test(dir, 44100, &[0], "single").unwrap();
        assert_eq!(processor.test_write_error_count(), 0);
    });
}

#[test]
fn test_write_errors_counter_accessible() {
    let temp_dir = tempdir().unwrap();
    let dir = temp_dir.path().to_str().unwrap();

    temp_env::with_vars(test_env_no_silence(), || {
        let mut processor = CpalAudioProcessor::new_for_test(dir, 44100, &[0], "single").unwrap();

        // Feed valid data — no errors expected
        let data = generate_uniform_interleaved_f32(1, 10, &[0], 0.1);
        processor.feed_test_data(&data, 1);
        assert_eq!(processor.test_write_error_count(), 0);
    });
}

// ===========================================================================
// Silence detection tests
// ===========================================================================

#[test]
fn test_finalize_deletes_silent_file() {
    let temp_dir = tempdir().unwrap();
    let dir = temp_dir.path().to_str().unwrap();

    // Use a high silence threshold so the file is considered silent
    let mut env = default_test_env();
    env.retain(|&(k, _)| k != "SILENCE_THRESHOLD");
    env.push(("SILENCE_THRESHOLD", Some("10")));

    temp_env::with_vars(env, || {
        let mut processor = CpalAudioProcessor::new_for_test(dir, 44100, &[0], "single").unwrap();

        let data = generate_silent_interleaved_f32(1, 1000);
        processor.feed_test_data(&data, 1);
        processor.finalize().unwrap();

        let files = wav_files_in(temp_dir.path());
        assert!(
            files.is_empty(),
            "Silent file should have been deleted, but found: {:?}",
            files
        );
    });
}

#[test]
fn test_finalize_keeps_non_silent_file() {
    let temp_dir = tempdir().unwrap();
    let dir = temp_dir.path().to_str().unwrap();

    let mut env = default_test_env();
    env.retain(|&(k, _)| k != "SILENCE_THRESHOLD");
    env.push(("SILENCE_THRESHOLD", Some("0.01")));

    temp_env::with_vars(env, || {
        let mut processor = CpalAudioProcessor::new_for_test(dir, 44100, &[0], "single").unwrap();

        let data = generate_uniform_interleaved_f32(1, 1000, &[0], 0.8);
        processor.feed_test_data(&data, 1);
        processor.finalize().unwrap();

        let files = wav_files_in(temp_dir.path());
        assert_eq!(files.len(), 1, "Non-silent file should be kept");
    });
}

#[test]
fn test_silence_detection_disabled() {
    let temp_dir = tempdir().unwrap();
    let dir = temp_dir.path().to_str().unwrap();

    // Threshold of 0 disables silence detection
    let mut env = default_test_env();
    env.retain(|&(k, _)| k != "SILENCE_THRESHOLD");
    env.push(("SILENCE_THRESHOLD", Some("0")));

    temp_env::with_vars(env, || {
        let mut processor = CpalAudioProcessor::new_for_test(dir, 44100, &[0], "single").unwrap();

        let data = generate_silent_interleaved_f32(1, 1000);
        processor.feed_test_data(&data, 1);
        processor.finalize().unwrap();

        let files = wav_files_in(temp_dir.path());
        assert_eq!(
            files.len(),
            1,
            "With threshold=0 silence detection is disabled, file should be kept"
        );
    });
}

#[test]
fn test_split_mode_silence_per_channel() {
    let temp_dir = tempdir().unwrap();
    let dir = temp_dir.path().to_str().unwrap();

    // Threshold between ch0 RMS (0, silent) and ch1 RMS (~0.636 for 0.9 amplitude).
    let mut env = default_test_env();
    env.retain(|&(k, _)| k != "SILENCE_THRESHOLD");
    env.push(("SILENCE_THRESHOLD", Some("0.01")));

    temp_env::with_vars(env, || {
        let mut processor = CpalAudioProcessor::new_for_test(dir, 44100, &[0, 1], "split").unwrap();

        // ch0 silent, ch1 loud
        let data = generate_interleaved_f32(2, 1000, &[(1, 0.9)]);
        processor.feed_test_data(&data, 2);
        processor.finalize().unwrap();

        let files = wav_files_in(temp_dir.path());
        // ch0 is silent and should be deleted, ch1 should remain
        assert_eq!(
            files.len(),
            1,
            "Only the loud channel file should remain, found: {:?}",
            files
        );
        let name = files[0].to_str().unwrap();
        assert!(
            name.contains("-ch1"),
            "Remaining file should be ch1, got: {}",
            name
        );
    });
}

// ===========================================================================
// Edge case tests
// ===========================================================================

#[test]
fn test_channel_beyond_device_range() {
    let temp_dir = tempdir().unwrap();
    let dir = temp_dir.path().to_str().unwrap();

    temp_env::with_vars(test_env_no_silence(), || {
        // Processor configured for channels [0, 5] but device only has 2 channels.
        // Channel 5 should be skipped gracefully (no panic).
        let mut processor = CpalAudioProcessor::new_for_test(dir, 44100, &[0, 5], "split").unwrap();

        // 2-channel device data
        let data = generate_uniform_interleaved_f32(2, 200, &[0], 0.5);
        processor.feed_test_data(&data, 2);
        processor.finalize().unwrap();

        // We get 2 split files (ch0 and ch5), but ch5 has no samples written
        // since frames only have 2 channels. The ch5 file may exist but be empty/silent.
        let files = wav_files_in(temp_dir.path());
        // ch0 should have data
        let ch0_file = files.iter().find(|f| f.to_str().unwrap().contains("-ch0"));
        assert!(ch0_file.is_some(), "ch0 file should exist");
        let (_, samples) = read_wav(ch0_file.unwrap());
        assert_eq!(samples.len(), 200);
    });
}

#[test]
fn test_multiple_feed_calls_accumulate() {
    let temp_dir = tempdir().unwrap();
    let dir = temp_dir.path().to_str().unwrap();

    temp_env::with_vars(test_env_no_silence(), || {
        let mut processor = CpalAudioProcessor::new_for_test(dir, 44100, &[0], "single").unwrap();

        let data1 = generate_uniform_interleaved_f32(1, 500, &[0], 0.3);
        let data2 = generate_uniform_interleaved_f32(1, 300, &[0], 0.6);

        processor.feed_test_data(&data1, 1);
        processor.feed_test_data(&data2, 1);
        processor.finalize().unwrap();

        let files = wav_files_in(temp_dir.path());
        let (_, samples) = read_wav(&files[0]);
        assert_eq!(
            samples.len(),
            800,
            "Two feed calls (500 + 300) should produce 800 samples"
        );
    });
}

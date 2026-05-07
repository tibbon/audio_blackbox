use std::path::Path;

use hound::WavReader;
use tempfile::tempdir;

use crate::audio_processor::AudioProcessor;
use crate::constants::OutputMode;
use crate::cpal_processor::CpalAudioProcessor;
use crate::test_utils::default_test_env;
use crate::test_utils::{
    generate_interleaved_f32, generate_silent_interleaved_f32, generate_uniform_interleaved_f32,
};
use crate::writer_thread::f32_to_wav_sample;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Test env with silence detection disabled (threshold=0).
///
/// `CpalAudioProcessor` writes i16-range samples into a 16-bit WAV, but
// Test helpers consolidated to `crate::test_utils` (DOLL-118).
use crate::test_utils::test_env_no_silence;

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
        let mut processor =
            CpalAudioProcessor::new_for_test(dir, 44100, &[0], OutputMode::Single).unwrap();

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
            CpalAudioProcessor::new_for_test(dir, 44100, &[0, 1], OutputMode::Single).unwrap();

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
        let mut processor =
            CpalAudioProcessor::new_for_test(dir, 44100, &[0], OutputMode::Single).unwrap();

        // Feed a constant 0.5 signal
        let data = vec![0.5_f32; 100];
        processor.feed_test_data(&data, 1);
        processor.finalize().unwrap();

        let files = wav_files_in(temp_dir.path());
        let (_, samples) = read_wav(&files[0]);

        // 0.5 * 32767.0 = 16383.5 → rounded to 16384
        let expected = (0.5_f32 * 32767.0).round() as i32;
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
        let mut processor =
            CpalAudioProcessor::new_for_test(dir, 44100, &[0, 1], OutputMode::Split).unwrap();

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
        let mut processor =
            CpalAudioProcessor::new_for_test(dir, 44100, &[0, 2], OutputMode::Split).unwrap();

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
            CpalAudioProcessor::new_for_test(dir, 48000, &[0, 1, 2, 3], OutputMode::Split).unwrap();

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
            CpalAudioProcessor::new_for_test(dir, 44100, &[0, 1, 2], OutputMode::Single).unwrap();

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
            CpalAudioProcessor::new_for_test(dir, 44100, &[0, 1, 2], OutputMode::Single).unwrap();

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
        let expected_ch0 = (0.25_f32 * 32767.0).round() as i32;
        let expected_ch1 = (0.50_f32 * 32767.0).round() as i32;
        let expected_ch2 = (0.75_f32 * 32767.0).round() as i32;

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
        let mut processor =
            CpalAudioProcessor::new_for_test(dir, 44100, &[0], OutputMode::Single).unwrap();

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
        let mut processor =
            CpalAudioProcessor::new_for_test(dir, 44100, &[0, 1], OutputMode::Split).unwrap();

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
        let processor =
            CpalAudioProcessor::new_for_test(dir, 44100, &[0], OutputMode::Single).unwrap();
        assert_eq!(processor.test_write_error_count(), 0);
    });
}

#[test]
fn test_write_errors_counter_accessible() {
    let temp_dir = tempdir().unwrap();
    let dir = temp_dir.path().to_str().unwrap();

    temp_env::with_vars(test_env_no_silence(), || {
        let mut processor =
            CpalAudioProcessor::new_for_test(dir, 44100, &[0], OutputMode::Single).unwrap();

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
        let mut processor =
            CpalAudioProcessor::new_for_test(dir, 44100, &[0], OutputMode::Single).unwrap();

        let data = generate_silent_interleaved_f32(1, 1000);
        processor.feed_test_data(&data, 1);
        // finalize() drops the writer state, which drops the silence-check
        // worker, which joins the worker thread — by the time finalize()
        // returns, all submitted batches have been processed (no sleep
        // needed; previous 100ms sleep was the flake source DOLL-97 fixed).
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
        let mut processor =
            CpalAudioProcessor::new_for_test(dir, 44100, &[0], OutputMode::Single).unwrap();

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
        let mut processor =
            CpalAudioProcessor::new_for_test(dir, 44100, &[0], OutputMode::Single).unwrap();

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
        let mut processor =
            CpalAudioProcessor::new_for_test(dir, 44100, &[0, 1], OutputMode::Split).unwrap();

        // ch0 silent, ch1 loud
        let data = generate_interleaved_f32(2, 1000, &[(1, 0.9)]);
        processor.feed_test_data(&data, 2);
        // See test_finalize_deletes_silent_file: no sleep needed, finalize()
        // joins the silence-check worker before returning.
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
        let mut processor =
            CpalAudioProcessor::new_for_test(dir, 44100, &[0, 5], OutputMode::Split).unwrap();

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
        let mut processor =
            CpalAudioProcessor::new_for_test(dir, 44100, &[0], OutputMode::Single).unwrap();

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

// ===========================================================================
// Bit depth tests
// ===========================================================================

#[test]
fn test_24bit_recording() {
    let temp_dir = tempdir().unwrap();
    let dir = temp_dir.path().to_str().unwrap();

    temp_env::with_vars(test_env_no_silence(), || {
        let mut processor =
            CpalAudioProcessor::new_for_test_with_bits(dir, 44100, &[0], OutputMode::Single, 24)
                .unwrap();

        let data = vec![0.5_f32; 100];
        processor.feed_test_data(&data, 1);
        processor.finalize().unwrap();

        let files = wav_files_in(temp_dir.path());
        assert_eq!(files.len(), 1);

        let (spec, samples) = read_wav(&files[0]);
        assert_eq!(spec.bits_per_sample, 24);
        assert_eq!(samples.len(), 100);

        // 0.5 * 8_388_607.0 = 4_194_303.5 → rounded to 4_194_304
        let expected = (0.5_f32 * 8_388_607.0).round() as i32;
        for (i, &s) in samples.iter().enumerate() {
            assert_eq!(
                s, expected,
                "24-bit sample {} mismatch: got {}, want {}",
                i, s, expected
            );
        }
    });
}

#[test]
fn test_32bit_recording() {
    let temp_dir = tempdir().unwrap();
    let dir = temp_dir.path().to_str().unwrap();

    temp_env::with_vars(test_env_no_silence(), || {
        let mut processor =
            CpalAudioProcessor::new_for_test_with_bits(dir, 44100, &[0], OutputMode::Single, 32)
                .unwrap();

        let data = vec![0.5_f32; 100];
        processor.feed_test_data(&data, 1);
        processor.finalize().unwrap();

        let files = wav_files_in(temp_dir.path());
        assert_eq!(files.len(), 1);

        let (spec, samples) = read_wav(&files[0]);
        assert_eq!(spec.bits_per_sample, 32);
        assert_eq!(samples.len(), 100);

        let expected = (0.5_f32 * i32::MAX as f32).round() as i32;
        for (i, &s) in samples.iter().enumerate() {
            assert_eq!(
                s, expected,
                "32-bit sample {} mismatch: got {}, want {}",
                i, s, expected
            );
        }
    });
}

#[test]
fn test_16bit_backward_compat() {
    let temp_dir = tempdir().unwrap();
    let dir = temp_dir.path().to_str().unwrap();

    temp_env::with_vars(test_env_no_silence(), || {
        // new_for_test defaults to 16-bit
        let mut processor =
            CpalAudioProcessor::new_for_test(dir, 44100, &[0], OutputMode::Single).unwrap();

        let data = vec![0.5_f32; 100];
        processor.feed_test_data(&data, 1);
        processor.finalize().unwrap();

        let files = wav_files_in(temp_dir.path());
        let (spec, samples) = read_wav(&files[0]);
        assert_eq!(spec.bits_per_sample, 16);
        assert_eq!(samples.len(), 100);

        let expected = (0.5_f32 * 32767.0).round() as i32;
        for &s in &samples {
            assert_eq!(s, expected);
        }
    });
}

#[test]
fn test_f32_to_wav_sample_conversion() {
    // 16-bit
    assert_eq!(f32_to_wav_sample(0.0, 16), 0);
    assert_eq!(f32_to_wav_sample(1.0, 16), 32767);
    assert_eq!(f32_to_wav_sample(-1.0, 16), -32767);
    // 0.5 * 32767.0 = 16383.5; round-half-away-from-zero → 16384
    assert_eq!(
        f32_to_wav_sample(0.5, 16),
        (0.5_f32 * 32767.0).round() as i32
    );

    // 24-bit
    assert_eq!(f32_to_wav_sample(0.0, 24), 0);
    assert_eq!(f32_to_wav_sample(1.0, 24), 8_388_607);
    assert_eq!(f32_to_wav_sample(-1.0, 24), -8_388_607);

    // 32-bit
    assert_eq!(f32_to_wav_sample(0.0, 32), 0);
    // f32 precision means 1.0 * i32::MAX may not be exact
    let max32 = f32_to_wav_sample(1.0, 32);
    assert!(
        max32 > 2_000_000_000,
        "32-bit max should be large: {}",
        max32
    );
}

#[test]
fn test_f32_to_wav_sample_handles_nan_and_inf() {
    // NaN flows through clamp/round/cast and lands on 0 — silent dropout, not
    // a max-amplitude click. Acceptable sentinel for an unusable sample.
    assert_eq!(f32_to_wav_sample(f32::NAN, 16), 0);
    assert_eq!(f32_to_wav_sample(f32::NAN, 24), 0);
    assert_eq!(f32_to_wav_sample(f32::NAN, 32), 0);

    // ±Inf clamps to ±1.0 then scales to the bit-depth max — not i32::MIN/MAX.
    assert_eq!(f32_to_wav_sample(f32::INFINITY, 16), 32767);
    assert_eq!(f32_to_wav_sample(f32::NEG_INFINITY, 16), -32767);
    assert_eq!(f32_to_wav_sample(f32::INFINITY, 24), 8_388_607);
    assert_eq!(f32_to_wav_sample(f32::NEG_INFINITY, 24), -8_388_607);
}

#[test]
fn test_f32_to_wav_sample_clamps_out_of_range() {
    // cpal does not guarantee [-1.0, 1.0] on all backends. Out-of-range
    // inputs must clamp, not wrap or saturate to i32::MIN/MAX.
    assert_eq!(f32_to_wav_sample(2.0, 16), 32767);
    assert_eq!(f32_to_wav_sample(-2.0, 16), -32767);
    assert_eq!(f32_to_wav_sample(100.0, 16), 32767);
    assert_eq!(f32_to_wav_sample(-100.0, 16), -32767);
}

#[test]
fn test_f32_to_wav_sample_roundtrip_within_quantization_error() {
    // For any amplitude in [-1.0, 1.0], converting to i16 and back must
    // round-trip within one LSB of quantization error.
    let cases = [-1.0, -0.5, -0.25, -0.125, 0.0, 0.125, 0.25, 0.5, 0.999, 1.0];
    for a in cases {
        let i = f32_to_wav_sample(a, 16);
        let recovered = i as f32 / 32767.0;
        let err = (a - recovered).abs();
        assert!(
            err <= 1.0 / 32767.0,
            "roundtrip error {} exceeds 1 LSB for amplitude {}",
            err,
            a
        );
    }
}

// ===========================================================================
// Partial frame tests
// ===========================================================================

#[test]
fn test_partial_frame_carryover() {
    let temp_dir = tempdir().unwrap();
    let dir = temp_dir.path().to_str().unwrap();

    temp_env::with_vars(test_env_no_silence(), || {
        let mut processor =
            CpalAudioProcessor::new_for_test(dir, 44100, &[0, 1], OutputMode::Single).unwrap();

        // Feed 2.5 frames of stereo data (5 samples for 2 channels)
        let data = vec![0.3_f32; 5];
        processor.feed_test_data(&data, 2);

        // Feed 1.5 more frames (3 samples) — should combine with leftover
        let data2 = vec![0.6_f32; 3];
        processor.feed_test_data(&data2, 2);

        processor.finalize().unwrap();

        let files = wav_files_in(temp_dir.path());
        let (spec, samples) = read_wav(&files[0]);
        assert_eq!(spec.channels, 2);
        // 5 samples = 2 full frames (4 samples used, 1 leftover)
        // 3 samples + 1 leftover = 4 samples = 2 full frames
        // Total: 4 frames * 2 channels = 8 samples
        assert_eq!(
            samples.len(),
            8,
            "Should have 4 complete frames (8 samples)"
        );
    });
}

#[test]
fn test_partial_frame_across_multiple_calls() {
    let temp_dir = tempdir().unwrap();
    let dir = temp_dir.path().to_str().unwrap();

    temp_env::with_vars(test_env_no_silence(), || {
        let mut processor =
            CpalAudioProcessor::new_for_test(dir, 44100, &[0, 1, 2], OutputMode::Single).unwrap();

        // Feed 1 sample (partial frame for 3 channels)
        processor.feed_test_data(&[0.1], 3);
        // Feed 2 more samples (still partial: 3 total = 1 frame)
        processor.feed_test_data(&[0.2, 0.3], 3);
        // Feed 5 more (8 total = 2 frames + 2 leftover)
        processor.feed_test_data(&[0.4, 0.5, 0.6, 0.7, 0.8], 3);
        // Feed 1 more (3 leftover = 1 frame)
        processor.feed_test_data(&[0.9], 3);

        processor.finalize().unwrap();

        let files = wav_files_in(temp_dir.path());
        let (spec, samples) = read_wav(&files[0]);
        assert_eq!(spec.channels, 3);
        // 9 samples / 3 channels = 3 full frames * 3 channels = 9 samples written
        assert_eq!(
            samples.len(),
            9,
            "Should have 3 complete frames (9 samples)"
        );
    });
}

#[test]
fn test_split_mode_partial_frames() {
    let temp_dir = tempdir().unwrap();
    let dir = temp_dir.path().to_str().unwrap();

    temp_env::with_vars(test_env_no_silence(), || {
        let mut processor =
            CpalAudioProcessor::new_for_test(dir, 44100, &[0, 1, 2, 3], OutputMode::Split).unwrap();

        // Feed 10 samples for a 4-channel device = 2 full frames + 2 leftover
        let data: Vec<f32> = (0..10).map(|i| (i as f32) * 0.1).collect();
        processor.feed_test_data(&data, 4);

        // Feed 6 more = 2 leftover + 6 = 8 = 2 full frames
        let data2: Vec<f32> = (0..6).map(|i| (i as f32) * 0.05).collect();
        processor.feed_test_data(&data2, 4);

        processor.finalize().unwrap();

        let files = wav_files_in(temp_dir.path());
        assert_eq!(files.len(), 4, "Should have 4 split files");

        for f in &files {
            let (spec, samples) = read_wav(f);
            assert_eq!(spec.channels, 1);
            assert_eq!(samples.len(), 4, "Each channel should have 4 frames");
        }
    });
}

// ===========================================================================
// Stream error propagation test
// ===========================================================================

#[test]
fn test_stream_error_flag_propagation() {
    let temp_dir = tempdir().unwrap();
    let dir = temp_dir.path().to_str().unwrap();

    temp_env::with_vars(test_env_no_silence(), || {
        let processor =
            CpalAudioProcessor::new_for_test(dir, 44100, &[0], OutputMode::Single).unwrap();

        // Initially no stream error
        assert!(!processor.stream_error());

        // Build the SAME closure the production cpal err_fn uses (via
        // `build_stream_err_callback`), then invoke it with a synthesized
        // cpal::StreamError. If the body of `build_stream_err_callback`
        // (or the production wiring that calls it from the input-stream
        // builder) is reverted, this test fails — DOLL-106.
        let mut err_fn = processor.build_stream_err_callback();
        err_fn(cpal::StreamError::DeviceNotAvailable);
        assert!(
            processor.stream_error(),
            "stream_error must propagate through the trait accessor after the cpal err_fn fires"
        );
    });
}

#[test]
fn test_finalize_clears_sample_rate_changed() {
    // DOLL-123 — without finalize clearing the flag, a stop+start cycle
    // could leave a stale `true` from the prior session visible to the
    // next FFI status poll between sessions.
    let temp_dir = tempdir().unwrap();
    let dir = temp_dir.path().to_str().unwrap();

    temp_env::with_vars(test_env_no_silence(), || {
        let mut processor =
            CpalAudioProcessor::new_for_test(dir, 44100, &[0], OutputMode::Single).unwrap();

        // Simulate the macOS CoreAudio rate-change listener firing during
        // a recording.
        processor.simulate_sample_rate_changed();
        assert!(
            processor.sample_rate_changed(),
            "sample_rate_changed must be observable after the listener fires"
        );

        // Feed some data + finalize. After finalize the flag must be back
        // to false so the next session doesn't inherit it.
        let data = generate_silent_interleaved_f32(1, 100);
        processor.feed_test_data(&data, 1);
        processor.finalize().unwrap();

        assert!(
            !processor.sample_rate_changed(),
            "finalize must reset sample_rate_changed (DOLL-123); leftover \
             true would survive a stop/start gap and mislead the FFI poll"
        );
    });
}

// ===========================================================================
// Peak level metering tests
// ===========================================================================

#[test]
fn test_peak_levels_during_recording() {
    let temp_dir = tempdir().unwrap();
    let dir = temp_dir.path().to_str().unwrap();

    temp_env::with_vars(test_env_no_silence(), || {
        let mut processor =
            CpalAudioProcessor::new_for_test(dir, 44100, &[0, 1], OutputMode::Single).unwrap();

        // Feed stereo data: ch0 at 0.8, ch1 at 0.2
        let data = generate_interleaved_f32(2, 100, &[(0, 0.8), (1, 0.2)]);
        processor.feed_test_data(&data, 2);

        let peaks = processor.peak_levels();
        assert_eq!(peaks.len(), 2, "Should have 2 peak levels for 2 channels");
        assert!(
            peaks[0] > peaks[1],
            "Ch0 peak ({}) should be > ch1 peak ({})",
            peaks[0],
            peaks[1]
        );
        assert!(
            peaks[0] > 0.5,
            "Ch0 peak should be around 0.8, got {}",
            peaks[0]
        );
    });
}

#[test]
fn test_peak_levels_silent() {
    let temp_dir = tempdir().unwrap();
    let dir = temp_dir.path().to_str().unwrap();

    temp_env::with_vars(test_env_no_silence(), || {
        let mut processor =
            CpalAudioProcessor::new_for_test(dir, 44100, &[0], OutputMode::Single).unwrap();

        // Feed silence
        let data = generate_silent_interleaved_f32(1, 100);
        processor.feed_test_data(&data, 1);

        let peaks = processor.peak_levels();
        assert_eq!(peaks.len(), 1);
        assert!(
            peaks[0] < f32::EPSILON,
            "Silent data should have peak of 0.0, got {}",
            peaks[0]
        );
    });
}

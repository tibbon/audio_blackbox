use crate::error::BlackboxError;
use crate::numeric::saturating_i32_from_f64;
use crate::utils::is_silent;
use hound::WavSpec;
use hound::WavWriter;
use std::fs;
use std::path::Path;
use tempfile::tempdir;

fn create_test_wav_file(
    path: &Path,
    samples: &[i32],
    spec: WavSpec,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut writer = WavWriter::create(path, spec)?;
    for &sample in samples {
        writer.write_sample(sample)?;
    }
    writer.finalize()?;
    Ok(())
}

#[test]
fn test_silent_file() {
    let temp_dir = tempdir().unwrap();
    let file_path = temp_dir.path().join("silent.wav");

    // Create a WAV file with very low amplitude samples
    let spec = WavSpec {
        channels: 1,
        sample_rate: 44100,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let samples = vec![0; 1000]; // All samples are 0
    create_test_wav_file(&file_path, &samples, spec).unwrap();

    assert!(is_silent(file_path.to_str().unwrap(), 0.1).unwrap());
}

#[test]
fn test_non_silent_file() {
    let temp_dir = tempdir().unwrap();
    let file_path = temp_dir.path().join("non_silent.wav");

    // Create a WAV file with high amplitude samples
    let spec = WavSpec {
        channels: 1,
        sample_rate: 44100,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Int,
    };

    // Generate a non-silent file with high RMS amplitude
    let mut writer = WavWriter::create(&file_path, spec).unwrap();
    for _ in 0..1000 {
        // Generate a sine wave with 90% of max amplitude
        let sample = saturating_i32_from_f64(
            f64::from(i32::MAX) * 0.9 * (2.0 * std::f64::consts::PI * 440.0 / 44100.0).sin(),
        );
        writer.write_sample(sample).unwrap();
    }
    writer.finalize().unwrap();

    // Test that the file is not silent
    assert!(!is_silent(file_path.to_str().unwrap(), 0.01).unwrap());
}

#[test]
fn test_threshold_disabled() {
    let temp_dir = tempdir().unwrap();
    let file_path = temp_dir.path().join("test.wav");

    // Create a WAV file with very low amplitude samples
    let spec = WavSpec {
        channels: 1,
        sample_rate: 44100,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let samples = vec![0; 1000];
    create_test_wav_file(&file_path, &samples, spec).unwrap();

    // When threshold is 0 or negative, silence detection is disabled
    assert!(!is_silent(file_path.to_str().unwrap(), 0.0).unwrap());
    assert!(!is_silent(file_path.to_str().unwrap(), -1.0).unwrap());
}

#[test]
fn test_empty_file() {
    let temp_dir = tempdir().unwrap();
    let file_path = temp_dir.path().join("empty.wav");

    // Create an empty WAV file
    let spec = WavSpec {
        channels: 1,
        sample_rate: 44100,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let samples = vec![];
    create_test_wav_file(&file_path, &samples, spec).unwrap();

    // Empty files are considered silent
    assert!(is_silent(file_path.to_str().unwrap(), 0.1).unwrap());
}

#[test]
fn test_nonexistent_file() {
    let err = is_silent("nonexistent.wav", 0.1).expect_err("missing file must error");
    let BlackboxError::WavSource { context, source } = &err else {
        panic!("expected WavSource, got {err:?}");
    };
    assert!(
        context.contains("Failed to open WAV file") && context.contains("nonexistent.wav"),
        "context should mention the file path: {context}"
    );
    // Source chain must be populated so callers can downcast to hound::Error.
    assert!(
        std::error::Error::source(&**source).is_some()
            || source.downcast_ref::<hound::Error>().is_some(),
        "source chain should be populated"
    );
}

#[test]
fn test_invalid_wav_file() {
    let temp_dir = tempdir().unwrap();
    let file_path = temp_dir.path().join("invalid.wav");

    // Create an invalid WAV file
    fs::write(&file_path, "not a wav file").unwrap();

    let err = is_silent(file_path.to_str().unwrap(), 0.1).expect_err("non-WAV content must error");
    assert!(
        matches!(err, BlackboxError::WavSource { .. }),
        "expected WavSource variant, got {err:?}"
    );
}

#[test]
fn test_multichannel_silence() {
    let temp_dir = tempdir().unwrap();
    let file_path = temp_dir.path().join("multichannel.wav");

    // Create a stereo WAV file with very low amplitude samples
    let spec = WavSpec {
        channels: 2,
        sample_rate: 44100,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let samples = vec![0; 2000]; // 1000 samples per channel
    create_test_wav_file(&file_path, &samples, spec).unwrap();

    assert!(is_silent(file_path.to_str().unwrap(), 0.1).unwrap());
}

#[test]
fn test_mixed_amplitude() {
    let temp_dir = tempdir().unwrap();
    let file_path = temp_dir.path().join("mixed.wav");

    // Create a WAV file with mixed amplitude samples
    let spec = WavSpec {
        channels: 1,
        sample_rate: 44100,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Int,
    };

    // Generate a file with mixed amplitudes
    let mut writer = WavWriter::create(&file_path, spec).unwrap();
    for i in 0..1000 {
        if i % 2 == 0 {
            // Silent samples
            writer.write_sample(0).unwrap();
        } else {
            // Loud samples with 90% of max amplitude
            let sample = saturating_i32_from_f64(
                f64::from(i32::MAX) * 0.9 * (2.0 * std::f64::consts::PI * 440.0 / 44100.0).sin(),
            );
            writer.write_sample(sample).unwrap();
        }
    }
    writer.finalize().unwrap();

    // Test that the file is not silent
    assert!(!is_silent(file_path.to_str().unwrap(), 0.01).unwrap());
}

/// A file whose header was never rewritten (a finalize that failed on a full
/// disk leaves the placeholder "0 data bytes") reads as zero samples even
/// though audio follows the header. That must not count as silent: the silence
/// check would delete the take.
#[test]
fn test_placeholder_header_with_audio_is_kept() {
    let temp_dir = tempdir().unwrap();
    let file_path = temp_dir.path().join("placeholder.wav");
    let spec = WavSpec {
        channels: 1,
        sample_rate: 44100,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    create_test_wav_file(&file_path, &[0; 1000], spec).unwrap();
    // Zero the RIFF and data sizes, as RawWavWriter::create writes them.
    let mut bytes = fs::read(&file_path).unwrap();
    bytes[4..8].fill(0);
    bytes[40..44].fill(0);
    fs::write(&file_path, &bytes).unwrap();

    assert!(!is_silent(file_path.to_str().unwrap(), 0.1).unwrap());
}

/// The same two cases for a 24-bit file, which `RawWavWriter` writes as
/// `WAVE_FORMAT_EXTENSIBLE` with a 68-byte header: a header-only file is
/// silent, and one with audio behind a placeholder header is kept. Under the
/// old fixed 44-byte assumption a header-only EXTENSIBLE file looked like it
/// held 24 bytes of unknown audio.
#[test]
fn test_extensible_header_only_vs_placeholder_with_audio() {
    use crate::raw_wav_writer::{RawWavWriter, WavSpec as RawSpec};
    let temp_dir = tempdir().unwrap();
    let spec = RawSpec {
        channels: 1,
        sample_rate: 48_000,
        bits_per_sample: 24,
    };

    let empty = temp_dir.path().join("empty.wav");
    RawWavWriter::create(empty.to_str().unwrap(), spec)
        .unwrap()
        .finalize()
        .unwrap();
    assert_eq!(fs::metadata(&empty).unwrap().len(), 68);
    assert!(is_silent(empty.to_str().unwrap(), 0.1).unwrap());

    // Audio on disk, header still at the placeholder (dropped unfinalized).
    let placeholder = temp_dir.path().join("placeholder.wav");
    let mut w = RawWavWriter::create(placeholder.to_str().unwrap(), spec).unwrap();
    for _ in 0..1_000 {
        w.write_sample(0).unwrap();
    }
    drop(w);
    assert!(!is_silent(placeholder.to_str().unwrap(), 0.1).unwrap());
}

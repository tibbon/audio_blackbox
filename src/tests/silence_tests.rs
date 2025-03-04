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
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 44100,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Int,
    };

    // Generate a non-silent file with high RMS amplitude
    let mut writer = hound::WavWriter::create(&file_path, spec).unwrap();
    for _ in 0..1000 {
        // Generate a sine wave with 90% of max amplitude
        let sample =
            (i32::MAX as f64 * 0.9 * (2.0 * std::f64::consts::PI * 440.0 / 44100.0).sin()) as i32;
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
    assert!(is_silent("nonexistent.wav", 0.1).is_err());
}

#[test]
fn test_invalid_wav_file() {
    let temp_dir = tempdir().unwrap();
    let file_path = temp_dir.path().join("invalid.wav");

    // Create an invalid WAV file
    fs::write(&file_path, "not a wav file").unwrap();

    assert!(is_silent(file_path.to_str().unwrap(), 0.1).is_err());
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
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 44100,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Int,
    };

    // Generate a file with mixed amplitudes
    let mut writer = hound::WavWriter::create(&file_path, spec).unwrap();
    for i in 0..1000 {
        if i % 2 == 0 {
            // Silent samples
            writer.write_sample(0).unwrap();
        } else {
            // Loud samples with 90% of max amplitude
            let sample =
                (i32::MAX as f64 * 0.9 * (2.0 * std::f64::consts::PI * 440.0 / 44100.0).sin())
                    as i32;
            writer.write_sample(sample).unwrap();
        }
    }
    writer.finalize().unwrap();

    // Test that the file is not silent
    assert!(!is_silent(file_path.to_str().unwrap(), 0.01).unwrap());
}

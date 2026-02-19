use std::collections::BTreeSet;

use crate::constants::MAX_CHANNELS;
use crate::error::BlackboxError;
#[cfg(target_os = "linux")]
use log::warn;
#[cfg(target_os = "linux")]
use std::process::Command;

/// Parse a string of channel specifications and return a vector of channel numbers.
///
/// The input string can include individual channels (e.g., "0,1,5")
/// and ranges of channels (e.g., "1-24"). The resulting vector is sorted
/// and contains no duplicates.
pub fn parse_channel_string(input: &str) -> Result<Vec<usize>, BlackboxError> {
    let mut channels = BTreeSet::new();

    for part in input.split(',') {
        if part.contains('-') {
            // Handle range like "1-24"
            let range_parts: Vec<&str> = part.split('-').collect();
            if range_parts.len() != 2 {
                return Err(BlackboxError::ChannelParse(format!(
                    "Invalid range format: {}",
                    part
                )));
            }

            let start = range_parts[0].trim().parse::<usize>().map_err(|_| {
                BlackboxError::ChannelParse(format!("Invalid start of range: {}", range_parts[0]))
            })?;
            let end = range_parts[1].trim().parse::<usize>().map_err(|_| {
                BlackboxError::ChannelParse(format!("Invalid end of range: {}", range_parts[1]))
            })?;

            if start > end {
                return Err(BlackboxError::ChannelParse(format!(
                    "Invalid range: start {} greater than end {}",
                    start, end
                )));
            }

            if end >= MAX_CHANNELS {
                return Err(BlackboxError::ChannelParse(format!(
                    "Channel number {} exceeds maximum of {}",
                    end,
                    MAX_CHANNELS - 1
                )));
            }

            channels.extend(start..=end);
        } else {
            // Handle individual channel
            let channel = part.trim().parse::<usize>().map_err(|_| {
                BlackboxError::ChannelParse(format!("Invalid channel number: {}", part))
            })?;

            if channel >= MAX_CHANNELS {
                return Err(BlackboxError::ChannelParse(format!(
                    "Channel number {} exceeds maximum of {}",
                    channel,
                    MAX_CHANNELS - 1
                )));
            }

            channels.insert(channel);
        }
    }

    if channels.is_empty() {
        return Err(BlackboxError::ChannelParse(
            "No valid channels specified".to_string(),
        ));
    }

    Ok(channels.into_iter().collect())
}

/// Checks if ALSA is available on Linux systems.
///
/// Returns a warning message if ALSA is not available, but does not
/// prevent execution as CPAL might fall back to another backend.
#[cfg(target_os = "linux")]
pub fn check_alsa_availability() -> Result<(), BlackboxError> {
    // Check if alsa is available using pkg-config
    let output = Command::new("pkg-config")
        .args(["--exists", "alsa"])
        .output();

    match output {
        Ok(o) if o.status.success() => Ok(()),
        _ => {
            warn!("ALSA libraries not found. Audio recording might not work correctly on Linux.");
            warn!("Try installing libasound2-dev package: sudo apt-get install libasound2-dev");
            // Continue execution anyway, as cpal might fall back to another backend
            Ok(())
        }
    }
}

/// No-op implementation for non-Linux platforms.
#[cfg(not(target_os = "linux"))]
pub fn check_alsa_availability() -> Result<(), BlackboxError> {
    // No-op on non-Linux platforms
    Ok(())
}

/// Helper function that checks if a WAV file is mostly silent by calculating its RMS amplitude
/// and comparing it to the provided threshold.
///
/// Parameters:
/// - file_path: Path to the WAV file to analyze
/// - threshold: RMS amplitude threshold. If the file's RMS is below this value, it's considered silent.
///   A threshold of 0 or negative disables silence detection.
///
/// Returns:
/// - Ok(true) if the file is silent (RMS < threshold)
/// - Ok(false) if the file is not silent (RMS >= threshold) or if silence detection is disabled
/// - Err if there was an error reading or analyzing the file
pub fn is_silent(file_path: &str, threshold: f32) -> Result<bool, BlackboxError> {
    let threshold_f64 = threshold as f64;

    if threshold_f64 <= 0.0 {
        return Ok(false);
    }

    let reader = hound::WavReader::open(file_path).map_err(|e| {
        BlackboxError::Wav(format!("Failed to open WAV file for silence check: {}", e))
    })?;

    // Stream through samples without collecting into memory
    let mut sum_of_squares: f64 = 0.0;
    let mut count: u64 = 0;
    let norm = f64::from(i32::MAX);

    for sample in reader.into_samples::<i32>() {
        let s = sample.map_err(|e| BlackboxError::Wav(format!("Failed to read sample: {}", e)))?;
        let normalized = f64::from(s) / norm;
        sum_of_squares += normalized * normalized;
        count += 1;
    }

    if count == 0 {
        return Ok(true); // Empty file is silent
    }

    let rms = (sum_of_squares / count as f64).sqrt();
    Ok(rms < threshold_f64)
}

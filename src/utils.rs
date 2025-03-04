use crate::constants::MAX_CHANNELS;
use std::vec::Vec;
#[cfg(target_os = "linux")]
use std::process::Command;

/// Parse a string of channel specifications and return a vector of channel numbers.
///
/// The input string can include individual channels (e.g., "0,1,5")
/// and ranges of channels (e.g., "1-24"). The resulting vector is sorted
/// and contains no duplicates.
pub fn parse_channel_string(input: &str) -> Result<Vec<usize>, String> {
    let mut channels = Vec::new();

    for part in input.split(',') {
        if part.contains('-') {
            // Handle range like "1-24"
            let range_parts: Vec<&str> = part.split('-').collect();
            if range_parts.len() != 2 {
                return Err(format!("Invalid range format: {}", part));
            }

            let start = range_parts[0]
                .trim()
                .parse::<usize>()
                .map_err(|_| format!("Invalid start of range: {}", range_parts[0]))?;
            let end = range_parts[1]
                .trim()
                .parse::<usize>()
                .map_err(|_| format!("Invalid end of range: {}", range_parts[1]))?;

            if start > end {
                return Err(format!(
                    "Invalid range: start {} greater than end {}",
                    start, end
                ));
            }

            if end >= MAX_CHANNELS {
                return Err(format!(
                    "Channel number {} exceeds maximum of {}",
                    end,
                    MAX_CHANNELS - 1
                ));
            }

            // Add all channels in the range
            for channel in start..=end {
                channels.push(channel);
            }
        } else {
            // Handle individual channel
            let channel = part
                .trim()
                .parse::<usize>()
                .map_err(|_| format!("Invalid channel number: {}", part))?;

            if channel >= MAX_CHANNELS {
                return Err(format!(
                    "Channel number {} exceeds maximum of {}",
                    channel,
                    MAX_CHANNELS - 1
                ));
            }

            channels.push(channel);
        }
    }

    if channels.is_empty() {
        return Err("No valid channels specified".to_string());
    }

    // Remove duplicate channels
    channels.sort();
    channels.dedup();

    Ok(channels)
}

/// Checks if ALSA is available on Linux systems.
///
/// Returns a warning message if ALSA is not available, but does not
/// prevent execution as CPAL might fall back to another backend.
#[cfg(target_os = "linux")]
pub fn check_alsa_availability() -> Result<(), String> {
    // Check if alsa is available using pkg-config
    let output = Command::new("pkg-config")
        .args(["--exists", "alsa"])
        .output();

    match output {
        Ok(o) if o.status.success() => Ok(()),
        _ => {
            eprintln!("WARNING: ALSA libraries not found. Audio recording might not work correctly on Linux.");
            eprintln!("Try installing libasound2-dev package: sudo apt-get install libasound2-dev");
            // Continue execution anyway, as cpal might fall back to another backend
            Ok(())
        }
    }
}

/// No-op implementation for non-Linux platforms.
#[cfg(not(target_os = "linux"))]
pub fn check_alsa_availability() -> Result<(), String> {
    // No-op on non-Linux platforms
    Ok(())
}

/// Helper function that checks if a WAV file is mostly silent by calculating its RMS amplitude
/// and comparing it to the provided threshold.
///
/// Parameters:
/// - file_path: Path to the WAV file to analyze
/// - threshold: RMS amplitude threshold. If the file's RMS is below this value, it's considered silent.
///              A threshold of 0 or negative disables silence detection.
///
/// Returns:
/// - Ok(true) if the file is silent (RMS < threshold)
/// - Ok(false) if the file is not silent (RMS >= threshold) or if silence detection is disabled
/// - Err(String) if there was an error reading or analyzing the file
pub fn is_silent(file_path: &str, threshold: f32) -> Result<bool, String> {
    let threshold_i32 = threshold as i32;

    if threshold_i32 <= 0 {
        // If threshold is 0 or negative, we don't check for silence
        return Ok(false);
    }

    // Open the WAV file for reading
    let reader = hound::WavReader::open(file_path)
        .map_err(|e| format!("Failed to open WAV file for silence check: {}", e))?;

    // Read all samples
    let samples: Vec<i32> = reader
        .into_samples()
        .collect::<Result<Vec<i32>, _>>()
        .map_err(|e| format!("Failed to read samples: {}", e))?;

    if samples.is_empty() {
        return Ok(true); // Empty file is silent
    }

    // Calculate RMS (Root Mean Square) amplitude
    // RMS is a measure of the average power of the audio signal
    let sum_of_squares: i64 = samples.iter().map(|&s| s as i64 * s as i64).sum();
    let mean_square = sum_of_squares as f64 / samples.len() as f64;
    let rms = mean_square.sqrt() as i32;

    // If RMS is below threshold, consider it silent
    Ok(rms < threshold_i32)
}

// Parse a string like "0,1,2" into a vector of usize values
pub fn parse_channel_string_old(channels_str: &str) -> Vec<usize> {
    channels_str
        .split(',')
        .filter_map(|s| s.trim().parse::<usize>().ok())
        .collect()
}

// Check if an audio buffer is silent based on a threshold
pub fn is_silent_old(buffer: &[f32], threshold: f32) -> bool {
    if threshold <= 0.0 {
        return false;
    }

    buffer.iter().all(|&sample| sample.abs() < threshold)
}

// Check if ALSA is available on the system (Linux only)
#[cfg(target_os = "linux")]
pub fn check_alsa_availability_old() -> bool {
    // Check if ALSA library is available
    let alsa_check = Command::new("sh")
        .arg("-c")
        .arg("ldconfig -p | grep -q libasound")
        .status();

    match alsa_check {
        Ok(status) => status.success(),
        Err(_) => false,
    }
}

// For non-Linux systems, ALSA is not relevant
#[cfg(not(target_os = "linux"))]
pub fn check_alsa_availability_old() -> bool {
    true // Not applicable on non-Linux systems
}
